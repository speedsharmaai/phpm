//! Placing store trees into `vendor/` and removing what the lock no longer has.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use crate::error::{Error, IoContext, Result};

/// How a package directory gets from the store into `vendor/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LinkMode {
    /// Copy-on-write: `clonefile` per directory on macOS, `FICLONE` per file on Linux.
    Clone,
    /// Hard links per file. Editing `vendor/` in place edits the store.
    Hardlink,
    /// Plain copies.
    Copy,
}

impl LinkMode {
    /// Clone on macOS and Linux, hardlink elsewhere; slower modes are tried on failure.
    pub fn platform_default() -> Self {
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            Self::Clone
        } else {
            Self::Hardlink
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Clone,
            1 => Self::Hardlink,
            _ => Self::Copy,
        }
    }

    fn as_u8(self) -> u8 {
        match self {
            Self::Clone => 0,
            Self::Hardlink => 1,
            Self::Copy => 2,
        }
    }
}

/// One package to place: its tree in the store and its path under `vendor/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub source: PathBuf,
    pub install_path: PathBuf,
}

/// Place every package, replacing whatever is at its install path.
///
/// Runs on all cores. Returns the slowest mode that had to be used, so a
/// caller can say "fell back to copy" once instead of per package.
pub fn place(vendor_dir: &Path, packages: &[Placement], mode: LinkMode) -> Result<LinkMode> {
    place_with(vendor_dir, packages, mode, true)
}

/// Like [`place`], but a clone that fails falls back to copies, never to hard
/// links: for installs where scripts or Composer will run in `vendor/` and a
/// write through a hard link would change the store for every project.
pub fn place_unshared(
    vendor_dir: &Path,
    packages: &[Placement],
    mode: LinkMode,
) -> Result<LinkMode> {
    place_with(vendor_dir, packages, mode, false)
}

/// The mode to try after `mode` failed.
fn next_mode(mode: LinkMode, hardlinks: bool) -> LinkMode {
    match mode {
        LinkMode::Clone if hardlinks => LinkMode::Hardlink,
        LinkMode::Clone | LinkMode::Hardlink | LinkMode::Copy => LinkMode::Copy,
    }
}

fn place_with(
    vendor_dir: &Path,
    packages: &[Placement],
    mode: LinkMode,
    hardlinks: bool,
) -> Result<LinkMode> {
    for p in packages {
        check_install_path(&p.install_path)?;
    }
    let effective = AtomicU8::new(mode.as_u8());
    let next = AtomicUsize::new(0);
    let first_error: Mutex<Option<Error>> = Mutex::new(None);
    let workers = std::thread::available_parallelism()
        .map_or(4, std::num::NonZero::get)
        .min(packages.len().max(1));

    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(p) = packages.get(i) else { break };
                    let dst = vendor_dir.join(&p.install_path);
                    if let Err(e) = place_one(&p.source, &dst, &effective, hardlinks) {
                        let mut slot = first_error
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        slot.get_or_insert(e);
                        next.store(packages.len(), Ordering::Relaxed);
                        break;
                    }
                }
            });
        }
    });

    match first_error
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
    {
        Some(e) => Err(e),
        None => Ok(LinkMode::from_u8(effective.load(Ordering::Relaxed))),
    }
}

fn check_install_path(path: &Path) -> Result<()> {
    let ok = path.components().count() > 0
        && path.components().all(|c| matches!(c, Component::Normal(_)));
    if ok {
        Ok(())
    } else {
        Err(Error::InvalidPackage {
            package: path.display().to_string(),
            reason: "install path must be relative to vendor/ and stay inside it".into(),
        })
    }
}

