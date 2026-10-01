//! Composer's autoload files (`vendor/autoload.php` and the
//! `vendor/composer/autoload_*.php` family), written byte for byte the way
//! Composer 2.10 writes them.

mod autoloads;
mod classmap;
mod package;
mod paths;
mod platform;
pub mod scan;
mod sorter;
mod static_file;

use crate::autoloads::{Entry, key_string, parse_autoloads};
use crate::classmap::ClassMap;
use crate::package::Package;
use crate::paths::{Code, Dirs, find_shortest_path_code, real_path};
use crate::static_file::{Loader, static_file};
use phpm_lock::{CLASS_LOADER_PHP, COMPOSER_LICENSE, ComposerJson, Lock, normalize_path};
use phpm_php::{PhpKey, is_absolute_path, var_export_str};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::fmt::Write as _;
use std::hash::{BuildHasher, Hasher};
use std::path::Path;
use std::sync::Arc;

pub use crate::platform::PlatformRequirements;

/// Why the autoload files could not be generated the way Composer would.
#[derive(Debug)]
pub enum Error {
    Lock(phpm_lock::Error),
    InvalidPackage { package: String, reason: String },
    Constraint { package: String, reason: String },
    Scan { path: String, reason: String },
    Unsupported(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lock(e) => e.fmt(f),
            Self::InvalidPackage { package, reason } => write!(f, "package {package}: {reason}"),
            Self::Constraint { package, reason } => {
                write!(f, "package {package}: bad php constraint: {reason}")
            }
            Self::Scan { path, reason } => write!(f, "cannot scan {path}: {reason}"),
            Self::Unsupported(what) => write!(f, "not supported yet: {what}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lock(e) => Some(e),
            _ => None,
        }
    }
}

impl From<phpm_lock::Error> for Error {
    fn from(e: phpm_lock::Error) -> Self {
        Self::Lock(e)
    }
}

/// How to dump: the `install` / `dump-autoload` flags, already merged with
/// `config` by [`Options::with_config`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[expect(clippy::struct_excessive_bools, reason = "one per Composer flag")]
pub struct Options {
    pub dev_mode: bool,
    /// `-o`: scan PSR-0/PSR-4 directories into the class map.
    pub optimize: bool,
    pub classmap_authoritative: bool,
    pub apcu: bool,
    pub apcu_prefix: Option<String>,
    /// `--autoloader-suffix`-style override; `config.autoloader-suffix`
    /// and the existing `vendor/autoload.php` are consulted when `None`.
    pub suffix: Option<String>,
    pub platform: PlatformRequirements,
    /// [`scan::file_classes`] results the caller already has, keyed by real
    /// path; those files are not read again.
    pub known_classes: Option<Arc<HashMap<String, Vec<String>>>>,
}

/// PHP truthiness of a JSON config value.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null | Value::Bool(false)) => false,
        Some(Value::String(s)) => !(s.is_empty() || s == "0"),
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::Bool(true)) => true,
    }
}

fn config<'a>(composer_json: &'a ComposerJson, key: &str) -> Option<&'a Value> {
    composer_json
        .data()
        .get("config")
        .and_then(|c| c.get(key))
        .filter(|v| !v.is_null())
}

impl Options {
    /// ORs in `optimize-autoloader`, `classmap-authoritative` and
    /// `apcu-autoloader` from `config`, as `InstallCommand` does.
    #[must_use]
    pub fn with_config(mut self, composer_json: &ComposerJson) -> Self {
        self.optimize |= truthy(config(composer_json, "optimize-autoloader"));
        self.classmap_authoritative |= truthy(config(composer_json, "classmap-authoritative"));
        self.apcu |= self.apcu_prefix.is_some() || truthy(config(composer_json, "apcu-autoloader"));
        if self.classmap_authoritative {
            self.optimize = true;
        }
        self
    }
}

