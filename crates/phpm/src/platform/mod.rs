//! What `php` on PATH offers, as Composer's `PlatformRepository` sees it.
//!
//! One `php` subprocess prints everything (probe.php); the answer is cached
//! under `<cache>/platform/v1/` keyed by the binary's path, size, mtime and
//! inode and the ini environment, and revalidated against the ini files it
//! loaded.

mod check;

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use phpm_lock::COMPOSER_VERSION;
use phpm_lock::version::normalize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::error::Error;
use crate::project::Env;
use crate::state::{Stamp, hex};

pub(crate) use check::{Filter, Requirements, glob, verify};

const PROBE: &str = include_str!("probe.php");
const PLUGIN_API_VERSION: &str = "2.9.0";
const RUNTIME_API_VERSION: &str = "2.2.2";

/// `PlatformRepository::isPlatformPackage`.
pub(crate) fn is_platform_package(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    if matches!(
        name.as_str(),
        "php"
            | "php-64bit"
            | "php-ipv6"
            | "php-zts"
            | "php-debug"
            | "hhvm"
            | "composer"
            | "composer-plugin-api"
            | "composer-runtime-api"
    ) {
        return true;
    }
    let Some(rest) = name
        .strip_prefix("ext-")
        .or_else(|| name.strip_prefix("lib-"))
    else {
        return false;
    };
    let b = rest.as_bytes();
    let alnum = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    if b.is_empty() || !alnum(b[0]) || !alnum(b[b.len() - 1]) {
        return false;
    }
    b.windows(2)
        .all(|w| alnum(w[1]) || (alnum(w[0]) && matches!(w[1], b'_' | b'.' | b'-')))
}

/// What probe.php printed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Probe {
    pub(crate) version: String,
    pub(crate) debug: bool,
    pub(crate) zts: bool,
    pub(crate) int_size: u64,
    pub(crate) ipv6: bool,
    pub(crate) extensions: Vec<(String, String)>,
    pub(crate) libraries: Vec<Library>,
    /// `php_ini_loaded_file()` (empty if none), then the scanned files.
    pub(crate) ini: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Library {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) replaces: Vec<String>,
    pub(crate) provides: Vec<String>,
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

impl Probe {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let v: Value = serde_json::from_str(text).ok()?;
        let pair = |item: &Value| {
            let a = item.as_array()?;
            Some((
                a.first()?.as_str()?.to_owned(),
                a.get(1)?.as_str()?.to_owned(),
            ))
        };
        Some(Self {
            version: v.get("version")?.as_str()?.to_owned(),
            debug: v.get("debug")?.as_bool()?,
            zts: v.get("zts")?.as_bool()?,
            int_size: v.get("int_size")?.as_u64()?,
            ipv6: v.get("ipv6")?.as_bool()?,
            extensions: v
                .get("extensions")?
                .as_array()?
                .iter()
                .map(pair)
                .collect::<Option<_>>()?,
            libraries: v
                .get("libraries")?
                .as_array()?
                .iter()
                .map(|item| {
                    let a = item.as_array()?;
                    Some(Library {
                        name: a.first()?.as_str()?.to_owned(),
                        version: a.get(1)?.as_str()?.to_owned(),
                        replaces: strings(a.get(2)?),
                        provides: strings(a.get(3)?),
                    })
                })
                .collect::<Option<_>>()?,
            ini: strings(v.get("ini")?),
        })
    }

    fn to_json(&self) -> Value {
        json!({
            "version": self.version,
            "debug": self.debug,
            "zts": self.zts,
            "int_size": self.int_size,
            "ipv6": self.ipv6,
            "extensions": self.extensions.iter().map(|(n, v)| json!([n, v])).collect::<Vec<_>>(),
            "libraries": self.libraries.iter().map(|l| json!([l.name, l.version, l.replaces, l.provides])).collect::<Vec<_>>(),
            "ini": self.ini,
        })
    }

    /// The extension names `extension_loaded()` would accept, lowercased.
    pub(crate) fn loaded(&self) -> BTreeSet<String> {
        let mut names: BTreeSet<String> = self
            .extensions
            .iter()
            .map(|(n, _)| n.to_ascii_lowercase())
            .collect();
        names.extend(["standard".to_owned(), "core".to_owned()]);
        names
    }
}

