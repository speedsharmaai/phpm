//! Dist downloads: bounded per host class, retried, verified, buffered in memory.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Once};
use std::time::Duration;

use reqwest::{StatusCode, Url};
use tokio::sync::{Mutex, Semaphore};

use crate::auth::{Auth, Credential, sanitize, split_inline_credentials};
use crate::error::{Error, Result};
use crate::store::hex_sha1;

/// `phpm/<version> (+https://github.com/speedsharmaai/phpm)`, per decision 0006.
pub const USER_AGENT: &str = concat!(
    "phpm/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/speedsharmaai/phpm)"
);

const PACKAGIST_HOSTS: [&str; 2] = ["repo.packagist.org", "packagist.org"];
const GITHUB_HOSTS: [&str; 3] = ["api.github.com", "codeload.github.com", "github.com"];
const MAX_REDIRECTS: usize = 10;

/// Knobs for [`Fetcher`]. The defaults are the published limits.
#[derive(Debug, Clone)]
pub struct FetchOptions {
    pub packagist_concurrency: usize,
    pub github_concurrency: usize,
    pub other_concurrency: usize,
    pub retries: u32,
    pub retry_delay: Duration,
    pub max_bytes: u64,
    /// Hosts whose HTTP 400 is retried (codeload answers 400 on some reused connections).
    pub retry_400_hosts: Vec<String>,
    /// HTTP/2 connections per host for dists, so one TCP window does not cap
    /// a download that is bound by bandwidth.
    pub dist_connections: usize,
    /// Fetch public GitHub zipballs pinned to a commit straight from
    /// codeload, skipping the api.github.com redirect.
    pub direct_codeload: bool,
    pub auth: Auth,
}

impl Default for FetchOptions {
    fn default() -> Self {
        Self {
            packagist_concurrency: 10,
            github_concurrency: 48,
            other_concurrency: 10,
            retries: 3,
            retry_delay: Duration::from_millis(500),
            max_bytes: 512 << 20,
            retry_400_hosts: vec!["codeload.github.com".into()],
            dist_connections: 8,
            direct_codeload: true,
            auth: Auth::default(),
        }
    }
}

/// An HTTP/2 client with per-host-class concurrency limits. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Fetcher {
    client: reqwest::Client,
    dist_clients: Arc<Vec<reqwest::Client>>,
    next_client: Arc<AtomicUsize>,
    packagist: Arc<Semaphore>,
    github: Arc<Semaphore>,
    other: Arc<Semaphore>,
    bitbucket_tokens: Arc<Mutex<BTreeMap<String, String>>>,
    options: Arc<FetchOptions>,
}

enum Failure {
    Retry(String),
    Fatal(Error),
}

impl Fetcher {
    pub fn new(options: FetchOptions) -> Result<Self> {
        static PROVIDER: Once = Once::new();
        PROVIDER.call_once(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
        let build = || {
            reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .connect_timeout(Duration::from_secs(30))
                .read_timeout(Duration::from_secs(60))
                .pool_max_idle_per_host(options.github_concurrency)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| Error::Download {
                    url: String::new(),
                    reason: e.to_string(),
                })
        };
        let client = build()?;
        let mut dist_clients = vec![client.clone()];
        for _ in 1..options.dist_connections {
            dist_clients.push(build()?);
        }
        Ok(Self {
            client,
            dist_clients: Arc::new(dist_clients),
            next_client: Arc::new(AtomicUsize::new(0)),
            packagist: Arc::new(Semaphore::new(options.packagist_concurrency.max(1))),
            github: Arc::new(Semaphore::new(options.github_concurrency.max(1))),
            other: Arc::new(Semaphore::new(options.other_concurrency.max(1))),
            bitbucket_tokens: Arc::new(Mutex::new(BTreeMap::new())),
            options: Arc::new(options),
        })
    }

    /// The auth this fetcher applies.
    pub fn auth(&self) -> &Auth {
        &self.options.auth
    }

