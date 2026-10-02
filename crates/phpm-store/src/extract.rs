//! Zip extraction matching Composer's `ArchiveDownloader` + `unzip -qq`.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};

use zip::ZipArchive;
use zip::read::HasZipMetadata;

use crate::error::{Error, IoContext, Result};

const S_IFMT: u32 = 0o170_000;
const S_IFLNK: u32 = 0o120_000;
const DOS_READONLY: u32 = 0x01;
const MAX_LINK_TARGET: u64 = 4096;

/// Caps that stop a hostile archive from filling the disk.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) max_entries: usize,
    pub(crate) max_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: 200_000,
            max_bytes: 2 << 30,
        }
    }
}

#[cfg_attr(not(unix), allow(dead_code, reason = "file modes only exist on unix"))]
enum Mode {
    /// Unix mode from the archive, applied as is (setuid and friends dropped).
    Exact(u32),
    /// No Unix mode: 0666/0777 (0444/0555 if DOS read-only), minus umask.
    Default { readonly: bool },
}

impl Mode {
    fn of(system: zip::System, attrs: u32) -> (Self, bool) {
        let unix = attrs >> 16;
        if unix != 0 {
            return (Self::Exact(unix & 0o777), unix & S_IFMT == S_IFLNK);
        }
        let readonly = system != zip::System::Unix && attrs & DOS_READONLY != 0;
        (Self::Default { readonly }, false)
    }
}

pub(crate) struct Entry {
    pub(crate) index: usize,
    pub(crate) rel: PathBuf,
    pub(crate) is_dir: bool,
}

/// Extract `bytes` into `dest`, which must not exist yet.
///
/// A single top-level directory is unwrapped, as `ArchiveDownloader::install`
/// does; a top-level `.DS_Store` does not count and is dropped with it.
pub(crate) fn extract(package: &str, bytes: &[u8], dest: &Path, limits: Limits) -> Result<()> {
    let archive_err = |reason: String| Error::Archive {
        package: package.to_owned(),
        reason,
    };
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).map_err(|e| archive_err(e.to_string()))?;
    if archive.len() > limits.max_entries {
        return Err(archive_err(format!(
            "{} entries is more than the limit of {}",
            archive.len(),
            limits.max_entries
        )));
    }

    let mut entries = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let file = archive
            .by_index_raw(index)
            .map_err(|e| archive_err(e.to_string()))?;
        let rel = safe_relative(file.name()).map_err(archive_err)?;
        entries.push(Entry {
            index,
            rel,
            is_dir: file.is_dir(),
        });
    }
    let strip = single_top_dir(&entries);

    fs::create_dir(dest).at(dest)?;
    let mut budget = limits.max_bytes;
    let mut dir_modes: Vec<(PathBuf, Mode)> = Vec::new();
    let mut links: BTreeSet<PathBuf> = BTreeSet::new();

    for entry in &entries {
        let rel = match &strip {
            Some(top) => match entry.rel.strip_prefix(top) {
                Ok(rest) => rest.to_owned(),
                Err(_) => continue,
            },
            None => entry.rel.clone(),
        };
        if rel.ancestors().skip(1).any(|a| links.contains(a)) {
            return Err(archive_err(format!(
                "{} is written through a symlink",
                entry.rel.display()
            )));
        }
        let mut file = archive
            .by_index(entry.index)
            .map_err(|e| archive_err(e.to_string()))?;
        let meta = file.get_metadata();
        let (mode, is_link) = Mode::of(meta.system, meta.external_attributes);
        let path = dest.join(&rel);

        if entry.is_dir {
            fs::create_dir_all(&path).at(&path)?;
            dir_modes.push((rel, mode));
            continue;
        }
        if rel.as_os_str().is_empty() {
            return Err(archive_err("a file entry has an empty name".into()));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).at(parent)?;
        }
        if is_link {
            let mut target = String::new();
            file.by_ref()
                .take(MAX_LINK_TARGET)
                .read_to_string(&mut target)
                .map_err(|e| archive_err(e.to_string()))?;
            check_link(&rel, &target).map_err(archive_err)?;
            write_link(&target, &path)?;
            links.insert(rel);
            continue;
        }
        let mut out = create_file(&path, &mode)?;
        let written = io::copy(&mut file.by_ref().take(budget.saturating_add(1)), &mut out)
            .map_err(|e| archive_err(format!("{}: {e}", entry.rel.display())))?;
        if written > budget {
            return Err(archive_err(format!(
                "more than {} bytes uncompressed",
                limits.max_bytes
            )));
        }
        budget -= written;
        out.flush().at(&path)?;
        set_file_mode(&out, &mode).at(&path)?;
    }

    dir_modes.sort_by_key(|(rel, _)| std::cmp::Reverse(rel.components().count()));
    for (rel, mode) in &dir_modes {
        let path = dest.join(rel);
        set_dir_mode(&path, mode).at(&path)?;
    }
    Ok(())
}

