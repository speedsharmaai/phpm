//! What covered plugins do on the events phpm fires itself, in place of
//! their Composer event listeners.

use std::path::{Path, PathBuf};

use phpm_lock::ComposerJson;
use serde_json::{Map, Value};

use crate::error::Error;
use crate::fsutil::write_if_changed;

use super::{
    Paths, Role, dealerdirect_phpcs, drupal_scaffold, php_http_discovery,
    phpstan_extension_installer, symfony_runtime,
};

/// The install as the plugins' listeners see it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Installed<'a> {
    /// The project directory, absolute.
    pub(crate) root: &'a str,
    /// `config.vendor-dir`, absolute.
    pub(crate) vendor: &'a str,
    pub(crate) composer: &'a ComposerJson,
    pub(crate) root_version: &'a phpm_lock::RootVersion,
    /// The local repository in `installed.json`'s order.
    pub(crate) packages: &'a [&'a Map<String, Value>],
    /// Where composer/installers chose to put packages, if it ran too.
    pub(crate) paths: &'a Paths,
}

impl Installed<'_> {
    fn root_extra(&self) -> Option<&Map<String, Value>> {
        self.composer.data().get("extra").and_then(Value::as_object)
    }

    fn root_type(&self) -> Option<&str> {
        self.composer.data().get("type").and_then(Value::as_str)
    }

    fn root_name(&self) -> String {
        self.composer
            .data()
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("__root__")
            .to_ascii_lowercase()
    }
}

/// Listeners of the covered plugins, in the order Composer activates them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Hooks {
    pub(crate) roles: Vec<Role>,
}

// Composer: Util/Filesystem::ensureDirectoryExists, called by every writer
// (ReplaceOp, the various GeneratedConfig/autoload_runtime writers) before
// it touches its destination.
fn write(path: &Path, bytes: &[u8]) -> Result<PathBuf, Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, &e))?;
    }
    write_if_changed(path, bytes).map_err(|e| Error::io(path, &e))?;
    Ok(path.to_path_buf())
}

/// What `pre-autoload-dump` adds, so the native autoload generator sees it
/// before it runs.
#[derive(Debug, Default)]
pub(crate) struct PreAutoload {
    pub(crate) written: Vec<PathBuf>,
    pub(crate) extra_root_classmap: Vec<String>,
}

