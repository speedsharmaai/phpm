//! `phpm install`: composer.json + composer.lock to a `vendor/` identical to
//! Composer's.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use phpm_autoload::PlatformRequirements;
use phpm_lock::{ComposerJson, INSTALLED_VERSIONS_PHP, InstallContext, Lock, normalize_path};
use phpm_store::{Auth, Dist, FetchOptions, Fetcher, LinkMode, Package, Store, place};
use serde_json::{Map, Value};

use crate::bins::{BinInstaller, php_basename, php_dirname};
use crate::error::Error;
use crate::fsutil::{Modes, path_string, write_if_changed};
use crate::out::Out;
use crate::project::{Env, ProjectFiles, dirs, locate, with_vendor_dir};
use crate::state::{Inputs, State, git_fingerprint, state_path};

/// What the command line asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[expect(clippy::struct_excessive_bools, reason = "one per Composer flag")]
pub(crate) struct Request {
    pub(crate) working_dir: Option<PathBuf>,
    pub(crate) dev: bool,
    pub(crate) link_mode: Option<LinkMode>,
    pub(crate) optimize: bool,
    pub(crate) classmap_authoritative: bool,
    pub(crate) no_autoloader: bool,
    pub(crate) no_scripts: bool,
    pub(crate) no_plugins: bool,
    pub(crate) ignore_platform_reqs: bool,
    pub(crate) ignore_platform_req: Vec<String>,
}

impl Request {
    /// The flags that change what ends up in `vendor/`, for the state digest.
    fn flags(&self) -> String {
        format!(
            "dev={} link={:?} o={} a={} no-autoloader={} no-scripts={} no-plugins={} ignore-all={} ignore={:?}",
            self.dev,
            self.link_mode,
            self.optimize,
            self.classmap_authoritative,
            self.no_autoloader,
            self.no_scripts,
            self.no_plugins,
            self.ignore_platform_reqs,
            self.ignore_platform_req,
        )
    }

    fn platform(&self) -> PlatformRequirements {
        if self.ignore_platform_reqs {
            PlatformRequirements::IgnoreAll
        } else if self.ignore_platform_req.is_empty() {
            PlatformRequirements::Check
        } else {
            PlatformRequirements::Ignore(self.ignore_platform_req.clone())
        }
    }
}

/// Environment variables that change phpm's output.
const ENV_INPUTS: [&str; 6] = [
    "COMPOSER",
    "COMPOSER_VENDOR_DIR",
    "COMPOSER_BIN_DIR",
    "COMPOSER_BIN_COMPAT",
    "COMPOSER_ROOT_VERSION",
    "HOME",
];

fn read_input(path: &Path, missing: impl FnOnce() -> Error) -> Result<Vec<u8>, Error> {
    fs::read(path).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            missing()
        } else {
            Error::io(path, &e)
        }
    })
}

pub(crate) fn run(req: &Request, env: Env<'_>, out: &mut Out<'_>) -> Result<(), Error> {
    let started = Instant::now();
    let cwd = match &req.working_dir {
        Some(dir) => dir.clone(),
        None => std::env::current_dir().map_err(|e| Error::usage(e.to_string()))?,
    };
    let root = fs::canonicalize(&cwd)
        .map_err(|e| Error::usage(format!("working directory {}: {e}", cwd.display())))?;
    let files = locate(root, env);
    let json = read_input(&files.composer_file, || {
        Error::usage(format!("{} not found", files.composer_file.display()))
    })?;
    let lock = read_input(&files.lock_file, || {
        Error::install(format!(
            "{} not found; phpm installs from a lock file only, so run composer update first",
            files.lock_file.display()
        ))
    })?;

    let mut inputs = Inputs::default();
    inputs
        .add("phpm", env!("CARGO_PKG_VERSION").as_bytes())
        .add("root", files.root.to_string_lossy().as_bytes())
        .add("flags", req.flags().as_bytes())
        .add("json", &json)
        .add("lock", &lock)
        .add("git", git_fingerprint(&files.root).as_bytes());
    for key in ENV_INPUTS {
        inputs.add(key, env(key).as_deref().unwrap_or("\0").as_bytes());
    }
    let inputs = inputs.finish();
    let state_file = phpm_store::cache_dir()
        .ok()
        .map(|cache| state_path(&cache, &files.root));
    if let Some(file) = &state_file
        && State::load(file).is_some_and(|s| s.is_current(&inputs))
    {
        out.info("Nothing to install, update or remove");
        out.detail(&format!(
            "vendor/ matches {} ({:.1?})",
            files.lock_file.display(),
            started.elapsed()
        ));
        return Ok(());
    }

    let written = install(req, env, &files, &json, &lock, out)?;
    if let Some(file) = state_file
        && let Some(state) = State::capture(inputs, &written)
        && let Err(e) = state.save(&file)
    {
        out.detail(&format!("could not save {}: {e}", file.display()));
    }
    out.detail(&format!("done in {:.1?}", started.elapsed()));
    Ok(())
}

