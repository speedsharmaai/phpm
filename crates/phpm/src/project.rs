//! Where a project's files are, and the directories its config points at.

use std::path::{Path, PathBuf};

use phpm_lock::{ComposerJson, normalize_path};
use serde_json::Value;

use crate::error::Error;
use crate::fsutil::path_string;

/// Reads an environment variable; injected so tests need no process env.
pub(crate) type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

/// `composer.json` (or `$COMPOSER`) and its lock file, relative to `root`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectFiles {
    pub(crate) root: PathBuf,
    pub(crate) composer_file: PathBuf,
    pub(crate) lock_file: PathBuf,
}

// Composer: Factory.php getComposerFile, getLockFile
pub(crate) fn locate(root: PathBuf, env: Env<'_>) -> ProjectFiles {
    let name = env("COMPOSER")
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "composer.json".to_owned());
    let lock = match name.strip_suffix(".json") {
        Some(stem) if Path::new(&name).extension().is_some_and(|e| e == "json") => {
            format!("{stem}.lock")
        }
        _ => format!("{name}.lock"),
    };
    ProjectFiles {
        composer_file: root.join(&name),
        lock_file: root.join(lock),
        root,
    }
}

/// The resolved `vendor-dir`, `bin-dir` and `bin-compat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Dirs {
    pub(crate) vendor: String,
    /// `vendor-dir` as Composer reports it with `RELATIVE_PATHS`.
    pub(crate) vendor_relative: String,
    pub(crate) bin: String,
    pub(crate) full_bin_compat: bool,
}

fn config_str(composer: &ComposerJson, key: &str) -> Option<String> {
    composer
        .data()
        .get("config")
        .and_then(|c| c.get(key))
        .and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
}

fn non_empty(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.is_empty())
}

// Composer: Util/Platform.php expandPath
fn expand_path(path: &str, env: Env<'_>) -> String {
    if let Some(rest) = path.strip_prefix('~')
        && (rest.starts_with('/') || rest.starts_with('\\'))
        && let Some(home) = non_empty(env("HOME"))
    {
        return format!("{home}{rest}");
    }
    if let Some(rest) = path.strip_prefix('$') {
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if end > 0 {
            let value = env(&rest[..end]).unwrap_or_default();
            return format!("{value}{}", &rest[end..]);
        }
    }
    path.to_owned()
}

fn is_absolute_config_path(path: &str) -> bool {
    let b = path.as_bytes();
    path.starts_with('/')
        || path.starts_with("\\\\")
        || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
        || path.find("://").is_some_and(|i| {
            i > 0
                && path[..i]
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'.')
        })
}

// Composer: Config.php get for '*-dir' keys, with RELATIVE_PATHS
fn config_dir_relative(raw: &str, vendor: Option<&str>, env: Env<'_>) -> String {
    let processed = match vendor {
        Some(v) => raw.replace("{$vendor-dir}", v),
        None => raw.to_owned(),
    };
    expand_path(processed.trim_end_matches(['/', '\\']), env)
}

// Composer: Config.php get for '*-dir' keys, without RELATIVE_PATHS
fn config_dir(raw: &str, base: &str, vendor: Option<&str>, env: Env<'_>) -> String {
    let expanded = config_dir_relative(raw, vendor, env);
    if is_absolute_config_path(&expanded) {
        expanded
    } else {
        format!("{base}/{expanded}")
    }
}

pub(crate) fn dirs(composer: &ComposerJson, root: &Path, env: Env<'_>) -> Result<Dirs, Error> {
    let base = path_string(root);
    let vendor_raw = non_empty(env("COMPOSER_VENDOR_DIR"))
        .or_else(|| config_str(composer, "vendor-dir"))
        .unwrap_or_else(|| "vendor".to_owned());
    let vendor = config_dir(&vendor_raw, &base, None, env);
    let vendor_relative = config_dir_relative(&vendor_raw, None, env);
    let bin_raw = non_empty(env("COMPOSER_BIN_DIR"))
        .or_else(|| config_str(composer, "bin-dir"))
        .unwrap_or_else(|| "{$vendor-dir}/bin".to_owned());
    let bin = config_dir(&bin_raw, &base, Some(&vendor), env);
    let compat = non_empty(env("COMPOSER_BIN_COMPAT"))
        .or_else(|| config_str(composer, "bin-compat"))
        .unwrap_or_else(|| "auto".to_owned());
    let full_bin_compat = match compat.as_str() {
        "full" => true,
        "auto" => cfg!(windows),
        "proxy" | "symlink" => false,
        other => {
            return Err(Error::install(format!(
                "Invalid value for 'bin-compat': {other}. Expected auto, full or proxy"
            )));
        }
    };
    Ok(Dirs {
        vendor,
        vendor_relative,
        bin,
        full_bin_compat,
    })
}

