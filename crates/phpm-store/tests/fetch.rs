//! Fetch, retry, redirect, auth and placement against a local HTTP server.

#![allow(clippy::unwrap_used, reason = "test helpers panic on setup failures")]

use std::collections::BTreeMap;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use phpm_store::{
    Auth, Dist, Error, FetchOptions, Fetcher, LinkMode, Package, Store, USER_AGENT, place,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use zip::write::SimpleFileOptions;

#[derive(Default)]
struct Log {
    hits: BTreeMap<String, usize>,
    headers: Vec<String>,
}

struct Server {
    base: String,
    log: Arc<Mutex<Log>>,
}

impl Server {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let log = Arc::new(Mutex::new(Log::default()));
        let shared = Arc::clone(&log);
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                let log = Arc::clone(&shared);
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0_u8; 4096];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        let n = sock.read(&mut chunk).await.unwrap();
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    let head = String::from_utf8_lossy(&buf).into_owned();
                    let path = head.split(' ').nth(1).unwrap_or("/").to_owned();
                    let hit = {
                        let mut log = log.lock().unwrap();
                        log.headers.push(head.clone());
                        let n = log.hits.entry(path.clone()).or_default();
                        *n += 1;
                        *n
                    };
                    let (status, extra, body) = respond(&path, hit, &head);
                    let mut out = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
                        body.len()
                    )
                    .into_bytes();
                    out.extend_from_slice(&body);
                    let _ = sock.write_all(&out).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        Self { base, log }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn hits(&self, path: &str) -> usize {
        self.log
            .lock()
            .unwrap()
            .hits
            .get(path)
            .copied()
            .unwrap_or(0)
    }
}

fn respond(path: &str, hit: usize, head: &str) -> (&'static str, String, Vec<u8>) {
    let ok = |name: &str| ("200 OK", String::new(), package_zip(name));
    match path {
        "/flaky.zip" if hit <= 2 => ("503 Service Unavailable", String::new(), Vec::new()),
        "/flaky.zip" => ok("flaky"),
        "/redirect.zip" => (
            "302 Found",
            "Location: /pkg/moved.zip\r\n".into(),
            Vec::new(),
        ),
        "/bad-400.zip" if hit == 1 => ("400 Bad Request", String::new(), Vec::new()),
        "/bad-400.zip" => ok("bad400"),
        "/always-503.zip" => ("503 Service Unavailable", String::new(), Vec::new()),
        "/private.zip" if head.contains("authorization: Basic dTpw") => ok("private"),
        "/bearer.zip" if head.contains("authorization: Bearer bt") => ok("bearer"),
        "/custom.zip" if head.contains("x-api-key: k1") => ok("custom"),
        "/hop.zip" => {
            let host = head
                .lines()
                .find_map(|l| l.strip_prefix("host: "))
                .unwrap_or_default()
                .replace("127.0.0.1", "localhost");
            (
                "302 Found",
                format!("Location: http://{host}/no-auth.zip\r\n"),
                Vec::new(),
            )
        }
        "/no-auth.zip" if head.contains("authorization") => {
            ("403 Forbidden", String::new(), Vec::new())
        }
        "/no-auth.zip" => ok("hopped"),
        "/loop.zip" => ("302 Found", "Location: /loop.zip\r\n".into(), Vec::new()),
        "/private.zip" => ("401 Unauthorized", String::new(), Vec::new()),
        "/big.zip" => ("200 OK", String::new(), vec![0; 4096]),
        p if p.starts_with("/pkg/") => {
            let name = p.trim_start_matches("/pkg/").trim_end_matches(".zip");
            ok(name)
        }
        _ => ("404 Not Found", String::new(), Vec::new()),
    }
}

fn package_zip(name: &str) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let top = format!("owner-{name}-abc1234/");
    let file = SimpleFileOptions::default().unix_permissions(0o644);
    zip.add_directory(&top, SimpleFileOptions::default())
        .unwrap();
    zip.start_file(format!("{top}src/{name}.php"), file)
        .unwrap();
    zip.write_all(format!("<?php // {name}\n").as_bytes())
        .unwrap();
    zip.start_file(
        format!("{top}bin/{name}"),
        SimpleFileOptions::default().unix_permissions(0o755),
    )
    .unwrap();
    zip.write_all(b"#!/usr/bin/env php\n").unwrap();
    zip.finish().unwrap().into_inner()
}

