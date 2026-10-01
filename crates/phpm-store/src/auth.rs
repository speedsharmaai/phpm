//! Credentials from `auth.json` and `COMPOSER_AUTH`, the subset the spike needs.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::{Error, Result};

/// Credentials by host, merged from every source Composer reads.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Auth {
    github_oauth: BTreeMap<String, String>,
    http_basic: BTreeMap<String, (String, String)>,
}

/// What to send for one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    /// `Authorization: token <t>`, as Composer sends for `github-oauth`.
    GithubToken(String),
    Basic {
        username: String,
        password: String,
    },
}

impl Auth {
    /// `COMPOSER_HOME/auth.json`, then `<project>/auth.json`, then `COMPOSER_AUTH`;
    /// later sources override earlier ones per host.
    pub fn load(project_dir: Option<&Path>) -> Result<Self> {
        Self::load_from(project_dir, |k| std::env::var_os(k), &|d| d.is_dir())
    }

    fn load_from(
        project_dir: Option<&Path>,
        env: impl Fn(&str) -> Option<OsString>,
        is_dir: &dyn Fn(&Path) -> bool,
    ) -> Result<Self> {
        let mut auth = Self::default();
        let mut files: Vec<PathBuf> = Vec::new();
        if let Some(home) = composer_home(&env, is_dir) {
            files.push(home.join("auth.json"));
        }
        if let Some(dir) = project_dir {
            files.push(dir.join("auth.json"));
        }
        for file in files {
            match std::fs::read_to_string(&file) {
                Ok(text) => auth.merge_json(&file.display().to_string(), &text)?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => return Err(Error::Io { path: file, source }),
            }
        }
        if let Some(text) = env("COMPOSER_AUTH").filter(|v| !v.is_empty()) {
            auth.merge_json("COMPOSER_AUTH", &text.to_string_lossy())?;
        }
        Ok(auth)
    }

    /// Merge one `auth.json`-shaped document. `source` only labels errors.
    pub fn merge_json(&mut self, source: &str, text: &str) -> Result<()> {
        let bad = |reason: &str| Error::Auth {
            source: source.to_owned(),
            reason: reason.to_owned(),
        };
        let doc: Value = serde_json::from_str(text).map_err(|e| bad(&e.to_string()))?;
        let Value::Object(doc) = doc else {
            return Err(bad("expected a JSON object"));
        };
        if let Some(section) = doc.get("github-oauth") {
            let map = section
                .as_object()
                .ok_or_else(|| bad("github-oauth must be an object"))?;
            for (host, token) in map {
                let token = token
                    .as_str()
                    .ok_or_else(|| bad("github-oauth tokens must be strings"))?;
                self.github_oauth.insert(host.clone(), token.to_owned());
            }
        }
        if let Some(section) = doc.get("http-basic") {
            let map = section
                .as_object()
                .ok_or_else(|| bad("http-basic must be an object"))?;
            for (host, entry) in map {
                let field = |k: &str| entry.get(k).and_then(Value::as_str).map(str::to_owned);
                let (Some(user), Some(pass)) = (field("username"), field("password")) else {
                    return Err(bad("http-basic entries need username and password strings"));
                };
                self.http_basic.insert(host.clone(), (user, pass));
            }
        }
        Ok(())
    }

    /// The credential for a request to `host` (and `port`, if not the default).
    pub fn credential(&self, host: &str, port: Option<u16>) -> Option<Credential> {
        // Composer: RemoteFilesystem::getOrigin maps api.github.com to github.com.
        let origin = if host == "api.github.com" {
            "github.com"
        } else {
            host
        };
        if let Some(token) = self.github_oauth.get(origin) {
            return Some(Credential::GithubToken(token.clone()));
        }
        let with_port = port.map(|p| format!("{host}:{p}"));
        with_port
            .as_deref()
            .and_then(|k| self.http_basic.get(k))
            .or_else(|| self.http_basic.get(host))
            .map(|(username, password)| Credential::Basic {
                username: username.clone(),
                password: password.clone(),
            })
    }
}

