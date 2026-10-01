//! Credentials from every place Composer reads them, applied per request the
//! way `AuthHelper::addAuthenticationOptions` applies them.
//!
//! Sources, later ones winning per domain (Composer: Factory.php createConfig,
//! createComposer): `COMPOSER_HOME/config.json`, `COMPOSER_HOME/auth.json`,
//! `COMPOSER_AUTH`, the project's composer.json `config`, the project's
//! `auth.json`, then `COMPOSER_AUTH` again. Per domain one credential is kept,
//! set in Composer's type order (IO/BaseIO.php loadConfiguration), so a later
//! type replaces an earlier one for the same domain.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::Url;
use serde_json::{Map, Value};

use crate::error::{Error, Result};

/// Composer: Util/Bitbucket.php `OAUTH2_ACCESS_TOKEN_URL`.
pub const BITBUCKET_TOKEN_URL: &str = "https://bitbucket.org/site/oauth2/access_token";

/// Which config key a domain's credential came from; Composer encodes this
/// as a marker in the stored password (`bearer`, `x-oauth-basic`, ...).
#[derive(Clone, PartialEq, Eq)]
enum Kind {
    Basic,
    Bearer,
    Github,
    GitlabOauth,
    GitlabPrivate,
    CustomHeaders,
    /// `bitbucket-oauth`, with the stored access token and its expiry if any.
    Bitbucket(Option<(String, i64)>),
}

/// One domain's credential as Composer's IO keeps it.
#[derive(Clone, PartialEq, Eq)]
struct Entry {
    kind: Kind,
    user: String,
    pass: String,
}

impl Entry {
    fn new(kind: Kind, user: String, pass: String) -> Self {
        Self { kind, user, pass }
    }

    /// The pair Composer would base64 for basic auth.
    fn basic_pair(&self) -> (&str, &str) {
        let marker = match self.kind {
            Kind::Github => "x-oauth-basic",
            Kind::GitlabOauth => "oauth2",
            Kind::GitlabPrivate => "private-token",
            _ => self.pass.as_str(),
        };
        (self.user.as_str(), marker)
    }
}

/// What to send with one request, after the auth for its origin is applied.
#[derive(Clone, PartialEq, Eq)]
pub enum Credential {
    /// Headers to add, as `(name, value)`.
    Headers(Vec<(String, String)>),
    /// A Bitbucket OAuth consumer: exchange it for an access token first.
    BitbucketConsumer { key: String, secret: String },
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Headers(h) => f
                .debug_list()
                .entries(h.iter().map(|(name, _)| name))
                .finish(),
            Self::BitbucketConsumer { .. } => f.write_str("BitbucketConsumer"),
        }
    }
}

/// Credentials by origin, plus the domain lists that change how they apply.
#[derive(Clone, PartialEq, Eq)]
pub struct Auth {
    entries: BTreeMap<String, Entry>,
    github_domains: Vec<String>,
    gitlab_domains: Vec<String>,
    /// Where Bitbucket consumers get access tokens; a test server in tests.
    pub bitbucket_token_url: String,
}

impl Default for Auth {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            github_domains: vec!["github.com".to_owned()],
            gitlab_domains: vec!["gitlab.com".to_owned()],
            bitbucket_token_url: BITBUCKET_TOKEN_URL.to_owned(),
        }
    }
}

impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Auth")
            .field("origins", &self.entries.keys().collect::<Vec<_>>())
            .field("github_domains", &self.github_domains)
            .field("gitlab_domains", &self.gitlab_domains)
            .finish_non_exhaustive()
    }
}

/// The auth keys of Composer's merged config.
#[derive(Debug, Default)]
struct Merged {
    sections: BTreeMap<&'static str, Map<String, Value>>,
    github_domains: Vec<String>,
    gitlab_domains: Vec<String>,
}

// Composer: Config.php merge
const PER_DOMAIN: [&str; 8] = [
    "bitbucket-oauth",
    "github-oauth",
    "gitlab-oauth",
    "gitlab-token",
    "http-basic",
    "bearer",
    "client-certificate",
    "forgejo-token",
];