fn utf8(bytes: &[u8], path: &Path) -> Result<String, Error> {
    String::from_utf8(bytes.to_vec())
        .map_err(|_| Error::install(format!("{} is not valid UTF-8", path.display())))
}

// Composer: Script/ScriptEvents.php, Installer/PackageEvents.php, Installer/InstallerEvents.php
const INSTALL_EVENTS: [&str; 15] = [
    "pre-install-cmd",
    "post-install-cmd",
    "pre-autoload-dump",
    "post-autoload-dump",
    "pre-package-install",
    "post-package-install",
    "pre-package-update",
    "post-package-update",
    "pre-package-uninstall",
    "post-package-uninstall",
    "pre-operations-exec",
    "pre-file-download",
    "post-file-download",
    "pre-command-run",
    "init",
];

const FALLBACK_LATER: &str =
    "phpm cannot run them yet; Phase 02 adds a fallback to Composer for this";

fn guard(
    req: &Request,
    composer: &ComposerJson,
    packages: &[&Map<String, Value>],
) -> Result<(), Error> {
    if !req.no_plugins {
        let plugins: Vec<&str> = packages
            .iter()
            .filter(|p| {
                p.get("type").and_then(Value::as_str).is_some_and(|t| {
                    t.eq_ignore_ascii_case("composer-plugin")
                        || t.eq_ignore_ascii_case("composer-installer")
                })
            })
            .filter_map(|p| p.get("name").and_then(Value::as_str))
            .collect();
        if !plugins.is_empty() {
            return Err(Error::install(format!(
                "the lock file has Composer plugins ({}) and {FALLBACK_LATER}. \
                 Run with --no-plugins to install without them, as composer install --no-plugins does",
                plugins.join(", ")
            )));
        }
    }
    if !req.no_scripts {
        let scripts = composer.data().get("scripts").and_then(Value::as_object);
        let events: Vec<&str> = INSTALL_EVENTS
            .iter()
            .copied()
            .filter(|e| {
                scripts.and_then(|s| s.get(*e)).is_some_and(|v| match v {
                    Value::String(s) => !s.is_empty(),
                    Value::Array(a) => !a.is_empty(),
                    Value::Null | Value::Bool(false) => false,
                    _ => true,
                })
            })
            .collect();
        if !events.is_empty() {
            return Err(Error::install(format!(
                "composer.json has scripts for {} and {FALLBACK_LATER}. \
                 Run with --no-scripts to install without them",
                events.join(", ")
            )));
        }
    }
    Ok(())
}

/// One locked package as the install needs it.
#[derive(Debug, Clone)]
struct Locked {
    name: String,
    version: String,
    /// `None` for metapackages, which have nothing to place.
    package: Option<Package>,
    bins: Vec<String>,
}

fn text(map: &Map<String, Value>, key: &str) -> Option<String> {
    map.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn non_empty_text(map: Option<&Value>, key: &str) -> Option<String> {
    map.and_then(|m| m.get(key))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

// Composer: Package/Loader/ArrayLoader.php bin handling
fn bins(entry: &Map<String, Value>) -> Vec<String> {
    let list = match entry.get("bin") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    };
    list.into_iter()
        .map(|b| b.trim_start_matches('/').to_owned())
        .collect()
}

fn locked(entry: &Map<String, Value>, composer: &ComposerJson) -> Result<Locked, Error> {
    let name = text(entry, "name")
        .ok_or_else(|| Error::install("composer.lock has a package without a name"))?;
    let version = text(entry, "version").unwrap_or_default();
    let kind = text(entry, "type").unwrap_or_else(|| "library".to_owned());
    if kind.eq_ignore_ascii_case("metapackage") {
        return Ok(Locked {
            name,
            version,
            package: None,
            bins: Vec::new(),
        });
    }
    let dist = entry.get("dist");
    let dist_kind = non_empty_text(dist, "type");
    let has_source = non_empty_text(entry.get("source"), "type").is_some();
    let is_dev = version.starts_with("dev-") || version.ends_with("-dev");
    let wants_source = has_source
        && (dist_kind.is_none() || !composer.install_preferences().prefers_dist(&name, is_dev));
    let Some(kind) = dist_kind.filter(|_| !wants_source) else {
        return Err(Error::install(format!(
            "{name} installs from source, and phpm only installs dist archives so far"
        )));
    };
    let url = non_empty_text(dist, "url")
        .ok_or_else(|| Error::install(format!("{name} has a dist without a url")))?;
    let mut install_path = name.clone();
    if let Some(target) = text(entry, "target-dir").filter(|t| !t.is_empty() && t != "0") {
        install_path = format!("{install_path}/{target}");
    }
    let mut package = Package::new(
        name.clone(),
        Dist {
            kind,
            url,
            reference: non_empty_text(dist, "reference"),
            shasum: non_empty_text(dist, "shasum"),
        },
    );
    package.install_path = PathBuf::from(install_path);
    Ok(Locked {
        name,
        version,
        bins: bins(entry),
        package: Some(package),
    })
}

/// A package `vendor/composer/installed.json` says is installed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Previous {
    version: String,
    reference: Option<String>,
    source: Option<String>,
    install_path: Option<String>,
    bins: Vec<String>,
}

