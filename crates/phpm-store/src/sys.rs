//! The only FFI in phpm: whole-directory clones on macOS, file reflinks on Linux.

#![allow(
    unsafe_code,
    reason = "clonefile(2) and ioctl(FICLONE) have no safe std wrapper"
)]

use std::io;

/// Clone `src` (a directory tree) to `dst`, which must not exist. One syscall.
#[cfg(target_os = "macos")]
pub(crate) fn clone_tree(src: &std::path::Path, dst: &std::path::Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let src = CString::new(src.as_os_str().as_bytes())?;
    let dst = CString::new(dst.as_os_str().as_bytes())?;
    // SAFETY: both pointers are valid NUL-terminated strings that outlive the call.
    let rc = unsafe { libc::clonefile(src.as_ptr(), dst.as_ptr(), 0) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Make `dst` share `src`'s extents (reflink). Both must be open files.
#[cfg(target_os = "linux")]
pub(crate) fn ficlone(src: &std::fs::File, dst: &std::fs::File) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    // SAFETY: both descriptors are open for the duration of the call.
    let rc = unsafe { libc::ioctl(dst.as_raw_fd(), libc::FICLONE, src.as_raw_fd()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Errors that mean "this filesystem cannot clone", so a slower mode should be tried.
pub(crate) fn clone_unsupported(err: &io::Error) -> bool {
    #[cfg(unix)]
    {
        const CODES: [i32; 5] = [
            libc::ENOTSUP,
            libc::EOPNOTSUPP,
            libc::EXDEV,
            libc::EINVAL,
            libc::ENOTTY,
        ];
        err.raw_os_error().is_some_and(|c| CODES.contains(&c))
    }
    #[cfg(not(unix))]
    {
        err.kind() == io::ErrorKind::Unsupported
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::clone_unsupported;
    #[cfg(unix)]
    use std::io;

    #[cfg(unix)]
    #[test]
    fn treats_cross_device_as_unsupported() {
        assert!(clone_unsupported(&io::Error::from_raw_os_error(
            libc::EXDEV
        )));
        assert!(!clone_unsupported(&io::Error::from_raw_os_error(
            libc::ENOENT
        )));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn clones_a_directory_tree_in_one_call() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = crate::testutil::TempDir::new("sys-clone");
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("bin")).unwrap();
        std::fs::write(src.join("bin/tool"), b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(src.join("bin/tool"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        let dst = tmp.path().join("dst");
        super::clone_tree(&src, &dst).unwrap();
        assert_eq!(std::fs::read(dst.join("bin/tool")).unwrap(), b"#!/bin/sh\n");
        let mode = std::fs::metadata(dst.join("bin/tool"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
        let err = super::clone_tree(&src, &dst).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn ficlone_succeeds_or_reports_unsupported() {
        let tmp = crate::testutil::TempDir::new("sys-ficlone");
        let src_path = tmp.path().join("a");
        std::fs::write(&src_path, b"hello").unwrap();
        let src = std::fs::File::open(&src_path).unwrap();
        let dst = std::fs::File::create(tmp.path().join("b")).unwrap();
        match super::ficlone(&src, &dst) {
            Ok(()) => assert_eq!(std::fs::read(tmp.path().join("b")).unwrap(), b"hello"),
            Err(e) => assert!(clone_unsupported(&e), "{e}"),
        }
    }
}
