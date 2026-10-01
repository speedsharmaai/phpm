//! Dist downloads: bounded per host class, retried, verified, buffered in memory.

use std::sync::{Arc, Once};
use std::time::Duration;

use reqwest::{StatusCode, Url};
use tokio::sync::Semaphore;

use crate::auth::{Auth, Credential};
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
    pub auth: Auth,
}

impl Default for FetchOptions {
    fn default() -> Self {
        Self {
            packagist_concurrency: 10,
            github_concurrency: 24,
            other_concurrency: 10,
            retries: 3,
            retry_delay: Duration::from_millis(500),
            max_bytes: 512 << 20,
            retry_400_hosts: vec!["codeload.github.com".into()],
            auth: Auth::default(),
        }
    }
}

/// An HTTP/2 client with per-host-class concurrency limits. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Fetcher {
    client: reqwest::Client,
    packagist: Arc<Semaphore>,
    github: Arc<Semaphore>,
    other: Arc<Semaphore>,
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
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(options.github_concurrency)
            .build()
            .map_err(|e| Error::Download {
                url: String::new(),
                reason: e.to_string(),
            })?;
        Ok(Self {
            client,
            packagist: Arc::new(Semaphore::new(options.packagist_concurrency.max(1))),
            github: Arc::new(Semaphore::new(options.github_concurrency.max(1))),
            other: Arc::new(Semaphore::new(options.other_concurrency.max(1))),
            options: Arc::new(options),
        })
    }

    /// Download `url`, retrying transient failures, and check `shasum` if given.
    pub async fn fetch(&self, url: &str, shasum: Option<&str>) -> Result<Vec<u8>> {
        let download_err = |reason: String| Error::Download {
            url: url.to_owned(),
            reason,
        };
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

        let mut attempt = 0;
        let bytes = loop {
            match self.attempt(&parsed).await {
                Ok(bytes) => break bytes,
                Err(Failure::Retry(_)) if attempt < self.options.retries => {
                    tokio::time::sleep(self.options.retry_delay * 2_u32.pow(attempt)).await;
                    attempt += 1;
                }
                Err(Failure::Retry(reason)) => return Err(download_err(reason)),
                Err(Failure::Fatal(e)) => return Err(e),
            }
        };

        if let Some(expected) = shasum.filter(|s| !s.is_empty()) {
            let actual = hex_sha1(&bytes);
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(Error::Checksum {
                    url: url.to_owned(),
                    expected: expected.to_owned(),
                    actual,
                });
            }
        }
        Ok(bytes)
    }

    async fn attempt(&self, url: &Url) -> Result<Vec<u8>, Failure> {
        let fatal = |reason: String| {
            Failure::Fatal(Error::Download {
                url: url.to_string(),
                reason,
            })
        };
        let mut request = self.client.get(url.clone());
        match self
            .options
            .auth
            .credential(url.host_str().unwrap_or_default(), url.port())
        {
            Some(Credential::GithubToken(token)) => {
                request = request.header("Authorization", format!("token {token}"));
            }
            Some(Credential::Basic { username, password }) => {
                request = request.basic_auth(username, Some(password));
            }
            None => {}
        }
        let mut response = request
            .send()
            .await
            .map_err(|e| Failure::Retry(chain(&e)))?;
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

    #[test]
    fn user_agent_names_the_project() {
        assert!(USER_AGENT.starts_with("phpm/"));
        assert!(USER_AGENT.ends_with("(+https://github.com/speedsharmaai/phpm)"));
    }

    #[tokio::test]
    async fn rejects_urls_it_cannot_parse() {
        let fetcher = Fetcher::new(FetchOptions::default()).unwrap();
        let err = fetcher.fetch("not a url", None).await.unwrap_err();
        assert!(err.to_string().starts_with("could not download not a url"));
    }
}
