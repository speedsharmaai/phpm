//! Download notifications, so package authors keep their install counts:
//! one POST per `notification-url` with every package installed or updated
//! in this run, as `InstallationManager::notifyInstalls` sends them.
//!
//! Composer: Installer/InstallationManager.php notifyInstalls, markForNotification.

use std::fmt::Write as _;

use phpm_store::Fetcher;
use serde_json::{Map, Value};

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