fn previous(vendor: &str) -> BTreeMap<String, Previous> {
    let repo = format!("{vendor}/composer");
    let Ok(bytes) = fs::read(format!("{repo}/installed.json")) else {
        return BTreeMap::new();
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return BTreeMap::new();
    };
    let list = match &value {
        Value::Object(m) => m.get("packages").and_then(Value::as_array),
        Value::Array(a) => Some(a),
        _ => None,
    };
    list.into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter_map(|p| {
            let name = text(p, "name")?;
            let install_path =
                text(p, "install-path").map(|rel| normalize_path(&format!("{repo}/{rel}")));
            Some((
                name,
                Previous {
                    version: text(p, "version").unwrap_or_default(),
                    reference: non_empty_text(p.get("dist"), "reference"),
                    source: text(p, "installation-source"),
                    install_path,
                    bins: bins(p),
                },
            ))
        })
        .collect()
}

fn abs_install_path(vendor: &str, package: &Package) -> String {
    normalize_path(&format!("{vendor}/{}", path_string(&package.install_path)))
}

fn unchanged(prev: Option<&Previous>, l: &Locked, vendor: &str) -> bool {
    let (Some(prev), Some(package)) = (prev, &l.package) else {
        return false;
    };
    let path = abs_install_path(vendor, package);
    prev.version == l.version
        && prev.reference == package.dist.reference
        && prev.source.as_deref() == Some("dist")
        && prev.install_path.as_deref() == Some(path.as_str())
        && Path::new(&path).is_dir()
}

// Composer: Installer/LibraryInstaller.php uninstall
fn remove_package(vendor: &str, name: &str, path: &str) -> Result<bool, Error> {
    if !path.starts_with(&format!("{vendor}/")) {
        return Ok(false);
    }
    let dir = Path::new(path);
    let removed = match fs::symlink_metadata(dir) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(dir).map(|()| true),
        Ok(_) => fs::remove_file(dir).map(|()| true),
        Err(_) => Ok(false),
    }
    .map_err(|e| Error::io(dir, &e))?;
    if name.contains('/') {
        let parent = php_dirname(path);
        if fs::read_dir(&parent).is_ok_and(|mut d| d.next().is_none()) {
            let _ = fs::remove_dir(&parent);
        }
    }
    Ok(removed)
}

fn runtime() -> Result<tokio::runtime::Runtime, Error> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::install(format!("cannot start the download runtime: {e}")))
}

fn write_file(path: &Path, content: &[u8]) -> Result<(), Error> {
    write_if_changed(path, content).map_err(|e| Error::io(path, &e))
}

