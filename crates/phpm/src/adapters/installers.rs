//! Install paths as composer/installers 2.3.0 and
//! roots/wordpress-core-installer choose them.

use phpm_php::is_absolute_path;
use serde_json::{Map, Value};

use super::php::{
    At, camel_dashes, empty, keep_word_chars, lcfirst, lower, replace_each, strip, ucfirst,
    ucwords, underscore_capitals,
};

/// How an installer class rewrites `{$name}` and `{$vendor}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Inflect {
    Plain,
    Camel,
    Agl,
    Asgard,
    Cockpit,
    Croogo,
    DokuWiki,
    ForkCms,
    Grav,
    Majima,
    Mautic,
    Maya,
    MediaWiki,
    Microweber,
    October,
    OntoWiki,
    Plentymarkets,
    Pxcms,
    Roundcube,
    Shopware,
    SiteDirect,
    Sydes,
    Tao,
    TastyIgniter,
    Winter,
    SilverStripe,
    Oxid,
    Bitrix,
    CakePhp,
}

#[derive(Debug)]
struct Framework {
    prefix: &'static str,
    locations: &'static [(&'static str, &'static str)],
    inflect: Inflect,
}

/// The template variables of one package.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Vars {
    name: String,
    vendor: String,
    kind: String,
    install_item_dir: Option<String>,
}

impl Vars {
    fn get(&self, var: &str) -> Option<&str> {
        match var {
            "name" => Some(&self.name),
            "vendor" => Some(&self.vendor),
            "type" => Some(&self.kind),
            "install_item_dir" => self.install_item_dir.as_deref(),
            _ => None,
        }
    }
}

/// One locked package as the installers see it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Locked<'a> {
    pub(crate) entry: &'a Map<String, Value>,
}

impl<'a> Locked<'a> {
    fn pretty_name(self) -> &'a str {
        self.entry.get("name").and_then(Value::as_str).unwrap_or("")
    }

    pub(crate) fn kind(self) -> String {
        self.entry
            .get("type")
            .and_then(Value::as_str)
            .map_or_else(|| "library".to_owned(), lower)
    }

    fn extra(self) -> Option<&'a Map<String, Value>> {
        self.entry.get("extra").and_then(Value::as_object)
    }

    fn target_dir(self) -> Option<&'a str> {
        self.entry
            .get("target-dir")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
    }
}

/// What the root package and Composer's config say about paths.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Root<'a> {
    pub(crate) extra: Option<&'a Map<String, Value>>,
    /// `getcwd()`: the project directory, forward slashes.
    pub(crate) cwd: &'a str,
    /// `vendor-dir` as configured, before it is made absolute.
    pub(crate) vendor_relative: &'a str,
}

/// composer/installers' `Installer`, with `installer-disable` applied.
#[derive(Debug)]
pub(crate) struct Installers {
    frameworks: Vec<&'static Framework>,
}

/// `(string) $v` for the scalars `array_intersect` compares.
fn php_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Bool(true) => Some("1".to_owned()),
        Value::Bool(false) | Value::Null => Some(String::new()),
        Value::Number(n) if n.as_f64() == Some(1.0) => Some("1".to_owned()),
        Value::Number(n) => Some(n.to_string()),
        Value::Array(_) | Value::Object(_) => None,
    }
}

impl Installers {
    // Composer: installers Installer::removeDisabledInstallers
    pub(crate) fn new(root_extra: Option<&Map<String, Value>>) -> Result<Self, String> {
        let disable = root_extra.and_then(|e| e.get("installer-disable"));
        let list: Vec<&Value> = match disable {
            None | Some(Value::Bool(false)) => {
                return Ok(Self {
                    frameworks: FRAMEWORKS.iter().collect(),
                });
            }
            Some(Value::Array(items)) => items.iter().collect(),
            Some(Value::Object(map)) => map.values().collect(),
            Some(other) => vec![other],
        };
        let strings = list
            .iter()
            .map(|v| php_string(v))
            .collect::<Option<Vec<String>>>()
            .ok_or("extra.installer-disable has a nested list")?;
        if strings.iter().any(|s| s == "1" || s == "all" || s == "*") {
            return Ok(Self {
                frameworks: Vec::new(),
            });
        }
        let off: Vec<&str> = list.iter().filter_map(|v| v.as_str()).collect();
        Ok(Self {
            frameworks: FRAMEWORKS
                .iter()
                .filter(|f| !off.contains(&f.prefix))
                .collect(),
        })
    }