impl Merged {
    fn merge(&mut self, source: &str, config: &Map<String, Value>) -> Result<()> {
        let bad = |reason: String| Error::Auth {
            source: source.to_owned(),
            reason,
        };
        for key in PER_DOMAIN.iter().copied().chain(["custom-headers"]) {
            let Some(value) = config.get(key) else {
                continue;
            };
            let map = match value {
                Value::Object(m) => m.clone(),
                Value::Array(a) if a.is_empty() => Map::new(),
                _ => return Err(bad(format!("{key} must be an object"))),
            };
            let section = self.sections.entry(key).or_default();
            if key == "custom-headers" {
                *section = map;
            } else {
                section.extend(map);
            }
        }
        for (key, list) in [
            ("github-domains", &mut self.github_domains),
            ("gitlab-domains", &mut self.gitlab_domains),
        ] {
            if let Some(value) = config.get(key) {
                let items = value
                    .as_array()
                    .ok_or_else(|| bad(format!("{key} must be a list")))?;
                for item in items.iter().filter_map(Value::as_str) {
                    if !list.iter().any(|d| d == item) {
                        list.push(item.to_owned());
                    }
                }
            }
        }
        Ok(())
    }

    fn merge_json(&mut self, source: &str, text: &str, under_config: bool) -> Result<()> {
        let bad = |reason: String| Error::Auth {
            source: source.to_owned(),
            reason,
        };
        let doc: Value = serde_json::from_str(text).map_err(|e| bad(e.to_string()))?;
        let Value::Object(doc) = doc else {
            return Err(bad("expected a JSON object".to_owned()));
        };
        if under_config {
            match doc.get("config") {
                Some(Value::Object(config)) => self.merge(source, config),
                _ => Ok(()),
            }
        } else {
            self.merge(source, &doc)
        }
    }
}

fn str_field(entry: &Value, key: &str) -> Option<String> {
    entry.get(key).and_then(Value::as_str).map(str::to_owned)
}

impl Auth {
    /// Every source Composer reads, for a project whose composer.json is in
    /// `project_dir` and has `config`.
    pub fn load(project_dir: Option<&Path>, config: Option<&Map<String, Value>>) -> Result<Self> {
        Self::load_from(project_dir, config, |k| std::env::var_os(k), &|d| {
            d.is_dir()
        })
    }

