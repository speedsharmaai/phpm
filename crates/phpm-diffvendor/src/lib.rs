//! Byte-level comparison of two directory trees.
//!
//! Used to check phpm's `vendor/` against Composer's: every path must exist on
//! both sides with the same kind, the same bytes and the same Unix mode.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    File { bytes: Vec<u8>, mode: u32 },
    Dir,
    Symlink(PathBuf),
}

/// One way the two trees disagree about a relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Difference {
    OnlyInLeft(PathBuf),
    OnlyInRight(PathBuf),
    KindDiffers(PathBuf),
    ContentDiffers {
        path: PathBuf,
        offset: usize,
    },
    ModeDiffers {
        path: PathBuf,
        left: u32,
        right: u32,
    },
    SymlinkTargetDiffers(PathBuf),
}

impl fmt::Display for Difference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OnlyInLeft(p) => write!(f, "only in left:  {}", p.display()),
            Self::OnlyInRight(p) => write!(f, "only in right: {}", p.display()),
            Self::KindDiffers(p) => write!(f, "kind differs:  {}", p.display()),
            Self::ContentDiffers { path, offset } => {
                write!(
                    f,
                    "bytes differ:  {} (first at offset {offset})",
                    path.display()
                )
            }
            Self::ModeDiffers { path, left, right } => {
                write!(
                    f,
                    "mode differs:  {} ({left:o} vs {right:o})",
                    path.display()
                )
            }
            Self::SymlinkTargetDiffers(p) => write!(f, "link differs:  {}", p.display()),
        }
    }
}

/// Paths skipped on both sides, relative to the compared roots.
#[derive(Debug, Default, Clone)]
pub struct Ignore {
    names: Vec<String>,
}

impl Ignore {
    #[must_use]
    pub fn names(names: &[&str]) -> Self {
        Self {
            names: names.iter().map(|n| (*n).to_owned()).collect(),
        }
    }

    fn skips(&self, path: &Path) -> bool {
        path.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| self.names.iter().any(|i| i == n))
    }
}

/// Compare `left` and `right`, returning every difference in path order.
pub fn compare(left: &Path, right: &Path, ignore: &Ignore) -> io::Result<Vec<Difference>> {
    let a = snapshot(left, ignore)?;
    let b = snapshot(right, ignore)?;
    let mut out = Vec::new();

    for (path, left_entry) in &a {
        match b.get(path) {
            None => out.push(Difference::OnlyInLeft(path.clone())),
            Some(right_entry) => {
                if let Some(d) = compare_entry(path, left_entry, right_entry) {
                    out.push(d);
                }
            }
        }
    }
    for path in b.keys() {
        if !a.contains_key(path) {
            out.push(Difference::OnlyInRight(path.clone()));
        }
    }
    out.sort_by(|x, y| path_of(x).cmp(path_of(y)));
    Ok(out)
}

fn path_of(d: &Difference) -> &Path {
    match d {
        Difference::OnlyInLeft(p)
        | Difference::OnlyInRight(p)
        | Difference::KindDiffers(p)
        | Difference::SymlinkTargetDiffers(p)
        | Difference::ContentDiffers { path: p, .. }
        | Difference::ModeDiffers { path: p, .. } => p,
    }
}

fn compare_entry(path: &Path, left: &Entry, right: &Entry) -> Option<Difference> {
    match (left, right) {
        (Entry::Dir, Entry::Dir) => None,
        (Entry::Symlink(x), Entry::Symlink(y)) => {
            (x != y).then(|| Difference::SymlinkTargetDiffers(path.to_owned()))
        }
        (
            Entry::File {
                bytes: xb,
                mode: xm,
            },
            Entry::File {
                bytes: yb,
                mode: ym,
            },
        ) => {
            if xb != yb {
                Some(Difference::ContentDiffers {
                    path: path.to_owned(),
                    offset: first_difference(xb, yb),
                })
            } else if xm != ym {
                Some(Difference::ModeDiffers {
                    path: path.to_owned(),
                    left: *xm,
                    right: *ym,
                })
            } else {
                None
            }
        }
        _ => Some(Difference::KindDiffers(path.to_owned())),
    }
}

fn first_difference(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .unwrap_or_else(|| a.len().min(b.len()))
}

fn snapshot(root: &Path, ignore: &Ignore) -> io::Result<BTreeMap<PathBuf, Entry>> {
    let mut entries = BTreeMap::new();
    walk(root, Path::new(""), ignore, &mut entries)?;
    Ok(entries)
}

