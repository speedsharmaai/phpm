//! Reads freshly placed files once, in parallel, before Composer scans them.
//! A clone starts with a cold page cache and Composer reads one file at a
//! time, so on the Laravel skeleton this halves the fallback's cost.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

// Composer: ClassMapGenerator scans these extensions
const EXTENSIONS: [&str; 3] = ["php", "inc", "hh"];

/// The files Composer's class scan reads for one package placed at `dir`:
/// its `classmap` entries, plus its PSR-0/4 dirs when `psr` is set.
pub(crate) fn scan_roots(package: &Map<String, Value>, dir: &Path, psr: bool) -> Vec<PathBuf> {
    let Some(autoload) = package.get("autoload").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut roots = Vec::new();
    let mut add = |v: &Value| match v {
        Value::String(s) => roots.push(dir.join(s)),
        Value::Array(items) => {
            roots.extend(items.iter().filter_map(Value::as_str).map(|s| dir.join(s)));
        }
        _ => {}
    };
    if let Some(Value::Array(items)) = autoload.get("classmap") {
        items.iter().for_each(&mut add);
    }
    if psr {
        for key in ["psr-4", "psr-0"] {
            if let Some(Value::Object(map)) = autoload.get(key) {
                map.values().for_each(&mut add);
            }
        }
    }
    roots
}

fn collect(path: &Path, into: &mut Vec<PathBuf>) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    if meta.is_file() {
        if path
            .extension()
            .is_some_and(|e| EXTENSIONS.iter().any(|x| e == *x))
        {
            into.push(path.to_path_buf());
        }
        return;
    }
    if !meta.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        collect(&entry.path(), into);
    }
}

/// Read every PHP file under `roots`; returns how many were read.
pub(crate) fn warm(roots: &[PathBuf]) -> usize {
    let mut files = Vec::new();
    for root in roots {
        collect(root, &mut files);
    }
    let threads = std::thread::available_parallelism().map_or(4, usize::from);
    let chunk = files.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        for part in files.chunks(chunk) {
            scope.spawn(move || {
                for file in part {
                    let _ = fs::read(file);
                }
            });
        }
    });
    files.len()
}

#[cfg(test)]
mod tests {
    use super::{scan_roots, warm};
    use serde_json::{Map, Value, json};
    use std::path::Path;

    fn obj(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => unreachable!(),
        }
    }

    #[test]
    fn finds_what_composer_scans() {
        let p = obj(json!({"autoload": {
            "classmap": ["src/", "lib.php"],
            "psr-4": {"A\\": "a/", "B\\": ["b1", "b2"]},
            "psr-0": {"C": "c/"},
            "files": ["f.php"],
        }}));
        let dir = Path::new("/v/x");
        assert_eq!(
            scan_roots(&p, dir, false),
            [dir.join("src/"), dir.join("lib.php")]
        );
        assert_eq!(scan_roots(&p, dir, true).len(), 6);
        assert!(scan_roots(&obj(json!({})), dir, true).is_empty());
        let odd = obj(json!({"autoload": {"classmap": "src", "psr-4": {"A\\": 1}}}));
        assert!(scan_roots(&odd, dir, true).is_empty());
    }

    #[test]
    fn reads_php_files_under_the_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        for f in ["src/a.php", "src/deep/b.inc", "src/c.txt", "one.hh"] {
            std::fs::write(root.join(f), b"<?php").unwrap();
        }
        let roots = [root.join("src"), root.join("one.hh"), root.join("missing")];
        assert_eq!(warm(&roots), 3);
        assert_eq!(warm(&[]), 0);
    }
}
