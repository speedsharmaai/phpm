//! Which Composer plugins an install would load, decided the way Composer's
//! `PluginManager` decides it, without running PHP.

use std::fs;
use std::path::{Path, PathBuf};

use phpm_lock::ComposerJson;
use serde_json::{Map, Value};

use crate::error::Error;
use crate::project::Env;

/// `config.allow-plugins` after Composer's config merge.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AllowConfig {
    All(bool),
    Map(Vec<(String, Value)>),
}

impl AllowConfig {
    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Bool(b) => Some(Self::All(*b)),
            Value::Object(map) => Some(Self::Map(
                map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            )),
            Value::Array(items) if items.is_empty() => Some(Self::Map(Vec::new())),
            _ => None,
        }
    }

    // Composer: Config.php merge, allow-plugins branch
    fn merge(self, newer: Self) -> Self {
        match (self, newer) {
            (Self::Map(old), Self::Map(new)) => {
                let mut merged = new.clone();
                for (k, v) in old {
                    if !new.iter().any(|(n, _)| *n == k) {
                        merged.push((k, v));
                    }
                }
                Self::Map(merged)
            }
            (_, newer) => newer,
        }
    }
}

fn allow_config(json: Option<&Map<String, Value>>) -> Option<AllowConfig> {
    json.and_then(|d| d.get("config"))
        .and_then(|c| c.get("allow-plugins"))
        .and_then(AllowConfig::from_value)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pattern {
    Any,
    Glob(String),
}

impl Pattern {
    fn matches(&self, name: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Glob(glob) => glob_matches(&glob.to_lowercase(), &name.to_lowercase()),
        }
    }
}

// Composer: BasePackage::packageNameToRegexp, where only `*` is special
fn glob_matches(glob: &str, name: &str) -> bool {
    let mut parts = glob.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let tail: Vec<&str> = parts.collect();
    let Some((last, middle)) = tail.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(i) => rest = &rest[i + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

/// Parsed allow rules; `None` is Composer's pre-2.2 lock mode, where a
/// non-interactive run refuses every plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rules(Option<Vec<(Pattern, bool)>>);

impl Rules {
    // Composer: PluginManager::parseAllowedPlugins
    fn parse(config: AllowConfig, lock_plugin_api: Option<&str>) -> Self {
        match config {
            AllowConfig::Map(map)
                if map.is_empty() && lock_plugin_api.is_some_and(older_than_2_2) =>
            {
                Self(None)
            }
            AllowConfig::All(allow) => Self(Some(vec![(Pattern::Any, allow)])),
            AllowConfig::Map(map) => Self(Some(
                map.into_iter()
                    .map(|(k, v)| (Pattern::Glob(k), v == Value::Bool(true)))
                    .collect(),
            )),
        }
    }

    /// `Ok(true)` loads the plugin, `Ok(false)` skips it quietly.
    // Composer: PluginManager::isPluginAllowed, non-interactive
    fn allows(&self, name: &str, global: bool, optional: bool) -> Result<bool, Error> {
        let Some(rules) = &self.0 else {
            return Err(Error::install(
                "Your composer.lock was generated before the allow-plugins security feature was introduced \
                 and your composer.json does not define allow-plugins. Run \"composer update --lock\" locally \
                 and commit the updated composer.lock, then add an explicit allow-plugins section to \
                 composer.json. See https://getcomposer.org/allow-plugins",
            ));
        };
        if let Some((_, allow)) = rules.iter().find(|(p, _)| p.matches(name)) {
            return Ok(*allow);
        }
        if name == "composer/package-versions-deprecated" || optional {
            return Ok(false);
        }
        let (label, cmd) = if global {
            (" (installed globally)", "global ")
        } else {
            ("", "")
        };
        Err(Error::install(format!(
            "{name}{label} contains a Composer plugin which is blocked by your allow-plugins config. \
             You may add it to the list if you consider it safe.\n\
             You can run \"composer {cmd}config --no-plugins allow-plugins.{name} [true|false]\" to enable it (true) \
             or disable it explicitly and suppress this exception (false)\n\
             See https://getcomposer.org/allow-plugins"
        )))
    }
}

fn older_than_2_2(version: &str) -> bool {
    let parts: Vec<u64> = version.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    parts.as_slice() < [2, 2, 0].as_slice()
}

/// A plugin Composer would load for this install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Plugin {
    pub(crate) name: String,
    pub(crate) global: bool,
    /// Why only a full `composer install` gets this plugin's effect right.
    pub(crate) needs_full_install: Option<String>,
}

/// A plugin package Composer would not load, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Skipped {
    pub(crate) name: String,
    pub(crate) why: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Plugins {
    pub(crate) active: Vec<Plugin>,
    pub(crate) skipped: Vec<Skipped>,
}

const PLACES_PACKAGES: &str = "it changes where packages are installed";

// Plugins that act on package events or rewrite files after placement,
// which `composer dump-autoload` and `run-script` never trigger.
const FULL_INSTALL: [(&str, &str); 6] = [
    ("composer/installers", PLACES_PACKAGES),
    ("oomphinc/composer-installers-extender", PLACES_PACKAGES),
    ("roots/wordpress-core-installer", PLACES_PACKAGES),
    ("johnpbloch/wordpress-core-installer", PLACES_PACKAGES),
    (
        "cweagans/composer-patches",
        "it patches packages as they are installed",
    ),
    (
        "drupal/core-composer-scaffold",
        "it scaffolds files outside vendor/ from package events",
    ),
];

fn is_plugin_type(kind: &str) -> bool {
    kind.eq_ignore_ascii_case("composer-plugin") || kind.eq_ignore_ascii_case("composer-installer")
}

fn needs_full_install(package: &Map<String, Value>, name: &str) -> Option<&'static str> {
    let extra = package.get("extra");
    let flag = |key: &str| extra.and_then(|e| e.get(key)) == Some(&Value::Bool(true));
    let kind = package.get("type").and_then(Value::as_str).unwrap_or("");
    if kind.eq_ignore_ascii_case("composer-installer") {
        return Some("it is a composer-installer, which places packages itself");
    }
    if let Some((_, why)) = FULL_INSTALL
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
    {
        return Some(why);
    }
    if flag("plugin-modifies-install-path") {
        return Some(PLACES_PACKAGES);
    }
    if flag("plugin-modifies-downloads") {
        return Some("it changes how packages are downloaded");
    }
    None
}

