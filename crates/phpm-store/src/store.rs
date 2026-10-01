//! `<cache>/pkgs/v1/<vendor~pkg>/<reference>/`: extracted trees, written once.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sha1::{Digest, Sha1};

use crate::error::{Error, IoContext, Result};
use crate::extract::{Limits, extract};
use crate::link::remove_tree;

/// Where extracted packages live, shared by every project on the machine.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// A store under `cache_dir` (the phpm cache root, not the bucket).
    pub fn new(cache_dir: &Path) -> Self {
        Self {
            root: cache_dir.join("pkgs").join("v1"),
        }
    }

    /// The store under [`cache_dir`].
    pub fn from_env() -> Result<Self> {
        Ok(Self::new(&cache_dir()?))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory holding `name` at `key` (a dist reference, see [`store_key`]).
    pub fn path(&self, name: &str, key: &str) -> Result<PathBuf> {
        Ok(self.root.join(name_dir(name)?).join(key_dir(key)))
    }

    pub fn contains(&self, name: &str, key: &str) -> bool {
        self.path(name, key).is_ok_and(|p| p.is_dir())
    }

    /// Extract a zip into the store. A no-op if another process got there first.
    pub fn insert_zip(&self, name: &str, key: &str, zip: &[u8]) -> Result<PathBuf> {
        self.insert_zip_with(name, key, zip, Limits::default())
    }

    pub(crate) fn insert_zip_with(
        &self,
        name: &str,
        key: &str,
        zip: &[u8],
        limits: Limits,
    ) -> Result<PathBuf> {
        let dest = self.path(name, key)?;
        if dest.is_dir() {
            return Ok(dest);
        }
        let tmp_root = self.root.join(".tmp");
        fs::create_dir_all(&tmp_root).at(&tmp_root)?;
        let tmp = tmp_root.join(unique_name());
        if let Err(e) = extract(name, zip, &tmp, limits) {
            let _ = remove_tree(&tmp);
            return Err(e);
        }
        publish(&tmp, &dest)?;
        Ok(dest)
    }
}

fn publish(tmp: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).at(parent)?;
    }
    match fs::rename(tmp, dest) {
        Ok(()) => Ok(()),
        Err(_) if dest.is_dir() => {
            let _ = remove_tree(tmp);
            Ok(())
        }
        Err(e) => {
            let _ = remove_tree(tmp);
            Err(e).at(dest)
        }
    }
}