/// Runs the install and returns the files whose stamps make the next run a no-op.
fn install(
    req: &Request,
    env: Env<'_>,
    files: &ProjectFiles,
    json: &[u8],
    lock: &[u8],
    out: &mut Out<'_>,
) -> Result<Vec<PathBuf>, Error> {
    let composer = ComposerJson::parse(&utf8(json, &files.composer_file)?)?;
    let lock = Lock::parse(&utf8(lock, &files.lock_file)?)?;
    let mut entries = lock.packages()?;
    if req.dev {
        entries.extend(lock.packages_dev()?);
    }
    guard(req, &composer, &entries)?;
    let dirs = dirs(&composer, &files.root, env)?;
    let composer = with_vendor_dir(composer, &files.root, &dirs.vendor);
    let vendor = normalize_path(&dirs.vendor);
    let root_dir = path_string(&files.root);

    let locked = entries
        .iter()
        .map(|e| locked(e, &composer))
        .collect::<Result<Vec<_>, _>>()?;
    let before = previous(&vendor);
    let to_place: Vec<&Package> = locked
        .iter()
        .filter(|l| !unchanged(before.get(&l.name), l, &vendor))
        .filter_map(|l| l.package.as_ref())
        .collect();

    let started = Instant::now();
    let store = Store::from_env()?;
    let missing: Vec<Package> = to_place
        .iter()
        .filter(|p| !store.contains(&p.name, &p.key()))
        .map(|p| (*p).clone())
        .collect();
    if !missing.is_empty() {
        let fetcher = Fetcher::new(FetchOptions {
            auth: Auth::load(Some(&files.root))?,
            ..FetchOptions::default()
        })?;
        let fetched = runtime()?.block_on(store.fetch_missing(&fetcher, &missing))?;
        out.info(&format!(
            "Downloaded {} packages in {:.2?}",
            fetched.len(),
            started.elapsed()
        ));
    }

    let wanted: BTreeSet<String> = locked
        .iter()
        .filter_map(|l| l.package.as_ref())
        .map(|p| abs_install_path(&vendor, p))
        .collect();
    let mut removed = 0;
    for (name, prev) in &before {
        if let Some(path) = &prev.install_path
            && !wanted.contains(path)
            && remove_package(&vendor, name, path)?
        {
            removed += 1;
        }
    }

    let started = Instant::now();
    let placements = to_place
        .iter()
        .map(|p| store.placement(p))
        .collect::<Result<Vec<_>, _>>()?;
    let mode = req.link_mode.unwrap_or_else(LinkMode::platform_default);
    let used = place(Path::new(&vendor), &placements, mode)?;
    if used != mode {
        out.detail(&format!("{mode:?} is not available here, used {used:?}"));
    }
    out.detail(&format!(
        "placed {} packages ({used:?}) in {:.1?}",
        placements.len(),
        started.elapsed()
    ));

    let repo = PathBuf::from(format!("{vendor}/composer"));
    fs::create_dir_all(&repo).map_err(|e| Error::io(&repo, &e))?;
    let root_version = phpm_lock::root_version(
        composer.data(),
        &files.root,
        env("COMPOSER_ROOT_VERSION").as_deref(),
    )?;
    let installed = phpm_lock::installed_files(&InstallContext {
        composer_json: &composer,
        lock: &lock,
        root_version: &root_version,
        root_dir: &root_dir,
        dev_mode: req.dev,
    })?;
    let mut written = vec![
        repo.join("installed.json"),
        repo.join("installed.php"),
        repo.join("InstalledVersions.php"),
    ];
    write_file(&written[0], installed.installed_json.as_bytes())?;
    write_file(&written[1], installed.installed_php.as_bytes())?;
    write_file(&written[2], INSTALLED_VERSIONS_PHP.as_bytes())?;

    let vendor_real =
        fs::canonicalize(&vendor).map_or_else(|_| vendor.clone(), |p| path_string(&p));
    let modes = Modes::probe(&repo).map_err(|e| Error::io(&repo, &e))?;
    let mut bins = BinInstaller::new(
        PathBuf::from(&dirs.bin),
        vendor.clone(),
        vendor_real,
        dirs.full_bin_compat,
        modes,
    );
    for l in &locked {
        if let Some(p) = &l.package {
            bins.install(&l.name, &abs_install_path(&vendor, p), &l.bins)
                .map_err(|e| Error::install(format!("bin proxies for {}: {e}", l.name)))?;
        }
    }
    let stale: Vec<&str> = before
        .values()
        .flat_map(|p| p.bins.iter().map(|b| php_basename(b)))
        .collect();
    bins.remove_stale(stale)
        .map_err(|e| Error::install(format!("removing old bin proxies: {e}")))?;
    for w in &bins.warnings {
        out.warn(w);
    }
    written.extend(bins.written().iter().map(|n| Path::new(&dirs.bin).join(n)));

    if !req.no_autoloader {
        let started = Instant::now();
        let options = phpm_autoload::Options {
            dev_mode: req.dev,
            optimize: req.optimize,
            classmap_authoritative: req.classmap_authoritative,
            platform: req.platform(),
            ..phpm_autoload::Options::default()
        }
        .with_config(&composer);
        let autoload = phpm_autoload::generate(
            &phpm_autoload::Project {
                composer_json: &composer,
                lock: &lock,
                root_dir: &root_dir,
            },
            &options,
        )?;
        autoload
            .write()
            .map_err(|e| Error::install(format!("writing the autoloader: {e}")))?;
        for w in &autoload.warnings {
            out.warn(w);
        }
        written.extend(
            autoload
                .files
                .iter()
                .map(|(name, _)| Path::new(&autoload.vendor_dir).join(name)),
        );
        out.detail(&format!(
            "wrote the autoloader in {:.1?}",
            started.elapsed()
        ));
    }
    written.extend(wanted.iter().map(PathBuf::from));

    out.info(&summary(placements.len(), removed));
    Ok(written)
}

