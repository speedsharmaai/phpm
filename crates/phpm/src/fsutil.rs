//! Small filesystem helpers shared by the install steps.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// A path as Composer prints it: forward slashes.
pub(crate) fn path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// `realpath()`: `canonicalize` without the `\\?\` prefix Windows adds, which
/// PHP never shows and which breaks every path built on top of it.
pub(crate) fn canonical(path: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(path).map(without_verbatim_prefix)
}

pub(crate) fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path,
    }
}

pub(crate) fn read_head(path: &Path, len: u64) -> io::Result<Vec<u8>> {
    let mut head = Vec::new();
    fs::File::open(path)?.take(len).read_to_end(&mut head)?;
    Ok(head)
}

/// Write `bytes` unless the file already holds exactly them.
pub(crate) fn write_if_changed(path: &Path, bytes: &[u8]) -> io::Result<()> {
    // `fs::write` follows a symlink and overwrites whatever it points to;
    // a bin proxy that used to be a symlink (an older Composer, or one
    // hand-made) needs removing first so the write lands on `path` itself.
    if fs::symlink_metadata(path).is_ok_and(|m| m.is_symlink()) {
        fs::remove_file(path)?;
    } else if fs::read(path).is_ok_and(|old| old == bytes) {
        return Ok(());
    }
    fs::write(path, bytes)
}

/// The mode Composer gives executables: `0777 & ~umask()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Modes {
    exec: u32,
}

impl Modes {
    #[cfg(all(test, unix))]
    pub(crate) fn with_exec(exec: u32) -> Self {
        Self { exec }
    }

    /// Learn the umask by creating a 0777 file in `dir`: no `unsafe`, no race
    /// with other threads reading the process umask.
    #[cfg(unix)]
    pub(crate) fn probe(dir: &Path) -> io::Result<Self> {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        fs::create_dir_all(dir)?;
        let probe = dir.join(format!(".phpm-umask-{}", std::process::id()));
        let _ = fs::remove_file(&probe);
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o777) // NOSONAR: empty probe file, removed at once; the kernel masks it with the umask being measured
            .open(&probe)?;
        let mode = file.metadata().map(|m| m.permissions().mode() & 0o777);
        drop(file);
        fs::remove_file(&probe)?;
        Ok(Self { exec: mode? })
    }

    #[cfg(not(unix))]
    #[allow(
        clippy::unnecessary_wraps,
        clippy::unused_self,
        reason = "file modes only exist on unix"
    )]
    pub(crate) fn probe(_dir: &Path) -> io::Result<Self> {
        Ok(Self { exec: 0o777 })
    }

    #[cfg(unix)]
    pub(crate) fn set_exec(self, path: &Path) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(path)?.permissions().mode() & 0o7777 != self.exec {
            fs::set_permissions(path, fs::Permissions::from_mode(self.exec))?;
        }
        Ok(())
    }

    #[cfg(not(unix))]
    #[allow(
        clippy::unnecessary_wraps,
        clippy::unused_self,
        reason = "file modes only exist on unix"
    )]
    pub(crate) fn set_exec(self, _path: &Path) -> io::Result<()> {
        Ok(())
    }

    /// Like [`Modes::set_exec`], but a file hard-linked from the store is
    /// copied first so the store's inode keeps its mode.
    #[cfg(unix)]
    pub(crate) fn set_exec_unshared(self, path: &Path) -> io::Result<()> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let meta = fs::metadata(path)?;
        if meta.permissions().mode() & 0o7777 == self.exec {
            return Ok(());
        }
        if meta.nlink() > 1 {
            let tmp = path.with_file_name(format!(
                ".{}.phpm-tmp",
                path.file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default()
            ));
            fs::copy(path, &tmp)?;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(self.exec))?;
            return fs::rename(&tmp, path);
        }
        fs::set_permissions(path, fs::Permissions::from_mode(self.exec))
    }

    #[cfg(not(unix))]
    #[allow(
        clippy::unnecessary_wraps,
        clippy::unused_self,
        reason = "file modes only exist on unix"
    )]
    pub(crate) fn set_exec_unshared(self, _path: &Path) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::Modes;
    use super::{canonical, path_string, read_head, without_verbatim_prefix, write_if_changed};
    use std::fs;
    use std::path::{Path, PathBuf};

    #[test]
    fn real_paths_never_start_with_the_verbatim_prefix() {
        let plain = |s: &str| without_verbatim_prefix(PathBuf::from(s));
        assert_eq!(plain(r"\\?\C:\proj"), PathBuf::from(r"C:\proj"));
        assert_eq!(
            plain(r"\\?\UNC\host\share"),
            PathBuf::from(r"\\?\UNC\host\share")
        );
        assert_eq!(plain("/srv/proj"), PathBuf::from("/srv/proj"));
        let here = canonical(Path::new(".")).unwrap();
        assert!(!here.to_string_lossy().starts_with(r"\\?\"));
        assert!(here.is_absolute());
    }

    #[test]
    fn reads_only_the_head() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("f");
        fs::write(&f, b"0123456789").unwrap();
        assert_eq!(read_head(&f, 4).unwrap(), b"0123");
        assert!(read_head(&tmp.path().join("missing"), 4).is_err());
        assert_eq!(path_string(Path::new("a/b")), "a/b");
    }

    #[test]
    fn leaves_identical_files_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("f");
        write_if_changed(&f, b"a").unwrap();
        let before = fs::metadata(&f).unwrap().modified().unwrap();
        write_if_changed(&f, b"a").unwrap();
        assert_eq!(fs::metadata(&f).unwrap().modified().unwrap(), before);
        write_if_changed(&f, b"b").unwrap();
        assert_eq!(fs::read(&f).unwrap(), b"b");
    }

    #[cfg(unix)]
    #[test]
    fn replaces_a_symlink_instead_of_writing_through_it() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        fs::write(&target, b"real content").unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        write_if_changed(&link, b"proxy").unwrap();
        assert!(!fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(fs::read(&link).unwrap(), b"proxy");
        assert_eq!(fs::read(&target).unwrap(), b"real content");
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).unwrap().permissions().mode() & 0o7777
    }

    #[cfg(unix)]
    #[test]
    fn probes_the_umask() {
        let tmp = tempfile::tempdir().unwrap();
        let modes = Modes::probe(tmp.path()).unwrap();
        assert_eq!(modes.exec & !0o777, 0);
        assert_eq!(modes.exec & 0o700, 0o700);
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn copies_hard_linked_files_before_chmod() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let vendor = tmp.path().join("vendor");
        fs::write(&store, b"#!/bin/sh\n").unwrap();
        fs::hard_link(&store, &vendor).unwrap();
        let modes = Modes::with_exec(0o755);
        modes.set_exec_unshared(&vendor).unwrap();
        assert_eq!(mode(&vendor), 0o755);
        assert_ne!(mode(&store), 0o755);
        assert_eq!(fs::read(&vendor).unwrap(), b"#!/bin/sh\n");
        modes.set_exec_unshared(&vendor).unwrap();

        let plain = tmp.path().join("plain");
        fs::write(&plain, b"x").unwrap();
        modes.set_exec_unshared(&plain).unwrap();
        assert_eq!(mode(&plain), 0o755);
        modes.set_exec(&store).unwrap();
        assert_eq!(mode(&store), 0o755);
    }
}