/// A project as `composer install` sees it after installing from the lock.
#[derive(Debug, Clone, Copy)]
pub struct Project<'a> {
    pub composer_json: &'a ComposerJson,
    pub lock: &'a Lock,
    /// `realpath(getcwd())`, forward slashes.
    pub root_dir: &'a str,
}

/// Every file Composer writes for the autoloader, as paths relative to the
/// vendor directory, plus the ones it deletes when they are not needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub vendor_dir: String,
    pub files: Vec<(String, Vec<u8>)>,
    pub remove: Vec<String>,
    pub suffix: String,
    /// Ambiguous class warnings, in Composer's wording.
    pub warnings: Vec<String>,
}

impl Output {
    /// Writes the files under [`Output::vendor_dir`], leaving unchanged
    /// files untouched like `filePutContentsIfModified`.
    pub fn write(&self) -> std::io::Result<()> {
        let vendor = Path::new(&self.vendor_dir);
        std::fs::create_dir_all(vendor.join("composer"))?;
        for (name, content) in &self.files {
            let path = vendor.join(name);
            if std::fs::read(&path).is_ok_and(|old| old == *content) {
                continue;
            }
            std::fs::write(&path, content)?;
        }
        for name in &self.remove {
            let path = vendor.join(name);
            if path.exists() {
                std::fs::remove_file(path)?;
            }
        }
        Ok(())
    }

    pub fn file(&self, name: &str) -> Option<&[u8]> {
        self.files
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, c)| c.as_slice())
    }
}

fn random_hex(bytes: usize) -> String {
    let mut out = String::with_capacity(bytes * 2);
    while out.len() < bytes * 2 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_usize(out.len());
        let _ = write!(out, "{:016x}", hasher.finish());
    }
    out.truncate(bytes * 2);
    out
}

fn is_hex(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

// Composer: Autoload/AutoloadGenerator.php dump, suffix block
fn suffix(project: &Project<'_>, options: &Options, vendor: &str) -> String {
    if let Some(s) = options.suffix.as_ref().filter(|s| !s.is_empty()) {
        return s.clone();
    }
    if let Some(Value::String(s)) = config(project.composer_json, "autoloader-suffix")
        && !s.is_empty()
    {
        return s.clone();
    }
    if let Ok(existing) = std::fs::read_to_string(format!("{vendor}/autoload.php"))
        && let Some(start) = existing.find("ComposerAutoloaderInit")
    {
        let rest = &existing[start + "ComposerAutoloaderInit".len()..];
        let end = rest
            .find(|c: char| c == ':' || c.is_ascii_whitespace())
            .unwrap_or(rest.len());
        if end > 0 && rest[end..].starts_with("::") {
            return rest[..end].to_owned();
        }
    }
    match project.lock.content_hash() {
        Some(hash) if is_hex(hash) => hash.to_owned(),
        _ => random_hex(16),
    }
}

fn header(name: &str, vendor_code: &str, base_code: &str) -> String {
    format!(
        "<?php\n\n// {name} @generated by Composer\n\n$vendorDir = {vendor_code};\n$baseDir = {base_code};\n\nreturn array(\n"
    )
}

fn export_key(key: &PhpKey) -> String {
    match key {
        PhpKey::Int(i) => i.to_string(),
        PhpKey::String(s) => var_export_str(s),
    }
}

// Composer: Autoload/AutoloadGenerator.php validatePackage
fn validate(package: &Package) -> Result<(), Error> {
    let Some(phpm_php::PhpValue::Array(psr4)) = package.autoload.get(&PhpKey::from("psr-4")) else {
        return Ok(());
    };
    if psr4.is_empty() {
        return Ok(());
    }
    let invalid = |reason: String| Error::InvalidPackage {
        package: package.name.clone(),
        reason,
    };
    if package.target_dir.is_some() {
        return Err(invalid(
            "PSR-4 autoloading is incompatible with the target-dir property".to_owned(),
        ));
    }
    for namespace in psr4.keys() {
        let namespace = key_string(namespace);
        if !namespace.is_empty() && !namespace.ends_with('\\') {
            return Err(invalid(format!(
                "psr-4 namespaces must end with a namespace separator, '{namespace}' does not"
            )));
        }
    }
    Ok(())
}

// Composer: Autoload/AutoloadGenerator.php filterPackageMap
fn reachable_from_root<'a>(root: &Package, packages: &[Entry<'a>]) -> Vec<Entry<'a>> {
    let by_name: HashMap<&str, &Package> = packages
        .iter()
        .map(|e| (e.package.name.as_str(), e.package))
        .collect();
    let mut replaced_by: HashMap<&str, &str> = HashMap::new();
    for e in packages {
        for r in &e.package.replaces {
            replaced_by.insert(r.target.as_str(), e.package.name.as_str());
        }
    }
    let mut include: HashSet<String> = HashSet::new();
    let mut stack: Vec<&Package> = vec![root];
    while let Some(package) = stack.pop() {
        for link in &package.requires {
            let target = replaced_by
                .get(link.target.as_str())
                .copied()
                .unwrap_or(link.target.as_str());
            if include.insert(target.to_owned())
                && let Some(p) = by_name.get(target)
            {
                stack.push(p);
            }
        }
    }
    packages
        .iter()
        .filter(|e| e.package.names().any(|n| include.contains(n)))
        .cloned()
        .collect()
}