/// A package in the platform repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlatformPackage {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) pretty: String,
    /// `(target, replace|provide)` links at this package's version.
    pub(crate) links: Vec<(String, &'static str)>,
    /// Set when `config.platform` put this version here.
    pub(crate) overridden: Option<String>,
}

/// The platform repository: what is installed plus `config.platform` overrides.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Platform {
    pub(crate) packages: Vec<PlatformPackage>,
    pub(crate) disabled: BTreeSet<String>,
    pub(crate) loaded: BTreeSet<String>,
    pub(crate) ini: Vec<String>,
}

/// `config.platform` as the lock's `platform-overrides` records it.
pub(crate) type Overrides<'a> = Vec<(&'a str, &'a Value)>;

fn normalized(pretty: &str) -> Result<String, Error> {
    normalize(pretty).map_err(|e| Error::install(e.to_string()))
}

// Composer: Repository/PlatformRepository.php initialize, addPackage, addExtension, addLibrary
impl Platform {
    pub(crate) fn build(probe: &Probe, overrides: &Overrides<'_>) -> Result<Self, Error> {
        let mut repo = Builder {
            platform: Self {
                loaded: probe.loaded(),
                ini: probe.ini.clone(),
                ..Self::default()
            },
            overrides: Vec::new(),
            libraries: BTreeSet::new(),
        };
        for (name, version) in overrides {
            let version = match version {
                Value::String(s) => Some(s.clone()),
                Value::Bool(false) if !name.eq_ignore_ascii_case("php") => None,
                Value::Bool(false) => {
                    return Err(Error::install(format!(
                        "config.platform.{name} cannot be set to false as you cannot disable php entirely."
                    )));
                }
                other => {
                    return Err(Error::install(format!(
                        "config.platform.{name} should be a string or false, but got {other}"
                    )));
                }
            };
            if !is_platform_package(name) {
                return Err(Error::install(format!(
                    "Invalid platform package name in config.platform: {name}"
                )));
            }
            repo.overrides
                .push((name.to_ascii_lowercase(), version.clone()));
            if let Some(v) = version {
                repo.add_overridden(&name.to_ascii_lowercase(), &v)?;
            }
        }
        for (name, pretty) in [
            ("composer", COMPOSER_VERSION),
            ("composer-plugin-api", PLUGIN_API_VERSION),
            ("composer-runtime-api", RUNTIME_API_VERSION),
        ] {
            repo.add(name, pretty.to_owned(), Vec::new())?;
        }
        let php = match normalize(&probe.version) {
            Ok(_) => probe.version.clone(),
            Err(_) => probe
                .version
                .split(['~', '+', '-'])
                .next()
                .unwrap_or_default()
                .to_owned(),
        };
        repo.add("php", php.clone(), Vec::new())?;
        let flags = [
            ("php-debug", probe.debug),
            ("php-zts", probe.zts),
            ("php-64bit", probe.int_size == 8),
            ("php-ipv6", probe.ipv6),
        ];
        for (name, present) in flags {
            if present {
                repo.add(name, php.clone(), Vec::new())?;
            }
        }
        for (name, version) in &probe.extensions {
            repo.add_extension(name, version)?;
        }
        for lib in &probe.libraries {
            repo.add_library(lib)?;
        }
        Ok(repo.platform)
    }

    #[cfg(test)]
    pub(crate) fn find(&self, name: &str) -> Option<&PlatformPackage> {
        self.packages.iter().find(|p| p.name == name)
    }
}

struct Builder {
    platform: Platform,
    overrides: Vec<(String, Option<String>)>,
    libraries: BTreeSet<String>,
}

impl Builder {
    fn override_of(&self, name: &str) -> Option<&Option<String>> {
        self.overrides
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
    }

    fn add_overridden(&mut self, name: &str, pretty: &str) -> Result<usize, Error> {
        self.platform.packages.push(PlatformPackage {
            name: name.to_owned(),
            version: normalized(pretty)?,
            pretty: pretty.to_owned(),
            links: Vec::new(),
            overridden: Some("overridden via config.platform".to_owned()),
        });
        Ok(self.platform.packages.len() - 1)
    }

