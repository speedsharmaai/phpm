//! Download notifications, so package authors keep their install counts:
//! one POST per `notification-url` with every package installed or updated
//! in this run, as `InstallationManager::notifyInstalls` sends them.
//!
//! Composer: Installer/InstallationManager.php notifyInstalls, markForNotification.

use std::fmt::Write as _;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use phpm_store::{Auth, FetchOptions, Fetcher};
use serde_json::{Map, Value, json};

/// One installed package with a `notification-url`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Download {
    pub(crate) url: String,
    pub(crate) name: String,
    pub(crate) version: String,
    /// Bytes fetched from the network this run; `None` when the store had it.
    pub(crate) downloaded: Option<u64>,
}

impl Download {
    /// The notification for a lock entry, when it names a `notification-url`.
    pub(crate) fn from_lock(entry: &Map<String, Value>, downloaded: Option<u64>) -> Option<Self> {
        let text = |k: &str| {
            entry
                .get(k)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        };
        let pretty = text("version").unwrap_or_default();
        Some(Self {
            url: text("notification-url")?.to_owned(),
            name: text("name")?.to_owned(),
            version: phpm_lock::version::normalize(pretty).unwrap_or_else(|_| pretty.to_owned()),
            downloaded,
        })
    }
}

/// `json_encode($s)` with PHP's default flags: `/` and non-ASCII escaped.
fn php_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '/' => out.push_str("\\/"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || !c.is_ascii() => {
                let mut buf = [0_u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The POSTs to send: `(url, body)` per notification URL, in first-seen order.
pub(crate) fn batches(downloads: &[Download]) -> Vec<(String, Vec<u8>)> {
    let mut groups: Vec<(&str, Vec<&Download>)> = Vec::new();
    for d in downloads {
        match groups.iter_mut().find(|(u, _)| *u == d.url) {
            Some((_, list)) => {
                if !list.iter().any(|x| x.name.eq_ignore_ascii_case(&d.name)) {
                    list.push(d);
                }
            }
            None => groups.push((&d.url, vec![d])),
        }
    }
    groups
        .into_iter()
        .filter(|(url, _)| !url.contains("%package%"))
        .map(|(url, list)| {
            let packagist = url.contains("packagist.org/");
            let mut body = String::from("{\"downloads\":[");
            for (i, d) in list.iter().enumerate() {
                if i > 0 {
                    body.push(',');
                }
                body.push_str("{\"name\":");
                php_string(&d.name, &mut body);
                body.push_str(",\"version\":");
                php_string(&d.version, &mut body);
                if packagist {
                    match d.downloaded {
                        Some(size) => {
                            let _ = write!(body, ",\"downloaded\":{size}");
                        }
                        None => body.push_str(",\"downloaded\":false"),
                    }
                }
                body.push('}');
            }
            body.push_str("]}");
            (url.to_owned(), body.into_bytes())
        })
        .collect()
}

/// `(url, body)` POSTs.
type Posts = Vec<(String, Vec<u8>)>;

/// What a detached sender reads on stdin.
#[derive(Debug)]
struct Payload {
    root: String,
    config: Option<Map<String, Value>>,
    posts: Posts,
}

/// The hidden subcommand a detached sender runs as.
pub(crate) const SUBCOMMAND: &str = "__notify";

/// What a detached sender needs: the project (for auth) and the POSTs.
fn payload(
    root: &Path,
    config: Option<&Map<String, Value>>,
    batches: &[(String, Vec<u8>)],
) -> Vec<u8> {
    let posts: Vec<Value> = batches
        .iter()
        .map(|(url, body)| json!({"url": url, "body": String::from_utf8_lossy(body)}))
        .collect();
    json!({"root": root.to_string_lossy(), "config": config, "posts": posts})
        .to_string()
        .into_bytes()
}

fn parse_payload(bytes: &[u8]) -> Option<Payload> {
    let doc: Value = serde_json::from_slice(bytes).ok()?;
    let root = doc.get("root")?.as_str()?.to_owned();
    let config = doc.get("config").and_then(Value::as_object).cloned();
    let posts = doc
        .get("posts")?
        .as_array()?
        .iter()
        .filter_map(|p| {
            Some((
                p.get("url")?.as_str()?.to_owned(),
                p.get("body")?.as_str()?.as_bytes().to_vec(),
            ))
        })
        .collect();
    Some(Payload {
        root,
        config,
        posts,
    })
}

/// Hand the POSTs to a copy of phpm that sends them after this process has
/// exited, so an install never waits on Packagist's notification endpoint.
/// `false` when no such process could be started.
pub(crate) fn detach(
    root: &Path,
    config: Option<&Map<String, Value>>,
    batches: &[(String, Vec<u8>)],
) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let Ok(mut child) = Command::new(exe)
        .arg(SUBCOMMAND)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let written = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(&payload(root, config, batches)).is_ok());
    if !written {
        let _ = child.kill();
        let _ = child.wait();
    }
    written
}

/// The detached sender: read the payload on stdin and send it.
pub(crate) fn run_detached(mut input: impl Read) {
    let mut bytes = Vec::new();
    if input.read_to_end(&mut bytes).is_err() {
        return;
    }
    if let Some(p) = parse_payload(&bytes) {
        send_blocking(Path::new(&p.root), p.config.as_ref(), p.posts);
    }
}

/// Send the POSTs on a runtime of their own; failures are ignored.
pub(crate) fn send_blocking(root: &Path, config: Option<&Map<String, Value>>, posts: Posts) {
    let Ok(auth) = Auth::load(Some(root), config) else {
        return;
    };
    let fetcher = Fetcher::new(FetchOptions {
        auth,
        retries: 0,
        ..FetchOptions::default()
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    if let (Ok(fetcher), Ok(runtime)) = (fetcher, runtime) {
        runtime.block_on(send(&fetcher, posts));
    }
}

/// Send every batch; failures are ignored, as Composer ignores them.
pub(crate) async fn send(fetcher: &Fetcher, batches: Vec<(String, Vec<u8>)>) {
    for (url, body) in batches {
        let post = fetcher.request(&url, None, Some(("application/json", body)));
        let _ = tokio::time::timeout(std::time::Duration::from_secs(6), post).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{Download, batches};
    use serde_json::json;

    fn d(url: &str, name: &str, downloaded: Option<u64>) -> Download {
        Download {
            url: url.into(),
            name: name.into(),
            version: "1.2.0.0".into(),
            downloaded,
        }
    }

    #[test]
    fn batches_per_url_like_composer() {
        let packagist = "https://packagist.org/downloads/";
        let out = batches(&[
            d(packagist, "Monolog/monolog", Some(1234)),
            d("https://private.test/notify", "a/b", Some(5)),
            d(packagist, "psr/log", None),
            d(packagist, "monolog/monolog", None),
            d("https://old.test/%package%/downloads", "x/y", None),
        ]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, packagist);
        assert_eq!(
            String::from_utf8(out[0].1.clone()).unwrap(),
            r#"{"downloads":[{"name":"Monolog\/monolog","version":"1.2.0.0","downloaded":1234},{"name":"psr\/log","version":"1.2.0.0","downloaded":false}]}"#
        );
        assert_eq!(
            String::from_utf8(out[1].1.clone()).unwrap(),
            r#"{"downloads":[{"name":"a\/b","version":"1.2.0.0"}]}"#
        );
    }

    #[test]
    fn round_trips_the_detached_payload() {
        let config = json!({"notify-on-install": true});
        let posts = vec![(
            "https://p.test/downloads/".to_owned(),
            br#"{"downloads":[]}"#.to_vec(),
        )];
        let bytes = super::payload(std::path::Path::new("/app"), config.as_object(), &posts);
        let got = super::parse_payload(&bytes).unwrap();
        assert_eq!(got.root, "/app");
        assert_eq!(got.config.as_ref(), config.as_object());
        assert_eq!(got.posts, posts);
        assert!(super::parse_payload(b"{}").is_none());
        super::run_detached(&b"not json"[..]);
    }

    #[test]
    fn escapes_strings_like_php() {
        let mut s = String::new();
        super::php_string("a\"\\/\n\r\t\u{8}\u{c}\u{1}é😀", &mut s);
        assert_eq!(s, r#""a\"\\\/\n\r\t\b\f\u0001\u00e9\ud83d\ude00""#);
    }

    #[test]
    fn reads_lock_entries() {
        let entry = json!({"name": "a/b", "version": "v1.2", "notification-url": "https://packagist.org/downloads/"});
        let got = Download::from_lock(entry.as_object().unwrap(), Some(3)).unwrap();
        assert_eq!(got.version, "1.2.0.0");
        assert_eq!(got.downloaded, Some(3));
        assert!(Download::from_lock(json!({"name": "a/b"}).as_object().unwrap(), None).is_none());
        let odd = json!({"name": "a/b", "version": "weird version", "notification-url": "u"});
        assert_eq!(
            Download::from_lock(odd.as_object().unwrap(), None)
                .unwrap()
                .version,
            "weird version"
        );
    }
}