/// `composer` with `config.vendor-dir` pinned to `vendor`, so the metadata
/// and autoload writers see the same directory env vars and `{$refs}` gave.
pub(crate) fn with_vendor_dir(composer: ComposerJson, root: &Path, vendor: &str) -> ComposerJson {
    let default = normalize_path(&format!("{}/{}", path_string(root), composer.vendor_dir()));
    if default == normalize_path(vendor) {
        return composer;
    }
    let mut data = composer.data().clone();
    let config = data
        .entry("config")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !config.is_object() {
        *config = Value::Object(serde_json::Map::new());
    }
    if let Value::Object(map) = config {
        map.insert("vendor-dir".into(), Value::String(vendor.to_owned()));
    }
    ComposerJson::from_value(Value::Object(data)).unwrap_or(composer)
}

#[cfg(test)]
mod tests {
    use super::{dirs, expand_path, locate, with_vendor_dir};
    use phpm_lock::ComposerJson;
    use std::path::{Path, PathBuf};

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    fn json(text: &str) -> ComposerJson {
        ComposerJson::parse(text).unwrap()
    }

    #[test]
    fn finds_the_lock_next_to_the_json() {
        let root = PathBuf::from("/p");
        let f = locate(root.clone(), &env(&[]));
        assert_eq!(f.composer_file, Path::new("/p/composer.json"));
        assert_eq!(f.lock_file, Path::new("/p/composer.lock"));
        let f = locate(root.clone(), &env(&[("COMPOSER", " other.json ")]));
        assert_eq!(f.composer_file, Path::new("/p/other.json"));
        assert_eq!(f.lock_file, Path::new("/p/other.lock"));
        let f = locate(root, &env(&[("COMPOSER", "deps")]));
        assert_eq!(f.lock_file, Path::new("/p/deps.lock"));
    }

    #[test]
    fn resolves_vendor_and_bin_dirs() {
        let d = dirs(&json("{}"), Path::new("/p"), &env(&[])).unwrap();
        assert_eq!(d.vendor, "/p/vendor");
        assert_eq!(d.bin, "/p/vendor/bin");
        assert_eq!(d.full_bin_compat, cfg!(windows));

        let c = json(
            r#"{"config":{"vendor-dir":"lib/","bin-dir":"{$vendor-dir}/../tools","bin-compat":"full"}}"#,
        );
        let d = dirs(&c, Path::new("/p"), &env(&[])).unwrap();
        assert_eq!(d.vendor, "/p/lib");
        assert_eq!(d.vendor_relative, "lib");
        assert_eq!(d.bin, "/p/lib/../tools");
        assert!(d.full_bin_compat);

        let d = dirs(
            &c,
            Path::new("/p"),
            &env(&[
                ("COMPOSER_VENDOR_DIR", "/abs/v"),
                ("COMPOSER_BIN_DIR", "b"),
                ("COMPOSER_BIN_COMPAT", "proxy"),
            ]),
        )
        .unwrap();
        assert_eq!(d.vendor, "/abs/v");
        assert_eq!(d.bin, "/p/b");
        assert!(!d.full_bin_compat);

        let bad = json(r#"{"config":{"bin-compat":"nope"}}"#);
        assert!(dirs(&bad, Path::new("/p"), &env(&[])).is_err());
    }

    #[test]
    fn expands_home_and_env_vars() {
        let e = env(&[("HOME", "/home/me"), ("DEPS", "/deps")]);
        assert_eq!(expand_path("~/v", &e), "/home/me/v");
        assert_eq!(expand_path("$DEPS/v", &e), "/deps/v");
        assert_eq!(expand_path("$/v", &e), "$/v");
        assert_eq!(expand_path("~x", &e), "~x");
        let d = dirs(
            &json(r#"{"config":{"vendor-dir":"C:/v","bin-dir":"s3://x"}}"#),
            Path::new("/p"),
            &e,
        )
        .unwrap();
        assert_eq!((d.vendor.as_str(), d.bin.as_str()), ("C:/v", "s3://x"));
    }

    #[test]
    fn pins_vendor_dir_only_when_it_moved() {
        let c = json(r#"{"name":"a/b"}"#);
        let same = with_vendor_dir(c.clone(), Path::new("/p"), "/p/vendor");
        assert!(same.data().get("config").is_none());
        let moved = with_vendor_dir(c, Path::new("/p"), "/elsewhere");
        assert_eq!(moved.vendor_dir(), "/elsewhere");
        let odd = with_vendor_dir(json(r#"{"config":1}"#), Path::new("/p"), "/x");
        assert_eq!(odd.vendor_dir(), "/x");
    }
}