    fn note_actual(&mut self, index: usize, version: &str, pretty: &str) {
        let package = &mut self.platform.packages[index];
        let actual = if package.version == version {
            "same as actual".to_owned()
        } else {
            format!("actual: {pretty}")
        };
        if let Some(text) = &mut package.overridden {
            text.push_str(", ");
            text.push_str(&actual);
        }
    }

    fn add(
        &mut self,
        name: &str,
        pretty: String,
        links: Vec<(String, &'static str)>,
    ) -> Result<(), Error> {
        let version = normalized(&pretty)?;
        match self.override_of(name).cloned() {
            Some(None) => {
                self.platform.disabled.insert(name.to_owned());
            }
            Some(Some(_)) => {
                if let Some(i) = self.platform.packages.iter().position(|p| p.name == name) {
                    self.note_actual(i, &version, &pretty);
                }
            }
            None => {
                if name.starts_with("php-")
                    && let Some(Some(php)) = self.override_of("php").cloned()
                {
                    let i = self.add_overridden(name, &php)?;
                    self.note_actual(i, &version, &pretty);
                    return Ok(());
                }
                self.platform.packages.push(PlatformPackage {
                    name: name.to_owned(),
                    version,
                    pretty,
                    links,
                    overridden: None,
                });
            }
        }
        Ok(())
    }

    fn add_extension(&mut self, name: &str, pretty: &str) -> Result<(), Error> {
        let pretty = if normalize(pretty).is_ok() {
            pretty.to_owned()
        } else {
            let digits: String = pretty
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            let parts: Vec<&str> = digits.split('.').collect();
            if parts.len() >= 3 && parts.iter().take(3).all(|p| !p.is_empty()) {
                let take = if parts.len() >= 4 && !parts[3].is_empty() {
                    4
                } else {
                    3
                };
                parts[..take].join(".")
            } else {
                "0".to_owned()
            }
        };
        let package = format!("ext-{}", name.to_ascii_lowercase().replace(' ', "-"));
        let links = if name == "uuid" {
            vec![("lib-uuid".to_owned(), "replace")]
        } else {
            Vec::new()
        };
        self.add(&package, pretty, links)
    }

    fn add_library(&mut self, lib: &Library) -> Result<(), Error> {
        let name = format!("lib-{}", lib.name).to_ascii_lowercase();
        if normalize(&lib.version).is_err() || !is_platform_package(&name) {
            return Ok(());
        }
        if !self.libraries.insert(name.clone()) {
            return Ok(());
        }
        let mut links: Vec<(String, &'static str)> = lib
            .replaces
            .iter()
            .map(|r| (format!("lib-{}", r.to_ascii_lowercase()), "replace"))
            .collect();
        links.extend(
            lib.provides
                .iter()
                .map(|p| (format!("lib-{}", p.to_ascii_lowercase()), "provide")),
        );
        self.add(&name, lib.version.clone(), links)
    }
}

/// `php` on PATH, as a shell would find it.
pub(crate) fn find_php(env: Env<'_>) -> Option<PathBuf> {
    let path = env("PATH")?;
    let names: &[&str] = if cfg!(windows) {
        &["php.exe", "php.bat", "php.cmd", "php"]
    } else {
        &["php"]
    };
    std::env::split_paths(&OsString::from(path))
        .filter(|dir| !dir.as_os_str().is_empty())
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|p| p.is_file())
}

const INI_ENV: [&str; 2] = ["PHPRC", "PHP_INI_SCAN_DIR"];

fn stamp_text(path: &Path) -> String {
    Stamp::of(path).map_or_else(|| "-".to_owned(), |s| s.to_string())
}

/// Cheap enough for the no-op path: which php, its stamp, and the ini env.
pub(crate) fn fingerprint(php: Option<&Path>, env: Env<'_>) -> String {
    let Some(php) = php else {
        return "no php".to_owned();
    };
    let mut out = format!("{}\n{}", php.display(), stamp_text(php));
    for key in INI_ENV {
        out.push('\n');
        out.push_str(&env(key).unwrap_or_default());
    }
    out
}

fn ini_stamps(ini: &[String]) -> Vec<(String, String)> {
    let mut paths: BTreeSet<PathBuf> = BTreeSet::new();
    for file in ini.iter().filter(|f| !f.is_empty()) {
        let file = PathBuf::from(file);
        if let Some(dir) = file.parent() {
            paths.insert(dir.to_owned());
        }
        paths.insert(file);
    }
    paths
        .into_iter()
        .map(|p| {
            let stamp = stamp_text(&p);
            (p.to_string_lossy().into_owned(), stamp)
        })
        .collect()
}

fn cache_file(cache_dir: &Path, key: &str) -> PathBuf {
    let name = hex(&Sha256::digest(key.as_bytes()));
    cache_dir
        .join("platform")
        .join("v1")
        .join(format!("{name}.json"))
}

fn load_cached(file: &Path, key: &str) -> Option<Probe> {
    let record: Value = serde_json::from_slice(&fs::read(file).ok()?).ok()?;
    if record.get("key")?.as_str()? != key {
        return None;
    }
    let fresh = record.get("ini")?.as_array()?.iter().all(|item| {
        let pair = item.as_array();
        pair.and_then(|p| Some((p.first()?.as_str()?, p.get(1)?.as_str()?)))
            .is_some_and(|(path, stamp)| stamp_text(Path::new(path)) == stamp)
    });
    if !fresh {
        return None;
    }
    Probe::parse(&record.get("probe")?.to_string())
}

fn save_cached(file: &Path, key: &str, probe: &Probe) {
    let ini: Vec<Value> = ini_stamps(&probe.ini)
        .into_iter()
        .map(|(p, s)| json!([p, s]))
        .collect();
    let record = json!({"key": key, "ini": ini, "probe": probe.to_json()});
    if let Some(dir) = file.parent()
        && fs::create_dir_all(dir).is_ok()
    {
        let tmp = file.with_extension(format!("tmp{}", std::process::id()));
        if fs::write(&tmp, record.to_string()).is_ok() {
            let _ = fs::rename(&tmp, file);
        }
    }
}

/// Run probe.php with `php`.
pub(crate) fn run_probe(php: &Path) -> Result<Probe, Error> {
    let failed = |reason: String| {
        Error::install(format!(
            "could not read the platform from {}: {reason}",
            php.display()
        ))
    };
    let mut child = Command::new(php)
        .args(["-d", "display_errors=stderr", "-d", "html_errors=0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| failed(e.to_string()))?;
    if let Some(mut stdin) = child.stdin.take() {
        // A php that exits early fails below with its own output.
        let _ = stdin.write_all(PROBE.as_bytes());
    }
    let output = child
        .wait_with_output()
        .map_err(|e| failed(e.to_string()))?;
    let text = String::from_utf8_lossy(&output.stdout);
    match Probe::parse(&text) {
        Some(probe) if output.status.success() => Ok(probe),
        _ => Err(failed(format!(
            "{} {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

/// The probe for `php`, from the cache when the binary and its ini files
/// have not changed since it was taken.
pub(crate) fn probe(php: &Path, cache_dir: Option<&Path>, env: Env<'_>) -> Result<Probe, Error> {
    let key = fingerprint(Some(php), env);
    let file = cache_dir.map(|c| cache_file(c, &key));
    if let Some(probe) = file.as_deref().and_then(|f| load_cached(f, &key)) {
        return Ok(probe);
    }
    let probe = run_probe(php)?;
    if let Some(file) = &file {
        save_cached(file, &key, &probe);
    }
    Ok(probe)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{
        Library, Platform, Probe, cache_file, find_php, fingerprint, is_platform_package,
        load_cached, save_cached,
    };
    use serde_json::{Value, json};

    pub(crate) fn sample() -> Probe {
        Probe {
            version: "8.4.13".into(),
            debug: false,
            zts: false,
            int_size: 8,
            ipv6: true,
            extensions: vec![
                ("mbstring".into(), "8.4.13".into()),
                ("Zend OPcache".into(), "8.4.13".into()),
                ("odd".into(), "1.2.3.4.5-x y".into()),
                ("weird".into(), "garbage".into()),
                ("uuid".into(), "1.2.0".into()),
            ],
            libraries: vec![
                Library {
                    name: "pcre".into(),
                    version: "10.44".into(),
                    ..Library::default()
                },
                Library {
                    name: "pcre".into(),
                    version: "11".into(),
                    ..Library::default()
                },
                Library {
                    name: "bad name!".into(),
                    version: "1".into(),
                    ..Library::default()
                },
                Library {
                    name: "nover".into(),
                    version: "not a version".into(),
                    ..Library::default()
                },
                Library {
                    name: "libxml".into(),
                    version: "2.9.13".into(),
                    replaces: vec!["Old".into()],
                    provides: vec!["dom-libxml".into()],
                },
            ],
            ini: vec!["/etc/php.ini".into()],
        }
    }

    fn versions(p: &Platform) -> Vec<(String, String)> {
        p.packages
            .iter()
            .map(|p| (p.name.clone(), p.pretty.clone()))
            .collect()
    }

    #[test]
    fn recognises_platform_package_names() {
        for yes in [
            "php",
            "PHP-64bit",
            "ext-mbstring",
            "ext-zend-opcache",
            "lib-icu-cldr",
            "lib-a.b_c",
            "composer-runtime-api",
            "hhvm",
        ] {
            assert!(is_platform_package(yes), "{yes}");
        }
        for no in [
            "php-x",
            "ext-",
            "ext--a",
            "ext-a-",
            "lib-a..b",
            "monolog/monolog",
            "ext-a b",
            "composer-api",
        ] {
            assert!(!is_platform_package(no), "{no}");
        }
    }

    #[test]
    fn builds_the_repository_like_composer() {
        let p = Platform::build(&sample(), &Vec::new()).unwrap();
        let v = versions(&p);
        let names: Vec<&str> = v.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "composer",
                "composer-plugin-api",
                "composer-runtime-api",
                "php",
                "php-64bit",
                "php-ipv6",
                "ext-mbstring",
                "ext-zend-opcache",
                "ext-odd",
                "ext-weird",
                "ext-uuid",
                "lib-pcre",
                "lib-libxml",
            ]
        );
        assert_eq!(p.find("ext-odd").unwrap().pretty, "1.2.3.4");
        assert_eq!(p.find("ext-weird").unwrap().pretty, "0");
        assert_eq!(p.find("lib-pcre").unwrap().version, "10.44.0.0");
        assert_eq!(p.find("composer").unwrap().pretty, "2.10.3");
        assert_eq!(
            p.find("lib-libxml").unwrap().links,
            [
                ("lib-old".to_owned(), "replace"),
                ("lib-dom-libxml".to_owned(), "provide")
            ]
        );
        assert_eq!(p.find("ext-uuid").unwrap().links[0].0, "lib-uuid");
        assert!(p.loaded.contains("zend opcache") && p.loaded.contains("standard"));
    }

    #[test]
    fn applies_config_platform_overrides() {
        let php = json!("8.1.0");
        let off = json!(false);
        let intl = json!("74.1");
        let overrides: Vec<(&str, &Value)> =
            vec![("php", &php), ("ext-mbstring", &off), ("ext-intl", &intl)];
        let p = Platform::build(&sample(), &overrides).unwrap();
        assert_eq!(p.find("php").unwrap().pretty, "8.1.0");
        assert_eq!(
            p.find("php").unwrap().overridden.as_deref(),
            Some("overridden via config.platform, actual: 8.4.13")
        );
        assert_eq!(p.find("php-64bit").unwrap().pretty, "8.1.0");
        assert!(p.find("ext-mbstring").is_none());
        assert!(p.disabled.contains("ext-mbstring"));
        assert_eq!(p.find("ext-intl").unwrap().version, "74.1.0.0");

        let same = json!("8.4.13");
        let p = Platform::build(&sample(), &vec![("php", &same)]).unwrap();
        assert!(
            p.find("php")
                .unwrap()
                .overridden
                .as_deref()
                .unwrap()
                .ends_with("same as actual")
        );

        for (name, bad) in [
            ("php", json!(false)),
            ("ext-x", json!(1)),
            ("nope/pkg", json!("1.0")),
            ("ext-x", json!("not a version")),
        ] {
            assert!(
                Platform::build(&sample(), &vec![(name, &bad)]).is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn strips_suffixes_php_cannot_normalise() {
        let mut probe = sample();
        probe.version = "8.4.0~ubuntu+1".into();
        let p = Platform::build(&probe, &Vec::new()).unwrap();
        assert_eq!(p.find("php").unwrap().pretty, "8.4.0");
    }

    #[test]
    fn parses_and_round_trips_probe_output() {
        let probe = sample();
        let again = Probe::parse(&probe.to_json().to_string()).unwrap();
        assert_eq!(again, probe);
        assert!(Probe::parse("nope").is_none());
        assert!(Probe::parse("{}").is_none());
    }

    #[test]
    fn caches_until_an_ini_file_changes() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("etc")).unwrap();
        let ini = tmp.path().join("etc/php.ini");
        std::fs::write(&ini, "a=1").unwrap();
        let mut probe = sample();
        probe.ini = vec![ini.to_string_lossy().into_owned(), String::new()];
        let file = cache_file(tmp.path(), "key");
        save_cached(&file, "key", &probe);
        assert_eq!(load_cached(&file, "key"), Some(probe));
        assert_eq!(load_cached(&file, "other"), None);
        std::fs::write(&ini, "a=22").unwrap();
        assert_eq!(load_cached(&file, "key"), None);
    }

    #[test]
    fn finds_php_on_path() {
        let tmp = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) { "php.exe" } else { "php" };
        std::fs::write(tmp.path().join(name), "").unwrap();
        let path = tmp.path().to_string_lossy().into_owned();
        let env = move |k: &str| (k == "PATH").then(|| path.clone());
        assert_eq!(find_php(&env), Some(tmp.path().join(name)));
        assert_eq!(find_php(&|_: &str| None), None);
        let fp = fingerprint(find_php(&env).as_deref(), &env);
        assert!(fp.contains(&tmp.path().to_string_lossy().into_owned()));
        assert_eq!(fingerprint(None, &env), "no php");
    }

    #[cfg(unix)]
    #[test]
    fn runs_the_probe_and_reports_failures() {
        use super::probe;
        use std::os::unix::fs::PermissionsExt;
        use std::path::Path;
        let tmp = tempfile::tempdir().unwrap();
        let php = tmp.path().join("php");
        let out = sample().to_json().to_string();
        std::fs::write(
            &php,
            format!("#!/bin/sh\ncat >/dev/null\nprintf '%s' '{out}'\n"),
        )
        .unwrap();
        std::fs::set_permissions(&php, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cache = tmp.path().join("cache");
        let env = |_: &str| None;
        assert_eq!(probe(&php, Some(&cache), &env).unwrap(), sample());
        assert_eq!(probe(&php, Some(&cache), &env).unwrap(), sample());
        std::fs::write(&php, "#!/bin/sh\necho broken >&2\nexit 3\n").unwrap();
        let err = probe(&php, Some(&cache), &env).unwrap_err();
        assert!(err.message.contains("broken"), "{err}");
        assert!(probe(Path::new("/nonexistent/php"), None, &env).is_err());
    }

    #[test]
    #[ignore = "needs php and composer on PATH"]
    fn composer_live_platform_matches_composer_show() {
        use super::run_probe;
        use std::collections::BTreeSet;
        let path = std::env::var("PATH").unwrap();
        let env = move |k: &str| (k == "PATH").then(|| path.clone());
        let php = find_php(&env).unwrap();
        let ours: BTreeSet<(String, String)> =
            Platform::build(&run_probe(&php).unwrap(), &Vec::new())
                .unwrap()
                .packages
                .into_iter()
                .map(|p| (p.name, p.pretty))
                .collect();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("composer.json"), "{}").unwrap();
        let out = std::process::Command::new("composer")
            .args(["show", "--platform", "--format=json", "--no-plugins"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        let shown: Value = serde_json::from_slice(&out.stdout).unwrap();
        let theirs: BTreeSet<(String, String)> = shown["platform"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    p["name"].as_str().unwrap().to_owned(),
                    p["version"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        assert_eq!(ours, theirs);
    }
}