    // Composer: installers Installer::findFrameworkType
    fn framework(&self, kind: &str) -> Option<&'static Framework> {
        self.frameworks
            .iter()
            .copied()
            .find(|f| kind.starts_with(f.prefix))
    }

    // Composer: installers Installer::supports
    fn supports(framework: &Framework, kind: &str) -> bool {
        let head = format!("{}-", framework.prefix);
        kind.match_indices(&head).any(|(i, _)| {
            let rest = &kind[i + head.len()..];
            framework.locations.iter().any(|(k, _)| rest.starts_with(k))
        })
    }

    /// The path `Installer::getInstallPath` returns for `package`: `Ok(None)`
    /// when composer/installers does not install its type, `Err` when phpm
    /// cannot say for sure.
    // Composer: installers Installer::getInstallPath, BaseInstaller::getInstallPath
    pub(crate) fn path(
        &self,
        package: Locked<'_>,
        root: Root<'_>,
    ) -> Result<Option<String>, String> {
        let kind = package.kind();
        let Some(framework) = self.framework(&kind) else {
            return Ok(None);
        };
        if !Self::supports(framework, &kind) {
            return Ok(None);
        }
        let pretty = package.pretty_name();
        match framework.inflect {
            Inflect::CakePhp => {
                return Err("cakephp types depend on the installed cakephp version".into());
            }
            Inflect::Bitrix => {
                return Err("bitrix types can ask about duplicate packages".into());
            }
            Inflect::Oxid if kind == "oxid-module" => {
                return Err("oxid modules write vendor metadata files".into());
            }
            _ => {}
        }
        if package.target_dir().is_some() {
            return Err(format!("{pretty} has a target-dir"));
        }
        let mut parts = pretty.split('/');
        let (vendor, name) = match (parts.next(), parts.next()) {
            (Some(v), Some(n)) => (v, n),
            _ => ("", pretty),
        };
        let valid = |s: &str| {
            s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        };
        if !valid(vendor) || !valid(name) {
            return Err(format!("{pretty} is not a plain package name"));
        }
        let template = if framework.inflect == Inflect::SilverStripe
            && let Some(path) = silverstripe_framework(package)
        {
            path
        } else {
            let mut vars = inflect(
                framework.inflect,
                Vars {
                    name: name.to_owned(),
                    vendor: vendor.to_owned(),
                    kind: kind.clone(),
                    install_item_dir: None,
                },
                package,
            )?;
            let extra = package.extra();
            let installer_name = extra.and_then(|e| e.get("installer-name"));
            if !empty(installer_name) {
                installer_name
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("{pretty} has a non-string installer-name"))?
                    .clone_into(&mut vars.name);
            }
            let custom = custom_path(root.extra, pretty, &kind, vendor)?;
            let path = if let Some(path) = custom {
                path
            } else {
                let short = kind.get(framework.prefix.len() + 1..).unwrap_or("");
                framework
                    .locations
                    .iter()
                    .find(|(k, _)| *k == short)
                    .map(|(_, v)| (*v).to_owned())
                    .ok_or_else(|| format!("Package type \"{kind}\" is not supported"))?
            };
            template_path(&path, &vars)?
        };
        Ok(Some(if is_absolute_path(&template) {
            template
        } else {
            format!("{}/{template}", root.cwd)
        }))
    }
}

// Composer: installers SilverStripeInstaller::getInstallPath
fn silverstripe_framework(package: Locked<'_>) -> Option<String> {
    if !package
        .pretty_name()
        .eq_ignore_ascii_case("silverstripe/framework")
    {
        return None;
    }
    let pretty = package.entry.get("version").and_then(Value::as_str)?;
    let version = phpm_lock::version::normalize(pretty).ok()?;
    let mut digits = version.split('.');
    let numeric = (0..3).all(|_| {
        digits
            .next()
            .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
    });
    (numeric
        && phpm_lock::constraint::version_compare(&version, "2.999.999")
            == std::cmp::Ordering::Less)
        .then(|| "sapphire/".to_owned())
}

// Composer: installers BaseInstaller::mapCustomInstallPaths
fn custom_path(
    root_extra: Option<&Map<String, Value>>,
    pretty: &str,
    kind: &str,
    vendor: &str,
) -> Result<Option<String>, String> {
    let paths = root_extra.and_then(|e| e.get("installer-paths"));
    if empty(paths) {
        return Ok(None);
    }
    let Some(Value::Object(paths)) = paths else {
        return Err("extra.installer-paths is not an object".into());
    };
    let wanted = [
        pretty.to_owned(),
        format!("type:{kind}"),
        format!("vendor:{vendor}"),
    ];
    for (path, names) in paths {
        let names: Vec<&str> = match names {
            Value::Null => Vec::new(),
            Value::String(s) => vec![s.as_str()],
            Value::Array(items) => items
                .iter()
                .map(|v| {
                    v.as_str()
                        .ok_or("extra.installer-paths has a non-string entry")
                })
                .collect::<Result<_, _>>()?,
            _ => return Err("extra.installer-paths has a non-string entry".into()),
        };
        if names.iter().any(|n| wanted.iter().any(|w| w == n)) {
            return Ok(Some(path.clone()));
        }
    }
    Ok(None)
}