// Plugins that install some package types themselves, which only matters
// when the install has packages of those types.
const TYPE_INSTALLERS: [(&str, &str); 1] = [("symfony/flex", "symfony-pack")];

fn installs_types(plugin: &str, installed: &[&Map<String, Value>]) -> Option<String> {
    let (_, kind) = TYPE_INSTALLERS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(plugin))?;
    let names: Vec<&str> = installed
        .iter()
        .filter(|p| {
            p.get("type")
                .and_then(Value::as_str)
                .is_some_and(|t| t.eq_ignore_ascii_case(kind))
        })
        .filter_map(|p| p.get("name").and_then(Value::as_str))
        .collect();
    (!names.is_empty())
        .then(|| format!("it installs {kind} packages itself ({})", names.join(", ")))
}

fn collect(
    packages: &[&Map<String, Value>],
    installed: &[&Map<String, Value>],
    rules: &Rules,
    global: bool,
    into: &mut Plugins,
) -> Result<(), Error> {
    for package in packages {
        let kind = package.get("type").and_then(Value::as_str).unwrap_or("");
        let Some(name) = package.get("name").and_then(Value::as_str) else {
            continue;
        };
        if !is_plugin_type(kind) {
            continue;
        }
        let optional =
            package.get("extra").and_then(|e| e.get("plugin-optional")) == Some(&Value::Bool(true));
        if rules.allows(name, global, optional)? {
            into.active.push(Plugin {
                name: name.to_owned(),
                global,
                needs_full_install: needs_full_install(package, name)
                    .map(str::to_owned)
                    .or_else(|| installs_types(name, installed)),
            });
        } else {
            into.skipped.push(Skipped {
                name: name.to_owned(),
                why: "not in config.allow-plugins",
            });
        }
    }
    Ok(())
}

/// What Composer's global directory holds, read once per install.
#[derive(Debug, Clone, Default)]
pub(crate) struct Global {
    config: Option<Map<String, Value>>,
    composer: Option<Map<String, Value>>,
    installed: Vec<Map<String, Value>>,
}

