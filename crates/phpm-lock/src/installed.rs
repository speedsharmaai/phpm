use crate::error::Error;
use crate::manifest::{ComposerJson, Lock};
use crate::package::{Package, branch_alias, pretty_alias};
use crate::paths::{find_shortest_path, normalize_path};
use crate::root::RootVersion;
use crate::version::DEFAULT_BRANCH_ALIAS;
use indexmap::IndexMap;
use phpm_php::{
    PhpArray, PhpKey, PhpValue, dump_to_php_code, encode_pretty, is_absolute_path, smart_strcmp,
    strnatcmp,
};
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::sync::LazyLock;

/// Composer's `vendor/composer/InstalledVersions.php`, copied verbatim.
pub const INSTALLED_VERSIONS_PHP: &str = include_str!("../composer/InstalledVersions.php");

/// Composer's `vendor/composer/ClassLoader.php`, copied verbatim.
pub const CLASS_LOADER_PHP: &str = include_str!("../composer/ClassLoader.php");

/// The `vendor/composer/LICENSE` Composer writes next to them.
pub const COMPOSER_LICENSE: &str = include_str!("../composer/LICENSE");

/// The Composer release the vendored files and the output format match.
pub const COMPOSER_VERSION: &str = "2.10.3";

static PLATFORM_PACKAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i-u)^(?:php(?:-64bit|-ipv6|-zts|-debug)?|hhvm|(?:ext|lib)-[a-z0-9](?:[_.-]?[a-z0-9]+)*|composer(?:-(?:plugin|runtime)-api)?)\z",
    )
    .expect("valid pattern")
});

/// What an install from a lock file needs to reproduce Composer's
/// `vendor/composer/installed.json` and `installed.php`.
#[derive(Debug, Clone, Copy)]
pub struct InstallContext<'a> {
    pub composer_json: &'a ComposerJson,
    pub lock: &'a Lock,
    pub root_version: &'a RootVersion,
    /// The project directory as an absolute path with symlinks resolved,
    /// the way `realpath(getcwd())` sees it.
    pub root_dir: &'a str,
    pub dev_mode: bool,
}

/// The two files Composer writes into `vendor/composer/` after an install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledFiles {
    pub installed_json: String,
    pub installed_php: String,
}

struct Installed {
    package: Package,
    installation_source: Option<&'static str>,
    install_path: Option<String>,
}

impl Installed {
    // Composer: Repository/FilesystemRepository.php dumpInstalledPackage
    fn reference(&self) -> Option<String> {
        let p = &self.package;
        let chosen = match self.installation_source {
            Some("source") => p.source_reference.clone(),
            Some(_) => p.dist_reference.clone(),
            None => None,
        };
        chosen.or_else(|| {
            truthy(p.source_reference.as_deref()).or_else(|| truthy(p.dist_reference.as_deref()))
        })
    }
}

fn truthy(s: Option<&str>) -> Option<String> {
    s.filter(|s| !s.is_empty() && *s != "0").map(str::to_owned)
}

/// An alias package as the installed repository holds it.
struct Alias<'a> {
    of: &'a Package,
    pretty_version: String,
}

fn pairs(links: &[(String, String)]) -> Vec<(&str, String)> {
    links.iter().map(|(t, c)| (t.as_str(), c.clone())).collect()
}

// Composer: Package/AliasPackage.php replaceSelfVersionDependencies
fn with_self_version<'a>(links: &'a [(String, String)], pretty: &str) -> Vec<(&'a str, String)> {
    let mut out = pairs(links);
    out.extend(
        links
            .iter()
            .filter(|(_, c)| c == "self.version")
            .map(|(t, _)| (t.as_str(), pretty.to_owned())),
    );
    out
}

fn alias_links<'a>(links: &'a [(String, String)], alias: &Alias<'_>) -> Vec<(&'a str, String)> {
    if alias.pretty_version == DEFAULT_BRANCH_ALIAS {
        with_self_version(links, &alias.of.pretty_version)
    } else {
        with_self_version(links, &alias.pretty_version)
    }
}