    fn load_from(
        project_dir: Option<&Path>,
        config: Option<&Map<String, Value>>,
        env: impl Fn(&str) -> Option<OsString>,
        is_dir: &dyn Fn(&Path) -> bool,
    ) -> Result<Self> {
        let mut merged = Merged::default();
        let read = |file: &Path| match std::fs::read_to_string(file) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(Error::Io {
                path: file.to_owned(),
                source,
            }),
        };
        let composer_auth = env("COMPOSER_AUTH")
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string_lossy().into_owned());
        if let Some(home) = composer_home(&env, is_dir) {
            for (name, under_config) in [("config.json", true), ("auth.json", false)] {
                let file = home.join(name);
                if let Some(text) = read(&file)? {
                    merged.merge_json(&file.display().to_string(), &text, under_config)?;
                }
            }
        }
        if let Some(text) = &composer_auth {
            merged.merge_json("COMPOSER_AUTH", text, false)?;
        }
        if let Some(config) = config {
            merged.merge("composer.json", config)?;
        }
        if let Some(dir) = project_dir {
            let file: PathBuf = dir.join("auth.json");
            if let Some(text) = read(&file)? {
                merged.merge_json(&file.display().to_string(), &text, false)?;
            }
        }
        if let Some(text) = &composer_auth {
            merged.merge_json("COMPOSER_AUTH", text, false)?;
        }
        Self::from_merged(merged)
    }

    /// Credentials from one `auth.json`-shaped document, over the defaults.
    pub fn from_json(source: &str, text: &str) -> Result<Self> {
        let mut merged = Merged::default();
        merged.merge_json(source, text, false)?;
        Self::from_merged(merged)
    }

    // Composer: IO/BaseIO.php loadConfiguration
    fn from_merged(merged: Merged) -> Result<Self> {
        let mut auth = Self::default();
        for d in merged.github_domains {
            add_domain(&mut auth.github_domains, &d);
        }
        for d in merged.gitlab_domains {
            add_domain(&mut auth.gitlab_domains, &d);
        }
        let bad = |key: &str, domain: &str, what: &str| Error::Auth {
            source: format!("{key}.{domain}"),
            reason: what.to_owned(),
        };
        let section = |key: &str| merged.sections.get(key).cloned().unwrap_or_default();
        for (domain, cred) in section("bitbucket-oauth") {
            let (Some(key), Some(secret)) = (
                str_field(&cred, "consumer-key"),
                str_field(&cred, "consumer-secret"),
            ) else {
                return Err(bad(
                    "bitbucket-oauth",
                    &domain,
                    "needs consumer-key and consumer-secret strings",
                ));
            };
            let token = str_field(&cred, "access-token")
                .zip(cred.get("access-token-expiration").and_then(Value::as_i64));
            auth.set(&domain, Entry::new(Kind::Bitbucket(token), key, secret));
        }
        for (domain, token) in section("github-oauth") {
            let token = token
                .as_str()
                .ok_or_else(|| bad("github-oauth", &domain, "tokens must be strings"))?;
            if domain != "github.com" {
                add_domain(&mut auth.github_domains, &domain);
            }
            auth.set(
                &domain,
                Entry::new(Kind::Github, token.to_owned(), String::new()),
            );
        }
        for (domain, token) in section("gitlab-oauth") {
            let token = match &token {
                Value::String(s) => s.clone(),
                other => str_field(other, "token")
                    .ok_or_else(|| bad("gitlab-oauth", &domain, "needs a token"))?,
            };
            if domain != "gitlab.com" {
                add_domain(&mut auth.gitlab_domains, &domain);
            }
            auth.set(&domain, Entry::new(Kind::GitlabOauth, token, String::new()));
        }
        for (domain, token) in section("gitlab-token") {
            let entry = match &token {
                Value::String(s) => Entry::new(Kind::GitlabPrivate, s.clone(), String::new()),
                other => str_field(other, "username")
                    .zip(str_field(other, "token"))
                    .map(|(user, token)| Entry::new(Kind::Basic, user, token))
                    .ok_or_else(|| bad("gitlab-token", &domain, "needs username and token"))?,
            };
            if domain != "gitlab.com" {
                add_domain(&mut auth.gitlab_domains, &domain);
            }
            auth.set(&domain, entry);
        }
        for (domain, cred) in section("forgejo-token") {
            let (user, token) = str_field(&cred, "username")
                .zip(str_field(&cred, "token"))
                .ok_or_else(|| bad("forgejo-token", &domain, "needs username and token"))?;
            auth.set(&domain, Entry::new(Kind::Basic, user, token));
        }
        for (domain, cred) in section("http-basic") {
            let (user, pass) = str_field(&cred, "username")
                .zip(str_field(&cred, "password"))
                .ok_or_else(|| {
                    bad(
                        "http-basic",
                        &domain,
                        "entries need username and password strings",
                    )
                })?;
            auth.set(&domain, Entry::new(Kind::Basic, user, pass));
        }
        for (domain, token) in section("bearer") {
            let token = token
                .as_str()
                .ok_or_else(|| bad("bearer", &domain, "tokens must be strings"))?;
            auth.set(
                &domain,
                Entry::new(Kind::Bearer, token.to_owned(), String::new()),
            );
        }
        for (domain, headers) in section("custom-headers") {
            if headers.is_null() {
                continue;
            }
            let list = headers
                .as_array()
                .filter(|l| l.iter().all(Value::is_string))
                .ok_or_else(|| bad("custom-headers", &domain, "must be a list of strings"))?;
            auth.set(
                &domain,
                Entry::new(
                    Kind::CustomHeaders,
                    Value::Array(list.clone()).to_string(),
                    String::new(),
                ),
            );
        }
        Ok(auth)
    }

    fn set(&mut self, domain: &str, entry: Entry) {
        self.entries.insert(domain.to_owned(), entry);
    }

    /// Composer: Util/Url.php getOrigin.
    pub fn origin(&self, url: &Url) -> String {
        let mut origin = url.host_str().unwrap_or_default().to_owned();
        if let Some(port) = url.port() {
            origin = format!("{origin}:{port}");
        }
        if origin.ends_with(".github.com") && origin != "codeload.github.com" {
            return "github.com".to_owned();
        }
        if origin == "repo.packagist.org" {
            return "packagist.org".to_owned();
        }
        if !origin.contains('/') && !self.gitlab_domains.contains(&origin) {
            for domain in &self.gitlab_domains {
                let bare = strip_port(domain);
                if !domain.is_empty() && (bare == origin || bare.starts_with(&format!("{origin}/")))
                {
                    return domain.clone();
                }
            }
        }
        origin
    }

    // Composer: Util/AuthHelper.php findAuthOrigin
    fn find(&self, origin: &str) -> Option<(&str, &Entry)> {
        if let Some((k, e)) = self.entries.get_key_value(origin) {
            return Some((k, e));
        }
        if matches!(origin, "api.bitbucket.org" | "api.github.com") {
            let canonical = &origin[4..];
            return self
                .entries
                .get_key_value(canonical)
                .map(|(k, e)| (k.as_str(), e));
        }
        None
    }

    /// What to send to `url`; `inline` is the `user:pass@` from the URL
    /// itself, which wins over the config as `HttpDownloader::addJob` does.
    // Composer: Util/AuthHelper.php addAuthenticationOptions
    pub fn credential(&self, url: &Url, inline: Option<(String, String)>) -> Option<Credential> {
        let origin = self.origin(url);
        let inline_entry = inline.map(|(user, pass)| Entry::new(Kind::Basic, user, pass));
        let (origin, entry) = match &inline_entry {
            Some(e) => (origin.as_str(), e),
            None => self.find(&origin)?,
        };
        let header =
            |name: &str, value: String| Some(Credential::Headers(vec![(name.to_owned(), value)]));
        let user = entry.user.as_str();
        match &entry.kind {
            Kind::Bearer => return header("Authorization", format!("Bearer {user}")),
            Kind::CustomHeaders => {
                let list: Vec<String> = serde_json::from_str(user).unwrap_or_default();
                return Some(Credential::Headers(
                    list.iter()
                        .filter_map(|h| h.split_once(':'))
                        .map(|(n, v)| (n.trim().to_owned(), v.trim().to_owned()))
                        .collect(),
                ));
            }
            Kind::Github if origin == "github.com" => {
                let api = matches!(url.scheme(), "http" | "https")
                    && url.host_str() == Some("api.github.com");
                return if api {
                    header("Authorization", format!("token {user}"))
                } else {
                    None
                };
            }
            Kind::GitlabOauth if self.gitlab_domains.iter().any(|d| d == origin) => {
                return header("Authorization", format!("Bearer {user}"));
            }
            Kind::GitlabPrivate if self.gitlab_domains.iter().any(|d| d == origin) => {
                return header("PRIVATE-TOKEN", user.to_owned());
            }
            _ => {}
        }
        if origin == "bitbucket.org" && url.as_str() != self.bitbucket_token_url {
            let public = is_public_bitbucket_download(url);
            let bearer = |token: &str| {
                if public {
                    None
                } else {
                    header("Authorization", format!("Bearer {token}"))
                }
            };
            match &entry.kind {
                Kind::Basic if user == "x-token-auth" => return bearer(&entry.pass),
                Kind::Bitbucket(Some((token, expires))) if now() <= *expires => {
                    return bearer(token);
                }
                Kind::Bitbucket(_) if !public => {
                    return Some(Credential::BitbucketConsumer {
                        key: entry.user.clone(),
                        secret: entry.pass.clone(),
                    });
                }
                _ => {}
            }
        }
        let (user, pass) = entry.basic_pair();
        header("Authorization", basic(user, pass))
    }

    /// The `Authorization` header for the Bitbucket token exchange.
    pub fn bitbucket_exchange_header(key: &str, secret: &str) -> String {
        basic(key, secret)
    }
}