// Composer: Factory::getHomeDir.
fn composer_home(
    env: &impl Fn(&str) -> Option<OsString>,
    is_dir: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let var = |k: &str| env(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(home) = var("COMPOSER_HOME") {
        return Some(home);
    }
    if cfg!(windows) {
        return var("APPDATA").map(|d| d.join("Composer"));
    }
    let user = var("HOME")?;
    let mut dirs = Vec::new();
    if uses_xdg(is_dir) {
        let config = var("XDG_CONFIG_HOME").unwrap_or_else(|| user.join(".config"));
        dirs.push(config.join("composer"));
    }
    dirs.push(user.join(".composer"));
    dirs.iter()
        .find(|d| is_dir(d))
        .or_else(|| dirs.first())
        .cloned()
}

fn uses_xdg(is_dir: &dyn Fn(&Path) -> bool) -> bool {
    std::env::vars_os().any(|(k, _)| k.to_string_lossy().starts_with("XDG_"))
        || is_dir(Path::new("/etc/xdg"))
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::composer_home;
    use super::{Auth, Credential};
    use crate::testutil::TempDir;
    use std::ffi::OsString;
    #[cfg(unix)]
    use std::path::{Path, PathBuf};

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn github_token_applies_to_the_api_host() {
        let mut auth = Auth::default();
        auth.merge_json("t", r#"{"github-oauth":{"github.com":"ghp_x"}}"#)
            .unwrap();
        let token = Some(Credential::GithubToken("ghp_x".into()));
        assert_eq!(auth.credential("api.github.com", None), token);
        assert_eq!(auth.credential("github.com", None), token);
        assert_eq!(auth.credential("codeload.github.com", None), None);
    }

    #[test]
    fn http_basic_matches_host_and_port() {
        let mut auth = Auth::default();
        auth.merge_json(
            "t",
            r#"{"http-basic":{"repo.example.com":{"username":"u","password":"p"},
                "localhost:8080":{"username":"l","password":"q"}}}"#,
        )
        .unwrap();
        let basic = |u: &str, p: &str| {
            Some(Credential::Basic {
                username: u.into(),
                password: p.into(),
            })
        };
        assert_eq!(auth.credential("repo.example.com", None), basic("u", "p"));
        assert_eq!(
            auth.credential("repo.example.com", Some(8443)),
            basic("u", "p")
        );
        assert_eq!(auth.credential("localhost", Some(8080)), basic("l", "q"));
        assert_eq!(auth.credential("localhost", Some(9090)), None);
    }

    #[test]
    fn rejects_malformed_documents() {
        for bad in [
            "[]",
            "{",
            r#"{"github-oauth":[]}"#,
            r#"{"github-oauth":{"github.com":1}}"#,
            r#"{"http-basic":"x"}"#,
            r#"{"http-basic":{"h":{"username":"u"}}}"#,
        ] {
            let err = Auth::default().merge_json("auth.json", bad).unwrap_err();
            assert!(err.to_string().contains("auth.json"), "{bad}: {err}");
        }
    }

    #[test]
    fn later_sources_override_earlier_ones() {
        let tmp = TempDir::new("auth-load");
        let home = tmp.path().join("home");
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            home.join("auth.json"),
            r#"{"github-oauth":{"github.com":"home","ghe.local":"home"}}"#,
        )
        .unwrap();
        std::fs::write(
            project.join("auth.json"),
            r#"{"github-oauth":{"github.com":"project"}}"#,
        )
        .unwrap();
        let home_s = home.to_string_lossy().into_owned();
        let pairs = [
            ("COMPOSER_HOME", home_s.as_str()),
            ("COMPOSER_AUTH", r#"{"github-oauth":{"ghe.local":"env"}}"#),
        ];
        let auth = Auth::load_from(Some(&project), env(&pairs), &|d| d.is_dir()).unwrap();
        assert_eq!(
            auth.credential("github.com", None),
            Some(Credential::GithubToken("project".into()))
        );
        assert_eq!(
            auth.credential("ghe.local", None),
            Some(Credential::GithubToken("env".into()))
        );
        let none = Auth::load_from(None, env(&[]), &|_| false).unwrap();
        assert_eq!(none, Auth::default());
        let bad = [("COMPOSER_AUTH", "nope")];
        assert!(Auth::load_from(None, env(&bad), &|_| false).is_err());
        assert!(Auth::load(None).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn finds_composer_home_like_composer() {
        assert_eq!(
            composer_home(&env(&[("COMPOSER_HOME", "/ch"), ("HOME", "/h")]), &|_| {
                false
            }),
            Some(PathBuf::from("/ch"))
        );
        let legacy = Path::new("/h/.composer");
        assert_eq!(
            composer_home(&env(&[("HOME", "/h")]), &|d| d == legacy),
            Some(legacy.to_owned())
        );
        let xdg = Path::new("/xdg/composer");
        assert_eq!(
            composer_home(&env(&[("HOME", "/h"), ("XDG_CONFIG_HOME", "/xdg")]), &|d| {
                d == xdg || d == Path::new("/etc/xdg")
            }),
            Some(xdg.to_owned())
        );
        assert_eq!(composer_home(&env(&[]), &|_| false), None);
    }
}