fn place_one(src: &Path, dst: &Path, effective: &AtomicU8, hardlinks: bool) -> Result<()> {
    if fs::symlink_metadata(dst).is_ok() {
        remove_tree(dst).at(dst)?;
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).at(parent)?;
    }
    loop {
        let mode = LinkMode::from_u8(effective.load(Ordering::Relaxed));
        let result = match mode {
            LinkMode::Clone => clone_package(src, dst),
            LinkMode::Hardlink => copy_tree(src, dst, &|s, d| fs::hard_link(s, d)),
            LinkMode::Copy => copy_tree(src, dst, &|s, d| fs::copy(s, d).map(drop)),
        };
        match result {
            Ok(()) => return Ok(()),
            Err(e) if mode != LinkMode::Copy && can_fall_back(mode, &e) => {
                let _ = remove_tree(dst);
                effective.fetch_max(next_mode(mode, hardlinks).as_u8(), Ordering::Relaxed);
            }
            Err(e) => return Err(e).at(dst),
        }
    }
}

fn can_fall_back(mode: LinkMode, err: &io::Error) -> bool {
    match mode {
        LinkMode::Clone => crate::sys::clone_unsupported(err),
        LinkMode::Hardlink => err.kind() != io::ErrorKind::NotFound,
        LinkMode::Copy => false,
    }
}

#[cfg(target_os = "macos")]
fn clone_package(src: &Path, dst: &Path) -> io::Result<()> {
    crate::sys::clone_tree(src, dst)
}

#[cfg(target_os = "linux")]
fn clone_package(src: &Path, dst: &Path) -> io::Result<()> {
    copy_tree(src, dst, &|s, d| {
        use std::os::unix::fs::PermissionsExt;
        let from = fs::File::open(s)?;
        let mode = from.metadata()?.permissions().mode();
        let to = fs::File::create(d)?;
        crate::sys::ficlone(&from, &to)?;
        to.set_permissions(fs::Permissions::from_mode(mode))
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn clone_package(_src: &Path, _dst: &Path) -> io::Result<()> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

type FileOp = dyn Fn(&Path, &Path) -> io::Result<()> + Sync;

fn copy_tree(src: &Path, dst: &Path, file: &FileOp) -> io::Result<()> {
    fs::create_dir(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&from, &to, file)?;
        } else if kind.is_symlink() {
            copy_link(&from, &to)?;
        } else {
            file(&from, &to)?;
        }
    }
    fs::set_permissions(dst, fs::metadata(src)?.permissions())
}

#[cfg(unix)]
fn copy_link(from: &Path, to: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(fs::read_link(from)?, to)
}

#[cfg(not(unix))]
fn copy_link(from: &Path, to: &Path) -> io::Result<()> {
    fs::copy(from, to).map(drop)
}

/// Remove `path` recursively, including read-only directories from odd archives.
pub(crate) fn remove_tree(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() {
        return fs::remove_file(path);
    }
    match fs::remove_dir_all(path) {
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            make_writable(path)?;
            fs::remove_dir_all(path)
        }
        other => other,
    }
}

#[cfg(unix)]
fn make_writable(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(dir)?.permissions().mode();
    fs::set_permissions(dir, fs::Permissions::from_mode(mode | 0o700))?;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            make_writable(&entry.path())?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn make_writable(dir: &Path) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let mut perms = entry.metadata()?.permissions();
        #[allow(
            clippy::permissions_set_readonly_false,
            reason = "Windows: read-only attribute blocks deletion"
        )]
        perms.set_readonly(false); // NOSONAR: only to delete our own vendor files on Windows
        fs::set_permissions(entry.path(), perms)?;
        if entry.file_type()?.is_dir() {
            make_writable(&entry.path())?;
        }
    }
    Ok(())
}

const RESERVED: [&str; 2] = ["composer", "bin"];