fn options() -> FetchOptions {
    FetchOptions {
        retry_delay: Duration::from_millis(1),
        ..FetchOptions::default()
    }
}

fn package(name: &str, url: String) -> Package {
    Package::new(
        name,
        Dist {
            kind: "zip".into(),
            url,
            reference: Some(format!("ref-{}", name.replace('/', "-"))),
            shasum: None,
        },
    )
}

struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("fetch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn fetches_extracts_and_places_packages() {
    let server = Server::start().await;
    let tmp = Temp::new("place");
    let store = Store::new(&tmp.0.join("cache"));
    let fetcher = Fetcher::new(options()).unwrap();
    let packages = vec![
        package("acme/plain", server.url("/pkg/plain.zip")),
        package("acme/moved", server.url("/redirect.zip")),
        package("acme/flaky", server.url("/flaky.zip")),
    ];

    let fetched = store.fetch_missing(&fetcher, &packages).await.unwrap();
    assert_eq!(fetched, ["acme/flaky", "acme/moved", "acme/plain"]);
    assert_eq!(server.hits("/flaky.zip"), 3);
    assert_eq!(server.hits("/pkg/moved.zip"), 1);

    let again = store.fetch_missing(&fetcher, &packages).await.unwrap();
    assert!(again.is_empty());
    assert_eq!(server.hits("/pkg/plain.zip"), 1);

    let vendor = tmp.0.join("vendor");
    let placements: Vec<_> = packages
        .iter()
        .map(|p| store.placement(p).unwrap())
        .collect();
    place(&vendor, &placements, LinkMode::platform_default()).unwrap();
    assert_eq!(
        std::fs::read(vendor.join("acme/moved/src/moved.php")).unwrap(),
        b"<?php // moved\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(vendor.join("acme/flaky/bin/flaky"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    let headers = server.log.lock().unwrap().headers.join("\n");
    assert!(
        headers.contains(&format!("user-agent: {USER_AGENT}")),
        "{headers}"
    );
}

#[tokio::test]
async fn gives_up_after_three_retries() {
    let server = Server::start().await;
    let fetcher = Fetcher::new(options()).unwrap();
    let err = fetcher
        .fetch(&server.url("/always-503.zip"), None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("HTTP 503"), "{err}");
    assert_eq!(server.hits("/always-503.zip"), 4);
}

#[tokio::test]
async fn does_not_retry_a_missing_file() {
    let server = Server::start().await;
    let fetcher = Fetcher::new(options()).unwrap();
    let err = fetcher
        .fetch(&server.url("/nope.zip"), None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("HTTP 404"), "{err}");
    assert_eq!(server.hits("/nope.zip"), 1);
}

#[tokio::test]
async fn retries_400_only_for_listed_hosts() {
    let server = Server::start().await;
    let plain = Fetcher::new(options()).unwrap();
    assert!(
        plain
            .fetch(&server.url("/bad-400.zip"), None)
            .await
            .is_err()
    );

    let codeload_like = Fetcher::new(FetchOptions {
        retry_400_hosts: vec!["127.0.0.1".into()],
        ..options()
    })
    .unwrap();
    let bytes = codeload_like
        .fetch(&server.url("/bad-400.zip"), None)
        .await
        .unwrap();
    assert!(!bytes.is_empty());
    assert_eq!(server.hits("/bad-400.zip"), 2);
}

#[tokio::test]
async fn verifies_sha1_when_the_lock_has_one() {
    let server = Server::start().await;
    let fetcher = Fetcher::new(options()).unwrap();
    let url = server.url("/pkg/sum.zip");
    let bytes = fetcher.fetch(&url, None).await.unwrap();
    let good = sha1_hex(&bytes);
    assert_eq!(fetcher.fetch(&url, Some(&good)).await.unwrap(), bytes);
    assert!(fetcher.fetch(&url, Some("")).await.is_ok());
    let err = fetcher
        .fetch(&url, Some("0000000000000000000000000000000000000000"))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Checksum { .. }), "{err}");
}