// Composer: installers BaseInstaller::templatePath
fn template_path(path: &str, vars: &Vars) -> Result<String, String> {
    if !path.contains('{') {
        return Ok(path.to_owned());
    }
    let mut names = Vec::new();
    let mut rest = path;
    while let Some(start) = rest.find("{$") {
        let after = &rest[start + 2..];
        let len = after
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
            .count();
        if after.as_bytes().get(len) == Some(&b'}') {
            names.push(&after[..len]);
            rest = &after[len + 1..];
        } else {
            rest = after;
        }
    }
    let mut out = path.to_owned();
    for var in names {
        let value = vars.get(var).ok_or_else(|| {
            format!("the install path {path} uses an unknown variable {{${var}}}")
        })?;
        out = out.replace(&format!("{{${var}}}"), value);
    }
    Ok(out)
}

fn camel(name: &str) -> String {
    let split = lower(&underscore_capitals(name));
    ucwords(&replace_each(&split, &["-", "_"], " ")).replace(' ', "")
}

fn words(name: &str) -> String {
    ucwords(&replace_each(name, &["-", "_"], " ")).replace(' ', "")
}

fn extra_string(package: Locked<'_>, path: &[&str]) -> Result<Option<String>, String> {
    let mut value = package.extra().map(|e| e as &Map<String, Value>);
    let mut found: Option<&Value> = None;
    for (i, key) in path.iter().enumerate() {
        let next = value.and_then(|m| m.get(*key));
        if i + 1 == path.len() {
            found = next;
        } else {
            value = next.and_then(Value::as_object);
        }
    }
    if empty(found) {
        return Ok(None);
    }
    found
        .and_then(Value::as_str)
        .map(|s| Some(s.to_owned()))
        .ok_or_else(|| format!("extra.{} is not a string", path.join(".")))
}

