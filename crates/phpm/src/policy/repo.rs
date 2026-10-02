//! Composer repositories as an install from a lock sees them: their root
//! `packages.json`, the filter summary and per-package metadata, each cached
//! under `<cache>/repo/<url>/` with the `Last-Modified` it came with.
//!
//! The root file is reused for 600 seconds, as `loadRootServerFile(600)`
//! does; everything else is revalidated with `If-Modified-Since`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use phpm_lock::ComposerJson;
use phpm_store::{Fetcher, sanitize};
use serde_json::{Value, json};

pub(crate) const PACKAGIST: &str = "https://repo.packagist.org";
pub(crate) const ROOT_MAX_AGE: u64 = 600;
const VERIFIED: &str = "filter-verified.json";

/// A `composer` repository and the `filter` option its config gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Repo {
    pub(crate) url: String,
    /// `"filter": false` on the repository.
    pub(crate) filter_off: bool,
    /// Lists turned off with `"filter": {"name": false}`.
    pub(crate) skip_lists: Vec<String>,
}

impl Repo {
    fn new(url: &str) -> Self {
        Self {
            url: url.trim_end_matches('/').to_owned(),
            filter_off: false,
            skip_lists: Vec::new(),
        }
    }

    fn packages_json(&self) -> String {
        if Path::new(&self.url)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        {
            self.url.clone()
        } else {
            format!("{}/packages.json", self.url)
        }
    }

    // Composer: Repository/ComposerRepository.php canonicalizeUrl
    pub(crate) fn canonical(&self, url: &str) -> String {
        if !url.starts_with('/') {
            return url.to_owned();
        }
        match self.url.find("://") {
            Some(i) if i > 0 && !self.url[..i].contains(':') => {
                let start = i + 3;
                let end = self.url[start..]
                    .find('/')
                    .map_or(self.url.len(), |j| start + j);
                format!("{}{url}", &self.url[..end])
            }
            _ => self.url.clone(),
        }
    }

    fn cache_dir(&self, cache: &Path) -> PathBuf {
        let name: String = sanitize(&self.url)
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        cache.join("repo").join(name)
    }
}

/// A `{"type": "composer", "url": ...}` entry with its `filter` option.
fn composer_repo(obj: &serde_json::Map<String, Value>) -> Option<Repo> {
    if obj.get("type").and_then(Value::as_str) != Some("composer") {
        return None;
    }
    let mut repo = Repo::new(obj.get("url").and_then(Value::as_str)?);
    match obj.get("filter") {
        Some(Value::Bool(false)) => repo.filter_off = true,
        Some(Value::Object(lists)) => {
            repo.skip_lists = lists
                .iter()
                .filter(|(_, v)| **v == Value::Bool(false))
                .map(|(k, _)| k.clone())
                .collect();
        }
        _ => {}
    }
    Some(repo)
}

// Composer: Config::merge, the packagist.org pattern
fn is_packagist_url(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or_default();
    host == "packagist.org"
        || host.strip_suffix(".packagist.org").is_some_and(|sub| {
            !sub.is_empty()
                && sub
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
        })
}

/// The composer repositories in composer.json, then Packagist unless it is
/// turned off or redefined; the order Composer's `RepositoryManager` keeps.
/// A repository named `packagist` or `packagist.org` takes Packagist's place.
// Composer: Config::merge, disableRepoByName
pub(crate) fn repos(composer: &ComposerJson) -> Vec<Repo> {
    let mut out = Vec::new();
    let mut packagist = Some(Repo::new(PACKAGIST));
    let entries: Vec<(Option<&str>, &Value)> = match composer.data().get("repositories") {
        Some(Value::Array(list)) => list.iter().map(|v| (None, v)).collect(),
        Some(Value::Object(map)) => map.iter().map(|(k, v)| (Some(k.as_str()), v)).collect(),
        _ => Vec::new(),
    };
    for (key, entry) in entries {
        let named_packagist = matches!(key, Some("packagist" | "packagist.org"));
        if entry == &Value::Bool(false) {
            if named_packagist {
                packagist = None;
            }
            continue;
        }
        let Some(obj) = entry.as_object() else {
            continue;
        };
        if obj.len() == 1
            && (obj.get("packagist.org") == Some(&Value::Bool(false))
                || obj.get("packagist") == Some(&Value::Bool(false)))
        {
            packagist = None;
            continue;
        }
        let repo = composer_repo(obj);
        if named_packagist {
            packagist = repo;
            continue;
        }
        let Some(repo) = repo else {
            continue;
        };
        if is_packagist_url(&repo.url) {
            packagist = None;
        }
        out.push(repo);
    }
    out.extend(packagist);
    out
}

