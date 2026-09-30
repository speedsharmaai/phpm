use crate::version::InvalidVersion;
use std::fmt;

/// Why phpm cannot reproduce what Composer would write.
#[derive(Debug)]
pub enum Error {
    Json(serde_json::Error),
    NotAnObject(&'static str),
    InvalidPackage { package: String, reason: String },
    InvalidVersion(InvalidVersion),
    UnsupportedVcs(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "invalid JSON: {e}"),
            Self::NotAnObject(what) => write!(f, "{what} is not a JSON object"),
            Self::InvalidPackage { package, reason } => write!(f, "package {package}: {reason}"),
            Self::InvalidVersion(e) => e.fmt(f),
            Self::UnsupportedVcs(vcs) => {
                write!(f, "cannot guess the root version from a {vcs} checkout")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(e) => Some(e),
            Self::InvalidVersion(e) => Some(e),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

impl From<InvalidVersion> for Error {
    fn from(e: InvalidVersion) -> Self {
        Self::InvalidVersion(e)
    }
}

#[cfg(test)]
mod tests {
    use super::Error;
    use crate::version::InvalidVersion;
    use std::error::Error as _;

    #[test]
    fn messages_name_the_problem() {
        let json = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        let e = Error::from(json);
        assert!(e.to_string().starts_with("invalid JSON"));
        assert!(e.source().is_some());
        assert_eq!(
            Error::NotAnObject("composer.lock").to_string(),
            "composer.lock is not a JSON object"
        );
        let e = Error::InvalidPackage {
            package: "a/b".into(),
            reason: "no name".into(),
        };
        assert_eq!(e.to_string(), "package a/b: no name");
        assert!(e.source().is_none());
        let e = Error::from(InvalidVersion("x".into()));
        assert_eq!(e.to_string(), "Invalid version string \"x\"");
        assert!(e.source().is_some());
        assert_eq!(
            Error::UnsupportedVcs("hg".into()).to_string(),
            "cannot guess the root version from a hg checkout"
        );
    }
}