/// Remove package directories under `vendor/` that are not in `keep`.
///
/// Looks two levels deep (`vendor/<vendor>/<package>`), never touches
/// `vendor/composer`, `vendor/bin`, dot entries or top-level files, and keeps
/// every ancestor of a kept install path. Returns what was removed, sorted.
pub fn prune(vendor_dir: &Path, keep: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let kept: BTreeSet<&Path> = keep.iter().map(PathBuf::as_path).collect();
    let ancestors: BTreeSet<&Path> = keep
        .iter()
        .flat_map(|p| p.ancestors().skip(1))
        .filter(|a| !a.as_os_str().is_empty())
        .collect();
    let mut removed = Vec::new();

    for top in sorted_dirs(vendor_dir)? {
        let name = top.as_os_str().to_string_lossy();
        if RESERVED.contains(&name.as_ref())
            || name.starts_with('.')
            || kept.contains(top.as_path())
        {
            continue;
        }
        if !ancestors.contains(top.as_path()) {
            remove_tree(&vendor_dir.join(&top)).at(&vendor_dir.join(&top))?;
            removed.push(top);
            continue;
        }
        for child in sorted_dirs(&vendor_dir.join(&top))? {
            let rel = top.join(&child);
            if kept.contains(rel.as_path()) || ancestors.contains(rel.as_path()) {
                continue;
            }
            remove_tree(&vendor_dir.join(&rel)).at(&vendor_dir.join(&rel))?;
            removed.push(rel);
        }
    }
    removed.sort();
    Ok(removed)
}