fn unique_name() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!(
        "{}-{}-{nanos}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// The store key for a dist: its reference, or a hash of the URL when there is none.
pub fn store_key(reference: Option<&str>, url: &str) -> String {
    match reference {
        Some(r) if !r.is_empty() => r.to_owned(),
        _ => format!("url-{}", hex_sha1(url.as_bytes())),
    }
}

pub(crate) fn hex_sha1(bytes: &[u8]) -> String {
    use std::fmt::Write;
    Sha1::digest(bytes)
        .iter()
        .fold(String::with_capacity(40), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn name_dir(name: &str) -> Result<String> {
    let valid = name.split('/').count() == 2
        && name.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        });
    if valid {
        Ok(name.to_ascii_lowercase().replace('/', "~"))
    } else {
        Err(Error::InvalidPackage {
            package: name.to_owned(),
            reason: "not a vendor/package name".into(),
        })
    }
}

fn key_dir(key: &str) -> String {
    let plain = !key.is_empty()
        && key.len() <= 128
        && !key.starts_with('.')
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    if plain {
        key.to_owned()
    } else {
        format!("sha1-{}", hex_sha1(key.as_bytes()))
    }
}

/// `$PHPM_CACHE_DIR`, else the platform cache directory plus `phpm`.
pub fn cache_dir() -> Result<PathBuf> {
    cache_dir_from(|k| std::env::var_os(k)).ok_or(Error::NoCacheDir)
}

fn cache_dir_from(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let var = |k: &str| env(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(dir) = var("PHPM_CACHE_DIR") {
        return Some(dir);
    }
    if cfg!(windows) {
        return var("LOCALAPPDATA").map(|d| d.join("phpm"));
    }
    let home = var("HOME");
    if cfg!(target_os = "macos") {
        return home.map(|h| h.join("Library/Caches/phpm"));
    }
    var("XDG_CACHE_HOME")
        .filter(|p| p.is_absolute())
        .or_else(|| home.map(|h| h.join(".cache")))
        .map(|d| d.join("phpm"))
}

#[cfg(test)]
mod tests {
    use super::{Store, cache_dir_from, key_dir, name_dir, store_key};
    use crate::extract::Limits;
    use crate::testutil::{TempDir, ZipBuilder};
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn zip() -> Vec<u8> {
        ZipBuilder::new()
            .dos_dir("o-r-1/")
            .dos_file("o-r-1/a.php", b"<?php\n")
            .finish()
    }

    #[test]
    fn lays_out_by_name_and_reference() {
        let store = Store::new(&PathBuf::from("/c"));
        assert_eq!(
            store.path("Monolog/monolog", "abc123").unwrap(),
            PathBuf::from("/c/pkgs/v1/monolog~monolog/abc123")
        );
        assert!(store.path("../x", "abc").is_err());
        assert!(store.path("a/b/c", "abc").is_err());
        assert!(store.path("a/..", "abc").is_err());
    }

    #[test]
    fn hashes_keys_that_are_not_plain() {
        assert_eq!(key_dir("1.2.3"), "1.2.3");
        assert!(key_dir("../../x").starts_with("sha1-"));
        assert!(key_dir("").starts_with("sha1-"));
        assert_eq!(name_dir("a/b").unwrap(), "a~b");
    }

    #[test]
    fn falls_back_to_the_url_when_there_is_no_reference() {
        assert_eq!(store_key(Some("abc"), "u"), "abc");
        assert_eq!(
            store_key(None, "abc"),
            "url-a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(store_key(Some(""), "abc"), store_key(None, "abc"));
    }

    #[test]
    fn inserts_atomically_and_only_once() {
        let tmp = TempDir::new("store-insert");
        let store = Store::new(tmp.path());
        assert!(!store.contains("o/r", "1"));
        let dir = store.insert_zip("o/r", "1", &zip()).unwrap();
        assert!(store.contains("o/r", "1"));
        assert_eq!(std::fs::read(dir.join("a.php")).unwrap(), b"<?php\n");
        let again = store.insert_zip("o/r", "1", b"not even a zip").unwrap();
        assert_eq!(dir, again);
        let leftovers = std::fs::read_dir(store.root().join(".tmp"))
            .unwrap()
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn a_failed_extraction_leaves_nothing_behind() {
        let tmp = TempDir::new("store-fail");
        let store = Store::new(tmp.path());
        assert!(store.insert_zip("o/r", "1", b"garbage").is_err());
        assert!(!store.contains("o/r", "1"));
        let small = Limits {
            max_entries: 10,
            max_bytes: 1,
        };
        assert!(store.insert_zip_with("o/r", "1", &zip(), small).is_err());
        assert_eq!(
            std::fs::read_dir(store.root().join(".tmp"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn concurrent_writers_agree() {
        let tmp = TempDir::new("store-race");
        let store = Arc::new(Store::new(tmp.path()));
        let bytes = Arc::new(zip());
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let store = Arc::clone(&store);
                let bytes = Arc::clone(&bytes);
                std::thread::spawn(move || store.insert_zip("o/r", "1", &bytes).unwrap())
            })
            .collect();
        let dirs: Vec<PathBuf> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(dirs.windows(2).all(|w| w[0] == w[1]));
        assert_eq!(std::fs::read(dirs[0].join("a.php")).unwrap(), b"<?php\n");
        assert_eq!(
            std::fs::read_dir(store.root().join(".tmp"))
                .unwrap()
                .count(),
            0
        );
    }

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn phpm_cache_dir_wins() {
        assert_eq!(
            cache_dir_from(env(&[("PHPM_CACHE_DIR", "/x"), ("HOME", "/h")])),
            Some(PathBuf::from("/x"))
        );
        assert_eq!(cache_dir_from(env(&[])), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn uses_library_caches_on_macos() {
        assert_eq!(
            cache_dir_from(env(&[("HOME", "/Users/me"), ("PHPM_CACHE_DIR", "")])),
            Some(PathBuf::from("/Users/me/Library/Caches/phpm"))
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn uses_xdg_cache_home_on_linux() {
        assert_eq!(
            cache_dir_from(env(&[("HOME", "/home/me"), ("XDG_CACHE_HOME", "/xdg")])),
            Some(PathBuf::from("/xdg/phpm"))
        );
        assert_eq!(
            cache_dir_from(env(&[("HOME", "/home/me"), ("XDG_CACHE_HOME", "rel")])),
            Some(PathBuf::from("/home/me/.cache/phpm"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn uses_local_app_data_on_windows() {
        assert_eq!(
            cache_dir_from(env(&[("LOCALAPPDATA", r"C:\Users\me\AppData\Local")])),
            Some(PathBuf::from(r"C:\Users\me\AppData\Local").join("phpm"))
        );
    }
}