    /// Download `url`, retrying transient failures, and check `shasum` if given.
    pub async fn fetch(&self, url: &str, shasum: Option<&str>) -> Result<Vec<u8>> {
        let shown = sanitize(url);
        let download_err = |reason: String| Error::Download {
            url: shown.clone(),
            reason,
        };
        if let Some(path) = local_path(url) {
            let bytes = tokio::task::spawn_blocking(move || std::fs::read(path))
                .await
                .map_err(|e| download_err(e.to_string()))?
                .map_err(|e| download_err(e.to_string()))?;
            return verify(url, shasum, bytes);
        }
        let parsed = Url::parse(url).map_err(|e| download_err(e.to_string()))?;
        let host = parsed.host_str().unwrap_or_default().to_owned();
        let gate = if PACKAGIST_HOSTS.contains(&host.as_str()) {
            &self.packagist
        } else if GITHUB_HOSTS.contains(&host.as_str()) {
            &self.github
        } else {
            &self.other
        };
        let _permit = gate
            .acquire()
            .await
            .map_err(|e| download_err(e.to_string()))?;

        let direct = if self.options.direct_codeload {
            codeload(&parsed, &self.options.auth)
        } else {
            None
        };
        let bytes = match direct {
            Some(direct) => match self.attempts(&direct).await {
                Ok(bytes) => bytes,
                Err(_) => self.attempts(&parsed).await?,
            },
            None => self.attempts(&parsed).await?,
        };
        verify(url, shasum, bytes)
    }

    /// GET `url`, retrying transient failures.
    async fn attempts(&self, url: &Url) -> Result<Vec<u8>> {
        let mut attempt = 0;
        loop {
            match self.attempt(url).await {
                Ok(bytes) => return Ok(bytes),
                Err(Failure::Retry(_)) if attempt < self.options.retries => {
                    tokio::time::sleep(self.options.retry_delay * 2_u32.pow(attempt)).await;
                    attempt += 1;
                }
                Err(Failure::Retry(reason)) => {
                    return Err(Error::Download {
                        url: sanitize(url.as_str()),
                        reason,
                    });
                }
                Err(Failure::Fatal(e)) => return Err(e),
            }
        }
    }