fn walk(
    root: &Path,
    rel: &Path,
    ignore: &Ignore,
    out: &mut BTreeMap<PathBuf, Entry>,
) -> io::Result<()> {
    for item in fs::read_dir(root.join(rel))? {
        let item = item?;
        let rel_path = rel.join(item.file_name());
        if ignore.skips(&rel_path) {
            continue;
        }
        let full = root.join(&rel_path);
        let meta = fs::symlink_metadata(&full)?;
        let kind = meta.file_type();
        if kind.is_symlink() {
            out.insert(rel_path, Entry::Symlink(fs::read_link(&full)?));
        } else if kind.is_dir() {
            out.insert(rel_path.clone(), Entry::Dir);
            walk(root, &rel_path, ignore, out)?;
        } else {
            out.insert(
                rel_path,
                Entry::File {
                    bytes: fs::read(&full)?,
                    mode: mode_of(&meta),
                },
            );
        }
    }
    Ok(())
}

#[cfg(unix)]
fn mode_of(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn mode_of(meta: &fs::Metadata) -> u32 {
    u32::from(meta.permissions().readonly())
}

#[cfg(test)]
mod tests {
    use super::{Difference, Ignore, compare};
    use std::fs;
    use std::path::{Path, PathBuf};

    struct TempTree(PathBuf);

    impl TempTree {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("phpm-diffvendor-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self, rel: &str, bytes: &[u8]) -> &Self {
            let path = self.0.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
            self
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn pair(name: &str) -> (TempTree, TempTree) {
        (
            TempTree::new(&format!("{name}-l")),
            TempTree::new(&format!("{name}-r")),
        )
    }

    #[test]
    fn identical_trees_have_no_differences() {
        let (l, r) = pair("same");
        l.file("composer/installed.json", b"{}\n");
        r.file("composer/installed.json", b"{}\n");
        assert!(compare(&l.0, &r.0, &Ignore::default()).unwrap().is_empty());
    }

    #[test]
    fn reports_missing_files_on_each_side() {
        let (l, r) = pair("missing");
        l.file("a.php", b"<?php");
        r.file("b.php", b"<?php");
        let diffs = compare(&l.0, &r.0, &Ignore::default()).unwrap();
        assert_eq!(
            diffs,
            vec![
                Difference::OnlyInLeft(Path::new("a.php").into()),
                Difference::OnlyInRight(Path::new("b.php").into()),
            ]
        );
    }

    #[test]
    fn reports_first_differing_byte() {
        let (l, r) = pair("bytes");
        l.file("autoload.php", b"array (\n");
        r.file("autoload.php", b"array ( \n");
        let diffs = compare(&l.0, &r.0, &Ignore::default()).unwrap();
        assert_eq!(
            diffs,
            vec![Difference::ContentDiffers {
                path: Path::new("autoload.php").into(),
                offset: 7
            }]
        );
    }

    #[test]
    fn ignored_names_are_skipped_on_both_sides() {
        let (l, r) = pair("ignore");
        l.file("composer/installed.json", b"{}");
        r.file("composer/installed.json", b"{}");
        r.file("composer/.vivace-state", b"x");
        let ignore = Ignore::names(&[".vivace-state"]);
        assert!(compare(&l.0, &r.0, &ignore).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn reports_mode_differences() {
        use std::os::unix::fs::PermissionsExt;
        let (l, r) = pair("mode");
        l.file("bin/tool", b"#!/bin/sh\n");
        r.file("bin/tool", b"#!/bin/sh\n");
        fs::set_permissions(l.0.join("bin/tool"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(r.0.join("bin/tool"), fs::Permissions::from_mode(0o644)).unwrap();
        let diffs = compare(&l.0, &r.0, &Ignore::default()).unwrap();
        assert_eq!(
            diffs,
            vec![Difference::ModeDiffers {
                path: Path::new("bin/tool").into(),
                left: 0o755,
                right: 0o644
            }]
        );
    }

    #[test]
    fn display_is_one_line_per_difference() {
        let d = Difference::ContentDiffers {
            path: Path::new("composer/installed.php").into(),
            offset: 12,
        };
        assert_eq!(
            d.to_string(),
            "bytes differ:  composer/installed.php (first at offset 12)"
        );
    }
}
