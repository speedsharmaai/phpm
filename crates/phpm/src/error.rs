use std::fmt;
use std::path::Path;
use std::process::ExitCode;

/// A failed run: what to print and how to exit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Error {
    pub(crate) message: String,
    pub(crate) code: u8,
}

impl Error {
    pub(crate) fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code,
        }
    }

    /// Exit code 1: the install could not be done.
    pub(crate) fn install(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 1,
        }
    }

    /// Exit code 2: the command line or project setup is not usable.
    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 2,
        }
    }

    pub(crate) fn io(path: &Path, err: &std::io::Error) -> Self {
        Self::install(format!("{}: {err}", path.display()))
    }

    pub(crate) fn exit_code(&self) -> ExitCode {
        ExitCode::from(self.code)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<phpm_store::Error> for Error {
    fn from(e: phpm_store::Error) -> Self {
        Self::install(e.to_string())
    }
}

impl From<phpm_lock::Error> for Error {
    fn from(e: phpm_lock::Error) -> Self {
        Self::install(e.to_string())
    }
}

impl From<phpm_autoload::Error> for Error {
    fn from(e: phpm_autoload::Error) -> Self {
        Self::install(format!("autoloader: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::Error;
    use std::path::Path;

    #[test]
    fn carries_exit_codes() {
        assert_eq!(Error::install("x").code, 1);
        assert_eq!(Error::usage("x").code, 2);
        assert_eq!(Error::new(5, "x").code, 5);
        let e = Error::io(Path::new("/a"), &std::io::Error::other("boom"));
        assert_eq!(e.to_string(), "/a: boom");
        let _ = e.exit_code();
        let e = Error::from(phpm_store::Error::NoCacheDir);
        assert!(e.message.contains("PHPM_CACHE_DIR"));
        let e = Error::from(phpm_lock::Error::NotAnObject("composer.lock"));
        assert_eq!(e.to_string(), "composer.lock is not a JSON object");
        let e = Error::from(phpm_autoload::Error::Unsupported("x".into()));
        assert_eq!(e.to_string(), "autoloader: not supported yet: x");
    }
}
