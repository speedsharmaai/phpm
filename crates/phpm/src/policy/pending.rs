//! The install-time malware check, run on a thread of its own from the start
//! of an install so its round trips overlap everything up to the first change
//! to `vendor/`. Nothing is placed or removed before the verdict.

use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use phpm_store::{Auth, FetchOptions, Fetcher, Package, Store};

use super::Policy;
use super::filter::{Locked, Outcome, Refusal, check_install};
use super::repo::{Client, Repo};
use crate::error::Error;

type Verdict = Result<Outcome, Refusal>;

/// A filter check that may still be running.
#[derive(Debug)]
pub(crate) struct Pending {
    started: Instant,
    handle: Option<JoinHandle<Verdict>>,
    done: Option<Outcome>,
}

impl Pending {
    /// Start the check when the policy blocks at install time.
    pub(crate) fn start(
        policy: &Policy,
        repos: &[Repo],
        packages: Vec<Locked>,
        auth: impl FnOnce() -> Result<Auth, Error>,
        cache: Option<PathBuf>,
    ) -> Result<Self, Error> {
        let started = Instant::now();
        if !policy.blocks_install() || repos.is_empty() {
            return Ok(Self {
                started,
                handle: None,
                done: Some(Outcome::default()),
            });
        }
        let auth = auth()?;
        let policy = policy.clone();
        let repos = repos.to_vec();
        let handle = std::thread::spawn(move || {
            let fetcher = Fetcher::new(FetchOptions {
                auth,
                ..FetchOptions::default()
            })
            .map_err(|e| Refusal::from(Error::from(e)))?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| {
                    Refusal::from(Error::install(format!(
                        "cannot start the filter runtime: {e}"
                    )))
                })?;
            let client = Client {
                fetcher: &fetcher,
                cache,
            };
            runtime.block_on(check_install(&client, &repos, &policy, &packages))
        });
        Ok(Self {
            started,
            handle: Some(handle),
            done: None,
        })
    }

    /// Wait for the verdict. Refused packages that `fetched` put in the store
    /// this run are taken out again.
    pub(crate) fn verdict(
        &mut self,
        store: &Store,
        fetched: &[Package],
    ) -> Result<(&Outcome, Option<Duration>), Error> {
        let waited = match self.handle.take() {
            None => None,
            Some(handle) => {
                let verdict = handle
                    .join()
                    .map_err(|_| Error::install("the malware filter check failed"))?;
                match verdict {
                    Ok(outcome) => self.done = Some(outcome),
                    Err(refusal) => {
                        for p in fetched.iter().filter(|p| refusal.blocked.contains(&p.name)) {
                            let _ = store.remove(&p.name, &p.key());
                        }
                        return Err(refusal.error);
                    }
                }
                Some(self.started.elapsed())
            }
        };
        let outcome = self.done.get_or_insert_with(Outcome::default);
        Ok((outcome, waited))
    }
}

#[cfg(test)]
mod tests {
    use super::Pending;
    use crate::policy::{Locked, Policy, repos};
    use phpm_lock::ComposerJson;
    use phpm_store::{Auth, Dist, Package, Store};
    use serde_json::json;

    fn policy(no_blocking: bool) -> Policy {
        Policy::from_config(None, &|_| None, no_blocking).unwrap()
    }

    #[test]
    fn nothing_runs_when_blocking_is_off() {
        let composer = ComposerJson::from_value(json!({})).unwrap();
        let mut p = Pending::start(
            &policy(true),
            &repos(&composer),
            Vec::new(),
            || panic!("auth is not loaded when nothing is checked"),
            None,
        )
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let (outcome, waited) = p.verdict(&Store::new(tmp.path()), &[]).unwrap();
        assert!(outcome.warnings.is_empty() && waited.is_none());
    }

    #[test]
    fn a_refusal_takes_fetched_packages_out_of_the_store() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut sock) = stream else { continue };
                let mut buf = vec![0_u8; 4096];
                let n = sock.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                let body = if head.starts_with("POST") {
                    r#"{"filter":{"malware":[{"package":"a/bad","constraint":"*"}]}}"#
                } else {
                    r#"{"filter":{"metadata":true,"lists":{"malware":{"enabled":true}},"api-url":"/api"}}"#
                };
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(reply.as_bytes());
            }
        });
        let composer = ComposerJson::from_value(json!({"repositories": [
            {"type": "composer", "url": base}, {"packagist.org": false},
        ]}))
        .unwrap();
        let locked = |name: &str| Locked {
            name: name.into(),
            pretty_name: name.into(),
            pretty: "1.0.0".into(),
            version: "1.0.0.0".into(),
        };
        let mut p = Pending::start(
            &policy(false),
            &repos(&composer),
            vec![locked("a/bad"), locked("a/good")],
            || Ok(Auth::default()),
            None,
        )
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        let package = |name: &str| {
            Package::new(
                name,
                Dist {
                    kind: "zip".into(),
                    url: String::new(),
                    reference: Some("r".into()),
                    shasum: None,
                },
            )
        };
        let fetched = [package("a/bad"), package("a/good")];
        for f in &fetched {
            std::fs::create_dir_all(store.path(&f.name, "r").unwrap()).unwrap();
        }
        let err = p.verdict(&store, &fetched).unwrap_err();
        assert_eq!(err.code, 2);
        assert!(err.message.contains("a/bad 1.0.0"), "{}", err.message);
        assert!(!store.contains("a/bad", "r"));
        assert!(store.contains("a/good", "r"));
    }
}