fn add_domain(list: &mut Vec<String>, domain: &str) {
    if !list.iter().any(|d| d == domain) {
        list.push(domain.to_owned());
    }
}

fn strip_port(domain: &str) -> String {
    match domain.split_once(':') {
        Some((host, rest)) => {
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            if digits > 0 {
                format!("{host}{}", &rest[digits..])
            } else {
                domain.to_owned()
            }
        }
        None => domain.to_owned(),
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn basic(user: &str, pass: &str) -> String {
    format!("Basic {}", base64(format!("{user}:{pass}").as_bytes()))
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(TABLE[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

// Composer: Util/AuthHelper.php isPublicBitBucketDownload
fn is_public_bitbucket_download(url: &Url) -> bool {
    if !url.host_str().unwrap_or_default().contains("bitbucket.org") {
        return true;
    }
    let parts: Vec<&str> = url.path().split('/').collect();
    parts.len() >= 4 && parts[3] == "downloads"
}

/// `user:pass@` taken out of `url`, percent-decoded, and the URL without it.
pub fn split_inline_credentials(url: &Url) -> (Url, Option<(String, String)>) {
    let Some(password) = url.password() else {
        return (url.clone(), None);
    };
    if !matches!(url.scheme(), "http" | "https") || url.username().is_empty() {
        return (url.clone(), None);
    }
    let decode = |s: &str| {
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%'
                && let Some(v) = s
                    .get(i + 1..i + 3)
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
            out.push(bytes[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    let creds = (decode(url.username()), decode(password));
    let mut bare = url.clone();
    let _ = bare.set_username("");
    let _ = bare.set_password(None);
    (bare, Some(creds))
}

/// A URL safe to print: passwords and token-like usernames masked.
// Composer: Util/Url.php sanitize, sanitizeUsername
pub fn sanitize(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    let mut rest = url;
    while let Some(i) = rest.find("access_token=") {
        let at = i + "access_token=".len();
        let preceded = i > 0 && matches!(rest.as_bytes()[i - 1], b'&' | b'?');
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        if preceded {
            let end = rest.find('&').unwrap_or(rest.len());
            if end > 0 {
                out.push_str("***");
            }
            rest = &rest[end..];
        }
    }
    out.push_str(rest);
    mask_userinfo(&out)
}

fn mask_username(user: &str) -> String {
    const PUBLIC: [&str; 5] = [
        "private-token",
        "x-token-auth",
        "oauth2",
        "gitlab-ci-token",
        "x-oauth-basic",
    ];
    if PUBLIC.contains(&user) {
        return user.to_owned();
    }
    let gh = user.len() > 4
        && user.starts_with("gh")
        && user.as_bytes()[2].is_ascii_lowercase()
        && user.as_bytes()[3] == b'_';
    if gh || user.len() >= 12 {
        let head: String = user.chars().take(3).collect();
        return format!("{head}***");
    }
    user.to_owned()
}

fn mask_userinfo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut at_start = true;
    loop {
        let start = if at_start {
            Some(0)
        } else {
            rest.find("://").map(|i| i + 3)
        };
        at_start = false;
        let Some(start) = start else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let end = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '/' | '?' | '#'))
            .unwrap_or(rest.len());
        let authority = &rest[..end];
        if let Some(at) = authority.rfind('@') {
            let info = &authority[..at];
            let masked = match info.split_once(':') {
                Some((user, pass)) if !pass.is_empty() => format!("{}:***", mask_username(user)),
                Some((user, _)) => mask_username(user),
                None => mask_username(info),
            };
            out.push_str(&masked);
            out.push('@');
            rest = &rest[at + 1..];
        }
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
    use super::{Auth, Credential, base64, sanitize, split_inline_credentials};
    use crate::testutil::TempDir;
    use reqwest::Url;
    use serde_json::json;
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

    fn auth(json: &str) -> Auth {
        Auth::from_json("auth.json", json).unwrap()
    }

    fn headers(auth: &Auth, url: &str) -> Vec<(String, String)> {
        match auth.credential(&Url::parse(url).unwrap(), None) {
            Some(Credential::Headers(h)) => h,
            Some(Credential::BitbucketConsumer { .. }) => vec![("consumer".into(), String::new())],
            None => Vec::new(),
        }
    }

    fn one(name: &str, value: &str) -> Vec<(String, String)> {
        vec![(name.to_owned(), value.to_owned())]
    }

    #[test]
    fn github_tokens_go_to_the_api_only() {
        let a = auth(r#"{"github-oauth":{"github.com":"ghp_x"}}"#);
        let token = one("Authorization", "token ghp_x");
        assert_eq!(
            headers(&a, "https://api.github.com/repos/a/b/zipball/c"),
            token
        );
        assert!(headers(&a, "https://codeload.github.com/a/b/legacy.zip/c").is_empty());
        assert!(headers(&a, "https://github.com/a/b/archive/c.zip").is_empty());
        let ghe = auth(r#"{"github-oauth":{"ghe.corp":"tok"}}"#);
        assert_eq!(
            headers(&ghe, "https://ghe.corp/api/v3/repos/a/b/zipball/c"),
            one(
                "Authorization",
                &format!("Basic {}", base64(b"tok:x-oauth-basic"))
            )
        );
    }

    #[test]
    fn gitlab_tokens_follow_gitlab_domains() {
        let a = auth(
            r#"{"gitlab-oauth":{"gitlab.com":"oa"},
                "gitlab-token":{"git.corp:8443":"pt","deploy.corp":{"username":"u","token":"t"}},
                "gitlab-domains":["gl.corp/sub"]}"#,
        );
        assert_eq!(
            headers(&a, "https://gitlab.com/api/v4/x"),
            one("Authorization", "Bearer oa")
        );
        assert_eq!(
            headers(&a, "https://git.corp/api/v4/x"),
            one("PRIVATE-TOKEN", "pt")
        );
        assert_eq!(
            headers(&a, "https://deploy.corp/api/v4/x"),
            one("Authorization", &format!("Basic {}", base64(b"u:t")))
        );
        assert_eq!(
            a.origin(&Url::parse("https://gl.corp/x").unwrap()),
            "gl.corp/sub"
        );
        assert_eq!(
            a.origin(&Url::parse("https://repo.packagist.org/p2/x.json").unwrap()),
            "packagist.org"
        );
        let nested = auth(r#"{"gitlab-oauth":{"gitlab.com":{"token":"nested"}}}"#);
        assert_eq!(
            headers(&nested, "https://gitlab.com/x"),
            one("Authorization", "Bearer nested")
        );
    }

    #[test]
    fn bearer_basic_forgejo_and_custom_headers() {
        let a = auth(
            r#"{"bearer":{"repo.packagist.com":"bt"},
                "http-basic":{"satis.corp":{"username":"u","password":"p"},"localhost:8080":{"username":"l","password":"q"}},
                "forgejo-token":{"codeberg.org":{"username":"fu","token":"ft"}},
                "custom-headers":{"api.corp":["X-Api-Key: k1", "X-Other:v2", "bogus"]},
                "client-certificate":{"cert.corp":{"local_cert":"/x.pem"}}}"#,
        );
        assert_eq!(
            headers(&a, "https://repo.packagist.com/a/dists/x.zip"),
            one("Authorization", "Bearer bt")
        );
        assert_eq!(
            headers(&a, "https://satis.corp/dist/x.zip"),
            one("Authorization", &format!("Basic {}", base64(b"u:p")))
        );
        assert_eq!(
            headers(&a, "http://localhost:8080/x.zip"),
            one("Authorization", &format!("Basic {}", base64(b"l:q")))
        );
        assert!(headers(&a, "http://localhost:9090/x.zip").is_empty());
        assert!(headers(&a, "https://satis.corp:8443/x.zip").is_empty());
        assert_eq!(
            headers(&a, "https://codeberg.org/api/v1/x"),
            one("Authorization", &format!("Basic {}", base64(b"fu:ft")))
        );
        assert_eq!(
            headers(&a, "https://api.corp/x"),
            vec![
                ("X-Api-Key".into(), "k1".into()),
                ("X-Other".into(), "v2".into())
            ]
        );
        assert!(headers(&a, "https://cert.corp/x").is_empty());
    }

    #[test]
    fn later_types_replace_earlier_ones_for_a_domain() {
        let a = auth(
            r#"{"github-oauth":{"x.corp":"gh"},"http-basic":{"x.corp":{"username":"u","password":"p"}},"bearer":{"x.corp":"b"}}"#,
        );
        assert_eq!(
            headers(&a, "https://x.corp/a"),
            one("Authorization", "Bearer b")
        );
    }

    #[test]
    fn bitbucket_consumers_need_a_token_first() {
        let a = auth(
            r#"{"bitbucket-oauth":{"bitbucket.org":{"consumer-key":"k","consumer-secret":"s"}}}"#,
        );
        let url = Url::parse("https://bitbucket.org/o/r/get/abc.zip").unwrap();
        assert_eq!(
            a.credential(&url, None),
            Some(Credential::BitbucketConsumer {
                key: "k".into(),
                secret: "s".into()
            })
        );
        assert_eq!(
            headers(&a, "https://api.bitbucket.org/2.0/repositories/o/r"),
            vec![("consumer".into(), String::new())]
        );
        assert!(headers(&a, "https://bbuseruploads.s3.amazonaws.com/x.zip").is_empty());
        assert_eq!(
            headers(&a, "https://bitbucket.org/o/r/downloads/x.zip"),
            one("Authorization", &format!("Basic {}", base64(b"k:s")))
        );
        let stored = auth(
            r#"{"bitbucket-oauth":{"bitbucket.org":{"consumer-key":"k","consumer-secret":"s","access-token":"at","access-token-expiration":99999999999}}}"#,
        );
        assert_eq!(
            headers(&stored, "https://bitbucket.org/o/r/get/abc.zip"),
            one("Authorization", "Bearer at")
        );
        assert!(headers(&stored, "https://bitbucket.org/o/r/downloads/x.zip").is_empty());
        let expired = auth(
            r#"{"bitbucket-oauth":{"bitbucket.org":{"consumer-key":"k","consumer-secret":"s","access-token":"at","access-token-expiration":1}}}"#,
        );
        assert_eq!(
            headers(&expired, "https://bitbucket.org/o/r/get/a.zip"),
            vec![("consumer".into(), String::new())]
        );
        let x = Url::parse("https://bitbucket.org/o/r/get/abc.zip").unwrap();
        assert_eq!(
            a.credential(&x, Some(("x-token-auth".into(), "tok".into()))),
            Some(Credential::Headers(one("Authorization", "Bearer tok")))
        );
        assert_eq!(
            a.credential(
                &Url::parse("https://bitbucket.org/o/r/downloads/a.zip").unwrap(),
                Some(("x-token-auth".into(), "tok".into()))
            ),
            None
        );
        assert_eq!(
            Auth::bitbucket_exchange_header("k", "s"),
            format!("Basic {}", base64(b"k:s"))
        );
    }

    #[test]
    fn inline_credentials_win_and_are_split_off() {
        let a = auth(r#"{"http-basic":{"satis.corp":{"username":"u","password":"p"}}}"#);
        let url = Url::parse("https://us%40er:p%3Ass@satis.corp/x.zip").unwrap();
        let (bare, inline) = split_inline_credentials(&url);
        assert_eq!(bare.as_str(), "https://satis.corp/x.zip");
        assert_eq!(inline, Some(("us@er".into(), "p:ss".into())));
        assert_eq!(
            a.credential(&bare, inline),
            Some(Credential::Headers(one(
                "Authorization",
                &format!("Basic {}", base64(b"us@er:p:ss"))
            )))
        );
        let plain = Url::parse("https://satis.corp/x.zip").unwrap();
        assert_eq!(split_inline_credentials(&plain).1, None);
        let user_only = Url::parse("https://user@satis.corp/x.zip").unwrap();
        assert_eq!(split_inline_credentials(&user_only).1, None);
    }

    #[test]
    fn encodes_base64() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"user:pass"), "dXNlcjpwYXNz");
    }

    #[test]
    fn sanitizes_urls_like_composer() {
        let cases = [
            ("https://user:secret@host/x", "https://user:***@host/x"),
            (
                "https://ghp_abcdef:x-oauth-basic@github.com/x",
                "https://ghp***:***@github.com/x",
            ),
            ("https://0123456789abcdef@host/x", "https://012***@host/x"),
            (
                "https://x-token-auth:tok@bitbucket.org/x",
                "https://x-token-auth:***@bitbucket.org/x",
            ),
            (
                "https://api.github.com/r/1?access_token=abc&x=1",
                "https://api.github.com/r/1?access_token=***&x=1",
            ),
            (
                "could not download https://u:p@h/x: HTTP 401",
                "could not download https://u:***@h/x: HTTP 401",
            ),
            ("https://host/plain", "https://host/plain"),
            ("u:p@host", "u:***@host"),
            ("noaccess_token=1", "noaccess_token=1"),
        ];
        for (input, want) in cases {
            assert_eq!(sanitize(input), want, "{input}");
        }
    }

    #[test]
    fn debug_output_never_shows_secrets() {
        let a = auth(
            r#"{"bearer":{"h":"supersecret"},"http-basic":{"g":{"username":"u","password":"pw-secret"}}}"#,
        );
        let text = format!("{a:?}");
        assert!(
            !text.contains("supersecret") && !text.contains("pw-secret"),
            "{text}"
        );
        let c = a
            .credential(&Url::parse("https://h/x").unwrap(), None)
            .unwrap();
        assert!(!format!("{c:?}").contains("supersecret"));
        let b = Credential::BitbucketConsumer {
            key: "k".into(),
            secret: "very-secret".into(),
        };
        assert!(!format!("{b:?}").contains("very-secret"));
    }

    #[test]
    fn rejects_malformed_documents() {
        for bad in [
            "[]",
            "{",
            r#"{"github-oauth":"x"}"#,
            r#"{"github-oauth":{"github.com":1}}"#,
            r#"{"http-basic":"x"}"#,
            r#"{"http-basic":{"h":{"username":"u"}}}"#,
            r#"{"bearer":{"h":1}}"#,
            r#"{"gitlab-oauth":{"h":{}}}"#,
            r#"{"gitlab-token":{"h":{"username":"u"}}}"#,
            r#"{"forgejo-token":{"h":{"username":"u"}}}"#,
            r#"{"bitbucket-oauth":{"h":{"consumer-key":"k"}}}"#,
            r#"{"custom-headers":{"h":[1]}}"#,
            r#"{"github-domains":"x"}"#,
        ] {
            let err = Auth::from_json("auth.json", bad).unwrap_err();
            assert!(
                err.to_string().contains("invalid auth config"),
                "{bad}: {err}"
            );
        }
        assert!(Auth::from_json("t", r#"{"http-basic":[],"custom-headers":{"h":null}}"#).is_ok());
    }

    #[test]
    fn sources_merge_in_composers_order() {
        let tmp = TempDir::new("auth-load");
        let home = tmp.path().join("home");
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            home.join("config.json"),
            r#"{"config":{"bearer":{"cfg.corp":"from-config","a.corp":"home-config"},"gitlab-domains":["gl.home"]}}"#,
        )
        .unwrap();
        std::fs::write(
            home.join("auth.json"),
            r#"{"bearer":{"a.corp":"home","b.corp":"home","c.corp":"home"},"custom-headers":{"h.corp":["X: home"]}}"#,
        )
        .unwrap();
        std::fs::write(
            project.join("auth.json"),
            r#"{"bearer":{"b.corp":"project","c.corp":"project"}}"#,
        )
        .unwrap();
        let config = json!({"bearer": {"c.corp": "composer-json", "d.corp": "composer-json"}, "custom-headers": {"i.corp": ["Y: json"]}});
        let home_s = home.to_string_lossy().into_owned();
        let pairs = [
            ("COMPOSER_HOME", home_s.as_str()),
            ("COMPOSER_AUTH", r#"{"bearer":{"c.corp":"env"}}"#),
        ];
        let a = Auth::load_from(Some(&project), config.as_object(), env(&pairs), &|d| {
            d.is_dir()
        })
        .unwrap();
        let bearer = |host: &str| headers(&a, &format!("https://{host}/x"));
        assert_eq!(
            bearer("cfg.corp"),
            one("Authorization", "Bearer from-config")
        );
        assert_eq!(bearer("a.corp"), one("Authorization", "Bearer home"));
        assert_eq!(bearer("b.corp"), one("Authorization", "Bearer project"));
        assert_eq!(bearer("c.corp"), one("Authorization", "Bearer env"));
        assert_eq!(
            bearer("d.corp"),
            one("Authorization", "Bearer composer-json")
        );
        assert!(
            bearer("h.corp").is_empty(),
            "custom-headers are replaced, not merged"
        );
        assert_eq!(bearer("i.corp"), one("Y", "json"));
        assert!(a.gitlab_domains.contains(&"gl.home".to_owned()));

        let none = Auth::load_from(None, None, env(&[]), &|_| false).unwrap();
        assert_eq!(none, Auth::default());
        let bad = [("COMPOSER_AUTH", "nope")];
        assert!(Auth::load_from(None, None, env(&bad), &|_| false).is_err());
        assert!(Auth::load(None, None).is_ok());
        std::fs::write(project.join("auth.json"), "nope").unwrap();
        assert!(Auth::load_from(Some(&project), None, env(&[]), &|_| false).is_err());
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