fn installed_packages(lock: &Lock, dev_mode: bool) -> Result<Vec<Package>, Error> {
    let mut configs = lock.packages()?;
    if dev_mode {
        configs.extend(lock.packages_dev()?);
    }
    let mut packages: Vec<Package> = configs
        .into_iter()
        .map(|c| Package::from_config(c, ""))
        .collect();
    packages.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    Ok(packages)
}

/// Generates every autoload file `composer install` (or `dump-autoload`)
/// would write for `project`, reading the vendor directory for class
/// scanning and the existing autoloader suffix.
// Composer: Autoload/AutoloadGenerator.php dump
pub fn generate(project: &Project<'_>, options: &Options) -> Result<Output, Error> {
    let base = normalize_path(project.root_dir);
    let vendor_config = project.composer_json.vendor_dir();
    let vendor_dir = if is_absolute_path(&vendor_config) {
        vendor_config
    } else {
        format!("{base}/{vendor_config}")
    };
    let vendor = real_path(Path::new(&vendor_dir)).unwrap_or_else(|| normalize_path(&vendor_dir));
    let target = format!("{vendor}/composer");
    let dirs = Dirs {
        base: base.clone(),
        vendor: vendor.clone(),
    };

    let root = Package::from_config(project.composer_json.data(), "__root__");
    let packages = installed_packages(project.lock, options.dev_mode)?;
    let dev_names: BTreeSet<String> = project
        .lock
        .packages_dev()?
        .iter()
        .filter_map(|p| p.get("name").and_then(Value::as_str))
        .map(str::to_ascii_lowercase)
        .collect();

    let mut package_map: Vec<Entry<'_>> = vec![Entry {
        package: &root,
        install_path: Some(String::new()),
        is_root: true,
    }];
    for package in &packages {
        validate(package)?;
        let install_path = (package.kind != "metapackage").then(|| {
            let mut path = format!("{vendor}/{}", package.pretty_name);
            if let Some(t) = package
                .target_dir
                .as_deref()
                .filter(|t| !t.is_empty() && *t != "0")
            {
                path = format!("{path}/{t}");
            }
            path
        });
        package_map.push(Entry {
            package,
            install_path,
            is_root: false,
        });
    }

    let non_root: Vec<Entry<'_>> = package_map[1..].to_vec();
    let filtered = if options.dev_mode {
        non_root
    } else if dev_names.is_empty() {
        reachable_from_root(&root, &non_root)
    } else {
        non_root
            .into_iter()
            .filter(|e| !dev_names.contains(&e.package.name))
            .collect()
    };
    let refs: Vec<&Package> = filtered.iter().map(|e| e.package).collect();
    let mut sorted: Vec<Entry<'_>> = sorter::sort_packages(&refs)
        .into_iter()
        .map(|i| filtered[i].clone())
        .collect();
    sorted.push(package_map[0].clone());
    let autoloads = parse_autoloads(&sorted, options.dev_mode, &base);

    if root.target_dir.is_some()
        && matches!(root.autoload.get(&PhpKey::from("psr-0")), Some(phpm_php::PhpValue::Array(a)) if !a.is_empty())
    {
        return Err(Error::Unsupported(
            "a root package with target-dir and psr-0".into(),
        ));
    }

    let vendor_code = find_shortest_path_code(&target, &vendor, true, false);
    let vendor_to_target = find_shortest_path_code(&vendor, &target, true, false);
    let base_code = find_shortest_path_code(&vendor, &base, true, false);
    let base_code_str = base_code.code.replace("__DIR__", "$vendorDir");
    let runtime = Dirs {
        base: base_code.value,
        vendor: vendor.clone(),
    };
    let code = |path: &str| -> Code {
        let mut c = dirs.path_code(path);
        if c.code.starts_with("$baseDir") || c.code.starts_with("'phar://' . $baseDir") {
            c.value = c.value.replacen(&dirs.base, &runtime.base, 1);
        }
        c
    };

    let mut loader = Loader::default();
    let mut namespaces_file = header("autoload_namespaces.php", &vendor_code.code, &base_code_str);
    for (namespace, paths) in &autoloads.psr0 {
        let codes: Vec<Code> = paths.iter().map(|p| code(p)).collect();
        let joined: Vec<&str> = codes.iter().map(|c| c.code.as_str()).collect();
        let _ = writeln!(
            namespaces_file,
            "    {} => array({}),",
            export_key(namespace),
            joined.join(", ")
        );
        let values: Vec<String> = codes.into_iter().map(|c| c.value).collect();
        loader.set_psr0(&key_string(namespace), &values);
    }
    namespaces_file.push_str(");\n");

    let mut psr4_file = header("autoload_psr4.php", &vendor_code.code, &base_code_str);
    for (namespace, paths) in &autoloads.psr4 {
        let codes: Vec<Code> = paths.iter().map(|p| code(p)).collect();
        let joined: Vec<&str> = codes.iter().map(|c| c.code.as_str()).collect();
        let _ = writeln!(
            psr4_file,
            "    {} => array({}),",
            export_key(namespace),
            joined.join(", ")
        );
        let values: Vec<String> = codes.into_iter().map(|c| c.value).collect();
        loader.set_psr4(&key_string(namespace), &values);
    }
    psr4_file.push_str(");\n");

    let mut class_map = ClassMap::scan(
        &autoloads,
        options.optimize,
        &base,
        &vendor,
        options.known_classes.as_deref(),
    )?;
    class_map.add_class(
        "Composer\\InstalledVersions",
        &format!("{vendor}/composer/InstalledVersions.php"),
    );
    class_map.sort();
    let mut classmap_file = header("autoload_classmap.php", &vendor_code.code, &base_code_str);
    for (class, path) in class_map.entries() {
        let c = code(path);
        let _ = writeln!(
            classmap_file,
            "    {} => {},",
            var_export_str(class),
            c.code
        );
        loader.add_class(class, &c.value);
    }
    classmap_file.push_str(");\n");

    let suffix = suffix(project, options, &vendor);
    let mut files: Vec<(String, String)> = Vec::new();
    let mut remove: Vec<String> = Vec::new();
    files.push(("composer/autoload_namespaces.php".into(), namespaces_file));
    files.push(("composer/autoload_psr4.php".into(), psr4_file));
    files.push(("composer/autoload_classmap.php".into(), classmap_file));

    let mut include_paths: Vec<String> = Vec::new();
    for entry in &package_map {
        let Some(install) = &entry.install_path else {
            continue;
        };
        let mut install = install.clone();
        if let Some(t) = entry
            .package
            .target_dir
            .as_deref()
            .filter(|t| !t.is_empty())
        {
            install.truncate(install.len().saturating_sub(t.len() + 1));
        }
        for path in &entry.package.include_paths {
            let path = path.trim_matches('/');
            include_paths.push(if install.is_empty() {
                path.to_owned()
            } else {
                format!("{install}/{path}")
            });
        }
    }
    let has_include_paths = !include_paths.is_empty();
    if has_include_paths {
        let mut body = String::new();
        for path in &include_paths {
            let _ = writeln!(body, "    {},", code(path).code);
        }
        let mut file = header("include_paths.php", &vendor_code.code, &base_code_str);
        file.push_str(&body);
        file.push_str(");\n");
        files.push(("composer/include_paths.php".into(), file));
    } else {
        remove.push("composer/include_paths.php".into());
    }

    let file_codes: Vec<(String, Code)> = autoloads
        .files
        .iter()
        .map(|(id, path)| (id.clone(), code(path)))
        .collect();
    let has_files = !file_codes.is_empty();
    if has_files {
        let mut file = header("autoload_files.php", &vendor_code.code, &base_code_str);
        for (id, c) in &file_codes {
            let _ = writeln!(file, "    {} => {},", var_export_str(id), c.code);
        }
        file.push_str(");\n");
        files.push(("composer/autoload_files.php".into(), file));
        let values: Vec<(String, String)> = file_codes
            .into_iter()
            .map(|(id, c)| (id, c.value))
            .collect();
        loader.set_files(&values);
    } else {
        remove.push("composer/autoload_files.php".into());
    }

    files.push((
        "composer/autoload_static.php".into(),
        static_file(&suffix, loader, &target, &vendor, &runtime.base),
    ));

    let platform_config = config(project.composer_json, "platform-check");
    let mut platform_content = None;
    if platform_config != Some(&Value::Bool(false))
        && options.platform != PlatformRequirements::IgnoreAll
    {
        platform_content = platform::platform_check(
            &package_map,
            platform_config == Some(&Value::Bool(true)),
            &dev_names,
            &options.platform,
        )?;
    }
    let check_platform = platform_content.is_some();
    match platform_content {
        Some(content) => files.push(("composer/platform_check.php".into(), content)),
        None => remove.push("composer/platform_check.php".into()),
    }

    files.push((
        "autoload.php".into(),
        autoload_file(&vendor_to_target.code, &suffix),
    ));
    let real = RealFile {
        suffix: &suffix,
        include_paths: has_include_paths,
        include_files: has_files,
        use_global_include_path: truthy(config(project.composer_json, "use-include-path")),
        prepend: if config(project.composer_json, "prepend-autoloader") == Some(&Value::Bool(false))
        {
            "false"
        } else {
            "true"
        },
        check_platform,
        classmap_authoritative: options.classmap_authoritative,
        apcu_prefix: options.apcu.then(|| {
            options
                .apcu_prefix
                .clone()
                .unwrap_or_else(|| random_hex(10))
        }),
    };
    files.push(("composer/autoload_real.php".into(), real.render()));
    files.push((
        "composer/ClassLoader.php".into(),
        CLASS_LOADER_PHP.to_owned(),
    ));
    files.push(("composer/LICENSE".into(), COMPOSER_LICENSE.to_owned()));

    Ok(Output {
        vendor_dir: vendor,
        files: files
            .into_iter()
            .map(|(name, content)| (name, scan::raw_bytes(&content)))
            .collect(),
        remove,
        suffix,
        warnings: class_map.warnings(),
    })
}

