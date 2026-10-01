//! Class scan results cached per store tree. Store trees never change, so a
//! warm `-o` install reads one small file per package instead of opening
//! every PHP file in `vendor/` again.
//!
//! `<cache>/classes/v1/<binary stamp>/<vendor~pkg>/<reference>`: one line per
//! PHP file, `path<TAB>Class<TAB>Class...`. The binary stamp keys the cache
//! to the phpm build that wrote it, so a scanner fix never reads stale results.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use phpm_autoload::scan::file_classes;
use sha2::{Digest, Sha256};

const EXTENSIONS: [&str; 3] = [".php", ".inc", ".hh"];

/// One placed package: its tree in the store, its cache file, and where it
/// sits in `vendor/` (a real path, forward slashes).
#[derive(Debug, Clone)]
pub(crate) struct Tree {
    pub(crate) store_dir: PathBuf,
    pub(crate) cache_file: PathBuf,
    pub(crate) vendor_dir: String,
}

/// The cache directory for this phpm build under `cache_dir`.
pub(crate) fn cache_root(cache_dir: &Path) -> PathBuf {
    let stamp = std::env::current_exe()
        .and_then(fs::metadata)
        .map(|m| {
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            format!("{}-{}-{mtime}", env!("CARGO_PKG_VERSION"), m.len())
        })
        .unwrap_or_default();
    let digest = Sha256::digest(stamp.as_bytes());
    let short = digest[..8].iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    });
    cache_dir.join("classes").join("v1").join(short)
}

fn scan_tree(dir: &Path, rel: &str, out: &mut String) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let child = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        let kind = entry.file_type()?;
        if kind.is_dir() {
            scan_tree(&entry.path(), &child, out)?;
        } else if kind.is_file()
            && EXTENSIONS.iter().any(|e| name.ends_with(e))
            && !child.contains(['\t', '\n', '\r'])
        {
            let classes = file_classes(&fs::read(entry.path())?);
            out.push_str(&child);
            for class in classes {
                out.push('\t');
                out.push_str(&class);
            }
            out.push('\n');
        }
    }
    Ok(())
}

fn build(tree: &Tree) -> io::Result<String> {
    let mut text = String::new();
    scan_tree(&tree.store_dir, "", &mut text)?;
    if let Some(dir) = tree.cache_file.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = tree
        .cache_file
        .with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, &text)?;
    fs::rename(&tmp, &tree.cache_file)?;
    Ok(text)
}

fn parse(text: &str, vendor_dir: &str, out: &mut Vec<(String, Vec<String>)>) {
    for line in text.lines() {
        let mut parts = line.split('\t');
        if let Some(rel) = parts.next() {
            out.push((
                format!("{vendor_dir}/{rel}"),
                parts.map(str::to_owned).collect(),
            ));
        }
    }
}

/// Every cached scan for `trees`, building what is missing on all cores.
/// A tree that cannot be cached is left out, and the autoloader reads it.
pub(crate) fn known_classes(trees: &[Tree]) -> HashMap<String, Vec<String>> {
    let next = AtomicUsize::new(0);
    let found: Mutex<Vec<(String, Vec<String>)>> = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism()
        .map_or(4, std::num::NonZero::get)
        .min(trees.len().max(1));
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                let mut local = Vec::new();
                while let Some(tree) = trees.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let text = fs::read_to_string(&tree.cache_file).or_else(|_| build(tree));
                    if let Ok(text) = text {
                        parse(&text, &tree.vendor_dir, &mut local);
                    }
                }
                found
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .extend(local);
            });
        }
    });
    found
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Tree, cache_root, known_classes};
    use std::fs;
    use std::path::Path;

    #[test]
    fn builds_once_then_reads_the_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store/a~b/ref");
        fs::create_dir_all(store.join("src/Deep")).unwrap();
        fs::write(store.join("src/A.php"), "<?php namespace X; class A {}").unwrap();
        fs::write(
            store.join("src/Deep/B.inc"),
            "<?php interface B {} enum E {}",
        )
        .unwrap();
        fs::write(store.join("src/none.php"), "<?php echo 1;").unwrap();
        fs::write(store.join("README.md"), "class NotPhp {}").unwrap();
        let tree = Tree {
            store_dir: store.clone(),
            cache_file: tmp.path().join("cache/a~b/ref"),
            vendor_dir: "/v/a/b".into(),
        };
        let known = known_classes(std::slice::from_ref(&tree));
        assert_eq!(known["/v/a/b/src/A.php"], ["X\\A"]);
        assert_eq!(known["/v/a/b/src/Deep/B.inc"], ["B", "E"]);
        assert!(known["/v/a/b/src/none.php"].is_empty());
        assert_eq!(known.len(), 3);
        assert!(tree.cache_file.is_file());

        fs::write(store.join("src/A.php"), "<?php class Changed {}").unwrap();
        let again = known_classes(std::slice::from_ref(&tree));
        assert_eq!(again["/v/a/b/src/A.php"], ["X\\A"], "read from the cache");
    }

    #[test]
    fn leaves_out_trees_it_cannot_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let tree = Tree {
            store_dir: tmp.path().join("missing"),
            cache_file: tmp.path().join("cache/x"),
            vendor_dir: "/v/x".into(),
        };
        assert!(known_classes(&[tree]).is_empty());
        assert!(known_classes(&[]).is_empty());
    }

    #[test]
    fn keys_the_cache_by_build() {
        let root = cache_root(Path::new("/c"));
        assert!(root.starts_with("/c/classes/v1"));
        assert_eq!(root, cache_root(Path::new("/c")));
    }
}
