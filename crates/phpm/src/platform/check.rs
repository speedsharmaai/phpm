//! Composer's install-time platform check: every locked package and the
//! root's platform requirements against the platform repository.
//!
//! Composer runs a SAT solve with every locked package fixed; with nothing to
//! choose, that reduces to "is each platform requirement provided, and does
//! no conflict hit". Problems are worded as `Problem::getPrettyString` words them.

use std::fmt::Write as _;

use phpm_lock::constraint::{Constraint, Op, parse};
use phpm_lock::version::normalize;
use phpm_lock::{ComposerJson, Lock};
use serde_json::{Map, Value};

use super::{Platform, is_platform_package};
use crate::error::Error;

/// `--ignore-platform-reqs` / `--ignore-platform-req`, as
/// `PlatformRequirementFilterFactory` builds them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Filter {
    Nothing,
    All,
    List {
        ignore: Vec<String>,
        upper: Vec<String>,
    },
}

/// `BasePackage::packageNameToRegexp`: case-insensitive, `*` matches anything.
pub(crate) fn glob(pattern: &str, name: &str) -> bool {
    let p = pattern.to_ascii_lowercase().into_bytes();
    let n = name.to_ascii_lowercase().into_bytes();
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() && p[pi] == b'*' {
            star = Some((pi, ni));
            pi += 1;
        } else if pi < p.len() && p[pi] == n[ni] {
            pi += 1;
            ni += 1;
        } else if let Some((sp, sn)) = star {
            pi = sp + 1;
            ni = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == b'*')
}

impl Filter {
    pub(crate) fn new(all: bool, list: &[String]) -> Self {
        if all {
            return Self::All;
        }
        if list.is_empty() {
            return Self::Nothing;
        }
        let (upper, ignore): (Vec<&String>, Vec<&String>) =
            list.iter().partition(|r| r.ends_with('+'));
        Self::List {
            ignore: ignore.into_iter().cloned().collect(),
            upper: upper
                .into_iter()
                .map(|r| r[..r.len() - 1].to_owned())
                .collect(),
        }
    }

    pub(crate) fn is_ignored(&self, name: &str) -> bool {
        if !is_platform_package(name) {
            return false;
        }
        match self {
            Self::Nothing => false,
            Self::All => true,
            Self::List { ignore, .. } => ignore.iter().any(|p| glob(p, name)),
        }
    }

