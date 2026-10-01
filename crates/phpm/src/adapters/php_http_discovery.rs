//! php-http/discovery's effect on `composer install`.
//!
//! Its `postUpdate` listener is bound to `post-update-cmd`, which `install`
//! never runs. Its `preAutoloadDump` listener only writes
//! `vendor/composer/GeneratedDiscoveryStrategy.php` and adds it to the root
//! package's classmap when `extra.discovery` pins an abstraction; phpm
//! declines whenever that is set, since reproducing the generated candidate
//! class is out of scope. When nothing is pinned (the common case), the
//! listener's only effect is removing that file if an earlier run left it
//! behind.
// Composer: php-http/discovery src/Composer/Plugin.php preAutoloadDump

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

fn pinned(root_extra: Option<&Map<String, Value>>) -> Option<&Map<String, Value>> {
    root_extra
        .and_then(|e| e.get("discovery"))
        .and_then(Value::as_object)
}

/// Whether phpm reproduces this plugin's effect: only when nothing is
/// pinned in `extra.discovery`.
pub(crate) fn check(root_extra: Option<&Map<String, Value>>) -> Result<(), String> {
    match pinned(root_extra) {
        Some(m) if !m.is_empty() => {
            Err("extra.discovery pins an implementation, which phpm does not generate".into())
        }
        _ => Ok(()),
    }
}

/// `vendor/composer/GeneratedDiscoveryStrategy.php`, removed when stale.
pub(crate) fn stale_file(vendor: &str) -> PathBuf {
    Path::new(vendor).join("composer/GeneratedDiscoveryStrategy.php")
}

#[cfg(test)]
mod tests {
    use super::{check, stale_file};
    use serde_json::{Map, Value, json};

    fn obj(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => unreachable!(),
        }
    }

    #[test]
    fn declines_only_when_an_abstraction_is_pinned() {
        assert_eq!(check(None), Ok(()));
        let empty = obj(json!({"discovery": {}}));
        assert_eq!(check(Some(&empty)), Ok(()));
        let pinned = obj(json!({"discovery": {"psr/http-client-implementation": "Foo"}}));
        assert!(check(Some(&pinned)).is_err());
    }

    #[test]
    fn stale_file_is_under_vendor_composer() {
        assert_eq!(
            stale_file("/p/vendor"),
            std::path::Path::new("/p/vendor/composer/GeneratedDiscoveryStrategy.php")
        );
    }
}