fn packages(n: usize) -> String {
    if n == 1 {
        "1 package".to_owned()
    } else {
        format!("{n} packages")
    }
}

fn summary(placed: usize, removed: usize) -> String {
    match (placed, removed) {
        (0, 0) => "Nothing to install, update or remove".to_owned(),
        (n, 0) => format!("Installed {}", packages(n)),
        (0, r) => format!("Removed {}", packages(r)),
        (n, r) => format!("Installed {}, removed {r}", packages(n)),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Previous, Request, bins, guard, locked, previous, remove_package, summary, unchanged,
    };
    use phpm_autoload::PlatformRequirements;
    use phpm_lock::ComposerJson;
    use serde_json::{Map, Value, json};
    use std::fs;

    fn obj(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => unreachable!(),
        }
    }

    fn composer(v: Value) -> ComposerJson {
        ComposerJson::from_value(v).unwrap()
    }

    fn zip_entry(name: &str) -> Map<String, Value> {
        obj(json!({
            "name": name,
            "version": "1.0.0",
            "dist": {"type": "zip", "url": "https://x/a.zip", "reference": "abc", "shasum": ""},
            "source": {"type": "git", "url": "https://x/a.git", "reference": "abc"},
            "bin": ["/bin/tool", "other"],
        }))
    }

    #[test]
    fn refuses_plugins_and_scripts_unless_told_not_to_run_them() {
        let plugin = obj(json!({"name": "a/plugin", "type": "composer-plugin"}));
        let lib = obj(json!({"name": "a/lib"}));
        let plain = composer(json!({}));
        let req = Request::default();
        let err = guard(&req, &plain, &[&lib, &plugin]).unwrap_err();
        assert!(err.message.contains("a/plugin"), "{err}");
        assert!(err.message.contains("--no-plugins"));
        assert_eq!(err.code, 1);
        let no_plugins = Request {
            no_plugins: true,
            ..Request::default()
        };
        assert!(guard(&no_plugins, &plain, &[&lib, &plugin]).is_ok());

        let scripted = composer(json!({"scripts": {
            "post-autoload-dump": ["@php artisan package:discover"],
            "post-update-cmd": "x",
            "pre-install-cmd": [],
            "post-install-cmd": null,
        }}));
        let err = guard(&req, &scripted, &[&lib]).unwrap_err();
        assert!(err.message.contains("post-autoload-dump"), "{err}");
        assert!(!err.message.contains("post-update-cmd"));
        assert!(!err.message.contains("pre-install-cmd"));
        let no_scripts = Request {
            no_scripts: true,
            ..Request::default()
        };
        assert!(guard(&no_scripts, &scripted, &[&lib]).is_ok());
        let update_only = composer(json!({"scripts": {"post-update-cmd": "x", "test": "y"}}));
        assert!(guard(&req, &update_only, &[&lib]).is_ok());
    }

    #[test]
    fn reads_locked_packages() {
        let c = composer(json!({}));
        let l = locked(&zip_entry("a/b"), &c).unwrap();
        let p = l.package.unwrap();
        assert_eq!(p.dist.reference.as_deref(), Some("abc"));
        assert_eq!(p.dist.shasum, None);
        assert_eq!(p.install_path, std::path::Path::new("a/b"));
        assert_eq!(l.bins, ["bin/tool", "other"]);

        let mut target = zip_entry("a/b");
        target.insert("target-dir".into(), json!("Sub/Dir"));
        let p = locked(&target, &c).unwrap().package.unwrap();
        assert_eq!(p.install_path, std::path::Path::new("a/b/Sub/Dir"));

        let meta = obj(json!({"name": "a/m", "type": "metapackage"}));
        assert!(locked(&meta, &c).unwrap().package.is_none());

        let source_only =
            obj(json!({"name": "a/s", "version": "1.0.0", "source": {"type": "git"}}));
        assert!(
            locked(&source_only, &c)
                .unwrap_err()
                .message
                .contains("source")
        );
        let prefers_source = composer(json!({"config": {"preferred-install": "source"}}));
        assert!(locked(&zip_entry("a/b"), &prefers_source).is_err());
        let no_url = obj(json!({"name": "a/u", "dist": {"type": "zip"}}));
        assert!(locked(&no_url, &c).unwrap_err().message.contains("url"));
        let nameless = obj(json!({"version": "1"}));
        assert!(locked(&nameless, &c).is_err());
    }

    #[test]
    fn normalises_bin_lists() {
        assert_eq!(bins(&obj(json!({"bin": "/x"}))), ["x"]);
        assert!(bins(&obj(json!({"bin": 1}))).is_empty());
    }

    #[test]
    fn reads_the_previous_install() {
        let tmp = tempfile::tempdir().unwrap();
        let vendor = tmp.path().to_string_lossy().replace('\\', "/");
        assert!(previous(&vendor).is_empty());
        fs::create_dir_all(tmp.path().join("composer")).unwrap();
        fs::write(tmp.path().join("composer/installed.json"), b"nope").unwrap();
        assert!(previous(&vendor).is_empty());
        fs::write(
            tmp.path().join("composer/installed.json"),
            json!({"packages": [
                {"name": "a/b", "version": "1.0.0", "dist": {"reference": "abc"},
                 "installation-source": "dist", "install-path": "../a/b", "bin": ["bin/tool"]},
                {"name": "a/m", "version": "1.0.0", "install-path": null},
            ]})
            .to_string(),
        )
        .unwrap();
        let before = previous(&vendor);
        assert_eq!(
            before["a/b"],
            Previous {
                version: "1.0.0".into(),
                reference: Some("abc".into()),
                source: Some("dist".into()),
                install_path: Some(format!("{vendor}/a/b")),
                bins: vec!["bin/tool".into()],
            }
        );
        assert_eq!(before["a/m"].install_path, None);

        let c = composer(json!({}));
        let l = locked(&zip_entry("a/b"), &c).unwrap();
        assert!(!unchanged(before.get("a/b"), &l, &vendor));
        fs::create_dir_all(tmp.path().join("a/b")).unwrap();
        assert!(unchanged(before.get("a/b"), &l, &vendor));
        assert!(!unchanged(None, &l, &vendor));

        fs::write(
            tmp.path().join("composer/installed.json"),
            b"[{\"name\":\"x/y\"}]",
        )
        .unwrap();
        assert!(previous(&vendor).contains_key("x/y"));
    }

    #[test]
    fn removes_packages_and_empty_vendor_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let vendor = tmp.path().to_string_lossy().replace('\\', "/");
        fs::create_dir_all(tmp.path().join("a/b/src")).unwrap();
        fs::create_dir_all(tmp.path().join("a/c")).unwrap();
        assert!(remove_package(&vendor, "a/b", &format!("{vendor}/a/b")).unwrap());
        assert!(tmp.path().join("a").exists());
        assert!(remove_package(&vendor, "a/c", &format!("{vendor}/a/c")).unwrap());
        assert!(!tmp.path().join("a").exists());
        assert!(!remove_package(&vendor, "a/c", &format!("{vendor}/a/c")).unwrap());
        assert!(!remove_package(&vendor, "x/y", "/etc").unwrap());
        fs::write(tmp.path().join("file"), b"").unwrap();
        assert!(remove_package(&vendor, "file", &format!("{vendor}/file")).unwrap());
    }

    #[test]
    fn summarises_in_one_line() {
        assert_eq!(summary(0, 0), "Nothing to install, update or remove");
        assert_eq!(summary(1, 0), "Installed 1 package");
        assert_eq!(summary(0, 2), "Removed 2 packages");
        assert_eq!(summary(3, 1), "Installed 3 packages, removed 1");
    }

    #[test]
    fn maps_platform_flags() {
        let mut r = Request::default();
        assert_eq!(r.platform(), PlatformRequirements::Check);
        r.ignore_platform_req = vec!["ext-*".into()];
        assert_eq!(
            r.platform(),
            PlatformRequirements::Ignore(vec!["ext-*".into()])
        );
        r.ignore_platform_reqs = true;
        assert_eq!(r.platform(), PlatformRequirements::IgnoreAll);
        assert_ne!(Request::default().flags(), r.flags());
    }
}
