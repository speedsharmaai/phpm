//! What covered plugins do on the events phpm fires itself, in place of
//! their Composer event listeners.

use std::path::{Path, PathBuf};

use phpm_lock::ComposerJson;
use serde_json::{Map, Value};

use crate::error::Error;
use crate::fsutil::write_if_changed;

use super::Role;

/// The install as the plugins' listeners see it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Installed<'a> {
    /// `config.vendor-dir`, absolute.
    pub(crate) vendor: &'a str,
    pub(crate) composer: &'a ComposerJson,
    /// The local repository in `installed.json`'s order.
    pub(crate) packages: &'a [&'a Map<String, Value>],
}

impl Installed<'_> {
    fn root_extra(&self) -> Option<&Map<String, Value>> {
        self.composer.data().get("extra").and_then(Value::as_object)
    }
}

/// Listeners of the covered plugins, in the order Composer activates them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Hooks {
    pub(crate) roles: Vec<Role>,
}

fn write(path: &Path, bytes: &[u8]) -> Result<PathBuf, Error> {
    write_if_changed(path, bytes).map_err(|e| Error::io(path, &e))?;
    Ok(path.to_path_buf())
}

impl Hooks {
    /// `post-autoload-dump` listeners; returns the files they wrote.
    pub(crate) fn post_autoload(&self, at: Installed<'_>) -> Result<Vec<PathBuf>, Error> {
        let mut written = Vec::new();
        for role in &self.roles {
            if *role == Role::Pest {
                written.push(pest(at)?);
            }
        }
        Ok(written)
    }
}

/// The `extra.pest.plugins` lists, merged in local repository order with the
/// root package last.
// Composer: pestphp/pest-plugin Commands/DumpCommand::execute
fn pest_plugins<'a>(
    packages: impl Iterator<Item = Option<&'a Map<String, Value>>>,
) -> Result<Vec<Value>, String> {
    let mut plugins = Vec::new();
    for extra in packages {
        match extra
            .and_then(|e| e.get("pest"))
            .and_then(|p| p.get("plugins"))
        {
            None | Some(Value::Null) => {}
            Some(Value::Array(items)) => plugins.extend(items.iter().cloned()),
            Some(_) => return Err("extra.pest.plugins is not a list".to_owned()),
        }
    }
    Ok(plugins)
}

/// Whether phpm writes `pest-plugins.json` exactly as the plugin would.
pub(crate) fn pest_check(
    packages: &[&Map<String, Value>],
    root_extra: Option<&Map<String, Value>>,
) -> Result<(), String> {
    pest_plugins(extras(packages, root_extra)).map(|_| ())
}

fn extras<'a>(
    packages: &'a [&'a Map<String, Value>],
    root_extra: Option<&'a Map<String, Value>>,
) -> impl Iterator<Item = Option<&'a Map<String, Value>>> {
    packages
        .iter()
        .map(|p| p.get("extra").and_then(Value::as_object))
        .chain(std::iter::once(root_extra))
}

fn pest(at: Installed<'_>) -> Result<PathBuf, Error> {
    let plugins = pest_plugins(extras(at.packages, at.root_extra())).map_err(Error::install)?;
    let json = phpm_php::encode_pretty_escaped(&Value::Array(plugins));
    write(
        &Path::new(at.vendor).join("pest-plugins.json"),
        json.as_bytes(),
    )
}

/// What a covered plugin's `uninstall()` removes when its package goes.
pub(crate) fn uninstalled(role: Role, vendor: &str) -> Option<PathBuf> {
    match role {
        Role::Pest => Some(Path::new(vendor).join("pest-plugins.json")),
        Role::Installers | Role::WordPressCore => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Hooks, Installed, pest_check, uninstalled};
    use crate::adapters::Role;
    use phpm_lock::ComposerJson;
    use serde_json::{Map, Value, json};

    fn obj(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => unreachable!(),
        }
    }

    #[test]
    fn dumps_pest_plugins_in_repository_order_root_last() {
        let tmp = tempfile::tempdir().unwrap();
        let vendor = tmp.path().to_string_lossy().replace('\\', "/");
        let packages = [
            obj(
                json!({"name": "pestphp/pest", "extra": {"pest": {"plugins": ["Pest\\Plugins\\Bail", "Pest\\Plugins\\Cache"]}}}),
            ),
            obj(json!({"name": "a/lib"})),
            obj(
                json!({"name": "pestphp/pest-plugin-arch", "extra": {"pest": {"plugins": ["Pest\\Arch\\Plugin"]}}}),
            ),
            obj(json!({"name": "x/odd", "extra": {"pest": ["not", "keyed"]}})),
        ];
        let refs: Vec<&Map<String, Value>> = packages.iter().collect();
        let composer =
            ComposerJson::from_value(json!({"extra": {"pest": {"plugins": ["App\\Mine/é"]}}}))
                .unwrap();
        let hooks = Hooks {
            roles: vec![Role::Installers, Role::Pest],
        };
        let written = hooks
            .post_autoload(Installed {
                vendor: &vendor,
                composer: &composer,
                packages: &refs,
            })
            .unwrap();
        assert_eq!(written, [tmp.path().join("pest-plugins.json")]);
        assert_eq!(
            std::fs::read_to_string(&written[0]).unwrap(),
            "[\n    \"Pest\\\\Plugins\\\\Bail\",\n    \"Pest\\\\Plugins\\\\Cache\",\n    \"Pest\\\\Arch\\\\Plugin\",\n    \"App\\\\Mine\\/\\u00e9\"\n]"
        );
        let none = ComposerJson::from_value(json!({})).unwrap();
        hooks
            .post_autoload(Installed {
                vendor: &vendor,
                composer: &none,
                packages: &[],
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("pest-plugins.json")).unwrap(),
            "[]"
        );
    }

    #[test]
    fn checks_pest_lists_before_installing() {
        let fine = obj(json!({"name": "a/b", "extra": {"pest": {"plugins": null}}}));
        assert_eq!(pest_check(&[&fine], None), Ok(()));
        let keyed = obj(json!({"name": "a/b", "extra": {"pest": {"plugins": {"x": "Y"}}}}));
        assert!(pest_check(&[&keyed], None).is_err());
        let root = obj(json!({"pest": {"plugins": "One"}}));
        assert!(pest_check(&[], Some(&root)).is_err());
    }

    #[test]
    fn knows_what_uninstall_removes() {
        assert_eq!(
            uninstalled(Role::Pest, "/p/vendor"),
            Some(std::path::Path::new("/p/vendor").join("pest-plugins.json"))
        );
        assert_eq!(uninstalled(Role::Installers, "/p/vendor"), None);
    }
}