// Composer: installers *Installer::inflectPackageVars
#[expect(clippy::too_many_lines, reason = "one arm per installer class")]
fn inflect(how: Inflect, mut v: Vars, package: Locked<'_>) -> Result<Vars, String> {
    let kind = v.kind.clone();
    match how {
        Inflect::Plain
        | Inflect::SilverStripe
        | Inflect::Oxid
        | Inflect::Bitrix
        | Inflect::CakePhp => {}
        Inflect::Camel => v.name = camel(&v.name),
        Inflect::Agl => v.name = agl(&v.name),
        Inflect::Asgard => match kind.as_str() {
            "asgard-module" => v.name = words(&strip(&v.name, &[("-module", At::End)], false)),
            "asgard-theme" => v.name = words(&strip(&v.name, &[("-theme", At::End)], false)),
            _ => {}
        },
        Inflect::Cockpit => {
            if kind == "cockpit-module" {
                v.name = ucfirst(&strip(&v.name, &[("cockpit-", At::Anywhere)], true));
            }
        }
        Inflect::Croogo => {
            v.name = ucwords(&lower(&replace_each(&v.name, &["-", "_"], " "))).replace(' ', "");
        }
        Inflect::DokuWiki => {
            let suffix = match kind.as_str() {
                "dokuwiki-plugin" => Some("-plugin"),
                "dokuwiki-template" => Some("-template"),
                _ => None,
            };
            if let Some(suffix) = suffix {
                let name = strip(&v.name, &[(suffix, At::End)], false);
                v.name = strip_dokuwiki(&name);
            }
        }
        Inflect::ForkCms => {
            let what = match kind.as_str() {
                "fork-cms-module" => Some(("-module", "Module")),
                "fork-cms-theme" => Some(("-theme", "Theme")),
                _ => None,
            };
            if let Some((dash, end)) = what {
                let stripped = strip(
                    &v.name,
                    &[
                        ("fork-cms-", At::Start),
                        (dash, At::Anywhere),
                        ("ForkCMS", At::Anywhere),
                        ("ForkCms", At::Anywhere),
                        ("Forkcms", At::Anywhere),
                        ("forkcms", At::Anywhere),
                        (end, At::End),
                    ],
                    false,
                );
                v.name = words(&stripped);
            }
        }
        Inflect::Grav => v.name = grav(&lower(&v.name)),
        Inflect::Majima => v.name = ucfirst(&camel_dashes(&v.name)),
        Inflect::Mautic => {
            if kind == "mautic-plugin" || kind == "mautic-theme" {
                v.name = if let Some(dir) = extra_string(package, &["install-directory-name"])? {
                    dir
                } else {
                    let base = package.pretty_name().rsplit('/').next().unwrap_or("");
                    ucwords(&base.replace('-', " ")).replace(' ', "")
                };
            }
        }
        Inflect::Maya => {
            if kind == "maya-module" {
                v.name = words(&strip(&v.name, &[("-module", At::End)], false));
            }
        }
        Inflect::MediaWiki => match kind.as_str() {
            "mediawiki-extension" => {
                let name = strip(&v.name, &[("-extension", At::End)], false);
                v.name = ucwords(&name.replace('-', " ")).replace(' ', "");
            }
            "mediawiki-skin" => v.name = strip(&v.name, &[("-skin", At::End)], false),
            _ => {}
        },
        Inflect::Microweber => {
            let dir = v.name.clone();
            let ends: &[&str] = match kind.as_str() {
                "microweber-template" => &["-template", "template-"],
                "microweber-templates" => &["-templates", "templates-"],
                "microweber-core" | "microweber-adapter" => {
                    &["-providers", "-provider", "-adapter"]
                }
                "microweber-module" => &["-module", "module-"],
                "microweber-modules" => &["-modules", "modules-"],
                "microweber-skin" => &["-skin", "skin-"],
                "microweber-element" | "microweber-elements" => {
                    &["-elements", "elements-", "-element", "element-"]
                }
                _ => &[],
            };
            v.install_item_dir = Some(
                ends.iter()
                    .fold(dir, |d, end| strip(&d, &[(end, At::End)], false)),
            );
        }
        Inflect::October => {
            let suffix = match kind.as_str() {
                "october-plugin" => Some("-plugin"),
                "october-theme" => Some("-theme"),
                _ => None,
            };
            if let Some(suffix) = suffix {
                v.name = strip(&v.name, &[("oc-", At::Start), (suffix, At::End)], false);
                v.vendor = keep_word_chars(&v.vendor);
            }
        }
        Inflect::OntoWiki => {
            let mut name = lower(&v.name);
            if name.len() > 8 && name.ends_with("ontowiki") {
                name.truncate(name.len() - 9);
            }
            let name = strip(&name, &[("-theme", At::End)], false);
            v.name = strip(&name, &[("-translation", At::End)], false);
        }
        Inflect::Plentymarkets => {
            v.name = v
                .name
                .split('-')
                .filter(|bit| !bit.eq_ignore_ascii_case("plugin"))
                .map(ucfirst)
                .collect();
        }
        Inflect::Pxcms => {
            let what = match kind.as_str() {
                "pxcms-module" => Some(("module-", "-module")),
                "pxcms-theme" => Some(("theme-", "-theme")),
                _ => None,
            };
            if let Some((prefix, suffix)) = what {
                let name = v.name.replace("pxcms-", "").replace(prefix, "");
                let name = strip(&name, &[(suffix, At::End)], false);
                v.name = ucwords(&name.replace('-', "_"));
            }
        }
        Inflect::Roundcube => v.name = lower(&v.name.replace('-', "_")),
        Inflect::Shopware => {
            if kind == "shopware-theme" {
                v.name = v.name.replace('-', "_");
            } else {
                v.name = format!("{}{}", ucfirst(&v.vendor), ucfirst(&camel_dashes(&v.name)));
            }
        }
        Inflect::SiteDirect => {
            if lower(&v.vendor) == "sitedirect" {
                "SiteDirect".clone_into(&mut v.vendor);
            }
            v.name = words(&v.name);
        }
        Inflect::Sydes => match kind.as_str() {
            "sydes-module" => {
                let name = strip(
                    &v.name,
                    &[("sydes-", At::Start), ("-module", At::End)],
                    true,
                );
                v.name = words(&name);
            }
            "sydes-theme" => {
                let name = strip(
                    &v.name,
                    &[("sydes-", At::Start), ("-theme", At::End)],
                    false,
                );
                v.name = lower(&name);
            }
            _ => {}
        },
        Inflect::Tao => {
            let named = package.extra().and_then(|e| e.get("tao-extension-name"));
            if let Some(value) = named {
                value
                    .as_str()
                    .ok_or("extra.tao-extension-name is not a string")?
                    .clone_into(&mut v.name);
            } else {
                let name = v.name.replace("extension-", "").replace('-', " ");
                v.name = lcfirst(&ucwords(&name).replace(' ', ""));
            }
        }
        Inflect::TastyIgniter => match kind.as_str() {
            "tastyigniter-module" => {
                v.name = strip(&v.name, &[("ti-module-", At::Start)], false);
            }
            "tastyigniter-extension" => {
                if let Some(code) = extra_string(package, &["tastyigniter-extension", "code"])? {
                    let mut parts = code.split('.');
                    parts.next().unwrap_or("").clone_into(&mut v.vendor);
                    parts.next().unwrap_or("").clone_into(&mut v.name);
                }
                v.vendor = keep_word_chars(&v.vendor);
                v.name = strip(&v.name, &[("ti-ext-", At::Start)], false);
            }
            "tastyigniter-theme" => {
                if let Some(code) = extra_string(package, &["tastyigniter-theme", "code"])? {
                    v.name = code;
                }
                v.name = strip(&v.name, &[("ti-theme-", At::Start)], false);
            }
            _ => {}
        },
        Inflect::Winter => match kind.as_str() {
            "winter-module" => {
                v.name = strip(&v.name, &[("wn-", At::Start), ("-module", At::End)], false);
            }
            "winter-plugin" => {
                v.name = strip(&v.name, &[("wn-", At::Start), ("-plugin", At::End)], false);
                v.vendor = keep_word_chars(&v.vendor);
            }
            "winter-theme" => {
                v.name = strip(&v.name, &[("wn-", At::Start), ("-theme", At::End)], false);
            }
            _ => {}
        },
    }
    Ok(v)
}

