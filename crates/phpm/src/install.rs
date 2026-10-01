//! `phpm install`: composer.json + composer.lock to a `vendor/` identical to
//! Composer's.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use phpm_autoload::PlatformRequirements;
use phpm_lock::{ComposerJson, INSTALLED_VERSIONS_PHP, InstallContext, Lock, normalize_path};
use phpm_store::{Auth, Dist, FetchOptions, Fetcher, LinkMode, Package, Placement, Store, place};
use serde_json::{Map, Value};

use crate::bins::{BinInstaller, php_basename, php_dirname};
use crate::classes::{Tree, cache_root, known_classes};
use crate::error::Error;
use crate::exec::{find_composer, find_php};
use crate::fallback::{self, Composer, Plan, Step};
use crate::fsutil::{Modes, path_string, write_if_changed};
use crate::notify::{self, Download};
use crate::out::Out;
use crate::pathrepo::{self, PathDist};
use crate::platform::{Filter, Platform, Requirements};
use crate::plugins::{self, Global, Plugins};
use crate::policy::{
    self, Abandonment, AuditFormat, Audited, Locked as PolicyLocked, Pending, Policy,
};
use crate::prefetch;
use crate::project::{Env, ProjectFiles, dirs, locate, with_vendor_dir};
use crate::runner::{self, Runner};
use crate::scripts::{self, Scripts};
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
    pub(crate) explain: bool,
    /// `--audit`, with its format.
    pub(crate) audit: Option<AuditFormat>,
    /// `--no-blocking` / `--no-security-blocking`.
    pub(crate) no_blocking: bool,
}