fn installation_source(ctx: &InstallContext<'_>, package: &Package) -> Option<&'static str> {
    if package.kind == "metapackage" {
        return None;
    }
    let has_source = package
        .source_type
        .as_deref()
        .is_some_and(|t| !t.is_empty() && t != "0");
    let has_dist = package
        .dist_type
        .as_deref()
        .is_some_and(|t| !t.is_empty() && t != "0");
    match (has_source, has_dist) {
        (true, true) => {
            if ctx
                .composer_json
                .install_preferences()
                .prefers_dist(&package.name, package.is_dev())
            {
                Some("dist")
            } else {
                Some("source")
            }
        }
        (true, false) => Some("source"),
        (false, true) => Some("dist"),
        (false, false) => None,
    }
}

fn push_list(target: &mut PhpArray, key: &str, value: String) {
    let slot = target
        .entry(PhpKey::from(key))
        .or_insert_with(|| PhpValue::Array(PhpArray::new()));
    if let PhpValue::Array(list) = slot {
        let exists = list.values().any(|v| *v == PhpValue::String(value.clone()));
        if !exists {
            let next = i64::try_from(list.len()).unwrap_or(i64::MAX);
            list.insert(PhpKey::Int(next), PhpValue::String(value));
        }
    }
}

fn package_array(
    pretty_version: &str,
    version: &str,
    reference: Option<String>,
    kind: &str,
    install_path: Option<String>,
    dev: bool,
) -> PhpArray {
    let mut a = PhpArray::new();
    a.insert("pretty_version".into(), pretty_version.into());
    a.insert("version".into(), version.into());
    a.insert("reference".into(), reference.into());
    a.insert("type".into(), kind.into());
    a.insert("install_path".into(), install_path.into());
    a.insert("aliases".into(), PhpArray::new().into());
    a.insert("dev_requirement".into(), dev.into());
    a
}

/// Builds `installed.json` and `installed.php` exactly as Composer 2.10's
/// `FilesystemRepository::write` does after `composer install` from a lock.
// Composer: Repository/FilesystemRepository.php write, generateInstalledVersions
pub fn installed_files(ctx: &InstallContext<'_>) -> Result<InstalledFiles, Error> {
    let root_dir = normalize_path(ctx.root_dir);
    let vendor = ctx.composer_json.vendor_dir();
    let vendor_dir = normalize_path(&if is_absolute_path(&vendor) {
        vendor
    } else {
        format!("{root_dir}/{vendor}")
    });
    let repo_dir = format!("{vendor_dir}/composer");

    let dev_names: BTreeSet<String> = ctx
        .lock
        .packages_dev()?
        .iter()
        .filter_map(|p| p.get("name").and_then(Value::as_str))
        .map(str::to_ascii_lowercase)
        .collect();

    let mut locked = ctx.lock.packages()?;
    if ctx.dev_mode {
        locked.extend(ctx.lock.packages_dev()?);
    }
    let mut installed: Vec<Installed> = Vec::with_capacity(locked.len());
    for config in locked {
        let package = Package::from_lock(config)?;
        let install_path = if package.kind == "metapackage" {
            None
        } else {
            let mut path = format!("{vendor_dir}/{}", package.pretty_name);
            if let Some(target) = package
                .target_dir
                .as_deref()
                .filter(|t| !t.is_empty() && *t != "0")
            {
                path = format!("{path}/{target}");
            }
            find_shortest_path(&repo_dir, &normalize_path(&path), true)
        };
        let installation_source = installation_source(ctx, &package);
        installed.push(Installed {
            package,
            installation_source,
            install_path,
        });
    }

    let installed_json = installed_json(&installed, &dev_names, ctx.dev_mode);
    let installed_php = installed_php(ctx, &installed, &dev_names, &root_dir, &repo_dir);
    Ok(InstalledFiles {
        installed_json,
        installed_php,
    })
}

fn installed_json(installed: &[Installed], dev_names: &BTreeSet<String>, dev_mode: bool) -> String {
    let mut packages: Vec<Map<String, Value>> = installed
        .iter()
        .map(|i| {
            let mut dumped = i.package.dump(i.installation_source);
            dumped.insert(
                "install-path".into(),
                i.install_path.clone().map_or(Value::Null, Value::String),
            );
            dumped
        })
        .collect();
    packages.sort_by(|a, b| {
        let name = |m: &Map<String, Value>| {
            m.get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .as_bytes()
                .to_vec()
        };
        name(a).cmp(&name(b))
    });
    let mut dev_package_names: Vec<String> = installed
        .iter()
        .map(|i| i.package.name.clone())
        .filter(|n| dev_names.contains(n))
        .collect();
    dev_package_names.sort_by(|a, b| smart_strcmp(a, b));

    let mut data = Map::new();
    data.insert(
        "packages".into(),
        Value::Array(packages.into_iter().map(Value::Object).collect()),
    );
    data.insert("dev".into(), Value::Bool(dev_mode));
    data.insert(
        "dev-package-names".into(),
        Value::Array(dev_package_names.into_iter().map(Value::String).collect()),
    );
    let mut out = encode_pretty(&Value::Object(data));
    out.push('\n');
    out
}

fn installed_php(
    ctx: &InstallContext<'_>,
    installed: &[Installed],
    dev_names: &BTreeSet<String>,
    root_dir: &str,
    repo_dir: &str,
) -> String {
    let config = ctx.composer_json.data();
    let root_name = config
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("__root__")
        .to_ascii_lowercase();
    let root_type = config
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("library")
        .to_ascii_lowercase();
    let root_links = |link_type: &str| -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        if let Some(Value::Object(map)) = config.get(link_type) {
            for (target, constraint) in map {
                let Value::String(c) = constraint else {
                    continue;
                };
                let target = target.to_ascii_lowercase();
                match out.iter_mut().find(|(t, _)| *t == target) {
                    Some(slot) => slot.1.clone_from(c),
                    None => out.push((target, c.clone())),
                }
            }
        }
        out
    };
    let root_replaces = root_links("replace");
    let root_provides = root_links("provide");
    let root_version = ctx.root_version;
    let root_install_path = find_shortest_path(repo_dir, root_dir, true);

    let mut aliases: Vec<Alias<'_>> = Vec::new();
    for i in installed {
        if let Some(normalized) = &i.package.branch_alias {
            aliases.push(Alias {
                of: &i.package,
                pretty_version: pretty_alias(normalized),
            });
        }
    }
    for lock_alias in ctx.lock.aliases() {
        if let Some(i) = installed
            .iter()
            .find(|i| i.package.name == lock_alias.package)
        {
            aliases.push(Alias {
                of: &i.package,
                pretty_version: lock_alias.alias.clone(),
            });
        }
    }

    let root_alias = branch_alias(config, &root_version.pretty).map(|n| pretty_alias(&n));

    let mut root = PhpArray::new();
    root.insert("name".into(), root_name.as_str().into());
    root.insert("pretty_version".into(), root_version.pretty.as_str().into());
    root.insert("version".into(), root_version.normalized.as_str().into());
    root.insert(
        "reference".into(),
        truthy(root_version.reference.as_deref()).into(),
    );
    root.insert("type".into(), root_type.as_str().into());
    root.insert("install_path".into(), root_install_path.clone().into());
    root.insert("aliases".into(), PhpArray::new().into());
    root.insert("dev".into(), ctx.dev_mode.into());

    let mut versions: IndexMap<String, PhpArray> = IndexMap::new();
    for i in installed {
        let p = &i.package;
        let array = package_array(
            &p.pretty_version,
            &p.version,
            i.reference(),
            &p.kind,
            i.install_path.clone(),
            dev_names.contains(&p.name),
        );
        versions.insert(p.name.clone(), array);
    }
    let root_array = package_array(
        &root_version.pretty,
        &root_version.normalized,
        truthy(root_version.reference.as_deref()),
        &root_type,
        root_install_path,
        dev_names.contains(&root_name),
    );
    versions.insert(root_name.clone(), root_array);

    let mut record = |is_dev: bool, key: &str, links: Vec<(&str, String)>, self_version: &str| {
        for (target, constraint) in links {
            if PLATFORM_PACKAGE.is_match(target) {
                continue;
            }
            let e = versions.entry(target.to_owned()).or_default();
            match e.get(&PhpKey::from("dev_requirement")) {
                None => {
                    e.insert("dev_requirement".into(), is_dev.into());
                }
                Some(_) if !is_dev => {
                    e.insert("dev_requirement".into(), false.into());
                }
                Some(_) => {}
            }
            let value = if constraint == "self.version" {
                self_version.to_owned()
            } else {
                constraint
            };
            push_list(e, key, value);
        }
    };

    for i in installed {
        let p = &i.package;
        let is_dev = dev_names.contains(&p.name);
        record(is_dev, "replaced", pairs(&p.replaces), &p.pretty_version);
        record(is_dev, "provided", pairs(&p.provides), &p.pretty_version);
    }
    for alias in &aliases {
        let is_dev = dev_names.contains(&alias.of.name);
        record(
            is_dev,
            "replaced",
            alias_links(&alias.of.replaces, alias),
            &alias.pretty_version,
        );
        record(
            is_dev,
            "provided",
            alias_links(&alias.of.provides, alias),
            &alias.pretty_version,
        );
    }
    let root_is_dev = dev_names.contains(&root_name);
    if let Some(pretty) = &root_alias {
        let self_pretty = if pretty == DEFAULT_BRANCH_ALIAS {
            root_version.pretty.as_str()
        } else {
            pretty.as_str()
        };
        record(
            root_is_dev,
            "replaced",
            with_self_version(&root_replaces, self_pretty),
            pretty,
        );
        record(
            root_is_dev,
            "provided",
            with_self_version(&root_provides, self_pretty),
            pretty,
        );
    }
    record(
        root_is_dev,
        "replaced",
        pairs(&root_replaces),
        &root_version.pretty,
    );
    record(
        root_is_dev,
        "provided",
        pairs(&root_provides),
        &root_version.pretty,
    );

    for alias in &aliases {
        push_list(
            versions.entry(alias.of.name.clone()).or_default(),
            "aliases",
            alias.pretty_version.clone(),
        );
    }
    if let Some(pretty) = &root_alias {
        push_list(
            versions.entry(root_name).or_default(),
            "aliases",
            pretty.clone(),
        );
        if let Some(PhpValue::Array(list)) = root.get_mut(&PhpKey::from("aliases")) {
            let next = i64::try_from(list.len()).unwrap_or(i64::MAX);
            list.insert(PhpKey::Int(next), pretty.as_str().into());
        }
    }

    versions.sort_by(|a, _, b, _| smart_strcmp(a, b));
    for package in versions.values_mut() {
        for key in ["aliases", "replaced", "provided"] {
            if let Some(PhpValue::Array(list)) = package.get_mut(&PhpKey::from(key)) {
                let mut items: Vec<PhpValue> = list.values().cloned().collect();
                items.sort_by(|a, b| match (a, b) {
                    (PhpValue::String(a), PhpValue::String(b)) => strnatcmp(a, b),
                    _ => std::cmp::Ordering::Equal,
                });
                *list = items
                    .into_iter()
                    .enumerate()
                    .map(|(i, v)| (PhpKey::Int(i64::try_from(i).unwrap_or(i64::MAX)), v))
                    .collect();
            }
        }
    }

    let mut data = PhpArray::new();
    data.insert("root".into(), root.into());
    let versions: PhpArray = versions
        .into_iter()
        .map(|(k, v)| (PhpKey::from(k), PhpValue::Array(v)))
        .collect();
    data.insert("versions".into(), versions.into());
    format!("<?php return {};\n", dump_to_php_code(&data))
}

