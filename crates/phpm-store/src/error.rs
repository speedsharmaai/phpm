use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// Everything that can go wrong between a lock entry and a placed package.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Download {
        url: String,
        reason: String,
    },
    Checksum {
        url: String,
        expected: String,
        actual: String,
    },
    Archive {
        package: String,
        reason: String,
    },
    InvalidPackage {
        package: String,
        reason: String,
    },
    Auth {
        source: String,
        reason: String,
    },
    NoCacheDir,
    Task(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Download { url, reason } => write!(f, "could not download {url}: {reason}"),
            Self::Checksum {
                url,
                expected,
                actual,
            } => write!(
                f,
                "checksum mismatch for {url}: expected sha1 {expected}, got {actual}"
            ),
            Self::Archive { package, reason } => write!(f, "cannot extract {package}: {reason}"),
            Self::InvalidPackage { package, reason } => write!(f, "{package}: {reason}"),
            Self::Auth { source, reason } => write!(f, "invalid auth config in {source}: {reason}"),
            Self::NoCacheDir => f.write_str(
                "cannot find a cache directory; set PHPM_CACHE_DIR or HOME (LOCALAPPDATA on Windows)",
            ),
            Self::Task(reason) => write!(f, "background task failed: {reason}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub(crate) trait IoContext<T> {
    fn at(self, path: &Path) -> Result<T>;
}

impl<T> IoContext<T> for io::Result<T> {
    fn at(self, path: &Path) -> Result<T> {
        self.map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, IoContext};
    use std::error::Error as _;
    use std::io;
    use std::path::Path;

    #[test]
    fn io_errors_carry_the_path() {
        let err = Err::<(), _>(io::Error::from(io::ErrorKind::NotFound))
            .at(Path::new("vendor/a/b"))
            .unwrap_err();
        assert!(err.to_string().starts_with("vendor/a/b: "));
        assert!(err.source().is_some());
    }

    #[test]
    fn messages_name_what_failed() {
        let cases = [
            (
                Error::Download {
                    url: "https://x/y.zip".into(),
                    reason: "HTTP 404".into(),
                },
                "could not download https://x/y.zip: HTTP 404",
            ),
            (
                Error::Checksum {
                    url: "u".into(),
                    expected: "a".into(),
                    actual: "b".into(),
                },
                "checksum mismatch for u: expected sha1 a, got b",
            ),
            (
                Error::Archive {
                    package: "a/b".into(),
                    reason: "bad".into(),
                },
                "cannot extract a/b: bad",
            ),
            (
                Error::InvalidPackage {
                    package: "a/b".into(),
                    reason: "bad".into(),
                },
                "a/b: bad",
            ),
            (
                Error::Auth {
                    source: "COMPOSER_AUTH".into(),
                    reason: "bad".into(),
                },
                "invalid auth config in COMPOSER_AUTH: bad",
            ),
            (Error::Task("boom".into()), "background task failed: boom"),
        ];
        for (err, msg) in cases {
            assert_eq!(err.to_string(), msg);
            assert!(err.source().is_none());
        }
        assert!(Error::NoCacheDir.to_string().contains("PHPM_CACHE_DIR"));
    }
}