    // Composer: Util/Bitbucket.php requestToken, requestAccessToken
    async fn bitbucket_token(
        &self,
        origin: &str,
        key: &str,
        secret: &str,
    ) -> Result<String, Failure> {
        let mut tokens = self.bitbucket_tokens.lock().await;
        if let Some(token) = tokens.get(origin) {
            return Ok(token.clone());
        }
        let url = &self.options.auth.bitbucket_token_url;
        let fatal = |reason: String| {
            Failure::Fatal(Error::Download {
                url: sanitize(url),
                reason,
            })
        };
        let response = self
            .client
            .post(url.as_str())
            .header(
                "Authorization",
                Auth::bitbucket_exchange_header(key, secret),
            )
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body("grant_type=client_credentials")
            .send()
            .await
            .map_err(|e| Failure::Retry(chain(&e)))?;
        let status = response.status();
        if !status.is_success() {
            return Err(fatal(format!(
                "HTTP {status}: invalid OAuth consumer for {origin}; check bitbucket-oauth in auth.json"
            )));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| Failure::Retry(chain(&e)))?;
        let body: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| fatal(e.to_string()))?;
        let token = body
            .get("access_token")
            .and_then(serde_json::Value::as_str)
            .filter(|_| body.get("expires_in").is_some())
            .ok_or_else(|| {
                fatal("expected access_token and expires_in in the response".to_owned())
            })?
            .to_owned();
        tokens.insert(origin.to_owned(), token.clone());
        Ok(token)
    }

    async fn headers_for(&self, url: &Url) -> Result<(Url, Vec<(String, String)>), Failure> {
        let (bare, inline) = split_inline_credentials(url);
        let auth = &self.options.auth;
        let headers = match auth.credential(&bare, inline) {
            None => Vec::new(),
            Some(Credential::Headers(h)) => h,
            Some(Credential::BitbucketConsumer { key, secret }) => {
                let token = self
                    .bitbucket_token(&auth.origin(&bare), &key, &secret)
                    .await?;
                vec![("Authorization".to_owned(), format!("Bearer {token}"))]
            }
        };
        Ok((bare, headers))
    }

    /// Send `method` to `start`, following redirects by hand so the
    /// credentials are worked out again for every host.
    async fn send(
        &self,
        client: &reqwest::Client,
        start: &Url,
        method: &reqwest::Method,
        body: Option<&[u8]>,
        extra: &[(&str, String)],
    ) -> Result<reqwest::Response, Failure> {
        let mut url = start.clone();
        for _ in 0..=MAX_REDIRECTS {
            let (bare, headers) = self.headers_for(&url).await?;
            let mut request = client.request(method.clone(), bare.clone());
            for (name, value) in headers {
                request = request.header(name, value);
            }
            for (name, value) in extra {
                request = request.header(*name, value);
            }
            if let Some(body) = body {
                request = request.body(body.to_vec());
            }
            let r = request
                .send()
                .await
                .map_err(|e| Failure::Retry(chain(&e)))?;
            if r.status().is_redirection()
                && *method == reqwest::Method::GET
                && let Some(next) = r
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|l| l.to_str().ok())
                    .and_then(|l| bare.join(l).ok())
            {
                url = next;
                continue;
            }
            return Ok(r);
        }
        Err(Failure::Fatal(Error::Download {
            url: sanitize(start.as_str()),
            reason: format!("more than {MAX_REDIRECTS} redirects"),
        }))
    }

    /// A metadata request: GET (conditional when `if_modified_since` is
    /// set) or POST, retried like dists. 304 and 404 are answers, not errors.
    pub async fn request(
        &self,
        url: &str,
        if_modified_since: Option<&str>,
        post: Option<(&str, Vec<u8>)>,
    ) -> Result<Response> {
        let shown = sanitize(url);
        let err = |reason: String| Error::Download {
            url: shown.clone(),
            reason,
        };
        let parsed = Url::parse(url).map_err(|e| err(e.to_string()))?;
        let _permit = self
            .packagist
            .acquire()
            .await
            .map_err(|e| err(e.to_string()))?;
        let mut extra: Vec<(&str, String)> = Vec::new();
        if let Some(since) = if_modified_since {
            extra.push(("If-Modified-Since", since.to_owned()));
        }
        let (method, body) = match &post {
            Some((content_type, body)) => {
                extra.push(("Content-Type", (*content_type).to_owned()));
                (reqwest::Method::POST, Some(body.as_slice()))
            }
            None => (reqwest::Method::GET, None),
        };
        let mut attempt = 0;
        loop {
            let outcome = match self
                .send(&self.client, &parsed, &method, body, &extra)
                .await
            {
                Ok(r) => {
                    let status = r.status();
                    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
                        Err(Failure::Retry(format!("HTTP {status}")))
                    } else {
                        let header = |name: reqwest::header::HeaderName| {
                            r.headers()
                                .get(name)
                                .and_then(|v| v.to_str().ok())
                                .map(str::to_owned)
                        };
                        let last_modified = header(reqwest::header::LAST_MODIFIED);
                        let max_age =
                            header(reqwest::header::CACHE_CONTROL).and_then(|c| max_age(&c));
                        match r.bytes().await {
                            Ok(bytes) => Ok(Response {
                                status: status.as_u16(),
                                body: bytes.to_vec(),
                                last_modified,
                                max_age,
                            }),
                            Err(e) => Err(Failure::Retry(chain(&e))),
                        }
                    }
                }
                Err(f) => Err(f),
            };
            match outcome {
                Ok(response) => return Ok(response),
                Err(Failure::Retry(_)) if attempt < self.options.retries => {
                    tokio::time::sleep(self.options.retry_delay * 2_u32.pow(attempt)).await;
                    attempt += 1;
                }
                Err(Failure::Retry(reason)) => return Err(err(reason)),
                Err(Failure::Fatal(e)) => return Err(e),
            }
        }
    }

    async fn attempt(&self, start: &Url) -> Result<Vec<u8>, Failure> {
        let fatal = |reason: String| {
            Failure::Fatal(Error::Download {
                url: sanitize(start.as_str()),
                reason,
            })
        };
        let i = self.next_client.fetch_add(1, Ordering::Relaxed) % self.dist_clients.len();
        let mut response = self
            .send(
                &self.dist_clients[i],
                start,
                &reqwest::Method::GET,
                None,
                &[],
            )
            .await?;
        let status = response.status();
        if !status.is_success() {
            let reason = format!("HTTP {status}");
            let final_host = response.url().host_str().unwrap_or_default();
            let retry_400 = status == StatusCode::BAD_REQUEST
                && self.options.retry_400_hosts.iter().any(|h| h == final_host);
            return Err(
                if retry_400
                    || status.is_server_error()
                    || status == StatusCode::TOO_MANY_REQUESTS
                    || status == StatusCode::REQUEST_TIMEOUT
                {
                    Failure::Retry(reason)
                } else {
                    fatal(reason)
                },
            );
        }
        let limit = self.options.max_bytes;
        if response.content_length().is_some_and(|n| n > limit) {
            return Err(fatal(format!("larger than {limit} bytes")));
        }
        let mut body = Vec::with_capacity(
            usize::try_from(response.content_length().unwrap_or(0).min(limit)).unwrap_or(0),
        );
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| Failure::Retry(chain(&e)))?
        {
            if body.len() as u64 + chunk.len() as u64 > limit {
                return Err(fatal(format!("larger than {limit} bytes")));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

/// `https://codeload.github.com/<owner>/<repo>/legacy.zip/<sha>` for a
/// public `api.github.com` zipball pinned to a full commit: the URL the API
/// redirects to, with the same bytes. `None` when a GitHub token applies,
/// since private repositories need the API's signed redirect.
fn codeload(url: &Url, auth: &Auth) -> Option<Url> {
    if url.scheme() != "https"
        || url.host_str() != Some("api.github.com")
        || url.query().is_some()
        || auth.credential(url, None).is_some()
    {
        return None;
    }
    let parts: Vec<&str> = url.path_segments()?.collect();
    let ["repos", owner, repo, "zipball", sha] = parts.as_slice() else {
        return None;
    };
    let pinned = sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit());
    let plain = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    if !pinned || !plain(owner) || !plain(repo) {
        return None;
    }
    Url::parse(&format!(
        "https://codeload.github.com/{owner}/{repo}/legacy.zip/{sha}"
    ))
    .ok()
}

fn verify(url: &str, shasum: Option<&str>, bytes: Vec<u8>) -> Result<Vec<u8>> {
    if let Some(expected) = shasum.filter(|s| !s.is_empty()) {
        let actual = hex_sha1(&bytes);
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(Error::Checksum {
                url: sanitize(url),
                expected: expected.to_owned(),
                actual,
            });
        }
    }
    Ok(bytes)
}