/// Normalises a zip entry name, rejecting anything that could leave the package.
pub(crate) fn safe_relative(name: &str) -> Result<PathBuf, String> {
    if name.starts_with('/') || name.contains('\0') {
        return Err(format!("unsafe path {name:?}"));
    }
    let mut out = PathBuf::new();
    for part in name.split('/') {
        let part = sanitize_component(part);
        match part.as_ref() {
            "" | "." => {}
            ".." => return Err(format!("unsafe path {name:?}")),
            part => out.push(part),
        }
    }
    if out.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(format!("unsafe path {name:?}"));
    }
    Ok(out)
}

/// A path component as Windows can hold it. Composer's `ZipDownloader`
/// extracts through 7-Zip there (`windows-latest` ships it), which
/// substitutes characters a Windows path cannot hold rather than failing
/// the archive; off Windows every byte here is legal, so this is a no-op.
///
/// 7-Zip: CPP/7zip/UI/Common/ExtractingFilePath.cpp `ReplaceIncorrectChars`,
/// `Correct_PathPart`, `CorrectUnsupportedName`, `IsSupportedName`.
fn sanitize_component(part: &str) -> std::borrow::Cow<'_, str> {
    #[cfg(not(windows))]
    {
        std::borrow::Cow::Borrowed(part)
    }
    #[cfg(windows)]
    {
        if matches!(part, "" | "." | "..") {
            return std::borrow::Cow::Borrowed(part);
        }
        let mut chars: Vec<char> = part
            .chars()
            .map(|c| {
                if matches!(c, ':' | '*' | '?' | '<' | '>' | '|' | '"' | '\\') || (c as u32) < 0x20
                {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        let mut i = chars.len();
        while i > 0 && matches!(chars[i - 1], '.' | ' ') {
            chars[i - 1] = '_';
            i -= 1;
        }
        if is_reserved_device_name(&chars) {
            chars.insert(0, '_');
        }
        std::borrow::Cow::Owned(if chars.is_empty() {
            "_".to_owned()
        } else {
            chars.into_iter().collect()
        })
    }
}

/// `CON`, `PRN`, `AUX`, `NUL`, `COM1`-`COM9`, `LPT1`-`LPT9`, with or without
/// an extension: Windows reserves these regardless of what follows the dot.
#[cfg(windows)]
fn is_reserved_device_name(chars: &[char]) -> bool {
    const PLAIN: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    const NUMBERED: [&str; 2] = ["COM", "LPT"];
    let name: String = chars.iter().collect();
    let after_prefix = |prefix: &str, needs_digit: bool| -> Option<usize> {
        if name.len() < prefix.len() || !name[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return None;
        }
        if !needs_digit {
            return Some(prefix.len());
        }
        name[prefix.len()..]
            .chars()
            .next()
            .filter(char::is_ascii_digit)
            .map(|_| prefix.len() + 1)
    };
    let candidates = PLAIN
        .iter()
        .map(|p| (*p, false))
        .chain(NUMBERED.iter().map(|p| (*p, true)));
    for (prefix, needs_digit) in candidates {
        let Some(rest_start) = after_prefix(prefix, needs_digit) else {
            continue;
        };
        let rest = name[rest_start..].trim_start_matches(' ');
        if rest.is_empty() || rest.starts_with('.') {
            return true;
        }
    }
    false
}

// Composer: ArchiveDownloader::install, getFolderContent() ignores .DS_Store.
pub(crate) fn single_top_dir(entries: &[Entry]) -> Option<PathBuf> {
    let mut tops: BTreeSet<&std::ffi::OsStr> = BTreeSet::new();
    let mut dirs: BTreeSet<&std::ffi::OsStr> = BTreeSet::new();
    for entry in entries {
        let mut parts = entry.rel.components();
        let Some(first) = parts.next() else { continue };
        let first = first.as_os_str();
        if first == ".DS_Store" {
            continue;
        }
        tops.insert(first);
        if entry.is_dir || parts.next().is_some() {
            dirs.insert(first);
        }
    }
    match (tops.len(), tops.first()) {
        (1, Some(top)) if dirs.contains(top) => Some(PathBuf::from(top)),
        _ => None,
    }
}

fn check_link(rel: &Path, target: &str) -> Result<(), String> {
    let escapes = || format!("symlink {} -> {target} leaves the package", rel.display());
    if target.is_empty() || target.starts_with('/') || target.contains('\0') {
        return Err(escapes());
    }
    let mut depth: usize = rel.components().count().saturating_sub(1);
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => depth = depth.checked_sub(1).ok_or_else(escapes)?,
            _ => depth += 1,
        }
    }
    Ok(())
}