/// A cached JSON document and when it was last confirmed fresh.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Cached {
    pub(crate) data: Value,
    last_modified: Option<String>,
    pub(crate) checked: u64,
    pub(crate) max_age: Option<u64>,
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn read(path: &Path) -> Option<Cached> {
    let record: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    Some(Cached {
        data: record.get("data")?.clone(),
        last_modified: record
            .get("last-modified")
            .and_then(Value::as_str)
            .map(str::to_owned),
        checked: record.get("checked")?.as_u64()?,
        max_age: record.get("max-age").and_then(Value::as_u64),
    })
}

fn write(path: &Path, cached: &Cached) {
    let record = json!({
        "last-modified": cached.last_modified,
        "checked": cached.checked,
        "max-age": cached.max_age,
        "data": cached.data,
    });
    save(path, &record);
}

fn save(path: &Path, record: &Value) {
    if let Some(dir) = path.parent()
        && fs::create_dir_all(dir).is_ok()
    {
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        if fs::write(&tmp, record.to_string()).is_ok() {
            let _ = fs::rename(&tmp, path);
        }
    }
}

/// Fetches repository metadata through the cache.
#[derive(Debug, Clone)]
pub(crate) struct Client<'a> {
    pub(crate) fetcher: &'a Fetcher,
    pub(crate) cache: Option<PathBuf>,
}

/// A fetch that could not be done: what to print for it.
pub(crate) type Unreachable = String;