#[cfg(test)]
mod tests {
    use super::{InstallContext, installed_files};
    use crate::manifest::{ComposerJson, Lock};
    use crate::root::RootVersion;
    use serde_json::json;

    fn run(
        composer: serde_json::Value,
        lock: serde_json::Value,
        dev: bool,
        version: &RootVersion,
    ) -> (String, String) {
        let composer = ComposerJson::from_value(composer).unwrap();
        let lock = Lock::from_value(lock).unwrap();
        let ctx = InstallContext {
            composer_json: &composer,
            lock: &lock,
            root_version: version,
            root_dir: "/p",
            dev_mode: dev,
        };
        let files = installed_files(&ctx).unwrap();
        (files.installed_json, files.installed_php)
    }

    fn default_version() -> RootVersion {
        RootVersion {
            pretty: "1.0.0+no-version-set".into(),
            normalized: "1.0.0.0".into(),
            reference: None,
        }
    }

    #[test]
    fn empty_install() {
        let (json, php) = run(
            json!({}),
            json!({"packages": [], "packages-dev": []}),
            false,
            &default_version(),
        );
        assert_eq!(
            json,
            "{\n    \"packages\": [],\n    \"dev\": false,\n    \"dev-package-names\": []\n}\n"
        );
        let expected = "<?php return array(
    'root' => array(
        'name' => '__root__',
        'pretty_version' => '1.0.0+no-version-set',
        'version' => '1.0.0.0',
        'reference' => null,
        'type' => 'library',
        'install_path' => __DIR__ . '/../../',
        'aliases' => array(),
        'dev' => false,
    ),
    'versions' => array(
        '__root__' => array(
            'pretty_version' => '1.0.0+no-version-set',
            'version' => '1.0.0.0',
            'reference' => null,
            'type' => 'library',
            'install_path' => __DIR__ . '/../../',
            'aliases' => array(),
            'dev_requirement' => false,
        ),
    ),
);
";
        assert_eq!(php, expected);
    }

    #[test]
    fn aliases_replacements_and_provides() {
        let lock = json!({
            "packages": [
                {"name": "a/lib", "version": "dev-master", "default-branch": true,
                 "source": {"type": "git", "url": "u", "reference": "s1"},
                 "replace": {"a/old": "self.version", "php": "*"}, "provide": {"psr/x-impl": "1.0"}},
                {"name": "b/lib", "version": "dev-main", "extra": {"branch-alias": {"dev-main": "2.x-dev"}},
                 "dist": {"type": "zip", "url": "d", "reference": "d1"}, "provide": {"psr/x-impl": "2.0"}},
                {"name": "c/meta", "version": "1.0.0", "type": "metapackage", "dist": {"type": "zip", "url": "d", "reference": "c1"}}
            ],
            "packages-dev": [
                {"name": "d/dev", "version": "1.0.0", "target-dir": "D/X", "dist": {"type": "zip", "url": "d", "reference": "d2"},
                 "provide": {"psr/y-impl": "1.0"}, "replace": {"psr/x-impl": "3.0"}}
            ],
            "aliases": [{"package": "b/lib", "version": "dev-main", "alias": "1.5.0", "alias_normalized": "1.5.0.0"},
                        {"package": "missing/pkg", "version": "1", "alias": "2", "alias_normalized": "2.0.0.0"}]
        });
        let composer = json!({"name": "Acme/App", "type": "Project", "extra": {"branch-alias": {"dev-main": "3.x-dev"}},
            "replace": {"acme/old": "self.version"}, "provide": {"ext-foo": "*"}});
        let version = RootVersion {
            pretty: "dev-main".into(),
            normalized: "dev-main".into(),
            reference: Some("abc".into()),
        };
        let (json, php) = run(composer, lock, true, &version);

        let decoded: serde_json::Value = serde_json::from_str(&json).unwrap();
        let names: Vec<&str> = decoded["packages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["a/lib", "b/lib", "c/meta", "d/dev"]);
        assert_eq!(decoded["dev-package-names"], json!(["d/dev"]));
        assert_eq!(decoded["packages"][0]["installation-source"], "source");
        assert_eq!(decoded["packages"][1]["installation-source"], "dist");
        assert!(decoded["packages"][2].get("installation-source").is_none());
        assert_eq!(
            decoded["packages"][2]["install-path"],
            serde_json::Value::Null
        );
        assert_eq!(decoded["packages"][3]["install-path"], "../d/dev/D/X");

        for needle in [
            "'a/old' => array(\n            'dev_requirement' => false,\n            'replaced' => array(\n                0 => '9999999-dev',\n                1 => 'dev-master',\n            ),",
            "'aliases' => array(\n                0 => '9999999-dev',\n            ),",
            "'aliases' => array(\n                0 => '1.5.0',\n                1 => '2.x-dev',\n            ),",
            "'psr/x-impl' => array(\n            'dev_requirement' => false,\n            'provided' => array(\n                0 => '1.0',\n                1 => '2.0',\n            ),\n            'replaced' => array(\n                0 => '3.0',\n            ),",
            "'psr/y-impl' => array(\n            'dev_requirement' => true,",
            "'acme/old' => array(\n            'dev_requirement' => false,\n            'replaced' => array(\n                0 => '3.x-dev',\n                1 => 'dev-main',\n            ),",
            "'name' => 'acme/app',\n        'pretty_version' => 'dev-main',\n        'version' => 'dev-main',\n        'reference' => 'abc',\n        'type' => 'project',",
            "'aliases' => array(\n            0 => '3.x-dev',\n        ),\n        'dev' => true,",
            "'install_path' => null,",
            "'reference' => 'c1',",
        ] {
            assert!(php.contains(needle), "missing:\n{needle}\n---\n{php}");
        }
        assert!(!php.contains("'php'"));
        assert!(!php.contains("ext-foo"));
    }

    #[test]
    fn preferred_install_source() {
        let lock = json!({"packages": [{"name": "a/b", "version": "1.0.0",
            "source": {"type": "git", "url": "u", "reference": "src"},
            "dist": {"type": "zip", "url": "d", "reference": "dst"}}]});
        let (json, php) = run(
            json!({"config": {"preferred-install": "source"}}),
            lock,
            false,
            &default_version(),
        );
        assert!(json.contains("\"installation-source\": \"source\""));
        assert!(php.contains("'reference' => 'src',"));
    }

    #[test]
    fn custom_vendor_dir_moves_paths() {
        let lock = json!({"packages": [{"name": "a/b", "version": "1.0.0", "dist": {"type": "zip", "url": "d", "reference": "r"}}]});
        let (json, php) = run(
            json!({"config": {"vendor-dir": "lib/vendor"}}),
            lock,
            false,
            &default_version(),
        );
        assert!(json.contains("\"install-path\": \"../a/b\""));
        assert!(php.contains("'install_path' => __DIR__ . '/../../../',"));
    }

    #[test]
    fn bad_lock_entries_are_errors() {
        let composer = ComposerJson::parse("{}").unwrap();
        let lock = Lock::parse(r#"{"packages":[{"name":"a/b"}]}"#).unwrap();
        let version = default_version();
        let ctx = InstallContext {
            composer_json: &composer,
            lock: &lock,
            root_version: &version,
            root_dir: "/p",
            dev_mode: false,
        };
        assert!(installed_files(&ctx).is_err());
    }
}