#[cfg(unix)]
fn write_link(target: &str, path: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, path).at(path)
}

// Composer on Windows has no symlinks either; the link text becomes the file.
#[cfg(not(unix))]
fn write_link(target: &str, path: &Path) -> Result<()> {
    fs::write(path, target).at(path)
}

#[cfg(unix)]
fn create_file(path: &Path, mode: &Mode) -> Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let base = match mode {
        Mode::Default { readonly: true } => 0o444,
        _ => 0o666,
    };
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(base)
        .open(path)
        .at(path)
}

#[cfg(not(unix))]
fn create_file(path: &Path, _mode: &Mode) -> Result<fs::File> {
    fs::File::create(path).at(path)
}

#[cfg(unix)]
fn set_file_mode(file: &fs::File, mode: &Mode) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    match mode {
        Mode::Exact(bits) => file.set_permissions(fs::Permissions::from_mode(*bits)),
        Mode::Default { .. } => Ok(()),
    }
}

#[cfg(not(unix))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "same signature as the unix version"
)]
fn set_file_mode(_file: &fs::File, _mode: &Mode) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_dir_mode(path: &Path, mode: &Mode) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let bits = match mode {
        Mode::Exact(bits) => *bits,
        Mode::Default { readonly: true } => fs::metadata(path)?.permissions().mode() & 0o555,
        Mode::Default { readonly: false } => return Ok(()),
    };
    fs::set_permissions(path, fs::Permissions::from_mode(bits))
}