// Composer: Autoload/AutoloadGenerator.php getAutoloadFile
fn autoload_file(vendor_to_target: &str, suffix: &str) -> String {
    let path_code = match vendor_to_target.chars().last() {
        Some(q @ ('\'' | '"')) => format!(
            "{}/autoload_real.php{q}",
            &vendor_to_target[..vendor_to_target.len() - 1]
        ),
        _ => format!("{vendor_to_target} . '/autoload_real.php'"),
    };
    format!(
        r#"<?php

// autoload.php @generated by Composer

if (PHP_VERSION_ID < 50600) {{
    if (!headers_sent()) {{
        header('HTTP/1.1 500 Internal Server Error');
    }}
    $err = 'Composer 2.3.0 dropped support for autoloading on PHP <5.6 and you are running '.PHP_VERSION.', please upgrade PHP or use Composer 2.2 LTS via "composer self-update --2.2". Aborting.'.PHP_EOL;
    if (!ini_get('display_errors')) {{
        if (PHP_SAPI === 'cli' || PHP_SAPI === 'phpdbg') {{
            fwrite(STDERR, $err);
        }} elseif (!headers_sent()) {{
            echo $err;
        }}
    }}
    throw new RuntimeException($err);
}}

require_once {path_code};

return ComposerAutoloaderInit{suffix}::getLoader();
"#
    )
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "one per autoload_real.php section"
)]
struct RealFile<'a> {
    suffix: &'a str,
    include_paths: bool,
    include_files: bool,
    use_global_include_path: bool,
    prepend: &'a str,
    check_platform: bool,
    classmap_authoritative: bool,
    apcu_prefix: Option<String>,
}