impl Request {
    /// The flags that change what ends up in `vendor/`, for the state digest.
    fn flags(&self) -> String {
        format!(
            "dev={} link={:?} o={} a={} no-autoloader={} no-scripts={} no-plugins={} ignore-all={} ignore={:?} no-blocking={}",
            self.dev,
            self.link_mode,
            self.optimize,
            self.classmap_authoritative,
            self.no_autoloader,
            self.no_scripts,
            self.no_plugins,
            self.ignore_platform_reqs,
            self.ignore_platform_req,
            self.no_blocking,
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
const ENV_INPUTS: [&str; 11] = [
    "COMPOSER",
    "COMPOSER_POLICY",
    "COMPOSER_POLICY_MALWARE_BLOCK",
    "COMPOSER_NO_BLOCKING",
    "COMPOSER_NO_SECURITY_BLOCKING",
    "COMPOSER_MIRROR_PATH_REPOS",
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
    if !req.ignore_platform_reqs {
        let php = crate::platform::find_php(env);
        inputs.add(
            "php",
            crate::platform::fingerprint(php.as_deref(), env).as_bytes(),
        );
    }
    let inputs = inputs.finish();
    let state_file = phpm_store::cache_dir()
        .ok()
        .map(|cache| state_path(&cache, &files.root));
    if let Some(file) = &state_file
        && req.audit.is_none()
        && State::load(file).is_some_and(|s| s.is_current(&inputs))
    {
        if req.explain {
            out.info("decision install: nothing to do, composer.json, composer.lock and vendor/ are as phpm left them");
        }
        out.info("Nothing to install, update or remove");
        out.detail(&format!(
            "vendor/ matches {} ({:.1?})",
            files.lock_file.display(),
            started.elapsed()
        ));
        return Ok(());
    }

    let done = install(req, env, &files, &json, &lock, out)?;
    if let Some(file) = state_file
        && let Some(written) = &done.written
        && let Some(mut state) = State::capture(inputs, written)
    {
        state.expires = done.expires;
        if let Err(e) = state.save(&file) {
            out.detail(&format!("could not save {}: {e}", file.display()));
        }
    }
    if done.audit_failed {
        return Err(Error::new(5, ""));
    }
    out.detail(&format!("done in {:.1?}", started.elapsed()));
    Ok(())
}

fn utf8(bytes: &[u8], path: &Path) -> Result<String, Error> {
    String::from_utf8(bytes.to_vec())
        .map_err(|_| Error::install(format!("{} is not valid UTF-8", path.display())))
}

/// One locked package as the install needs it.
#[derive(Debug, Clone)]
struct Locked {
    name: String,
    version: String,
    /// `None` for metapackages, which have nothing to place.
    package: Option<Package>,
    /// Set for `path` repository packages, which skip the store.
    path: Option<PathDist>,
    bins: Vec<String>,
    plugin: bool,
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
    let plugin = kind.eq_ignore_ascii_case("composer-plugin")
        || kind.eq_ignore_ascii_case("composer-installer");
    if kind.eq_ignore_ascii_case("metapackage") {
        return Ok(Locked {
            name,
            version,
            package: None,
            path: None,
            bins: Vec::new(),
            plugin,
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
    let path = (package.dist.kind == "path").then(|| PathDist::from_lock(&package.dist.url, entry));
    Ok(Locked {
        name,
        version,
        bins: bins(entry),
        package: Some(package),
        path,
        plugin,
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

/// Artifact dists are files, relative to the project like Composer's cwd.
fn local_dist_url(url: &str, root: &Path) -> String {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || url.contains("://") {
        return url.to_owned();
    }
    let path = Path::new(url);
    if path.is_absolute() {
        url.to_owned()
    } else {
        path_string(&root.join(path))
    }
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

/// Class scan caches for the packages placed this run; packages left in
/// place may have been edited in `vendor/`, so the autoloader reads those.
fn class_trees(store: &Store, placements: &[Placement], vendor_real: &str) -> Vec<Tree> {
    let Ok(cache) = phpm_store::cache_dir() else {
        return Vec::new();
    };
    let root = cache_root(&cache);
    placements
        .iter()
        .filter_map(|p| {
            let rel = p.source.strip_prefix(store.root()).ok()?;
            Some(Tree {
                store_dir: p.source.clone(),
                cache_file: root.join(rel),
                vendor_dir: format!("{vendor_real}/{}", path_string(&p.install_path)),
            })
        })
        .collect()
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

/// One line per package for `--explain`.
fn package_line(l: &Locked, state: &str, plan: &Plan, no_plugins: bool) -> String {
    let mut line = format!("package {} {}: {state}", l.name, l.version);
    if let Some(p) = plan.plugins.active.iter().find(|p| p.name == l.name) {
        line.push_str(
            if plan.full_install.is_some() || p.needs_full_install.is_some() {
                "; plugin, loaded by composer install"
            } else {
                "; plugin, Composer loads it for the steps it hooks"
            },
        );
    } else if let Some(s) = plan.plugins.skipped.iter().find(|s| s.name == l.name) {
        line.push_str("; plugin, not loaded: ");
        line.push_str(s.why);
    } else if no_plugins && l.plugin {
        line.push_str("; plugin, not loaded: --no-plugins");
    }
    line
}

/// Regular files directly in `dir`, for stamping what Composer rewrote.
fn top_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .collect();
    files.sort();
    files
}

/// Runs the install and returns the files whose stamps make the next run a
/// no-op, or `None` when Composer ran the whole install.
/// The network side of an install, made on first use: most no-change
/// installs never need it.
struct Net {
    fetcher: Fetcher,
    runtime: tokio::runtime::Runtime,
}

fn net<'a>(
    slot: &'a mut Option<Net>,
    root: &Path,
    config: Option<&Map<String, Value>>,
) -> Result<&'a Net, Error> {
    if slot.is_none() {
        *slot = Some(Net {
            fetcher: Fetcher::new(FetchOptions {
                auth: Auth::load(Some(root), config)?,
                ..FetchOptions::default()
            })?,
            runtime: runtime()?,
        });
    }
    Ok(slot.as_ref().expect("the slot was filled just above"))
}

/// The policy view of a lock entry.
fn policy_locked(entry: &Map<String, Value>) -> PolicyLocked {
    let pretty_name = text(entry, "name").unwrap_or_default();
    let pretty = text(entry, "version").unwrap_or_default();
    PolicyLocked {
        name: pretty_name.to_ascii_lowercase(),
        version: phpm_lock::version::normalize(&pretty).unwrap_or_else(|_| pretty.clone()),
        pretty_name,
        pretty,
    }
}

// Composer: Installer.php run, "Package %s is abandoned"
fn abandoned(entry: &Map<String, Value>) -> Option<Abandonment> {
    match entry.get("abandoned") {
        Some(Value::String(s)) if !s.is_empty() => Some(Abandonment(Some(s.clone()))),
        Some(Value::Bool(true) | Value::String(_)) => Some(Abandonment(None)),
        _ => None,
    }
}

/// `notify-on-install` from the project's config, else from
/// `COMPOSER_HOME/config.json`, read only when the project leaves it out.
// Composer: Factory::createConfig, the home config merged under the project's
fn notify_on_install(
    project: Option<&Map<String, Value>>,
    home: impl FnOnce() -> Option<Map<String, Value>>,
) -> bool {
    let flag = |c: &Map<String, Value>| c.get("notify-on-install").and_then(Value::as_bool);
    project.and_then(flag).unwrap_or_else(|| {
        home()
            .as_ref()
            .and_then(|h| h.get("config"))
            .and_then(Value::as_object)
            .and_then(flag)
            .unwrap_or(true)
    })
}

/// Send download notifications for what this run installed, from a detached
/// copy of phpm (or a thread of its own when that cannot start), so the POST
/// never holds the install up.
// Composer: Installer.php run, config notify-on-install
#[expect(
    clippy::too_many_arguments,
    reason = "everything the install already has in hand"
)]
fn notify_installs(
    entries: &[&Map<String, Value>],
    locked: &[Locked],
    changed: &[&Locked],
    before: &BTreeMap<String, Previous>,
    sizes: &BTreeMap<String, u64>,
    files: &ProjectFiles,
    config: Option<&Map<String, Value>>,
    env: Env<'_>,
) -> Option<std::thread::JoinHandle<()>> {
    let home_config = || {
        plugins::composer_home(env, plugins::system_uses_xdg(), &|d| d.is_dir())
            .and_then(|home| plugins::read_object(&home.join("config.json")))
    };
    let wanted = notify_on_install(config, home_config)
        && env("COMPOSER_DISABLE_NETWORK").is_none_or(|v| v.is_empty() || v == "0");
    if !wanted {
        return None;
    }
    let installed = |l: &Locked| match &l.package {
        Some(_) => changed.iter().any(|c| c.name == l.name),
        None => before.get(&l.name).is_none_or(|p| p.version != l.version),
    };
    let downloads: Vec<Download> = entries
        .iter()
        .zip(locked)
        .filter(|(_, l)| installed(l))
        .filter_map(|(e, l)| Download::from_lock(e, sizes.get(&l.name).copied()))
        .collect();
    if downloads.is_empty() {
        return None;
    }
    let batches = notify::batches(&downloads);
    if notify::detach(&files.root, config, &batches) {
        return None;
    }
    let root = files.root.clone();
    let config = config.cloned();
    Some(std::thread::spawn(move || {
        notify::send_blocking(&root, config.as_ref(), batches);
    }))
}

/// What an install leaves for the state file and the exit code.
#[derive(Debug)]
struct Installed {
    /// `None` when Composer ran the whole install.
    written: Option<Vec<PathBuf>>,
    expires: Option<u64>,
    audit_failed: bool,
}

fn install(
    req: &Request,
    env: Env<'_>,
    files: &ProjectFiles,
    json: &[u8],
    lock: &[u8],
    out: &mut Out<'_>,
) -> Result<Installed, Error> {
    let composer = ComposerJson::parse(&utf8(json, &files.composer_file)?)?;
    let lock = Lock::parse(&utf8(lock, &files.lock_file)?)?;
    let mut entries = lock.packages()?;
    if req.dev {
        entries.extend(lock.packages_dev()?);
    }
    let config = composer
        .data()
        .get("config")
        .and_then(Value::as_object)
        .cloned();
    let policy = Policy::from_config(config.as_ref(), env, req.no_blocking)?;
    let repos = policy::repos(&composer);
    let cache_dir = phpm_store::cache_dir().ok();
    let mut filter = Pending::start(
        &policy,
        &repos,
        entries.iter().map(|e| policy_locked(e)).collect(),
        || Ok(Auth::load(Some(&files.root), config.as_ref())?),
        cache_dir.clone(),
    )?;
    check_platform(req, env, &composer, &lock, out)?;
    let mut network: Option<Net> = None;
    let dirs = dirs(&composer, &files.root, env)?;
    let composer = with_vendor_dir(composer, &files.root, &dirs.vendor);
    let vendor = normalize_path(&dirs.vendor);
    let root_dir = path_string(&files.root);

    let locked = entries
        .iter()
        .map(|e| locked(e, &composer))
        .collect::<Result<Vec<_>, _>>()?;
    let before = previous(&vendor);
    let changed: Vec<&Locked> = locked
        .iter()
        .filter(|l| !unchanged(before.get(&l.name), l, &vendor))
        .collect();
    let from_paths: Vec<&Locked> = changed
        .iter()
        .copied()
        .filter(|l| l.path.is_some())
        .collect();
    let local: Vec<Package> = changed
        .iter()
        .filter(|l| l.path.is_none())
        .filter_map(|l| l.package.clone())
        .map(|mut p| {
            p.dist.url = local_dist_url(&p.dist.url, &files.root);
            p
        })
        .collect();
    let to_place: Vec<&Package> = local.iter().collect();
    let wanted: BTreeSet<String> = locked
        .iter()
        .filter_map(|l| l.package.as_ref())
        .map(|p| abs_install_path(&vendor, p))
        .collect();
    let removing = before.values().any(|p| {
        p.install_path
            .as_ref()
            .is_some_and(|path| !wanted.contains(path) && path.starts_with(&format!("{vendor}/")))
    });

    let plugins = if req.no_plugins {
        Plugins::default()
    } else {
        let home = plugins::composer_home(env, plugins::system_uses_xdg(), &|d| d.is_dir());
        plugins::detect(
            &composer,
            lock.plugin_api_version(),
            &entries,
            &Global::load(home.as_deref()),
        )?
    };
    let scripts = Scripts::new(&composer, env);
    let plan = fallback::plan(req, &scripts, plugins, !changed.is_empty(), removing);
    let runs_scripts = !req.no_scripts
        && [
            scripts::PRE_INSTALL,
            scripts::PRE_AUTOLOAD,
            scripts::POST_AUTOLOAD,
            scripts::POST_INSTALL,
        ]
        .iter()
        .any(|e| scripts.has(e));
    let composer_bin = if plan.uses_composer() || runs_scripts {
        find_composer(env)
    } else {
        None
    };
    if let Some(why) = plan.needs_composer()
        && composer_bin.is_none()
    {
        return Err(Error::install(format!(
            "{why}, and composer is not on PATH (or set PHPM_COMPOSER). Nothing was changed. \
             Install Composer, or run with --no-scripts and --no-plugins to skip what needs it"
        )));
    }
    let store = Store::from_env()?;
    if req.explain {
        for l in &locked {
            let state = match (&l.package, &plan.full_install) {
                (_, Some(_)) => "fallback, composer install places it",
                (None, _) => "native, metapackage, nothing to place",
                (Some(_), _) if l.path.is_some() => "native, from a path repository",
                (Some(p), _) if !to_place.iter().any(|t| t.name == p.name) => {
                    "native, already in vendor/"
                }
                (Some(p), _) if store.contains(&p.name, &p.key()) => {
                    "native, placed from the store"
                }
                (Some(_), _) => "native, downloaded to the store, then placed",
            };
            out.info(&package_line(l, state, &plan, req.no_plugins));
        }
        for line in plan.decision_lines(req) {
            out.info(&line);
        }
    }
    let fallback = composer_bin
        .clone()
        .map(|bin| Composer::new(bin, &files.root, out.verbosity()));
    if let (Some(why), Some(composer)) = (&plan.full_install, &fallback) {
        let expires = filter_verdict(&mut filter, &store, &[], out)?;
        out.info(&format!("Composer runs this install: {why}"));
        composer.run(&fallback::install_args(req), out)?;
        return Ok(Installed {
            written: None,
            expires,
            audit_failed: false,
        });
    }
    let runner = runs_scripts.then(|| {
        Runner::new(
            &scripts,
            runner::Context {
                root: files.root.clone(),
                bin_dir: dirs.bin.clone(),
                dev: req.dev,
                composer: composer_bin.clone(),
                php: find_php(env),
                timeout: runner::timeout(
                    composer
                        .data()
                        .get("config")
                        .and_then(|c| c.get("process-timeout")),
                    env,
                ),
            },
            env,
        )
    });
    let mut steps = Steps {
        composer: fallback,
        runner,
        warmup: None,
    };
    steps.event(req, &plan.pre_install, scripts::PRE_INSTALL, false, out)?;

    let root_version = {
        let data = composer.data().clone();
        let root = files.root.clone();
        let from_env = env("COMPOSER_ROOT_VERSION");
        std::thread::spawn(move || phpm_lock::root_version(&data, &root, from_env.as_deref()))
    };
    let started = Instant::now();
    let missing: Vec<Package> = to_place
        .iter()
        .filter(|p| !store.contains(&p.name, &p.key()))
        .map(|p| (*p).clone())
        .collect();
    let mut sizes: BTreeMap<String, u64> = BTreeMap::new();
    if !missing.is_empty() {
        let n = net(&mut network, &files.root, config.as_ref())?;
        let then: Arc<dyn Fn(&Path) + Send + Sync> = match &cache_dir {
            Some(cache) => {
                let (store_root, classes) = (store.root().to_owned(), cache_root(cache));
                Arc::new(move |dir: &Path| crate::classes::prebuild(&store_root, &classes, dir))
            }
            None => Arc::new(|_: &Path| {}),
        };
        let fetched = match n
            .runtime
            .block_on(store.fetch_missing_then(&n.fetcher, &missing, then))
        {
            Ok(fetched) => fetched,
            Err(e) => {
                filter_verdict(&mut filter, &store, &missing, out)?;
                return Err(e.into());
            }
        };
        out.info(&format!(
            "Downloaded {} packages in {:.2?}",
            fetched.len(),
            started.elapsed()
        ));
        sizes.extend(fetched);
    }

    let expires = filter_verdict(&mut filter, &store, &missing, out)?;
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
    let notifier = notify_installs(
        &entries,
        &locked,
        &changed,
        &before,
        &sizes,
        files,
        config.as_ref(),
        env,
    );
    let mirror_env = env("COMPOSER_MIRROR_PATH_REPOS");
    for l in &from_paths {
        if let (Some(dist), Some(p)) = (&l.path, &l.package) {
            let dest = PathBuf::from(abs_install_path(&vendor, p));
            let how = pathrepo::install(&l.name, dist, &files.root, &dest, mirror_env.as_deref())?;
            out.detail(&format!("{}: {how:?} from {}", l.name, dist.url));
        }
    }
    out.detail(&format!(
        "placed {} packages ({used:?}) in {:.1?}",
        placements.len() + from_paths.len(),
        started.elapsed()
    ));

    let vendor_real =
        fs::canonicalize(&vendor).map_or_else(|_| vendor.clone(), |p| path_string(&p));
    let repo = PathBuf::from(format!("{vendor}/composer"));
    fs::create_dir_all(&repo).map_err(|e| Error::io(&repo, &e))?;
    let root_version = root_version
        .join()
        .map_err(|_| Error::install("guessing the root version failed"))??;
    let started = Instant::now();
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

    out.detail(&format!(
        "wrote installed.json and installed.php in {:.1?}",
        started.elapsed()
    ));
    let started = Instant::now();
    let modes = Modes::probe(&repo).map_err(|e| Error::io(&repo, &e))?;
    let mut bins = BinInstaller::new(
        PathBuf::from(&dirs.bin),
        vendor.clone(),
        vendor_real.clone(),
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
    out.detail(&format!("wrote bin proxies in {:.1?}", started.elapsed()));

    out.info(&summary(placements.len() + from_paths.len(), removed));
    let all = lock.packages()?.into_iter().chain(lock.packages_dev()?);
    for entry in all {
        if let Some(Abandonment(replacement)) = abandoned(entry) {
            let hint = replacement.map_or_else(
                || "No replacement was suggested".to_owned(),
                |r| format!("Use {r} instead"),
            );
            out.info(&format!(
                "Package {} is abandoned, you should avoid using it. {hint}.",
                text(entry, "name").unwrap_or_default()
            ));
        }
    }

    if [
        &plan.autoload,
        &plan.pre_autoload,
        &plan.post_autoload,
        &plan.post_install,
    ]
    .iter()
    .any(|s| s.is_composer())
    {
        let psr = plan.autoload.is_composer()
            && phpm_autoload::Options {
                optimize: req.optimize || req.classmap_authoritative,
                ..phpm_autoload::Options::default()
            }
            .with_config(&composer)
            .optimize;
        let placed: BTreeSet<&str> = to_place.iter().map(|p| p.name.as_str()).collect();
        let roots: Vec<PathBuf> = entries
            .iter()
            .zip(&locked)
            .filter(|(_, l)| placed.contains(l.name.as_str()))
            .filter_map(|(e, l)| l.package.as_ref().map(|p| (e, p)))
            .flat_map(|(e, p)| {
                prefetch::scan_roots(e, Path::new(&abs_install_path(&vendor, p)), psr)
            })
            .collect();
        steps.warmup = Some((
            Instant::now(),
            std::thread::spawn(move || prefetch::warm(&roots)),
        ));
    }

    match &plan.autoload {
        Step::Native(_) => {
            steps.event(req, &plan.pre_autoload, scripts::PRE_AUTOLOAD, false, out)?;
            written.extend(native_autoload(
                req,
                &composer,
                &lock,
                &root_dir,
                &class_trees(&store, &placements, &vendor_real),
                out,
            )?);
            steps.event(req, &plan.post_autoload, scripts::POST_AUTOLOAD, false, out)?;
        }
        Step::Composer(why) => {
            if let Some(composer) = steps.composer(out).cloned() {
                out.info(&format!("Composer runs the autoload dump: {why}"));
                composer.run(&fallback::dump_args(req), out)?;
                written.extend(top_files(Path::new(&vendor)));
                written.extend(top_files(&repo));
            }
        }
        Step::Skip(_) => {}
    }
    steps.event(
        req,
        &plan.post_install,
        scripts::POST_INSTALL,
        plan.post_install_for_plugins && !scripts.has(scripts::POST_INSTALL),
        out,
    )?;
    written.extend(wanted.iter().map(PathBuf::from));
    if let Some(handle) = notifier {
        let _ = handle.join();
    }
    let mut audit_failed = false;
    if let Some(format) = req.audit {
        let audited: Vec<Audited> = entries
            .iter()
            .map(|e| Audited {
                locked: policy_locked(e),
                abandoned: abandoned(e),
            })
            .collect();
        if audited.is_empty() {
            out.info("No installed packages - skipping audit.");
        } else {
            let n = net(&mut network, &files.root, config.as_ref())?;
            let client = policy::Client {
                fetcher: &n.fetcher,
                cache: cache_dir,
            };
            match n.runtime.block_on(policy::run_audit(
                &client, &repos, &policy, &audited, format,
            )) {
                Ok(report) => {
                    for line in &report.stderr {
                        out.info(line);
                    }
                    out.stdout(&report.stdout);
                    audit_failed = report.failed;
                }
                Err(reason) => {
                    out.error("Failed to audit installed packages.");
                    out.detail(&reason);
                }
            }
        }
    }
    Ok(Installed {
        written: Some(written),
        expires,
        audit_failed,
    })
}

/// Wait for the malware filter before anything in `vendor/` changes; its
/// warnings print here, and the result's expiry bounds the no-op state.
fn filter_verdict(
    filter: &mut Pending,
    store: &Store,
    fetched: &[Package],
    out: &mut Out<'_>,
) -> Result<Option<u64>, Error> {
    let (outcome, waited) = filter.verdict(store, fetched)?;
    for line in &outcome.warnings {
        out.info(line);
    }
    if let Some(waited) = waited {
        out.detail(&format!(
            "checked the malware filter lists, verdict in hand {waited:.1?} after the install started"
        ));
    }
    Ok(outcome.expires)
}

/// Where the install's script events and Composer calls go.
struct Steps<'a> {
    composer: Option<Composer>,
    runner: Option<Runner<'a>>,
    warmup: Option<(Instant, std::thread::JoinHandle<usize>)>,
}

impl Steps<'_> {
    /// Let the read-ahead finish before Composer starts scanning.
    fn composer(&mut self, out: &mut Out<'_>) -> Option<&Composer> {
        if let Some((started, handle)) = self.warmup.take() {
            let read = handle.join().unwrap_or(0);
            out.detail(&format!(
                "read {read} files ahead of Composer in {:.1?}",
                started.elapsed()
            ));
        }
        self.composer.as_ref()
    }

    /// One script event, run where the plan says.
    fn event(
        &mut self,
        req: &Request,
        step: &Step,
        event: &str,
        only_plugins_listen: bool,
        out: &mut Out<'_>,
    ) -> Result<(), Error> {
        match step {
            Step::Native(_) => match self.runner.as_mut() {
                Some(runner) => runner.dispatch(event, &[], out),
                None => Ok(()),
            },
            Step::Composer(why) => {
                let Some(composer) = self.composer(out).cloned() else {
                    return Ok(());
                };
                out.info(&format!("Composer runs {event}: {why}"));
                let args = fallback::run_script_args(req, event);
                if only_plugins_listen {
                    composer.run_script_for_plugins(&args, out)
                } else {
                    composer.run(&args, out)
                }
            }
            Step::Skip(_) => Ok(()),
        }
    }
}

/// phpm's own autoloader, byte-identical to Composer's; returns what it wrote.
fn native_autoload(
    req: &Request,
    composer: &ComposerJson,
    lock: &Lock,
    root_dir: &str,
    trees: &[Tree],
    out: &mut Out<'_>,
) -> Result<Vec<PathBuf>, Error> {
    let started = Instant::now();
    let mut options = phpm_autoload::Options {
        dev_mode: req.dev,
        optimize: req.optimize,
        classmap_authoritative: req.classmap_authoritative,
        platform: req.platform(),
        ..phpm_autoload::Options::default()
    }
    .with_config(composer);
    if options.optimize {
        options.known_classes = Some(Arc::new(known_classes(trees)));
        out.detail(&format!(
            "loaded class scans for {} packages in {:.1?}",
            trees.len(),
            started.elapsed()
        ));
    }
    let autoload = phpm_autoload::generate(
        &phpm_autoload::Project {
            composer_json: composer,
            lock,
            root_dir,
        },
        &options,
    )?;
    autoload
        .write()
        .map_err(|e| Error::install(format!("writing the autoloader: {e}")))?;
    for w in &autoload.warnings {
        out.warn(w);
    }
    out.detail(&format!(
        "wrote the autoloader in {:.1?}",
        started.elapsed()
    ));
    Ok(autoload
        .files
        .iter()
        .map(|(name, _)| Path::new(&autoload.vendor_dir).join(name))
        .collect())
}

// Composer: Installer.php doInstall, "Verifying lock file contents can be installed on current platform."
fn check_platform(
    req: &Request,
    env: Env<'_>,
    composer: &ComposerJson,
    lock: &Lock,
    out: &mut Out<'_>,
) -> Result<(), Error> {
    let filter = Filter::new(req.ignore_platform_reqs, &req.ignore_platform_req);
    let reqs = Requirements::from_lock(composer, lock, req.dev)?;
    if !reqs.needs_platform(&filter) {
        return Ok(());
    }
    let started = Instant::now();
    let php = crate::platform::find_php(env).ok_or_else(|| {
        Error::install(
            "php is not on PATH, so the platform requirements in the lock file cannot be checked; \
             install PHP or run with --ignore-platform-reqs",
        )
    })?;
    let cache = phpm_store::cache_dir().ok();
    let probe = crate::platform::probe(&php, cache.as_deref(), env)?;
    let overrides: Vec<(&str, &Value)> = lock
        .data()
        .get("platform-overrides")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.as_str(), v))
        .collect();
    let platform = Platform::build(&probe, &overrides)?;
    crate::platform::verify(&platform, &reqs, &filter)?;
    out.detail(&format!(
        "platform requirements met by {} ({:.1?})",
        php.display(),
        started.elapsed()
    ));
    Ok(())
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
        Previous, Request, bins, locked, notify_on_install, package_line, previous, remove_package,
        summary, top_files, unchanged,
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

    #[test]
    fn notify_on_install_prefers_the_project_then_the_home_config() {
        let off = obj(json!({"notify-on-install": false}));
        let on = obj(json!({"notify-on-install": true}));
        let home_off = || Some(obj(json!({"config": {"notify-on-install": false}})));
        assert!(notify_on_install(None, || None));
        assert!(!notify_on_install(Some(&off), || None));
        assert!(!notify_on_install(None, home_off));
        assert!(!notify_on_install(Some(&obj(json!({}))), home_off));
        assert!(notify_on_install(Some(&on), home_off));
        assert!(notify_on_install(None, || Some(obj(json!({"config": []})))));
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
    fn explains_each_package() {
        use crate::fallback::plan;
        use crate::plugins::{Plugin, Plugins, Skipped};
        use crate::scripts::Scripts;
        let c = composer(json!({}));
        let mut entry = zip_entry("a/plugin");
        entry.insert("type".into(), json!("composer-plugin"));
        let l = locked(&entry, &c).unwrap();
        assert!(l.plugin);
        let scripts = Scripts::new(&c, &|_| None);
        let req = Request::default();
        let active = Plugins {
            active: vec![Plugin {
                name: "a/plugin".into(),
                global: false,
                needs_full_install: None,
            }],
            skipped: Vec::new(),
        };
        let p = plan(&req, &scripts, active, true, false);
        assert_eq!(
            package_line(&l, "native, placed from the store", &p, false),
            "package a/plugin 1.0.0: native, placed from the store; plugin, Composer loads it for the steps it hooks"
        );
        let skipped = Plugins {
            active: Vec::new(),
            skipped: vec![Skipped {
                name: "a/plugin".into(),
                why: "not in config.allow-plugins",
            }],
        };
        let p = plan(&req, &scripts, skipped, true, false);
        assert!(
            package_line(&l, "x", &p, false)
                .ends_with("; plugin, not loaded: not in config.allow-plugins")
        );
        let p = plan(&req, &scripts, Plugins::default(), true, false);
        assert!(package_line(&l, "x", &p, true).ends_with("not loaded: --no-plugins"));
        assert_eq!(
            package_line(&l, "x", &p, false),
            "package a/plugin 1.0.0: x"
        );
        let full = Plugins {
            active: vec![Plugin {
                name: "a/plugin".into(),
                global: false,
                needs_full_install: Some("why".into()),
            }],
            skipped: Vec::new(),
        };
        let p = plan(&req, &scripts, full, true, false);
        assert!(package_line(&l, "x", &p, false).ends_with("loaded by composer install"));
    }

    #[test]
    fn lists_top_level_files_only() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("sub")).unwrap();
        fs::write(tmp.path().join("b.php"), b"").unwrap();
        fs::write(tmp.path().join("a.php"), b"").unwrap();
        fs::write(tmp.path().join("sub/c.php"), b"").unwrap();
        let names: Vec<String> = top_files(tmp.path())
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.php", "b.php"]);
        assert!(top_files(&tmp.path().join("none")).is_empty());
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