    /// `filterConstraint`: a `name+` entry drops the upper bound.
    fn constrain(&self, name: &str, constraint: Constraint, allow_upper: bool) -> Constraint {
        match self {
            Self::List { upper, .. }
                if allow_upper
                    && is_platform_package(name)
                    && upper.iter().any(|p| glob(p, name)) =>
            {
                constraint.without_upper_bound()
            }
            _ => constraint,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Link {
    target: String,
    constraint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Locked {
    name: String,
    pretty: String,
    version: String,
    requires: Vec<Link>,
    conflicts: Vec<Link>,
    /// `(link, "provide" | "replace")`.
    provides: Vec<(Link, &'static str)>,
}

/// The platform requirements an install has to satisfy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Requirements {
    root: Vec<Link>,
    root_provides: Vec<(Link, &'static str)>,
    packages: Vec<Locked>,
}

fn links(map: Option<&Value>) -> Vec<Link> {
    map.and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(target, c)| {
            Some(Link {
                target: target.to_ascii_lowercase(),
                constraint: c.as_str()?.to_owned(),
            })
        })
        .collect()
}

fn provides(entry: &Map<String, Value>) -> Vec<(Link, &'static str)> {
    let mut out: Vec<(Link, &'static str)> = links(entry.get("provide"))
        .into_iter()
        .map(|l| (l, "provide"))
        .collect();
    out.extend(
        links(entry.get("replace"))
            .into_iter()
            .map(|l| (l, "replace")),
    );
    out
}

impl Requirements {
    // Composer: Installer.php doInstall, Package/Locker.php getPlatformRequirements
    pub(crate) fn from_lock(
        composer: &ComposerJson,
        lock: &Lock,
        dev: bool,
    ) -> Result<Self, Error> {
        let root = composer.data();
        let mut wanted: Vec<Link> = links(root.get("require"));
        if dev {
            wanted.extend(links(root.get("require-dev")));
        }
        wanted.retain(|l| is_platform_package(&l.target));
        let mut from_lock = links(lock.data().get("platform"));
        if dev {
            from_lock.extend(links(lock.data().get("platform-dev")));
        }
        for link in from_lock {
            if !wanted.iter().any(|w| w.target == link.target) {
                wanted.push(link);
            }
        }
        let mut entries = lock.packages()?;
        if dev {
            entries.extend(lock.packages_dev()?);
        }
        let packages = entries
            .into_iter()
            .map(|entry| {
                let text = |k: &str| entry.get(k).and_then(Value::as_str).unwrap_or_default();
                let pretty = text("version").to_owned();
                Locked {
                    name: text("name").to_ascii_lowercase(),
                    version: normalize(&pretty).unwrap_or_else(|_| pretty.clone()),
                    pretty,
                    requires: links(entry.get("require")),
                    conflicts: links(entry.get("conflict")),
                    provides: provides(entry),
                }
            })
            .collect();
        Ok(Self {
            root: wanted,
            root_provides: provides(root),
            packages,
        })
    }

    /// Whether anything is left to check once `filter` has had its say;
    /// when nothing is, phpm does not need to run `php` at all.
    pub(crate) fn needs_platform(&self, filter: &Filter) -> bool {
        let relevant = |l: &Link| is_platform_package(&l.target) && !filter.is_ignored(&l.target);
        self.root.iter().any(relevant)
            || self
                .packages
                .iter()
                .any(|p| p.requires.iter().chain(&p.conflicts).any(relevant))
    }
}

/// Something that provides a name: a platform package or a link to it.
struct Provider {
    constraint: Constraint,
    from_platform: bool,
    /// `(source package pretty string, link pretty constraint, provide|replace)`
    /// when the name comes from a link rather than a package of that name.
    via: Option<(String, String, &'static str)>,
    pretty: String,
    overridden: Option<String>,
}

fn parsed(constraint: &str, owner: &str) -> Result<Constraint, Error> {
    parse(constraint).map_err(|e| Error::install(format!("{owner}: {e}")))
}

fn exact(version: &str) -> Constraint {
    Constraint::Single {
        op: Op::Eq,
        version: version.to_owned(),
    }
}

struct Checker<'a> {
    platform: &'a Platform,
    reqs: &'a Requirements,
    filter: &'a Filter,
}

impl Checker<'_> {
    // Composer: Pool::whatProvides, repositories in RepositorySet order
    fn providers(&self, name: &str) -> Result<Vec<Provider>, Error> {
        let mut out = Vec::new();
        let link_provider = |link: &Link, kind, owner: &str, pretty: &str, version: &str| {
            let constraint = if link.constraint == "self.version" {
                exact(version)
            } else {
                parsed(&link.constraint, owner)?
            };
            Ok::<_, Error>(Provider {
                constraint,
                from_platform: false,
                via: Some((format!("{owner} {pretty}"), link.constraint.clone(), kind)),
                pretty: pretty.to_owned(),
                overridden: None,
            })
        };
        for (link, kind) in self
            .reqs
            .root_provides
            .iter()
            .filter(|(l, _)| l.target == name)
        {
            out.push(link_provider(link, *kind, "__root__", "", "")?);
        }
        for p in &self.platform.packages {
            if p.name == name {
                out.push(Provider {
                    constraint: exact(&p.version),
                    from_platform: true,
                    via: None,
                    pretty: p.pretty.clone(),
                    overridden: p.overridden.clone(),
                });
            }
            for (target, kind) in &p.links {
                if target == name {
                    out.push(Provider {
                        constraint: exact(&p.version),
                        from_platform: true,
                        via: Some((format!("{} {}", p.name, p.pretty), p.pretty.clone(), kind)),
                        pretty: p.pretty.clone(),
                        overridden: None,
                    });
                }
            }
        }
        for locked in &self.reqs.packages {
            for (link, kind) in locked.provides.iter().filter(|(l, _)| l.target == name) {
                out.push(link_provider(
                    link,
                    kind,
                    &locked.name,
                    &locked.pretty,
                    &locked.version,
                )?);
            }
        }
        Ok(out)
    }

    fn satisfied(&self, name: &str, constraint: &Constraint) -> Result<bool, Error> {
        Ok(self
            .providers(name)?
            .iter()
            .any(|p| constraint.matches(&p.constraint)))
    }

    // Composer: DependencyResolver/Problem.php getPlatformPackageVersion
    fn version_text(&self, name: &str) -> Result<Option<String>, Error> {
        let providers = self.providers(name)?;
        let Some(selected) = providers
            .iter()
            .find(|p| p.from_platform)
            .or_else(|| providers.first())
        else {
            return Ok(None);
        };
        if let Some((source, constraint, kind)) = &selected.via {
            return Ok(Some(format!("{constraint} {kind}d by {source}")));
        }
        Ok(Some(match &selected.overridden {
            Some(note) => format!("{}; {note}", selected.pretty),
            None => selected.pretty.clone(),
        }))
    }

    // Composer: DependencyResolver/Problem.php getMissingPackageReason
    fn missing_reason(&self, name: &str, constraint: &str) -> Result<(String, String), Error> {
        let text = constraint_text(constraint);
        let disabled = |what: &str| {
            format!(
                "the {name} package is disabled by your platform config. Enable it again with \"composer config platform.{name} --unset\".{what}"
            )
        };
        if name.starts_with("php") || name == "hhvm" {
            let prefix = format!("- Root composer.json requires {name}{text} but ");
            if name == "hhvm" {
                return Ok((
                    prefix,
                    "HHVM was not detected on this machine, make sure it is in your PATH."
                        .to_owned(),
                ));
            }
            return Ok(match self.version_text(name)? {
                None => (prefix, disabled("")),
                Some(v) => (
                    prefix,
                    format!("your {name} version ({v}) does not satisfy that requirement."),
                ),
            });
        }
        if let Some(ext) = name.strip_prefix("ext-") {
            let prefix = format!("- Root composer.json requires PHP extension {name}{text} but ");
            return Ok(match self.version_text(name)? {
                None if self.platform.loaded.contains(ext) => (prefix, disabled("")),
                None => (
                    prefix,
                    format!(
                        "it is missing from your system. Install or enable PHP's {ext} extension."
                    ),
                ),
                Some(v) => (prefix, format!("it has the wrong version installed ({v}).")),
            });
        }
        if name.starts_with("lib-") {
            let prefix = format!("- Root composer.json requires linked library {name}{text} but ");
            let reason = if name == "lib-icu" {
                if self.platform.loaded.contains("intl") {
                    "it has the wrong version installed, try upgrading the intl extension."
                } else {
                    "it is missing from your system, make sure the intl extension is loaded."
                }
            } else {
                "it has the wrong version installed or is missing from your system, make sure to load the extension providing it."
            };
            return Ok((prefix, reason.to_owned()));
        }
        let found: Vec<String> = self
            .platform
            .packages
            .iter()
            .filter(|p| p.name == name)
            .map(|p| p.pretty.clone())
            .collect();
        Ok((
            format!("- Root composer.json requires {name}{text}, "),
            format!(
                "found {name}[{}] but it does not match the constraint.",
                found.join(", ")
            ),
        ))
    }

    fn problems(&self) -> Result<(Vec<String>, Vec<String>), Error> {
        let mut problems: Vec<String> = Vec::new();
        let mut extensions: Vec<String> = Vec::new();
        let mut note_ext = |name: &str| {
            if name.starts_with("ext-") && !extensions.iter().any(|e| e == name) {
                extensions.push(name.to_owned());
            }
        };
        for link in &self.reqs.root {
            if self.filter.is_ignored(&link.target) {
                continue;
            }
            let c = self.filter.constrain(
                &link.target,
                parsed(&link.constraint, "composer.json")?,
                true,
            );
            if !self.satisfied(&link.target, &c)? {
                let (prefix, reason) = self.missing_reason(&link.target, &link.constraint)?;
                problems.push(format!("\n    {prefix}{reason}"));
                note_ext(&link.target);
            }
        }
        for p in &self.reqs.packages {
            let head = format!(
                "\n    - {} is locked to version {} and an update of this package was not requested.",
                p.name, p.pretty
            );
            if let Some(line) = self.failing_require(p)? {
                problems.push(format!("{head}\n    - {line}"));
                if let Some(target) = line.split(' ').nth(3) {
                    note_ext(target);
                }
                continue;
            }
            if let Some(line) = self.failing_conflict(p)? {
                problems.push(format!("{head}\n    - {line}"));
            }
        }
        let mut unique: Vec<String> = Vec::new();
        for p in problems {
            if !unique.contains(&p) {
                unique.push(p);
            }
        }
        Ok((unique, extensions))
    }

    fn failing_require(&self, p: &Locked) -> Result<Option<String>, Error> {
        for link in &p.requires {
            if !is_platform_package(&link.target) || self.filter.is_ignored(&link.target) {
                continue;
            }
            let c = self
                .filter
                .constrain(&link.target, parsed(&link.constraint, &p.name)?, true);
            if !self.satisfied(&link.target, &c)? {
                let (_, reason) = self.missing_reason(&link.target, &link.constraint)?;
                return Ok(Some(format!(
                    "{} {} requires {} {} -> {reason}",
                    p.name, p.pretty, link.target, link.constraint
                )));
            }
        }
        Ok(None)
    }

    fn failing_conflict(&self, p: &Locked) -> Result<Option<String>, Error> {
        for link in &p.conflicts {
            if !is_platform_package(&link.target) || self.filter.is_ignored(&link.target) {
                continue;
            }
            let c = self
                .filter
                .constrain(&link.target, parsed(&link.constraint, &p.name)?, false);
            for hit in self
                .platform
                .packages
                .iter()
                .filter(|x| x.name == link.target)
            {
                if c.matches(&exact(&hit.version)) {
                    return Ok(Some(format!(
                        "{} {} conflicts with {} {}.",
                        p.name, p.pretty, hit.name, hit.pretty
                    )));
                }
            }
        }
        Ok(None)
    }
}

// Composer: DependencyResolver/Problem.php constraintToText
fn constraint_text(pretty: &str) -> String {
    let plain = !pretty.is_empty()
        && pretty
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    if plain {
        let mut versions = vec![pretty.to_owned()];
        for _ in 0..3_usize.saturating_sub(pretty.matches('.').count()) {
            let next = format!("{}.0", versions[versions.len() - 1]);
            versions.push(next);
        }
        return if versions.len() > 1 {
            let last = versions.pop().unwrap_or_default();
            format!(
                " {pretty} (exact version match: {} or {last})",
                versions.join(", ")
            )
        } else {
            format!(" {pretty} (exact version match: {pretty})")
        };
    }
    format!(" {pretty}")
}

// Composer: DependencyResolver/SolverProblemsException.php createExtensionHint
fn extension_hint(ini: &[String], missing: &[String]) -> String {
    let mut paths: Vec<&str> = ini.iter().map(String::as_str).collect();
    if paths.first().is_none_or(|p| p.is_empty()) {
        if paths.len() <= 1 {
            return String::new();
        }
        paths.remove(0);
    }
    let args: Vec<String> = missing
        .iter()
        .map(|e| format!("--ignore-platform-req={e}"))
        .collect();
    format!(
        "To enable extensions, verify that they are enabled in your .ini files:\n    - {}\nYou can also run `php --ini` in a terminal to see which files are used by PHP in CLI mode.\nAlternatively, you can run phpm with `{}` to temporarily ignore these required extensions.",
        paths.join("\n    - "),
        args.join(" ")
    )
}

/// Check the lock against the platform; the error carries Composer's
/// wording and its exit code 2.
pub(crate) fn verify(
    platform: &Platform,
    reqs: &Requirements,
    filter: &Filter,
) -> Result<(), Error> {
    let checker = Checker {
        platform,
        reqs,
        filter,
    };
    let (problems, extensions) = checker.problems()?;
    if problems.is_empty() {
        return Ok(());
    }
    let mut text = "Your lock file does not contain a compatible set of packages. Please run composer update.\n".to_owned();
    for (i, problem) in problems.iter().enumerate() {
        let _ = write!(text, "\n  Problem {}{problem}", i + 1);
    }
    text.push('\n');
    if !extensions.is_empty() {
        text.push('\n');
        text.push_str(&extension_hint(&platform.ini, &extensions));
    }
    Err(Error::new(2, text.trim_end().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::{Filter, Requirements, constraint_text, extension_hint, glob, verify};
    use crate::platform::Platform;
    use crate::platform::tests::sample;
    use phpm_lock::{ComposerJson, Lock};
    use serde_json::{Value, json};

    fn reqs(root: Value, lock: Value, dev: bool) -> Requirements {
        Requirements::from_lock(
            &ComposerJson::from_value(root).unwrap(),
            &Lock::from_value(lock).unwrap(),
            dev,
        )
        .unwrap()
    }

    fn platform() -> Platform {
        let mut p = Platform::build(&sample(), &Vec::new()).unwrap();
        p.ini = vec!["/etc/php.ini".into(), "/etc/conf.d/x.ini".into()];
        p
    }

    fn check(root: Value, lock: Value, filter: &Filter) -> Result<(), String> {
        verify(&platform(), &reqs(root, lock, true), filter).map_err(|e| {
            assert_eq!(e.code, 2);
            e.message
        })
    }

    #[test]
    fn globs_like_composer() {
        assert!(glob("ext-*", "EXT-intl"));
        assert!(glob("*", "php"));
        assert!(glob("e*-i*l", "ext-intl"));
        assert!(!glob("ext-*", "lib-icu"));
        assert!(!glob("php", "php-64bit"));
        assert!(glob("a*b*c", "aXbYbZc"));
    }

    #[test]
    fn builds_filters_from_flags() {
        assert_eq!(Filter::new(false, &[]), Filter::Nothing);
        assert_eq!(Filter::new(true, &["x".into()]), Filter::All);
        let f = Filter::new(false, &["ext-*".into(), "php+".into()]);
        assert!(f.is_ignored("ext-intl"));
        assert!(!f.is_ignored("php"));
        assert!(!f.is_ignored("vendor/ext-thing"));
        assert!(Filter::All.is_ignored("lib-icu"));
        assert!(!Filter::All.is_ignored("monolog/monolog"));
    }

    #[test]
    fn accepts_a_satisfiable_lock() {
        let lock = json!({"packages": [
            {"name": "a/a", "version": "1.0.0", "require": {"php": "^8.2", "ext-mbstring": "*", "lib-pcre": ">=10", "composer-runtime-api": "^2.2", "ext-zend-opcache": "*", "psr/log": "^3"}},
            {"name": "p/poly", "version": "v1.30.0", "provide": {"ext-ctype": "*"}, "replace": {"ext-iconv": "self.version"}},
            {"name": "b/b", "version": "1.0.0", "require": {"ext-ctype": "^8", "ext-iconv": "1.*", "lib-dom-libxml": "^2.9", "lib-uuid": "*"}, "conflict": {"ext-psr": "*", "php": "<8"}},
        ], "platform": {"php": ">=8.1"}});
        assert_eq!(
            check(json!({"require": {"php": "^8.4"}}), lock, &Filter::Nothing),
            Ok(())
        );
    }

    #[test]
    fn reports_missing_extensions_like_composer() {
        let lock = json!({
            "packages": [
                {"name": "a/meta", "version": "1.0.0", "require": {"ext-intl": "*", "php": ">=9.0"}},
                {"name": "b/meta", "version": "2.0.0", "require": {"ext-intl": ">=1"}},
            ],
            "platform": {"php": ">=8.1", "ext-zzz": "*"},
        });
        let err = check(
            json!({"require": {"php": ">=8.1"}}),
            lock.clone(),
            &Filter::Nothing,
        )
        .unwrap_err();
        assert_eq!(
            err,
            "Your lock file does not contain a compatible set of packages. Please run composer update.

  Problem 1
    - Root composer.json requires PHP extension ext-zzz * but it is missing from your system. Install or enable PHP's zzz extension.
  Problem 2
    - a/meta is locked to version 1.0.0 and an update of this package was not requested.
    - a/meta 1.0.0 requires ext-intl * -> it is missing from your system. Install or enable PHP's intl extension.
  Problem 3
    - b/meta is locked to version 2.0.0 and an update of this package was not requested.
    - b/meta 2.0.0 requires ext-intl >=1 -> it is missing from your system. Install or enable PHP's intl extension.

To enable extensions, verify that they are enabled in your .ini files:
    - /etc/php.ini
    - /etc/conf.d/x.ini
You can also run `php --ini` in a terminal to see which files are used by PHP in CLI mode.
Alternatively, you can run phpm with `--ignore-platform-req=ext-zzz --ignore-platform-req=ext-intl` to temporarily ignore these required extensions."
        );
        let ignore = Filter::new(false, &["ext-intl".into(), "lib-*".into(), "php+".into()]);
        let err = check(json!({}), lock.clone(), &ignore).unwrap_err();
        assert!(err.contains("a/meta 1.0.0 requires php >=9.0 -> your php version (8.4.13) does not satisfy that requirement."), "{err}");
        assert!(!err.contains("b/meta"), "{err}");
        let ignore = Filter::new(false, &["ext-*".into(), "php".into()]);
        assert_eq!(check(json!({}), lock, &ignore), Ok(()));
    }

    #[test]
    fn upper_bound_ignores_let_newer_versions_through() {
        let lock = json!({"packages": [{"name": "a/a", "version": "1.0.0", "require": {"php": "^7.4 || ~8.0.0"}}]});
        assert!(check(json!({}), lock.clone(), &Filter::Nothing).is_err());
        assert_eq!(
            check(json!({}), lock, &Filter::new(false, &["php+".into()])),
            Ok(())
        );
        let old =
            json!({"packages": [{"name": "a/a", "version": "1.0.0", "require": {"php": ">=9"}}]});
        assert!(check(json!({}), old, &Filter::new(false, &["php+".into()])).is_err());
    }

    #[test]
    fn words_each_kind_of_failure() {
        let failing =
            |root: Value| check(root, json!({"packages": []}), &Filter::Nothing).unwrap_err();
        let err = failing(json!({"require": {"ext-mbstring": "^9"}}));
        assert!(
            err.contains(
                "PHP extension ext-mbstring ^9 but it has the wrong version installed (8.4.13)."
            ),
            "{err}"
        );
        let err = failing(json!({"require": {"lib-icu": "*"}}));
        assert!(err.contains("linked library lib-icu * but it is missing from your system, make sure the intl extension is loaded."), "{err}");
        let err = failing(json!({"require": {"lib-pcre": "^11"}}));
        assert!(
            err.contains("make sure to load the extension providing it."),
            "{err}"
        );
        let err = failing(json!({"require": {"composer-runtime-api": "^3"}}));
        assert!(err.contains("requires composer-runtime-api ^3, found composer-runtime-api[2.2.2] but it does not match the constraint."), "{err}");
        let err = failing(json!({"require": {"php-zts": "*"}}));
        assert!(
            err.contains("the php-zts package is disabled by your platform config"),
            "{err}"
        );
        let err = failing(json!({"require": {"hhvm": "*"}}));
        assert!(err.contains("HHVM was not detected"), "{err}");
        let err = failing(json!({"require-dev": {"php": "8.1"}}));
        assert!(err.contains("requires php 8.1 (exact version match: 8.1, 8.1.0 or 8.1.0.0) but your php version (8.4.13)"), "{err}");
        assert!(!err.contains("To enable extensions"));
        let err = failing(json!({"require": {"lib-old": "^3"}}));
        assert!(err.contains("make sure to load"), "{err}");

        let lock = json!({"packages": [
            {"name": "p/poly", "version": "v1.0.0", "provide": {"ext-ctype": "^1"}},
            {"name": "a/a", "version": "1.0.0", "require": {"ext-ctype": "^2"}, "conflict": {"php": ">=8"}},
            {"name": "c/c", "version": "1.0.0", "conflict": {"php": ">=8"}},
        ]});
        let err = check(json!({}), lock, &Filter::Nothing).unwrap_err();
        assert!(err.contains("a/a 1.0.0 requires ext-ctype ^2 -> it has the wrong version installed (^1 provided by p/poly v1.0.0)."), "{err}");
        assert!(
            err.contains("c/c 1.0.0 conflicts with php 8.4.13."),
            "{err}"
        );
    }

    #[test]
    fn honours_overrides_and_dev_mode() {
        let off = json!(false);
        let php = json!("7.4.0");
        let platform =
            Platform::build(&sample(), &vec![("ext-mbstring", &off), ("php", &php)]).unwrap();
        let r = reqs(
            json!({"require-dev": {"ext-mbstring": "*"}}),
            json!({"packages-dev": [{"name": "d/d", "version": "1.0.0", "require": {"php": "^8"}}], "platform-dev": {"ext-x": "*"}}),
            true,
        );
        let err = verify(&platform, &r, &Filter::Nothing).unwrap_err().message;
        assert!(
            err.contains("the ext-mbstring package is disabled by your platform config"),
            "{err}"
        );
        assert!(
            err.contains(
                "your php version (7.4.0; overridden via config.platform, actual: 8.4.13)"
            ),
            "{err}"
        );
        assert!(err.contains("ext-x"), "{err}");
        let no_dev = reqs(
            json!({"require-dev": {"ext-mbstring": "*"}}),
            json!({"packages-dev": [{"name": "d/d", "version": "1.0.0", "require": {"php": "^8"}}], "platform-dev": {"ext-x": "*"}}),
            false,
        );
        assert!(!no_dev.needs_platform(&Filter::Nothing));
        assert_eq!(
            verify(&platform, &no_dev, &Filter::Nothing).map_err(|e| e.message),
            Ok(())
        );
        assert!(r.needs_platform(&Filter::Nothing));
        assert!(!r.needs_platform(&Filter::All));
    }

    #[test]
    fn root_provides_satisfy_requirements() {
        let lock = json!({"packages": [{"name": "a/a", "version": "1.0.0", "require": {"ext-mongo": "^1.6"}}]});
        assert!(check(json!({}), lock.clone(), &Filter::Nothing).is_err());
        assert_eq!(
            check(
                json!({"provide": {"ext-mongo": "1.6.14"}}),
                lock,
                &Filter::Nothing
            ),
            Ok(())
        );
    }

    #[test]
    fn rejects_constraints_it_cannot_parse() {
        let lock =
            json!({"packages": [{"name": "a/a", "version": "1.0.0", "require": {"php": "~>8"}}]});
        let err = verify(&platform(), &reqs(json!({}), lock, true), &Filter::Nothing).unwrap_err();
        assert_eq!(err.code, 1);
        assert!(err.message.starts_with("a/a: "), "{}", err.message);
    }

    #[test]
    fn renders_constraints_and_hints_like_composer() {
        assert_eq!(constraint_text("^8.1"), " ^8.1");
        assert_eq!(
            constraint_text("8.1.2.3"),
            " 8.1.2.3 (exact version match: 8.1.2.3)"
        );
        assert_eq!(
            constraint_text("8"),
            " 8 (exact version match: 8, 8.0, 8.0.0 or 8.0.0.0)"
        );
        assert_eq!(extension_hint(&[String::new()], &["ext-a".into()]), "");
        let hint = extension_hint(&[String::new(), "/x.ini".into()], &["ext-a".into()]);
        assert!(hint.contains("    - /x.ini\n"), "{hint}");
    }
}