fn sorted_dirs(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e).at(dir),
    };
    for entry in entries {
        let entry = entry.at(dir)?;
        if entry.file_type().at(&entry.path())?.is_dir() {
            out.push(PathBuf::from(entry.file_name()));
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::remove_tree;
    use super::{LinkMode, Placement, next_mode, place, prune};
    use crate::testutil::TempDir;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn package(root: &Path, name: &str) -> PathBuf {
        let dir = root.join("store").join(name.replace('/', "~"));
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("composer.json"),
            format!("{{\"name\":\"{name}\"}}"),
        )
        .unwrap();
        fs::write(dir.join("src/A.php"), b"<?php\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::create_dir_all(dir.join("bin")).unwrap();
            fs::write(dir.join("bin/tool"), b"#!/bin/sh\n").unwrap();
            fs::set_permissions(dir.join("bin/tool"), fs::Permissions::from_mode(0o755)).unwrap();
            std::os::unix::fs::symlink("../src/A.php", dir.join("bin/link")).unwrap();
        }
        dir
    }

    fn placements(root: &Path, names: &[&str]) -> Vec<Placement> {
        names
            .iter()
            .map(|n| Placement {
                source: package(root, n),
                install_path: PathBuf::from(n),
            })
            .collect()
    }

    fn assert_placed(vendor: &Path, name: &str) {
        let dir = vendor.join(name);
        assert_eq!(fs::read(dir.join("src/A.php")).unwrap(), b"<?php\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join("bin/tool"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755);
            assert_eq!(
                fs::read_link(dir.join("bin/link")).unwrap(),
                Path::new("../src/A.php")
            );
        }
    }

    #[test]
    fn a_failed_clone_skips_hard_links_when_asked() {
        assert_eq!(next_mode(LinkMode::Clone, true), LinkMode::Hardlink);
        assert_eq!(next_mode(LinkMode::Clone, false), LinkMode::Copy);
        assert_eq!(next_mode(LinkMode::Hardlink, true), LinkMode::Copy);
        assert_eq!(next_mode(LinkMode::Copy, false), LinkMode::Copy);
    }

    #[cfg(unix)]
    #[test]
    fn unshared_placement_never_shares_inodes_with_the_store() {
        use super::place_unshared;
        use std::os::unix::fs::MetadataExt;
        let tmp = TempDir::new("link-unshared");
        let vendor = tmp.path().join("vendor");
        let list = placements(tmp.path(), &["a/one"]);
        let used = place_unshared(&vendor, &list, LinkMode::Clone).unwrap();
        assert_ne!(used, LinkMode::Hardlink);
        let placed = fs::metadata(vendor.join("a/one/src/A.php")).unwrap();
        let stored = fs::metadata(list[0].source.join("src/A.php")).unwrap();
        assert_ne!(placed.ino(), stored.ino());
        fs::write(vendor.join("a/one/src/A.php"), b"changed").unwrap();
        assert_eq!(
            fs::read(list[0].source.join("src/A.php")).unwrap(),
            b"<?php\n"
        );
    }

    #[test]
    fn places_packages_in_every_mode() {
        for mode in [LinkMode::Clone, LinkMode::Hardlink, LinkMode::Copy] {
            let tmp = TempDir::new("link-modes");
            let vendor = tmp.path().join("vendor");
            let list = placements(tmp.path(), &["a/one", "a/two", "b/three"]);
            let used = place(&vendor, &list, mode).unwrap();
            assert!(used >= mode);
            for n in ["a/one", "a/two", "b/three"] {
                assert_placed(&vendor, n);
            }
        }
    }

    #[test]
    fn copies_are_independent_of_the_store() {
        let tmp = TempDir::new("link-independent");
        let vendor = tmp.path().join("vendor");
        let list = placements(tmp.path(), &["a/one"]);
        let used = place(&vendor, &list, LinkMode::platform_default()).unwrap();
        if used != LinkMode::Hardlink {
            fs::write(vendor.join("a/one/src/A.php"), b"patched").unwrap();
            assert_eq!(
                fs::read(list[0].source.join("src/A.php")).unwrap(),
                b"<?php\n"
            );
        }
    }

    #[test]
    fn replaces_an_existing_install() {
        let tmp = TempDir::new("link-replace");
        let vendor = tmp.path().join("vendor");
        fs::create_dir_all(vendor.join("a/one/old")).unwrap();
        fs::write(vendor.join("a/one/old/x.php"), b"old").unwrap();
        let list = placements(tmp.path(), &["a/one"]);
        place(&vendor, &list, LinkMode::Copy).unwrap();
        assert!(!vendor.join("a/one/old").exists());
        assert_placed(&vendor, "a/one");
    }

    #[test]
    fn rejects_install_paths_outside_vendor() {
        let tmp = TempDir::new("link-escape");
        let source = package(tmp.path(), "a/one");
        for bad in ["../x", "/abs", ""] {
            let list = [Placement {
                source: source.clone(),
                install_path: PathBuf::from(bad),
            }];
            assert!(place(&tmp.path().join("vendor"), &list, LinkMode::Copy).is_err());
        }
    }

    #[test]
    fn reports_a_missing_source() {
        let tmp = TempDir::new("link-missing");
        let list = [Placement {
            source: tmp.path().join("nope"),
            install_path: PathBuf::from("a/b"),
        }];
        for mode in [LinkMode::Clone, LinkMode::Hardlink, LinkMode::Copy] {
            assert!(place(&tmp.path().join("vendor"), &list, mode).is_err());
        }
    }

    #[test]
    fn prunes_packages_missing_from_the_lock() {
        let tmp = TempDir::new("link-prune");
        let vendor = tmp.path().join("vendor");
        for dir in [
            "a/keep",
            "a/stale",
            "gone/pkg",
            "composer",
            "bin",
            ".cache",
            "deep/x/Target/Dir",
        ] {
            fs::create_dir_all(vendor.join(dir)).unwrap();
        }
        fs::write(vendor.join("autoload.php"), b"<?php").unwrap();
        let keep = [PathBuf::from("a/keep"), PathBuf::from("deep/x/Target/Dir")];
        let removed = prune(&vendor, &keep).unwrap();
        assert_eq!(removed, [PathBuf::from("a/stale"), PathBuf::from("gone")]);
        for dir in [
            "a/keep",
            "composer",
            "bin",
            ".cache",
            "deep/x/Target/Dir",
            "autoload.php",
        ] {
            assert!(vendor.join(dir).exists(), "{dir}");
        }
        assert!(
            prune(&tmp.path().join("missing"), &keep)
                .unwrap()
                .is_empty()
        );
    }

    #[cfg(unix)]
    #[test]
    fn removes_read_only_trees() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new("link-readonly");
        let dir = tmp.path().join("ro");
        fs::create_dir_all(dir.join("inner")).unwrap();
        fs::write(dir.join("inner/f"), b"x").unwrap();
        fs::set_permissions(dir.join("inner"), fs::Permissions::from_mode(0o555)).unwrap();
        remove_tree(&dir).unwrap();
        assert!(!dir.exists());
    }
}