fn read_object(path: &Path) -> Option<Map<String, Value>> {
    let bytes = fs::read(path).ok()?;
    match serde_json::from_slice(&bytes).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

impl Global {
    pub(crate) fn load(home: Option<&Path>) -> Self {
        let Some(home) = home else {
            return Self::default();
        };
        let composer = read_object(&home.join("composer.json"));
        let vendor = composer
            .as_ref()
            .and_then(|c| c.get("config"))
            .and_then(|c| c.get("vendor-dir"))
            .and_then(Value::as_str)
            .map_or_else(|| home.join("vendor"), |v| home.join(v));
        let installed = match read_object(&vendor.join("composer/installed.json")) {
            Some(mut doc) => match doc.remove("packages") {
                Some(Value::Array(list)) => list
                    .into_iter()
                    .filter_map(|v| match v {
                        Value::Object(m) => Some(m),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            },
            None => Vec::new(),
        };
        Self {
            config: read_object(&home.join("config.json")),
            composer,
            installed,
        }
    }
}

/// The plugins an install of `packages` loads: the project's allowed ones,
/// then global ones, in the order Composer reads them.
pub(crate) fn detect(
    composer: &ComposerJson,
    lock_plugin_api: &str,
    packages: &[&Map<String, Value>],
    global: &Global,
) -> Result<Plugins, Error> {
    let base = AllowConfig::Map(Vec::new());
    let with_home = match allow_config(global.config.as_ref()) {
        Some(c) => base.merge(c),
        None => base,
    };
    let local = match allow_config(Some(composer.data())) {
        Some(c) => with_home.clone().merge(c),
        None => with_home.clone(),
    };
    let mut plugins = Plugins::default();
    collect(
        packages,
        packages,
        &Rules::parse(local, Some(lock_plugin_api)),
        false,
        &mut plugins,
    )?;
    let global_rules = match allow_config(global.composer.as_ref()) {
        Some(c) => with_home.merge(c),
        None => with_home,
    };
    let installed: Vec<&Map<String, Value>> = global.installed.iter().collect();
    collect(
        &installed,
        packages,
        &Rules::parse(global_rules, None),
        true,
        &mut plugins,
    )?;
    Ok(plugins)
}

/// Composer's home directory. `xdg` says whether the system uses XDG dirs.
// Composer: Factory::getHomeDir
pub(crate) fn composer_home(
    env: Env<'_>,
    xdg: bool,
    is_dir: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let var = |k: &str| env(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(home) = var("COMPOSER_HOME") {
        return Some(home);
    }
    if cfg!(windows) {
        return var("APPDATA").map(|d| d.join("Composer"));
    }
    let user = var("HOME")?;
    let mut dirs = Vec::new();
    if xdg {
        let config = var("XDG_CONFIG_HOME").unwrap_or_else(|| user.join(".config"));
        dirs.push(config.join("composer"));
    }
    dirs.push(user.join(".composer"));
    dirs.iter()
        .find(|d| is_dir(d))
        .or_else(|| dirs.first())
        .cloned()
}

pub(crate) fn system_uses_xdg() -> bool {
    std::env::vars_os().any(|(k, _)| k.to_string_lossy().starts_with("XDG_"))
        || Path::new("/etc/xdg").is_dir()
}

#[cfg(test)]
mod tests {
    use super::{
        AllowConfig, Global, Pattern, Rules, composer_home, detect, glob_matches, older_than_2_2,
    };
    use phpm_lock::ComposerJson;
    use serde_json::{Map, Value, json};
    use std::path::{Path, PathBuf};

    fn obj(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => unreachable!(),
        }
    }

    fn plugin(name: &str, extra: &Value) -> Map<String, Value> {
        obj(json!({"name": name, "type": "composer-plugin", "extra": extra}))
    }

    fn composer(allow: &Value) -> ComposerJson {
        ComposerJson::from_value(json!({"config": {"allow-plugins": allow}})).unwrap()
    }

    #[test]
    fn globs_match_like_composer() {
        assert!(glob_matches("symfony/*", "symfony/flex"));
        assert!(glob_matches("*", "a/b"));
        assert!(glob_matches("a/*-plugin*", "a/pest-plugin-x"));
        assert!(glob_matches("a/b", "a/b"));
        assert!(!glob_matches("a/b", "a/bc"));
        assert!(!glob_matches("a/*x", "a/xy"));
        assert!(!glob_matches("b/*", "a/b"));
        assert!(!glob_matches("a/*b*c", "a/cb"));
        assert!(Pattern::Glob("Symfony/*".into()).matches("symfony/FLEX"));
        assert!(Pattern::Any.matches("x"));
    }

    #[test]
    fn compares_plugin_api_versions() {
        assert!(older_than_2_2("1.1.0"));
        assert!(older_than_2_2("2.1.99"));
        assert!(!older_than_2_2("2.2.0"));
        assert!(!older_than_2_2("2.9.0"));
    }

    #[test]
    fn merges_local_over_global_like_composer() {
        let global = AllowConfig::Map(vec![
            ("a/*".into(), json!(true)),
            ("b/b".into(), json!(true)),
        ]);
        let local = AllowConfig::Map(vec![
            ("b/b".into(), json!(false)),
            ("c/c".into(), json!(true)),
        ]);
        assert_eq!(
            global.clone().merge(local),
            AllowConfig::Map(vec![
                ("b/b".into(), json!(false)),
                ("c/c".into(), json!(true)),
                ("a/*".into(), json!(true)),
            ])
        );
        assert_eq!(
            global.clone().merge(AllowConfig::All(false)),
            AllowConfig::All(false)
        );
        assert_eq!(AllowConfig::All(true).merge(global.clone()), global);
        assert_eq!(
            AllowConfig::from_value(&json!([])),
            Some(AllowConfig::Map(Vec::new()))
        );
        assert_eq!(AllowConfig::from_value(&json!(["x"])), None);
    }

    #[test]
    fn applies_allow_rules() {
        let rules = Rules::parse(
            AllowConfig::Map(vec![
                ("a/off".into(), json!(false)),
                ("a/*".into(), json!(true)),
                ("b/odd".into(), json!("yes")),
            ]),
            Some("2.9.0"),
        );
        assert_eq!(rules.allows("a/off", false, false), Ok(false));
        assert_eq!(rules.allows("A/On", false, false), Ok(true));
        assert_eq!(rules.allows("b/odd", false, false), Ok(false));
        assert_eq!(rules.allows("c/opt", false, true), Ok(false));
        assert_eq!(
            rules.allows("composer/package-versions-deprecated", false, false),
            Ok(false)
        );
        let err = rules.allows("c/blocked", false, false).unwrap_err();
        assert!(
            err.message
                .starts_with("c/blocked contains a Composer plugin which is blocked"),
            "{err}"
        );
        assert!(
            err.message
                .contains("\"composer config --no-plugins allow-plugins.c/blocked")
        );
        let err = rules.allows("c/blocked", true, false).unwrap_err();
        assert!(
            err.message
                .contains("c/blocked (installed globally) contains"),
            "{err}"
        );
        assert!(err.message.contains("composer global config"));

        let all = Rules::parse(AllowConfig::All(true), Some("1.0.0"));
        assert_eq!(all.allows("x/y", false, false), Ok(true));
        let none = Rules::parse(AllowConfig::All(false), None);
        assert_eq!(none.allows("x/y", false, false), Ok(false));
        let legacy = Rules::parse(AllowConfig::Map(Vec::new()), Some("2.1.0"));
        assert!(
            legacy
                .allows("x/y", false, true)
                .unwrap_err()
                .message
                .contains("before the allow-plugins")
        );
        let modern = Rules::parse(AllowConfig::Map(Vec::new()), Some("2.6.0"));
        assert_eq!(modern.allows("x/y", false, true), Ok(false));
    }

    #[test]
    fn detects_local_plugins_and_what_they_need() {
        let packages = [
            plugin("symfony/flex", &json!({"class": "F"})),
            plugin("composer/installers", &json!({})),
            plugin("x/paths", &json!({"plugin-modifies-install-path": true})),
            plugin("x/downloads", &json!({"plugin-modifies-downloads": true})),
            obj(json!({"name": "x/old", "type": "composer-installer"})),
            plugin("php-http/discovery", &json!({"plugin-optional": true})),
            obj(json!({"name": "a/lib", "type": "library"})),
            obj(json!({"type": "composer-plugin"})),
        ];
        let packs = [
            plugin("symfony/flex", &json!({})),
            obj(json!({"name": "symfony/apache-pack", "type": "symfony-pack"})),
        ];
        let refs: Vec<&Map<String, Value>> = packs.iter().collect();
        let c = composer(&json!({"symfony/flex": true}));
        let found = detect(&c, "2.9.0", &refs, &Global::default()).unwrap();
        assert_eq!(
            found.active[0].needs_full_install.as_deref(),
            Some("it installs symfony-pack packages itself (symfony/apache-pack)")
        );
        let refs: Vec<&Map<String, Value>> = packages.iter().collect();
        let c = composer(&json!({"symfony/flex": true, "composer/installers": true, "x/*": true}));
        let found = detect(&c, "2.9.0", &refs, &Global::default()).unwrap();
        let names: Vec<(&str, Option<&str>)> = found
            .active
            .iter()
            .map(|p| (p.name.as_str(), p.needs_full_install.as_deref()))
            .collect();
        assert_eq!(
            names,
            [
                ("symfony/flex", None),
                (
                    "composer/installers",
                    Some("it changes where packages are installed")
                ),
                ("x/paths", Some("it changes where packages are installed")),
                (
                    "x/downloads",
                    Some("it changes how packages are downloaded")
                ),
                (
                    "x/old",
                    Some("it is a composer-installer, which places packages itself")
                ),
            ]
        );
        assert_eq!(found.skipped.len(), 1);
        assert_eq!(found.skipped[0].name, "php-http/discovery");
        assert!(!found.active[0].global);

        let blocked = composer(&json!({}));
        assert!(detect(&blocked, "2.9.0", &refs, &Global::default()).is_err());
        let off = composer(&json!(false));
        let found = detect(&off, "2.9.0", &refs, &Global::default()).unwrap();
        assert!(found.active.is_empty());
        assert_eq!(found.skipped.len(), 6);
    }

    #[test]
    fn reads_global_plugins_and_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        assert!(Global::load(Some(home)).installed.is_empty());
        std::fs::create_dir_all(home.join("vendor/composer")).unwrap();
        std::fs::write(
            home.join("vendor/composer/installed.json"),
            json!({"packages": [
                {"name": "g/plugin", "type": "composer-plugin"},
                {"name": "g/lib", "type": "library"},
                "odd",
            ]})
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            home.join("config.json"),
            r#"{"config":{"allow-plugins":{"p/*":true}}}"#,
        )
        .unwrap();
        let global = Global::load(Some(home));
        assert_eq!(global.installed.len(), 2);
        let local = [plugin("p/local", &json!({}))];
        let refs: Vec<&Map<String, Value>> = local.iter().collect();
        let err = detect(&composer(&json!({})), "2.9.0", &refs, &global).unwrap_err();
        assert!(
            err.message.contains("g/plugin (installed globally)"),
            "{err}"
        );

        std::fs::write(
            home.join("composer.json"),
            r#"{"config":{"allow-plugins":{"g/plugin":true},"vendor-dir":"vendor"}}"#,
        )
        .unwrap();
        let global = Global::load(Some(home));
        let found = detect(&composer(&json!({})), "2.9.0", &refs, &global).unwrap();
        let names: Vec<(&str, bool)> = found
            .active
            .iter()
            .map(|p| (p.name.as_str(), p.global))
            .collect();
        assert_eq!(names, [("p/local", false), ("g/plugin", true)]);

        std::fs::write(home.join("vendor/composer/installed.json"), "[]").unwrap();
        assert!(Global::load(Some(home)).installed.is_empty());
        assert!(Global::load(None).installed.is_empty());
    }

    #[test]
    fn finds_composer_home() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        let never = |_: &Path| false;
        assert_eq!(
            composer_home(
                &env(&[("COMPOSER_HOME", "/ch"), ("HOME", "/h")]),
                false,
                &never
            ),
            Some(PathBuf::from("/ch"))
        );
        if cfg!(windows) {
            return;
        }
        assert_eq!(composer_home(&env(&[]), false, &never), None);
        assert_eq!(
            composer_home(&env(&[("HOME", "/h")]), false, &never),
            Some(PathBuf::from("/h/.composer"))
        );
        assert_eq!(
            composer_home(&env(&[("HOME", "/h")]), true, &never),
            Some(PathBuf::from("/h/.config/composer"))
        );
        let legacy = |d: &Path| d == Path::new("/h/.composer");
        assert_eq!(
            composer_home(
                &env(&[("HOME", "/h"), ("XDG_CONFIG_HOME", "/x")]),
                true,
                &legacy
            ),
            Some(PathBuf::from("/h/.composer"))
        );
        let _ = super::system_uses_xdg();
    }
}