/// A dist URL that is a file on disk (artifact repos): `file://` or a path.
fn local_path(url: &str) -> Option<std::path::PathBuf> {
    if let Some(rest) = url.strip_prefix("file://") {
        return Some(rest.into());
    }
    let scheme = url.find("://").is_some_and(|i| {
        url[..i]
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-'))
    });
    (!scheme).then(|| url.into())
}

/// A metadata response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    pub last_modified: Option<String>,
    /// `Cache-Control: max-age`, in seconds.
    pub max_age: Option<u64>,
}

fn max_age(cache_control: &str) -> Option<u64> {
    cache_control
        .split(',')
        .filter_map(|d| d.trim().strip_prefix("max-age="))
        .find_map(|v| v.trim().parse().ok())
}

fn chain(err: &dyn std::error::Error) -> String {
    let mut out = err.to_string();
    let mut source = err.source();
    while let Some(s) = source {
        out.push_str(": ");
        out.push_str(&s.to_string());
        source = s.source();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{FetchOptions, Fetcher, USER_AGENT};
    use crate::auth::Auth;
    use reqwest::Url;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Answers every connection with `status` and `body`, and returns the
    /// request heads it saw.
    async fn token_server(
        status: &'static str,
        body: &'static str,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            let mut seen = Vec::new();
            while let Ok(Ok((mut sock, _))) =
                tokio::time::timeout(std::time::Duration::from_millis(300), listener.accept()).await
            {
                let mut buf = vec![0_u8; 8192];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                seen.push(String::from_utf8_lossy(&buf[..n]).into_owned());
                let reply = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(reply.as_bytes()).await;
            }
            seen
        });
        (base, handle)
    }

    fn consumer_fetcher(token_url: String) -> Fetcher {
        let mut auth = Auth::from_json(
            "t",
            r#"{"bitbucket-oauth":{"bitbucket.org":{"consumer-key":"k","consumer-secret":"s"}}}"#,
        )
        .unwrap();
        auth.bitbucket_token_url = token_url;
        Fetcher::new(FetchOptions {
            auth,
            ..FetchOptions::default()
        })
        .unwrap()
    }

    #[tokio::test]
    async fn exchanges_a_bitbucket_consumer_for_a_token_once() {
        let (base, seen) =
            token_server("200 OK", r#"{"access_token":"bbtok","expires_in":7200}"#).await;
        let fetcher = consumer_fetcher(format!("{base}/site/oauth2/access_token"));
        let url = Url::parse("https://bitbucket.org/o/r/get/abc.zip").unwrap();
        for _ in 0..2 {
            let (_, headers) = fetcher.headers_for(&url).await.ok().unwrap();
            assert_eq!(
                headers,
                [("Authorization".to_owned(), "Bearer bbtok".to_owned())]
            );
        }
        let seen = seen.await.unwrap();
        assert_eq!(seen.len(), 1);
        assert!(
            seen[0].starts_with("POST /site/oauth2/access_token"),
            "{}",
            seen[0]
        );
        assert!(seen[0].contains("authorization: Basic azpz"), "{}", seen[0]);
        assert!(
            seen[0].contains("grant_type=client_credentials"),
            "{}",
            seen[0]
        );
        assert_eq!(fetcher.auth().origin(&url), "bitbucket.org");
    }

    #[tokio::test]
    async fn reports_a_rejected_bitbucket_consumer() {
        let (base, _) = token_server("401 Unauthorized", "").await;
        let fetcher = consumer_fetcher(format!("{base}/token"));
        let url = Url::parse("https://bitbucket.org/o/r/get/abc.zip").unwrap();
        let Err(super::Failure::Fatal(err)) = fetcher.headers_for(&url).await else {
            panic!("expected a fatal error");
        };
        assert!(err.to_string().contains("invalid OAuth consumer"), "{err}");
        let (base, _) = token_server("200 OK", r#"{"nope":1}"#).await;
        let fetcher = consumer_fetcher(format!("{base}/token"));
        assert!(fetcher.headers_for(&url).await.is_err());
    }

    #[test]
    fn goes_straight_to_codeload_for_public_pinned_zipballs() {
        let sha = "2effe05d2177c451b86c6a073196a4034c02f211";
        let direct = |url: &str, auth: &Auth| {
            super::codeload(&Url::parse(url).unwrap(), auth).map(|u| u.to_string())
        };
        let none = Auth::default();
        assert_eq!(
            direct(
                &format!("https://api.github.com/repos/brick/math/zipball/{sha}"),
                &none
            ),
            Some(format!(
                "https://codeload.github.com/brick/math/legacy.zip/{sha}"
            ))
        );
        for url in [
            "https://api.github.com/repos/brick/math/zipball/main".to_owned(),
            format!("https://api.github.com/repos/brick/math/tarball/{sha}"),
            format!("https://api.github.com/repos/brick/math/zipball/{sha}?x=1"),
            format!("http://api.github.com/repos/brick/math/zipball/{sha}"),
            format!("https://github.example/repos/brick/math/zipball/{sha}"),
            format!("https://api.github.com/repos/br%20ick/math/zipball/{sha}"),
            format!("https://api.github.com/repos/brick/math/zipball/{sha}/extra"),
        ] {
            assert_eq!(direct(&url, &none), None, "{url}");
        }
        let token = Auth::from_json("t", r#"{"github-oauth":{"github.com":"ghp_x"}}"#).unwrap();
        assert_eq!(
            direct(
                &format!("https://api.github.com/repos/brick/math/zipball/{sha}"),
                &token
            ),
            None,
            "a token may be for a private repository"
        );
    }

    #[test]
    fn user_agent_names_the_project() {
        assert!(USER_AGENT.starts_with("phpm/"));
        assert!(USER_AGENT.ends_with("(+https://github.com/speedsharmaai/phpm)"));
    }

    #[tokio::test]
    async fn rejects_urls_it_cannot_parse() {
        let fetcher = Fetcher::new(FetchOptions::default()).unwrap();
        let err = fetcher.fetch("ht tp://x y", None).await.unwrap_err();
        assert!(
            err.to_string()
                .starts_with("could not download ht tp://x y"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn reads_local_artifacts_and_checks_their_sha1() {
        let tmp = crate::testutil::TempDir::new("local-dist");
        let file = tmp.path().join("a.zip");
        std::fs::write(&file, b"abc").unwrap();
        let fetcher = Fetcher::new(FetchOptions::default()).unwrap();
        let path = file.to_string_lossy().into_owned();
        assert_eq!(fetcher.fetch(&path, None).await.unwrap(), b"abc");
        let sha = "a9993e364706816aba3e25717850c26c9cd0d89d";
        assert_eq!(
            fetcher
                .fetch(&format!("file://{path}"), Some(sha))
                .await
                .unwrap(),
            b"abc"
        );
        assert!(fetcher.fetch(&path, Some("00")).await.is_err());
        let missing = tmp
            .path()
            .join("missing.zip")
            .to_string_lossy()
            .into_owned();
        assert!(fetcher.fetch(&missing, None).await.is_err());
    }
}