/// `preg_replace('/^dokuwiki_?-?/', '', $s)`.
fn strip_dokuwiki(s: &str) -> String {
    let Some(rest) = s.strip_prefix("dokuwiki") else {
        return s.to_owned();
    };
    let rest = rest.strip_prefix('_').unwrap_or(rest);
    rest.strip_prefix('-').unwrap_or(rest).to_owned()
}

/// `preg_replace_callback('/(?:^|_|-)(.?)/', fn ($m) => strtoupper($m[1]), $s)`.
fn agl(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut start = true;
    while let Some(c) = chars.peek().copied() {
        if start || c == '_' || c == '-' {
            if !start {
                chars.next();
            }
            start = false;
            if let Some(next) = chars.peek().copied()
                && next != '\n'
            {
                out.push(next.to_ascii_uppercase());
                chars.next();
            }
            continue;
        }
        out.push(c);
        chars.next();
    }
    out
}

/// `/^(?:grav-)?(?:(?:plugin|theme)-)?(.*?)(?:-(?:plugin|theme))?$/ui` to `$1`.
fn grav(s: &str) -> String {
    let s = s.strip_prefix("grav-").unwrap_or(s);
    let s = s
        .strip_prefix("plugin-")
        .or_else(|| s.strip_prefix("theme-"))
        .unwrap_or(s);
    let s = s
        .strip_suffix("-plugin")
        .or_else(|| s.strip_suffix("-theme"))
        .unwrap_or(s);
    s.to_owned()
}

/// The path roots/wordpress-core-installer installs a `wordpress-core`
/// package to, relative to the project unless configured absolute.
// Composer: roots/wordpress-core-installer WordPressCoreInstaller::getInstallPath
pub(crate) fn wordpress_core_path(
    package: Locked<'_>,
    root: Root<'_>,
) -> Result<Option<String>, String> {
    if package.kind() != "wordpress-core" {
        return Ok(None);
    }
    let pretty = package.pretty_name();
    let as_dir = |v: Option<&Value>| -> Result<Option<String>, String> {
        if empty(v) {
            return Ok(None);
        }
        v.and_then(Value::as_str)
            .map(|s| Some(s.to_owned()))
            .ok_or_else(|| "wordpress-install-dir is not a string".to_owned())
    };
    let mut dir = None;
    let top = root.extra.and_then(|e| e.get("wordpress-install-dir"));
    if !empty(top) {
        dir = match top {
            Some(Value::Object(per_package)) => as_dir(per_package.get(pretty))?,
            Some(Value::Array(_)) => None,
            other => as_dir(other)?,
        };
    }
    if dir.is_none() {
        dir = as_dir(package.extra().and_then(|e| e.get("wordpress-install-dir")))?;
    }
    let dir = dir.unwrap_or_else(|| "wordpress".to_owned());
    let vendor = if root.vendor_relative.is_empty() {
        "vendor"
    } else {
        root.vendor_relative
    };
    if dir == "." || dir == vendor {
        return Err(format!(
            "Warning! {dir} is an invalid WordPress install directory (from {pretty})!"
        ));
    }
    Ok(Some(dir))
}

include!("installers_table.rs");

#[cfg(test)]
mod tests {
    use super::{
        Installers, Locked, Root, agl, grav, strip_dokuwiki, template_path, wordpress_core_path,
    };
    use serde_json::{Map, Value, json};