impl RealFile<'_> {
    // Composer: Autoload/AutoloadGenerator.php getAutoloadRealFile
    fn render(&self) -> String {
        let suffix = self.suffix;
        let prepend = self.prepend;
        let mut file = format!(
            r"<?php

// autoload_real.php @generated by Composer

class ComposerAutoloaderInit{suffix}
{{
    private static $loader;

    public static function loadClassLoader($class)
    {{
        if ('Composer\Autoload\ClassLoader' === $class) {{
            require __DIR__ . '/ClassLoader.php';
        }}
    }}

    /**
     * @return \Composer\Autoload\ClassLoader
     */
    public static function getLoader()
    {{
        if (null !== self::$loader) {{
            return self::$loader;
        }}

"
        );
        if self.check_platform {
            file.push_str("        require __DIR__ . '/platform_check.php';\n\n");
        }
        let _ = write!(
            file,
            r"        spl_autoload_register(array('ComposerAutoloaderInit{suffix}', 'loadClassLoader'), true, {prepend});
        self::$loader = $loader = new \Composer\Autoload\ClassLoader(\dirname(__DIR__));
        spl_autoload_unregister(array('ComposerAutoloaderInit{suffix}', 'loadClassLoader'));

"
        );
        if self.include_paths {
            file.push_str(
                "        $includePaths = require __DIR__ . '/include_paths.php';\n        $includePaths[] = get_include_path();\n        set_include_path(implode(PATH_SEPARATOR, $includePaths));\n\n",
            );
        }
        let _ = write!(
            file,
            "        require __DIR__ . '/autoload_static.php';\n        call_user_func(\\Composer\\Autoload\\ComposerStaticInit{suffix}::getInitializer($loader));\n\n"
        );
        if self.classmap_authoritative {
            file.push_str("        $loader->setClassMapAuthoritative(true);\n");
        }
        if let Some(prefix) = &self.apcu_prefix {
            let _ = writeln!(
                file,
                "        $loader->setApcuPrefix({});",
                var_export_str(prefix)
            );
        }
        if self.use_global_include_path {
            file.push_str("        $loader->setUseIncludePath(true);\n");
        }
        let _ = write!(file, "        $loader->register({prepend});\n\n");
        if self.include_files {
            let _ = write!(
                file,
                r"        $filesToLoad = \Composer\Autoload\ComposerStaticInit{suffix}::$files;
        $requireFile = \Closure::bind(static function ($fileIdentifier, $file) {{
            if (empty($GLOBALS['__composer_autoload_files'][$fileIdentifier])) {{
                $GLOBALS['__composer_autoload_files'][$fileIdentifier] = true;

                require $file;
            }}
        }}, null, null);
        foreach ($filesToLoad as $fileIdentifier => $file) {{
            $requireFile($fileIdentifier, $file);
        }}

"
            );
        }
        file.push_str("        return $loader;\n    }\n}\n");
        file
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, Options, PlatformRequirements, Project, RealFile, generate, random_hex};
    use phpm_lock::{ComposerJson, Lock};
    use serde_json::json;

    fn project_dir() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let root = crate::paths::real_path(dir.path()).unwrap();
        (dir, root)
    }

    fn run(
        composer: serde_json::Value,
        lock: serde_json::Value,
        options: &Options,
    ) -> Result<super::Output, Error> {
        let (_dir, root) = project_dir();
        let composer = ComposerJson::from_value(composer).unwrap();
        let lock = Lock::from_value(lock).unwrap();
        generate(
            &Project {
                composer_json: &composer,
                lock: &lock,
                root_dir: &root,
            },
            options,
        )
    }

    #[test]
    fn options_follow_config() {
        let composer = ComposerJson::from_value(
            json!({"config": {"classmap-authoritative": true, "apcu-autoloader": "1"}}),
        )
        .unwrap();
        let o = Options::default().with_config(&composer);
        assert!(o.optimize && o.classmap_authoritative && o.apcu);
        let plain = Options::default().with_config(&ComposerJson::from_value(json!({})).unwrap());
        assert!(!plain.optimize && !plain.apcu);
        let prefixed = Options {
            apcu_prefix: Some("p".into()),
            ..Options::default()
        }
        .with_config(
            &ComposerJson::from_value(json!({"config": {"optimize-autoloader": 0}})).unwrap(),
        );
        assert!(prefixed.apcu && !prefixed.optimize);
    }

    #[test]
    fn suffix_sources_in_composer_order() {
        let lock = json!({"content-hash": "abc", "packages": []});
        let out = run(json!({}), lock.clone(), &Options::default()).unwrap();
        assert_eq!(out.suffix, "abc");
        let out = run(
            json!({"config": {"autoloader-suffix": "Mine"}}),
            lock.clone(),
            &Options::default(),
        )
        .unwrap();
        assert_eq!(out.suffix, "Mine");
        let out = run(
            json!({}),
            lock,
            &Options {
                suffix: Some("Cli".into()),
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(out.suffix, "Cli");
        let out = run(
            json!({}),
            json!({"content-hash": "<<<< merge"}),
            &Options::default(),
        )
        .unwrap();
        assert_eq!(out.suffix.len(), 32);
        assert!(
            out.remove
                .contains(&"composer/platform_check.php".to_owned())
        );
        assert!(
            out.remove
                .contains(&"composer/autoload_files.php".to_owned())
        );
        assert!(
            out.remove
                .contains(&"composer/include_paths.php".to_owned())
        );
        assert_eq!(random_hex(10).len(), 20);
    }

    #[test]
    fn keeps_suffix_of_existing_autoloader() {
        let (dir, root) = project_dir();
        std::fs::create_dir(dir.path().join("vendor")).unwrap();
        std::fs::write(
            dir.path().join("vendor/autoload.php"),
            "return ComposerAutoloaderInitKeepMe::getLoader();",
        )
        .unwrap();
        let composer = ComposerJson::from_value(json!({})).unwrap();
        let lock = Lock::from_value(json!({"content-hash": "abc"})).unwrap();
        let project = Project {
            composer_json: &composer,
            lock: &lock,
            root_dir: &root,
        };
        assert_eq!(
            generate(&project, &Options::default()).unwrap().suffix,
            "KeepMe"
        );
    }

    #[test]
    fn rejects_what_composer_rejects() {
        let bad_ns = json!({"packages": [{"name": "a/b", "version": "1.0", "autoload": {"psr-4": {"A": "src"}}}]});
        assert!(matches!(
            run(json!({}), bad_ns, &Options::default()),
            Err(Error::InvalidPackage { .. })
        ));
        let target = json!({"packages": [{"name": "a/b", "version": "1.0", "target-dir": "x", "autoload": {"psr-4": {"A\\": "src"}}}]});
        assert!(run(json!({}), target, &Options::default()).is_err());
        let root_target = json!({"target-dir": "x", "autoload": {"psr-0": {"A": "src"}}});
        assert!(matches!(
            run(root_target, json!({}), &Options::default()),
            Err(Error::Unsupported(_))
        ));
        let scan = json!({"packages": [{"name": "a/b", "version": "1.0", "autoload": {"classmap": ["src"]}}]});
        let dev = Options {
            dev_mode: true,
            ..Options::default()
        };
        assert!(matches!(
            run(json!({}), scan, &dev),
            Err(Error::Scan { .. })
        ));
        let bad_php =
            json!({"packages": [{"name": "a/b", "version": "1.0", "require": {"php": "~>8"}}]});
        let e = run(json!({}), bad_php, &Options::default()).unwrap_err();
        assert!(e.to_string().contains("a/b"));
        assert!(run(json!({}), json!({"packages": [1]}), &Options::default()).is_err());
    }

    #[test]
    fn no_dev_without_dev_names_keeps_what_root_reaches() {
        let lock = json!({"packages": [
            {"name": "a/used", "version": "1.0", "require": {"c/inner": "*"}, "autoload": {"psr-4": {"U\\": "src"}}},
            {"name": "c/real", "version": "1.0", "replace": {"c/inner": "*"}, "autoload": {"psr-4": {"C\\": "src"}}},
            {"name": "b/stray", "version": "1.0", "autoload": {"psr-4": {"S\\": "src"}}}
        ]});
        let out = run(
            json!({"require": {"a/used": "*"}}),
            lock,
            &Options::default(),
        )
        .unwrap();
        let psr4 =
            String::from_utf8(out.file("composer/autoload_psr4.php").unwrap().to_vec()).unwrap();
        assert!(psr4.contains("'U\\\\'") && psr4.contains("'C\\\\'"));
        assert!(!psr4.contains("'S\\\\'"));
    }

    #[test]
    fn platform_check_can_be_disabled() {
        let lock =
            json!({"packages": [{"name": "a/b", "version": "1.0", "require": {"php": ">=8"}}]});
        let out = run(json!({}), lock.clone(), &Options::default()).unwrap();
        assert!(out.file("composer/platform_check.php").is_some());
        let off = run(
            json!({"config": {"platform-check": false}}),
            lock.clone(),
            &Options::default(),
        )
        .unwrap();
        assert!(off.file("composer/platform_check.php").is_none());
        let ignored = Options {
            platform: PlatformRequirements::IgnoreAll,
            ..Options::default()
        };
        assert!(
            run(json!({}), lock, &ignored)
                .unwrap()
                .file("composer/platform_check.php")
                .is_none()
        );
    }

    #[test]
    fn real_file_flags() {
        let real = RealFile {
            suffix: "S",
            include_paths: false,
            include_files: false,
            use_global_include_path: false,
            prepend: "true",
            check_platform: false,
            classmap_authoritative: true,
            apcu_prefix: Some("pre".into()),
        }
        .render();
        assert!(real.contains(
            "getInitializer($loader));\n\n        $loader->setClassMapAuthoritative(true);\n        $loader->setApcuPrefix('pre');\n        $loader->register(true);\n\n        return $loader;\n    }\n}\n"
        ));
        assert!(!real.contains("platform_check"));
    }

    #[test]
    fn errors_read_well() {
        let e = Error::Scan {
            path: "x".into(),
            reason: "y".into(),
        };
        assert_eq!(e.to_string(), "cannot scan x: y");
        assert!(std::error::Error::source(&e).is_none());
        let lock = Error::from(phpm_lock::Error::NotAnObject("composer.lock"));
        assert!(std::error::Error::source(&lock).is_some());
        assert_eq!(lock.to_string(), "composer.lock is not a JSON object");
        assert!(Error::Unsupported("z".into()).to_string().contains('z'));
        assert!(
            Error::InvalidPackage {
                package: "p".into(),
                reason: "r".into()
            }
            .to_string()
            .contains("p: r")
        );
    }
}