impl Client<'_> {
    /// `url` as JSON; `max_age` reuses the cached copy that long without asking.
    pub(crate) async fn json(
        &self,
        repo: &Repo,
        key: &str,
        url: &str,
        max_age: Option<u64>,
    ) -> Result<Option<Cached>, Unreachable> {
        let path = self.cache.as_ref().map(|c| repo.cache_dir(c).join(key));
        let cached = path.as_deref().and_then(read);
        if let (Some(c), Some(age)) = (&cached, max_age)
            && now().saturating_sub(c.checked) <= age
        {
            return Ok(cached);
        }
        let since = cached.as_ref().and_then(|c| c.last_modified.clone());
        let response = self
            .fetcher
            .request(url, since.as_deref(), None)
            .await
            .map_err(|e| e.to_string())?;
        let fresh = match (response.status, cached) {
            (304, Some(mut c)) => {
                c.checked = now();
                c.max_age = response.max_age.or(c.max_age);
                c
            }
            (404, _) => return Ok(None),
            (200..=299, _) => Cached {
                data: serde_json::from_slice(&response.body)
                    .map_err(|e| format!("{}: {e}", sanitize(url)))?,
                last_modified: response.last_modified,
                checked: now(),
                max_age: response.max_age,
            },
            (status, _) => {
                return Err(format!(
                    "The \"{}\" file could not be downloaded (HTTP {status})",
                    sanitize(url)
                ));
            }
        };
        if let Some(path) = &path {
            write(path, &fresh);
        }
        Ok(Some(fresh))
    }

    /// When `packages` (as `name version`) were all found clean on `lists`
    /// by a check that is still fresh: the time that check goes stale.
    pub(crate) fn verified(
        &self,
        repo: &Repo,
        lists: &[String],
        packages: &[String],
    ) -> Option<u64> {
        let path = repo.cache_dir(self.cache.as_ref()?).join(VERIFIED);
        let record: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
        let until = record.get("until")?.as_u64()?;
        let same_lists = record.get("lists")? == &json!(lists);
        let clean: Vec<&str> = record
            .get("clean")?
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .collect();
        (same_lists && now() < until && packages.iter().all(|p| clean.contains(&p.as_str())))
            .then_some(until)
    }

    /// Remember that `clean` had no entries on `lists` until `until`.
    pub(crate) fn record(&self, repo: &Repo, lists: &[String], until: u64, clean: &[String]) {
        let Some(cache) = &self.cache else {
            return;
        };
        save(
            &repo.cache_dir(cache).join(VERIFIED),
            &json!({"lists": lists, "until": until, "clean": clean}),
        );
    }

    /// The repository's root `packages.json`.
    pub(crate) async fn root(&self, repo: &Repo) -> Result<Option<Cached>, Unreachable> {
        self.json(
            repo,
            "packages.json",
            &repo.packages_json(),
            Some(ROOT_MAX_AGE),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::{Cached, Client, PACKAGIST, Repo, is_packagist_url, read, repos, write};
    use phpm_lock::ComposerJson;
    use serde_json::json;
    use std::path::Path;

    fn composer(v: serde_json::Value) -> ComposerJson {
        ComposerJson::from_value(v).unwrap()
    }

    #[test]
    fn lists_composer_repos_then_packagist() {
        let r = repos(&composer(json!({})));
        assert_eq!(r, [Repo::new(PACKAGIST)]);
        let r = repos(&composer(json!({"repositories": [
            {"type": "composer", "url": "https://satis.corp/", "filter": false},
            {"type": "composer", "url": "https://p.corp", "filter": {"malware": false, "x": true}},
            {"type": "vcs", "url": "https://github.com/a/b"},
            {"type": "composer"},
            "junk",
        ]})));
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].url, "https://satis.corp");
        assert!(r[0].filter_off);
        assert_eq!(r[1].skip_lists, ["malware"]);
        assert_eq!(r[2].url, PACKAGIST);
        for off in [
            json!({"repositories": {"packagist": {"type": "vcs", "url": "https://x.test/r"}}}),
            json!({"repositories": [{"packagist.org": false}]}),
            json!({"repositories": [{"packagist": false}]}),
            json!({"repositories": {"packagist.org": false}}),
        ] {
            assert!(repos(&composer(off)).is_empty());
        }
    }

    #[test]
    fn a_repository_named_packagist_takes_its_place() {
        let r = repos(&composer(json!({"repositories": {
            "packagist": {"type": "composer", "url": "https://mirrors.aliyun.com/composer/"},
            "corp": {"type": "composer", "url": "https://satis.corp"},
        }})));
        let urls: Vec<&str> = r.iter().map(|r| r.url.as_str()).collect();
        assert_eq!(
            urls,
            ["https://satis.corp", "https://mirrors.aliyun.com/composer"]
        );
        let r = repos(&composer(json!({"repositories": [
            {"type": "composer", "url": "https://repo.packagist.org"},
        ]})));
        assert_eq!(r, [Repo::new(PACKAGIST)]);
        let r = repos(&composer(json!({"repositories": {"other": false}})));
        assert_eq!(r, [Repo::new(PACKAGIST)]);
        assert!(is_packagist_url("http://packagist.org"));
        assert!(is_packagist_url("https://repo.packagist.org/x"));
        assert!(!is_packagist_url("https://notpackagist.org"));
        assert!(!is_packagist_url("https://.packagist.org"));
        assert!(!is_packagist_url("ftp://packagist.org"));
    }

    #[test]
    fn builds_urls_like_composer() {
        let r = Repo::new("https://repo.example.com/sub/");
        assert_eq!(
            r.packages_json(),
            "https://repo.example.com/sub/packages.json"
        );
        assert_eq!(
            Repo::new("https://x/p.json").packages_json(),
            "https://x/p.json"
        );
        assert_eq!(
            r.canonical("/lists/s.json"),
            "https://repo.example.com/lists/s.json"
        );
        assert_eq!(
            r.canonical("//cdn/s.json"),
            "https://repo.example.com//cdn/s.json"
        );
        assert_eq!(r.canonical("https://o/s.json"), "https://o/s.json");
        assert_eq!(Repo::new("relative").canonical("/x"), "relative");
        let dir = Repo::new("https://u:p@repo.packagist.org").cache_dir(Path::new("/c"));
        assert_eq!(dir, Path::new("/c/repo/https---u-----repo.packagist.org"));
    }

    /// Answers `/packages.json` with 200 and a Last-Modified, or 304 when
    /// asked with If-Modified-Since; `/gone.json` is 404 and `/boom.json` 403.
    fn server() -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut sock) = stream else { continue };
                let mut buf = vec![0_u8; 4096];
                let n = sock.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
                let (status, extra, body) = if head.starts_with("get /gone.json") {
                    ("404 Not Found", "", "")
                } else if head.starts_with("get /boom.json") {
                    ("403 Forbidden", "", "")
                } else if head.contains("if-modified-since: mon") {
                    ("304 Not Modified", "", "")
                } else {
                    (
                        "200 OK",
                        "Last-Modified: Mon\r\nCache-Control: public, max-age=900\r\n",
                        r#"{"v":1}"#,
                    )
                };
                let reply = format!(
                    "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(reply.as_bytes());
            }
        });
        base
    }

    #[test]
    fn metadata_fetches_send_the_user_agent() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = vec![0_u8; 4096];
            let n = sock.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).into_owned();
            let agent = head
                .lines()
                .find_map(|l| {
                    l.split_once(':')
                        .filter(|(k, _)| k.eq_ignore_ascii_case("user-agent"))
                        .map(|(_, v)| v.trim().to_owned())
                })
                .unwrap_or_default();
            let body = json!({ "agent": agent }).to_string();
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(reply.as_bytes());
        });
        let fetcher = phpm_store::Fetcher::new(phpm_store::FetchOptions::default()).unwrap();
        let client = Client {
            fetcher: &fetcher,
            cache: None,
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let got = rt
            .block_on(client.json(
                &Repo::new(&base),
                "k.json",
                &format!("{base}/list.json"),
                None,
            ))
            .unwrap()
            .unwrap();
        assert_eq!(got.data, json!({ "agent": phpm_store::USER_AGENT }));
    }

    #[test]
    fn revalidates_with_if_modified_since() {
        let base = server();
        let tmp = tempfile::tempdir().unwrap();
        let fetcher = phpm_store::Fetcher::new(phpm_store::FetchOptions::default()).unwrap();
        let client = Client {
            fetcher: &fetcher,
            cache: Some(tmp.path().to_owned()),
        };
        let repo = Repo::new(&base);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let first = rt
            .block_on(client.json(&repo, "k.json", &format!("{base}/packages.json"), None))
            .unwrap()
            .unwrap();
        assert_eq!(first.data, json!({"v": 1}));
        assert_eq!(first.max_age, Some(900));
        let again = rt
            .block_on(client.json(&repo, "k.json", &format!("{base}/packages.json"), None))
            .unwrap()
            .unwrap();
        assert_eq!(again.data, json!({"v": 1}));
        let root = rt.block_on(client.root(&repo)).unwrap().unwrap();
        assert_eq!(root.data, json!({"v": 1}));
        let gone = rt
            .block_on(client.json(&repo, "g.json", &format!("{base}/gone.json"), None))
            .unwrap();
        assert!(gone.is_none());
        let err = rt
            .block_on(client.json(&repo, "b.json", &format!("{base}/boom.json"), None))
            .unwrap_err();
        assert!(err.contains("HTTP 403"), "{err}");
        let uncached = Client {
            fetcher: &fetcher,
            cache: None,
        };
        assert!(rt.block_on(uncached.root(&repo)).unwrap().is_some());
    }

    #[test]
    fn round_trips_cache_records() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("r/packages.json");
        let c = Cached {
            data: json!({"a": 1}),
            last_modified: Some("Mon".into()),
            checked: 5,
            max_age: Some(900),
        };
        write(&path, &c);
        assert_eq!(read(&path), Some(c));
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(read(&path), None);
    }
}