impl Hooks {
    /// `pre-autoload-dump` listeners; must run before the autoload itself
    /// is generated.
    pub(crate) fn pre_autoload(&self, at: Installed<'_>) -> Result<PreAutoload, Error> {
        let mut out = PreAutoload::default();
        for role in &self.roles {
            match role {
                Role::DrupalScaffold => {
                    let generated = drupal_scaffold::pre_autoload(
                        at.packages,
                        &at.root_name(),
                        &at.root_version.normalized,
                        at.root_version.reference.as_deref(),
                        at.vendor,
                    )
                    .map_err(Error::install)?;
                    out.written.push(write(
                        &generated.drupal_installed.path,
                        &generated.drupal_installed.bytes,
                    )?);
                    out.extra_root_classmap
                        .extend(generated.extra_root_classmap);
                }
                Role::PhpHttpDiscovery => {
                    let file = php_http_discovery::stale_file(at.vendor);
                    if file.exists() {
                        std::fs::remove_file(&file).map_err(|e| Error::io(&file, &e))?;
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// `post-autoload-dump` listeners; returns the files they wrote.
    pub(crate) fn post_autoload(&self, at: Installed<'_>) -> Result<Vec<PathBuf>, Error> {
        let mut written = Vec::new();
        for role in &self.roles {
            match role {
                Role::Pest => written.push(pest(at)?),
                Role::SymfonyRuntime => written.extend(runtime(at)?),
                Role::Installers
                | Role::WordPressCore
                | Role::PhpstanExtensionInstaller
                | Role::DealerdirectPhpcs
                | Role::DrupalScaffold
                | Role::PhpHttpDiscovery
                | Role::NoOp => {}
            }
        }
        Ok(written)
    }

    /// `post-install-cmd` listeners; returns the files they wrote.
    pub(crate) fn post_install(&self, at: Installed<'_>) -> Result<Vec<PathBuf>, Error> {
        let mut written = Vec::new();
        for role in &self.roles {
            match role {
                Role::PhpstanExtensionInstaller => written.push(phpstan(at)?),
                Role::DealerdirectPhpcs => written.extend(dealerdirect(at)?),
                Role::DrupalScaffold => written.extend(drupal(at)?),
                Role::Installers
                | Role::WordPressCore
                | Role::Pest
                | Role::SymfonyRuntime
                | Role::PhpHttpDiscovery
                | Role::NoOp => {}
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

/// Whether phpm reproduces `vendor/autoload_runtime.php` exactly.
pub(crate) fn symfony_runtime_check(
    root_extra: Option<&Map<String, Value>>,
    root: &str,
    vendor: &str,
) -> Result<(), String> {
    symfony_runtime::generate(root_extra, symfony_runtime::Context { root, vendor }).map(|_| ())
}

fn runtime(at: Installed<'_>) -> Result<Option<PathBuf>, Error> {
    let generated = symfony_runtime::generate(
        at.root_extra(),
        symfony_runtime::Context {
            root: at.root,
            vendor: at.vendor,
        },
    )
    .map_err(Error::install)?;
    let Some(bytes) = generated else {
        return Ok(None);
    };
    write(&Path::new(at.vendor).join("autoload_runtime.php"), &bytes).map(Some)
}

/// Whether phpm reproduces `GeneratedConfig.php` exactly.
pub(crate) fn phpstan_check(
    packages: &[&Map<String, Value>],
    root_extra: Option<&Map<String, Value>>,
    vendor: &str,
) -> Result<(), String> {
    phpstan_extension_installer::generate(packages, root_extra, &Paths::default(), vendor)
        .map(|_| ())
}

fn phpstan(at: Installed<'_>) -> Result<PathBuf, Error> {
    let bytes =
        phpstan_extension_installer::generate(at.packages, at.root_extra(), at.paths, at.vendor)
            .map_err(Error::install)?;
    let own_path = at
        .paths
        .normalized
        .get("phpstan/extension-installer")
        .cloned()
        .unwrap_or_else(|| format!("{}/phpstan/extension-installer", at.vendor));
    write(
        &Path::new(&own_path).join("src/GeneratedConfig.php"),
        &bytes,
    )
}

/// Whether phpm reproduces `CodeSniffer.conf`'s `installed_paths` exactly.
/// The filesystem walk this needs finds nothing before packages are placed,
/// so this only validates the parts that do not depend on it.
pub(crate) fn dealerdirect_check(
    packages: &[&Map<String, Value>],
    root_type: Option<&str>,
    root_extra: Option<&Map<String, Value>>,
    root: &str,
    vendor: &str,
) -> Result<(), String> {
    dealerdirect_phpcs::generate(
        packages,
        root_type,
        root_extra,
        &Paths::default(),
        root,
        vendor,
    )
    .map(|_| ())
}

fn dealerdirect(at: Installed<'_>) -> Result<Option<PathBuf>, Error> {
    let generated = dealerdirect_phpcs::generate(
        at.packages,
        at.root_type(),
        at.root_extra(),
        at.paths,
        at.root,
        at.vendor,
    )
    .map_err(Error::install)?;
    let Some(bytes) = generated else {
        return Ok(None);
    };
    let phpcs_path = at
        .paths
        .normalized
        .get("squizlabs/php_codesniffer")
        .cloned()
        .unwrap_or_else(|| format!("{}/squizlabs/php_codesniffer", at.vendor));
    write(&Path::new(&phpcs_path).join("CodeSniffer.conf"), &bytes).map(Some)
}

/// Whether phpm reproduces the scaffold files and reference files exactly.
/// The real file reads and the web-root directory this needs are not there
/// yet before packages are placed, so this only validates the shape of the
/// `drupal-scaffold` config (file-mapping, locations tokens, the
/// `.gitignore` gate), deferring the rest to the real hook.
pub(crate) fn drupal_scaffold_check(
    packages: &[&Map<String, Value>],
    root_extra: Option<&Map<String, Value>>,
    root: &str,
) -> Result<(), String> {
    drupal_scaffold::check(packages, root_extra, &Paths::default(), root)
}

/// Whether phpm reproduces php-http/discovery's `pre-autoload-dump` effect:
/// only when nothing is pinned in `extra.discovery`.
pub(crate) fn php_http_discovery_check(
    root_extra: Option<&Map<String, Value>>,
) -> Result<(), String> {
    php_http_discovery::check(root_extra)
}

fn drupal(at: Installed<'_>) -> Result<Vec<PathBuf>, Error> {
    let files =
        drupal_scaffold::post_install(at.packages, at.root_extra(), at.paths, at.root, at.vendor)
            .map_err(Error::install)?;
    files
        .into_iter()
        .map(|f| write(&f.path, &f.bytes))
        .collect()
}

/// What a covered plugin's `uninstall()` removes when its package goes.
// Composer: symfony/runtime Internal/ComposerPlugin::uninstall (always
// unlinks); pestphp/pest-plugin Manager::uninstall (always unlinks).
// phpstan/extension-installer's own uninstall() is a no-op: GeneratedConfig.php
// stays behind, stale, exactly as Composer leaves it.
pub(crate) fn uninstalled(role: Role, vendor: &str) -> Option<PathBuf> {
    match role {
        Role::Pest => Some(Path::new(vendor).join("pest-plugins.json")),
        Role::SymfonyRuntime => Some(Path::new(vendor).join("autoload_runtime.php")),
        Role::Installers
        | Role::WordPressCore
        | Role::PhpstanExtensionInstaller
        | Role::DealerdirectPhpcs
        | Role::DrupalScaffold
        | Role::PhpHttpDiscovery
        | Role::NoOp => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Hooks, Installed, pest_check, symfony_runtime_check, uninstalled};
    use crate::adapters::{Paths, Role};
    use phpm_lock::{ComposerJson, RootVersion};
    use serde_json::{Map, Value, json};

    fn obj(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => unreachable!(),
        }
    }

    fn no_version() -> RootVersion {
        RootVersion {
            pretty: "1.0.0".into(),
            normalized: "1.0.0.0".into(),
            reference: None,
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
                root: "/p",
                vendor: &vendor,
                composer: &composer,
                root_version: &no_version(),
                packages: &refs,
                paths: &Paths::default(),
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
                root: "/p",
                vendor: &vendor,
                composer: &none,
                root_version: &no_version(),
                packages: &[],
                paths: &Paths::default(),
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
        assert_eq!(
            uninstalled(Role::SymfonyRuntime, "/p/vendor"),
            Some(std::path::Path::new("/p/vendor").join("autoload_runtime.php"))
        );
        assert_eq!(uninstalled(Role::Installers, "/p/vendor"), None);
    }

    #[test]
    fn writes_autoload_runtime_through_the_post_autoload_hook() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_string_lossy().replace('\\', "/");
        let vendor = format!("{root}/vendor");
        std::fs::create_dir_all(&vendor).unwrap();
        let composer = ComposerJson::from_value(json!({})).unwrap();
        let hooks = Hooks {
            roles: vec![Role::SymfonyRuntime],
        };
        let written = hooks
            .post_autoload(Installed {
                root: &root,
                vendor: &vendor,
                composer: &composer,
                root_version: &no_version(),
                packages: &[],
                paths: &Paths::default(),
            })
            .unwrap();
        assert_eq!(
            written,
            [std::path::Path::new(&vendor).join("autoload_runtime.php")]
        );
        let content = std::fs::read_to_string(&written[0]).unwrap();
        assert!(content.contains("dirname(__DIR__, 1)"), "{content}");

        let off = ComposerJson::from_value(json!({"extra": {"runtime": false}})).unwrap();
        let written = hooks
            .post_autoload(Installed {
                root: &root,
                vendor: &vendor,
                composer: &off,
                root_version: &no_version(),
                packages: &[],
                paths: &Paths::default(),
            })
            .unwrap();
        assert!(written.is_empty());
    }

    #[test]
    fn symfony_runtime_check_matches_generate() {
        assert_eq!(symfony_runtime_check(None, "/p", "/p/vendor"), Ok(()));
        let bad = obj(json!({"runtime": "x"}));
        assert!(symfony_runtime_check(Some(&bad), "/p", "/p/vendor").is_err());
    }
}