#[cfg(not(unix))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "same signature as the unix version"
)]
fn set_dir_mode(_path: &Path, _mode: &Mode) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Limits, check_link, extract, safe_relative};
    use crate::testutil::{TempDir, ZipBuilder};
    use std::fs;
    use std::path::Path;

    fn names(root: &Path) -> Vec<String> {
        let mut out = Vec::new();
        walk(root, root, &mut out);
        out.sort();
        out
    }

    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            let rel = p
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if fs::symlink_metadata(&p).unwrap().is_dir() {
                out.push(format!("{rel}/"));
                walk(root, &p, out);
            } else {
                out.push(rel);
            }
        }
    }

    fn github_zip() -> Vec<u8> {
        ZipBuilder::new()
            .dos_dir("owner-repo-abc1234/")
            .dos_file("owner-repo-abc1234/README.md", b"# hi\n")
            .dos_dir("owner-repo-abc1234/src/")
            .dos_file("owner-repo-abc1234/src/A.php", b"<?php\n")
            .unix_file(
                "owner-repo-abc1234/bin/tool",
                0o100_755,
                b"#!/usr/bin/env php\n",
            )
            .finish()
    }

    #[test]
    fn strips_the_single_top_level_directory() {
        let tmp = TempDir::new("extract-strip");
        let dest = tmp.path().join("pkg");
        extract("a/b", &github_zip(), &dest, Limits::default()).unwrap();
        assert_eq!(
            names(&dest),
            ["README.md", "bin/", "bin/tool", "src/", "src/A.php"]
        );
        assert_eq!(fs::read(dest.join("src/A.php")).unwrap(), b"<?php\n");
    }

    #[test]
    fn keeps_everything_when_there_are_several_top_level_entries() {
        let tmp = TempDir::new("extract-nostrip");
        let zip = ZipBuilder::new()
            .dos_file("composer.json", b"{}")
            .dos_file("src/A.php", b"<?php\n")
            .finish();
        let dest = tmp.path().join("pkg");
        extract("a/b", &zip, &dest, Limits::default()).unwrap();
        assert_eq!(names(&dest), ["composer.json", "src/", "src/A.php"]);
    }

    #[test]
    fn a_single_top_level_file_is_not_unwrapped() {
        let tmp = TempDir::new("extract-single-file");
        let zip = ZipBuilder::new().dos_file("only.php", b"x").finish();
        let dest = tmp.path().join("pkg");
        extract("a/b", &zip, &dest, Limits::default()).unwrap();
        assert_eq!(names(&dest), ["only.php"]);
    }

    #[test]
    fn ignores_top_level_ds_store_when_unwrapping() {
        let tmp = TempDir::new("extract-dsstore");
        let zip = ZipBuilder::new()
            .dos_file(".DS_Store", b"junk")
            .dos_file("pkg-1/src/.DS_Store", b"kept")
            .dos_file("pkg-1/src/A.php", b"<?php\n")
            .finish();
        let dest = tmp.path().join("pkg");
        extract("a/b", &zip, &dest, Limits::default()).unwrap();
        assert_eq!(names(&dest), ["src/", "src/.DS_Store", "src/A.php"]);
    }

    #[test]
    fn keeps_top_level_ds_store_when_not_unwrapping() {
        let tmp = TempDir::new("extract-dsstore-keep");
        let zip = ZipBuilder::new()
            .dos_file(".DS_Store", b"junk")
            .dos_file("a.php", b"x")
            .dos_file("b.php", b"y")
            .finish();
        let dest = tmp.path().join("pkg");
        extract("a/b", &zip, &dest, Limits::default()).unwrap();
        assert_eq!(names(&dest), [".DS_Store", "a.php", "b.php"]);
    }

    #[test]
    fn a_dotfile_next_to_the_top_dir_prevents_unwrapping() {
        let tmp = TempDir::new("extract-dotfile");
        let zip = ZipBuilder::new()
            .dos_file(".gitattributes", b"x")
            .dos_file("pkg-1/a.php", b"y")
            .finish();
        let dest = tmp.path().join("pkg");
        extract("a/b", &zip, &dest, Limits::default()).unwrap();
        assert_eq!(names(&dest), [".gitattributes", "pkg-1/", "pkg-1/a.php"]);
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
    }

    #[cfg(unix)]
    fn umasked(bits: u32) -> u32 {
        let tmp = TempDir::new("umask-probe");
        let probe = tmp.path().join("p");
        fs::create_dir(&probe).unwrap();
        bits & mode(&probe)
    }

    #[cfg(unix)]
    #[test]
    fn keeps_modes_as_unzip_does() {
        let tmp = TempDir::new("extract-modes");
        let zip = ZipBuilder::new()
            .dos_dir("top/")
            .dos_file("top/plain.php", b"x")
            .dos_file_attrs("top/readonly.txt", 0x01, b"x")
            .dos_file_attrs("top/archive.txt", 0x20, b"x")
            .unix_file("top/exec", 0o100_755, b"x")
            .unix_file("top/group", 0o100_664, b"x")
            .unix_file("top/setuid", 0o104_755, b"x")
            .dos_dir_attrs("top/ro/", 0x11)
            .dos_file("top/ro/f", b"x")
            .unix_dir("top/private/", 0o040_750)
            .dos_file("top/implicit/f", b"x")
            .finish();
        let dest = tmp.path().join("pkg");
        extract("a/b", &zip, &dest, Limits::default()).unwrap();
        assert_eq!(mode(&dest), umasked(0o777));
        assert_eq!(mode(&dest.join("plain.php")), umasked(0o666));
        assert_eq!(mode(&dest.join("readonly.txt")), umasked(0o444));
        assert_eq!(mode(&dest.join("archive.txt")), umasked(0o666));
        assert_eq!(mode(&dest.join("exec")), 0o755);
        assert_eq!(mode(&dest.join("group")), 0o664);
        assert_eq!(mode(&dest.join("setuid")), 0o755);
        assert_eq!(mode(&dest.join("ro")), umasked(0o555));
        assert_eq!(mode(&dest.join("private")), 0o750);
        assert_eq!(mode(&dest.join("implicit")), umasked(0o777));
        crate::link::remove_tree(&dest).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_top_dir_mode_becomes_the_package_dir_mode() {
        let tmp = TempDir::new("extract-topmode");
        let zip = ZipBuilder::new()
            .unix_dir("top/", 0o040_700)
            .dos_file("top/a", b"x")
            .finish();
        let dest = tmp.path().join("pkg");
        extract("a/b", &zip, &dest, Limits::default()).unwrap();
        assert_eq!(mode(&dest), 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn creates_symlinks_that_stay_inside_the_package() {
        let tmp = TempDir::new("extract-links");
        let zip = ZipBuilder::new()
            .dos_file("top/src/real.php", b"x")
            .unix_file("top/bin/tool", 0o120_777, b"../src/real.php")
            .finish();
        let dest = tmp.path().join("pkg");
        extract("a/b", &zip, &dest, Limits::default()).unwrap();
        assert_eq!(
            fs::read_link(dest.join("bin/tool")).unwrap(),
            Path::new("../src/real.php")
        );
        assert_eq!(fs::read(dest.join("bin/tool")).unwrap(), b"x");
    }

    #[test]
    fn rejects_symlinks_that_escape() {
        let tmp = TempDir::new("extract-badlink");
        for target in ["../../etc/passwd", "/etc/passwd", "a/../../.."] {
            let zip = ZipBuilder::new()
                .dos_file("top/a", b"x")
                .unix_file("top/link", 0o120_777, target.as_bytes())
                .finish();
            let dest = tmp.path().join(format!("pkg{}", target.len()));
            let err = extract("a/b", &zip, &dest, Limits::default()).unwrap_err();
            assert!(err.to_string().contains("leaves the package"), "{err}");
        }
    }

    #[test]
    fn rejects_writing_through_a_symlink() {
        let tmp = TempDir::new("extract-through-link");
        let zip = ZipBuilder::new()
            .unix_file("top/link", 0o120_777, b".")
            .dos_file("top/link/evil", b"x")
            .finish();
        let err = extract("a/b", &zip, &tmp.path().join("pkg"), Limits::default()).unwrap_err();
        assert!(err.to_string().contains("through a symlink"), "{err}");
    }

    #[test]
    fn rejects_path_traversal() {
        let tmp = TempDir::new("extract-traversal");
        for name in ["../evil.php", "top/../../evil.php", "/etc/evil"] {
            let zip = ZipBuilder::new().dos_file(name, b"x").finish();
            let err = extract("a/b", &zip, &tmp.path().join("pkg"), Limits::default()).unwrap_err();
            assert!(err.to_string().contains("unsafe path"), "{err}");
            assert!(!tmp.path().join("evil.php").exists());
        }
    }

    #[test]
    fn enforces_size_and_entry_limits() {
        let tmp = TempDir::new("extract-limits");
        let zip = ZipBuilder::new()
            .dos_file("a", &[b'x'; 64])
            .dos_file("b", &[b'y'; 64])
            .finish();
        let small = Limits {
            max_entries: 10,
            max_bytes: 100,
        };
        let err = extract("a/b", &zip, &tmp.path().join("p1"), small).unwrap_err();
        assert!(err.to_string().contains("more than 100 bytes"), "{err}");
        let few = Limits {
            max_entries: 1,
            max_bytes: 1000,
        };
        let err = extract("a/b", &zip, &tmp.path().join("p2"), few).unwrap_err();
        assert!(err.to_string().contains("limit of 1"), "{err}");
    }

    #[test]
    fn rejects_garbage() {
        let tmp = TempDir::new("extract-garbage");
        let err = extract(
            "a/b",
            b"not a zip",
            &tmp.path().join("p"),
            Limits::default(),
        )
        .unwrap_err();
        assert!(err.to_string().starts_with("cannot extract a/b"));
    }

    #[test]
    fn normalises_entry_names() {
        assert_eq!(safe_relative("a//./b").unwrap(), Path::new("a/b"));
        assert!(safe_relative("a/\0b").is_err());
        assert!(check_link(Path::new("a/b"), "../c").is_ok());
        assert!(check_link(Path::new("a/b"), "").is_err());
    }

    // invoiceninja/invoiceninja's real dist archive has an entry literally
    // named "index.html?D=A", a query string some tool appended to a URL
    // before it was zipped; `?` is not valid in a Windows filename.
    #[cfg(windows)]
    #[test]
    fn sanitizes_windows_invalid_characters_like_7_zip() {
        assert_eq!(
            safe_relative("public/index.html?D=A").unwrap(),
            Path::new("public/index.html_D=A")
        );
        assert_eq!(
            safe_relative("a/weird<>:|\"*name.txt").unwrap(),
            Path::new("a/weird______name.txt")
        );
        assert_eq!(
            safe_relative("a/back\\slash").unwrap(),
            Path::new("a/back_slash")
        );
        assert_eq!(
            safe_relative("a/trailing. ").unwrap(),
            Path::new("a/trailing__")
        );
        assert_eq!(safe_relative("a/???").unwrap(), Path::new("a/___"));
        // "." and ".." stay exactly themselves (dropped/rejected upstream),
        // never run through the trailing-dot sanitizer.
        assert_eq!(safe_relative("a/./b").unwrap(), Path::new("a/b"));
        assert!(safe_relative("a/../b").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn prefixes_reserved_device_names_like_7_zip() {
        for reserved in ["CON", "con", "NUL", "NUL.txt", "COM1", "LPT9.log"] {
            assert_eq!(
                safe_relative(&format!("a/{reserved}")).unwrap(),
                Path::new("a").join(format!("_{reserved}"))
            );
        }
        for not_reserved in ["CONSOLE", "COM10", "COMPANY.txt", "NULL"] {
            assert_eq!(
                safe_relative(&format!("a/{not_reserved}")).unwrap(),
                Path::new("a").join(not_reserved)
            );
        }
    }

    #[test]
    fn sibling_at_top_level_keeps_a_subdir_from_being_unwrapped() {
        let tmp = TempDir::new("extract-no-unwrap-check");
        let zip = ZipBuilder::new()
            .dos_file("composer.json", b"{}")
            .dos_file("public/index.html", b"<html></html>")
            .finish();
        let dest = tmp.path().join("pkg");
        extract("a/b", &zip, &dest, Limits::default()).unwrap();
        assert_eq!(
            fs::read(dest.join("public/index.html")).unwrap(),
            b"<html></html>"
        );
    }

    #[cfg(windows)]
    #[test]
    fn extracts_a_windows_invalid_name_instead_of_failing() {
        let tmp = TempDir::new("extract-windows-names");
        // A sibling at the top level keeps "public/" from being unwrapped
        // as the archive's lone top-level directory (see
        // `strips_the_single_top_level_directory`), matching a real
        // invoiceninja-sized archive rather than this one entry alone.
        let zip = ZipBuilder::new()
            .dos_file("composer.json", b"{}")
            .dos_file("public/index.html?D=A", b"<html></html>")
            .finish();
        let dest = tmp.path().join("pkg");
        extract("a/b", &zip, &dest, Limits::default()).unwrap();
        assert_eq!(
            fs::read(dest.join("public/index.html_D=A")).unwrap(),
            b"<html></html>"
        );
    }
}