#[tokio::test]
async fn sends_http_basic_credentials() {
    let server = Server::start().await;
    let host = server.base.trim_start_matches("http://").to_owned();
    let url = server.url("/private.zip");

    let anonymous = Fetcher::new(options()).unwrap();
    let err = anonymous.fetch(&url, None).await.unwrap_err();
    assert!(err.to_string().contains("HTTP 401"), "{err}");

    let auth = Auth::from_json(
        "test",
        &format!(r#"{{"http-basic":{{"{host}":{{"username":"u","password":"p"}}}}}}"#),
    )
    .unwrap();
    let fetcher = Fetcher::new(FetchOptions { auth, ..options() }).unwrap();
    assert!(fetcher.fetch(&url, None).await.is_ok());
}

#[tokio::test]
async fn refuses_oversized_downloads() {
    let server = Server::start().await;
    let fetcher = Fetcher::new(FetchOptions {
        max_bytes: 1024,
        ..options()
    })
    .unwrap();
    let err = fetcher
        .fetch(&server.url("/big.zip"), None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("larger than 1024 bytes"), "{err}");
}

#[tokio::test]
async fn a_failed_download_fails_the_batch() {
    let server = Server::start().await;
    let tmp = Temp::new("batch");
    let store = Store::new(&tmp.0.join("cache"));
    let fetcher = Fetcher::new(options()).unwrap();
    let packages = [
        package("acme/ok", server.url("/pkg/ok.zip")),
        package("acme/gone", server.url("/gone.zip")),
    ];
    let err = store.fetch_missing(&fetcher, &packages).await.unwrap_err();
    assert!(err.to_string().contains("/gone.zip"), "{err}");
    assert!(!store.contains("acme/gone", "ref-acme-gone"));
}

fn sha1_hex(bytes: &[u8]) -> String {
    use sha1::Digest;
    use std::fmt::Write as _;
    sha1::Sha1::digest(bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

#[tokio::test]
async fn applies_bearer_and_custom_headers_by_host() {
    let server = Server::start().await;
    let host = server.base.trim_start_matches("http://").to_owned();
    let auth = Auth::from_json("test", &format!(r#"{{"bearer":{{"{host}":"bt"}}}}"#)).unwrap();
    let fetcher = Fetcher::new(FetchOptions { auth, ..options() }).unwrap();
    assert!(
        fetcher
            .fetch(&server.url("/bearer.zip"), None)
            .await
            .is_ok()
    );
    let auth = Auth::from_json(
        "test",
        &format!(r#"{{"custom-headers":{{"{host}":["X-Api-Key: k1"]}}}}"#),
    )
    .unwrap();
    let fetcher = Fetcher::new(FetchOptions { auth, ..options() }).unwrap();
    assert!(
        fetcher
            .fetch(&server.url("/custom.zip"), None)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn credentials_do_not_follow_a_redirect_to_another_host() {
    let server = Server::start().await;
    let host = server.base.trim_start_matches("http://").to_owned();
    let auth = Auth::from_json(
        "test",
        &format!(r#"{{"custom-headers":{{"{host}":["Authorization: Bearer leak"]}}}}"#),
    )
    .unwrap();
    let fetcher = Fetcher::new(FetchOptions { auth, ..options() }).unwrap();
    fetcher.fetch(&server.url("/hop.zip"), None).await.unwrap();
    assert_eq!(server.hits("/no-auth.zip"), 1);
    let err = fetcher
        .fetch(&server.url("/loop.zip"), None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("more than 10 redirects"), "{err}");
}

#[tokio::test]
async fn uses_inline_credentials_and_never_prints_them() {
    let server = Server::start().await;
    let host = server.base.trim_start_matches("http://").to_owned();
    let fetcher = Fetcher::new(options()).unwrap();
    fetcher
        .fetch(&format!("http://u:p@{host}/private.zip"), None)
        .await
        .unwrap();
    let err = fetcher
        .fetch(&format!("http://u:secretpw@{host}/gone.zip"), None)
        .await
        .unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("http://u:***@") && !text.contains("secretpw"),
        "{text}"
    );
}