    fn obj(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => unreachable!(),
        }
    }

    fn path_for(entry: &Value, root_extra: &Value) -> Result<Option<String>, String> {
        let entry = obj(entry.clone());
        let extra = obj(root_extra.clone());
        let installers = Installers::new(Some(&extra))?;
        installers.path(
            Locked { entry: &entry },
            Root {
                extra: Some(&extra),
                cwd: "/p",
                vendor_relative: "vendor",
            },
        )
    }

    fn simple(name: &str, kind: &str) -> Value {
        json!({"name": name, "version": "1.0.0", "type": kind})
    }

    #[test]
    fn maps_known_types_to_their_locations() {
        let none = json!({});
        for (name, kind, want) in [
            ("drupal/token", "drupal-module", "/p/modules/token/"),
            ("drupal/core", "drupal-core", "/p/core/"),
            ("drupal/x", "drupal-custom-module", "/p/modules/custom/x/"),
            ("drupal/r", "drupal-recipe", "/p/recipes/r"),
            (
                "wpackagist-plugin/akismet",
                "wordpress-plugin",
                "/p/wp-content/plugins/akismet/",
            ),
            ("a/b", "wordpress-muplugin", "/p/wp-content/mu-plugins/b/"),
            ("a/b", "zikula-module", "/p/modules/a-b/"),
            (
                "a/b",
                "ee2-addon",
                "/p/system/expressionengine/third_party/b/",
            ),
            ("a/b", "ee3-theme", "/p/themes/user/b/"),
            (
                "a/b",
                "quicksilver-script",
                "/p/web/private/scripts/quicksilver/b",
            ),
            ("a/b", "modxevo-snippet", "/p/assets/snippets/b/"),
        ] {
            assert_eq!(
                path_for(&simple(name, kind), &none),
                Ok(Some(want.to_owned())),
                "{kind}"
            );
        }
    }

    #[test]
    fn leaves_other_types_to_the_library_installer() {
        let none = json!({});
        for kind in [
            "library",
            "composer-plugin",
            "drupal-unknown",
            "wordpress-core",
            "vgmcp-bundle",
        ] {
            assert_eq!(path_for(&simple("a/b", kind), &none), Ok(None), "{kind}");
        }
    }

    #[test]
    fn root_installer_paths_win_by_name_type_or_vendor() {
        let extra = json!({"installer-paths": {
            "web/app/plugins/{$name}/": ["type:wordpress-plugin"],
            "custom/{$vendor}/{$name}/": ["a/special"],
            "by-vendor/{$type}/{$name}": "vendor:v",
        }});
        assert_eq!(
            path_for(&simple("x/akismet", "wordpress-plugin"), &extra),
            Ok(Some("/p/web/app/plugins/akismet/".into()))
        );
        assert_eq!(
            path_for(&simple("a/special", "drupal-module"), &extra),
            Ok(Some("/p/custom/a/special/".into()))
        );
        assert_eq!(
            path_for(&simple("v/thing", "drupal-theme"), &extra),
            Ok(Some("/p/by-vendor/drupal-theme/thing".into()))
        );
        assert_eq!(
            path_for(&simple("a/lib", "library"), &extra),
            Ok(None),
            "installer-paths only applies to types composer/installers installs"
        );
        let absolute = json!({"installer-paths": {"/srv/{$name}": ["type:drupal-module"]}});
        assert_eq!(
            path_for(&simple("a/m", "drupal-module"), &absolute),
            Ok(Some("/srv/m".into()))
        );
    }

    #[test]
    fn installer_name_replaces_the_name_after_inflection() {
        let mut entry = simple("a/my-plugin", "matomo-plugin");
        entry["extra"] = json!({"installer-name": "Other-Name"});
        assert_eq!(
            path_for(&entry, &json!({})),
            Ok(Some("/p/plugins/Other-Name/".into()))
        );
        entry["extra"] = json!({"installer-name": "0"});
        assert_eq!(
            path_for(&entry, &json!({})),
            Ok(Some("/p/plugins/MyPlugin/".into()))
        );
        entry["extra"] = json!({"installer-name": ["x"]});
        assert!(path_for(&entry, &json!({})).is_err());
    }

    #[test]
    fn disables_installers_like_the_plugin() {
        let entry = simple("a/b", "drupal-module");
        assert_eq!(
            path_for(&entry, &json!({"installer-disable": true})),
            Ok(None)
        );
        assert_eq!(
            path_for(&entry, &json!({"installer-disable": "all"})),
            Ok(None)
        );
        assert_eq!(
            path_for(&entry, &json!({"installer-disable": ["*"]})),
            Ok(None)
        );
        assert_eq!(path_for(&entry, &json!({"installer-disable": 1})), Ok(None));
        assert_eq!(
            path_for(&entry, &json!({"installer-disable": ["drupal"]})),
            Ok(None)
        );
        assert_eq!(
            path_for(&entry, &json!({"installer-disable": ["wordpress"]})),
            Ok(Some("/p/modules/b/".into()))
        );
        assert_eq!(
            path_for(&entry, &json!({"installer-disable": false})),
            Ok(Some("/p/modules/b/".into()))
        );
        assert_eq!(
            path_for(&entry, &json!({"installer-disable": {"x": "drupal"}})),
            Ok(None)
        );
        assert!(path_for(&entry, &json!({"installer-disable": [["drupal"]]})).is_err());
    }

    #[test]
    fn declines_what_it_cannot_reproduce() {
        let none = json!({});
        assert!(path_for(&simple("a/b", "cakephp-plugin"), &none).is_err());
        assert!(path_for(&simple("a/b", "bitrix-module"), &none).is_err());
        assert!(path_for(&simple("a/b", "oxid-module"), &none).is_err());
        assert!(path_for(&simple("a/b", "oxid-theme"), &none).is_ok());
        let mut target = simple("a/b", "drupal-module");
        target["target-dir"] = json!("x");
        assert!(path_for(&target, &none).is_err());
        assert!(path_for(&simple("a/b c", "drupal-module"), &none).is_err());
        assert!(path_for(&simple("a/b", "wordpress-plugin-x"), &none).is_err());
        let bad = json!({"installer-paths": {"x/{$nope}": ["type:drupal-module"]}});
        assert!(path_for(&simple("a/b", "drupal-module"), &bad).is_err());
        let odd = json!({"installer-paths": {"x/{$name}": [true]}});
        assert!(path_for(&simple("a/b", "drupal-module"), &odd).is_err());
        let list = json!({"installer-paths": ["x"]});
        assert!(path_for(&simple("a/b", "drupal-module"), &list).is_err());
    }

    #[test]
    fn inflects_names_like_each_installer_class() {
        let none = json!({});
        for (name, kind, want) in [
            ("a/MyPlugin", "piwik-plugin", "/p/plugins/MyPlugin/"),
            (
                "a/my_plugin-name",
                "mantisbt-plugin",
                "/p/plugins/MyPluginName/",
            ),
            ("a/my-module", "agl-module", "/p/More/MyModule/"),
            ("a/blog-module", "asgard-module", "/p/Modules/Blog/"),
            (
                "a/cockpit-Hello",
                "cockpit-module",
                "/p/cockpit/modules/addons/Hello/",
            ),
            ("a/my_plugin", "croogo-plugin", "/p/Plugin/MyPlugin/"),
            (
                "a/dokuwiki_-x-plugin",
                "dokuwiki-plugin",
                "/p/lib/plugins/x/",
            ),
            (
                "a/grav-plugin-foo-theme",
                "grav-plugin",
                "/p/user/plugins/foo/",
            ),
            (
                "a/my-plugin-name",
                "majima-plugin",
                "/p/plugins/MyPluginName/",
            ),
            ("a/foo-bar", "mautic-plugin", "/p/plugins/FooBar/"),
            ("a/foo-bar-module", "maya-module", "/p/modules/FooBar/"),
            (
                "a/foo-bar-extension",
                "mediawiki-extension",
                "/p/extensions/FooBar/",
            ),
            ("a/foo-skin", "mediawiki-skin", "/p/skins/foo/"),
            (
                "a/shop-template",
                "microweber-template",
                "/p/userfiles/templates/shop/",
            ),
            (
                "my-vendor/oc-blog-plugin",
                "october-plugin",
                "/p/plugins/myvendor/blog/",
            ),
            ("a/x.OntoWiki", "ontowiki-extension", "/p/extensions/x/"),
            ("a/my-plugin-thing", "plentymarkets-plugin", "/p/MyThing/"),
            (
                "a/pxcms-module-blog-module",
                "pxcms-module",
                "/p/app/Modules/Blog/",
            ),
            ("a/my-plugin", "roundcube-plugin", "/p/plugins/my_plugin/"),
            (
                "swag/my-plugin",
                "shopware-plugin",
                "/p/custom/plugins/SwagMyPlugin/",
            ),
            ("a/my-theme", "shopware-theme", "/p/templates/my_theme/"),
            (
                "sitedirect/my_mod",
                "sitedirect-module",
                "/p/modules/SiteDirect/MyMod/",
            ),
            (
                "a/SyDES-blog-Module",
                "sydes-module",
                "/p/app/modules/Blog/",
            ),
            ("a/sydes-Dark-theme", "sydes-theme", "/p/themes/dark/"),
            ("a/extension-my-ext", "tao-extension", "/p/myExt"),
            ("a/ti-module-x", "tastyigniter-module", "/p/app/x/"),
            (
                "a.b/ti-ext-x",
                "tastyigniter-extension",
                "/p/extensions/ab/x/",
            ),
            ("a/wn-blog-plugin", "winter-plugin", "/p/plugins/a/blog/"),
            (
                "a/fork-cms-blog-module",
                "fork-cms-module",
                "/p/src/Modules/Blog/",
            ),
        ] {
            let got = path_for(&simple(name, kind), &none);
            if kind.starts_with("fork-cms") {
                assert_eq!(
                    got,
                    Ok(None),
                    "fork-cms types never match fork-(module|theme)"
                );
                continue;
            }
            assert_eq!(got, Ok(Some(want.to_owned())), "{name} {kind}");
        }
    }

    #[test]
    fn reads_installer_specific_extra() {
        let none = json!({});
        let mut tao = simple("a/x", "tao-extension");
        tao["extra"] = json!({"tao-extension-name": "taoThing"});
        assert_eq!(path_for(&tao, &none), Ok(Some("/p/taoThing".into())));
        let mut mautic = simple("a/x", "mautic-theme");
        mautic["extra"] = json!({"install-directory-name": "Dir"});
        assert_eq!(path_for(&mautic, &none), Ok(Some("/p/themes/Dir/".into())));
        let mut ti = simple("a/x", "tastyigniter-extension");
        ti["extra"] = json!({"tastyigniter-extension": {"code": "igniter.cart"}});
        assert_eq!(
            path_for(&ti, &none),
            Ok(Some("/p/extensions/igniter/cart/".into()))
        );
        let mut theme = simple("a/x", "tastyigniter-theme");
        theme["extra"] = json!({"tastyigniter-theme": {"code": "ti-theme-orange"}});
        assert_eq!(
            path_for(&theme, &none),
            Ok(Some("/p/themes/orange/".into()))
        );
        let mut ss = simple("silverstripe/framework", "silverstripe-module");
        ss["version"] = json!("2.4.7");
        assert_eq!(path_for(&ss, &none), Ok(Some("/p/sapphire/".into())));
        ss["version"] = json!("3.0.0");
        assert_eq!(path_for(&ss, &none), Ok(Some("/p/framework/".into())));
    }

    #[test]
    fn matches_composer_installers_on_every_type() {
        let golden: Value =
            serde_json::from_str(include_str!("../../tests/golden/installers/paths.json")).unwrap();
        let cases = golden.as_array().unwrap();
        assert!(cases.len() > 800);
        for case in cases {
            let mut entry = json!({"name": case["name"], "type": case["type"], "version": case.get("version").cloned().unwrap_or(json!("1.0.0"))});
            if let Some(extra) = case.get("extra") {
                entry["extra"] = extra.clone();
            }
            let extra = obj(case.get("root_extra").cloned().unwrap_or(json!({})));
            let got = Installers::new(Some(&extra)).and_then(|i| {
                i.path(
                    Locked {
                        entry: &obj(entry.clone()),
                    },
                    Root {
                        extra: Some(&extra),
                        cwd: "{cwd}",
                        vendor_relative: "vendor",
                    },
                )
            });
            match &case["path"] {
                Value::String(want) => assert_eq!(got, Ok(Some(want.clone())), "{case}"),
                Value::Null => assert_eq!(got, Ok(None), "{case}"),
                _ => assert!(got.is_err(), "{case}: {got:?}"),
            }
        }
    }

    #[test]
    fn helper_patterns_match_pcre() {
        assert_eq!(agl("my_module-x"), "MyModuleX");
        assert_eq!(agl("-foo"), "-foo");
        assert_eq!(agl("a__b"), "A_b");
        assert_eq!(grav("grav-theme-x-plugin"), "x");
        assert_eq!(strip_dokuwiki("dokuwikix"), "x");
        assert_eq!(strip_dokuwiki("other"), "other");
    }

    #[test]
    fn templates_replace_in_match_order() {
        let vars = super::Vars {
            name: "{$vendor}".into(),
            vendor: "v".into(),
            kind: "t".into(),
            install_item_dir: None,
        };
        assert_eq!(
            template_path("a/{$name}/{$vendor}", &vars),
            Ok("a/v/v".into())
        );
        assert_eq!(template_path("plain/", &vars), Ok("plain/".into()));
        assert_eq!(template_path("x/{$name", &vars), Ok("x/{$name".into()));
        assert!(template_path("x/{$}", &vars).is_err());
    }

    #[test]
    fn wordpress_core_goes_where_the_root_says() {
        let entry = obj(
            json!({"name": "roots/wordpress-no-content", "type": "wordpress-core", "extra": {"wordpress-install-dir": "from-package"}}),
        );
        let run = |extra: Value, vendor: &str| {
            let extra = obj(extra);
            wordpress_core_path(
                Locked { entry: &entry },
                Root {
                    extra: Some(&extra),
                    cwd: "/p",
                    vendor_relative: vendor,
                },
            )
        };
        assert_eq!(
            run(json!({"wordpress-install-dir": "web/wp"}), "vendor"),
            Ok(Some("web/wp".into()))
        );
        assert_eq!(
            run(
                json!({"wordpress-install-dir": {"roots/wordpress-no-content": "wp"}}),
                "vendor"
            ),
            Ok(Some("wp".into()))
        );
        assert_eq!(
            run(
                json!({"wordpress-install-dir": {"other/pkg": "wp"}}),
                "vendor"
            ),
            Ok(Some("from-package".into()))
        );
        assert_eq!(run(json!({}), "vendor"), Ok(Some("from-package".into())));
        assert!(run(json!({"wordpress-install-dir": "."}), "vendor").is_err());
        assert!(run(json!({"wordpress-install-dir": "lib"}), "lib").is_err());
        let plain = obj(json!({"name": "a/wp", "type": "wordpress-core"}));
        let extra = Map::new();
        let root = Root {
            extra: Some(&extra),
            cwd: "/p",
            vendor_relative: "vendor",
        };
        assert_eq!(
            wordpress_core_path(Locked { entry: &plain }, root),
            Ok(Some("wordpress".into()))
        );
        let lib = obj(json!({"name": "a/lib"}));
        assert_eq!(wordpress_core_path(Locked { entry: &lib }, root), Ok(None));
    }
}
