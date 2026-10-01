use crate::Error;
use crate::autoloads::Entry;
use phpm_lock::constraint::{Bound, lower_bound};
use phpm_php::{smart_strcmp, var_export_str};
use regex::bytes::RegexBuilder;
use std::collections::BTreeSet;

/// Which platform requirements `platform_check.php` leaves out, as
/// `--ignore-platform-reqs` / `--ignore-platform-req` set them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PlatformRequirements {
    #[default]
    Check,
    /// `--ignore-platform-reqs`: no `platform_check.php` at all.
    IgnoreAll,
    /// `--ignore-platform-req=<name>`, `*` wildcards allowed.
    Ignore(Vec<String>),
}

impl PlatformRequirements {
    // Composer: Filter/PlatformRequirementFilter/IgnoreListPlatformRequirementFilter.php isIgnored
    fn is_ignored(&self, target: &str) -> bool {
        match self {
            Self::Check => false,
            Self::IgnoreAll => true,
            Self::Ignore(names) => names.iter().filter(|n| !n.ends_with('+')).any(|name| {
                let pattern = format!("^(?:{})$", regex::escape(name).replace("\\*", ".*"));
                RegexBuilder::new(&pattern)
                    .case_insensitive(true)
                    .unicode(false)
                    .build()
                    .is_ok_and(|re| re.is_match(target.as_bytes()))
            }),
        }
    }
}

fn ext_name(target: &str) -> Option<&str> {
    target
        .get(..4)
        .filter(|p| p.eq_ignore_ascii_case("ext-"))
        .map(|_| &target[4..])
        .filter(|n| !n.is_empty())
}

fn chunks(bound: &Bound) -> Vec<String> {
    bound
        .version
        .replace('-', ".")
        .split('.')
        .map(str::to_owned)
        .collect()
}

fn intval(s: &str) -> i64 {
    s.bytes()
        .take_while(u8::is_ascii_digit)
        .fold(0_i64, |n, c| {
            n.saturating_mul(10).saturating_add(i64::from(c - b'0'))
        })
}

fn version_id(bound: &Bound) -> i64 {
    let c = chunks(bound);
    let part = |i: usize| c.get(i).map_or(0, |s| intval(s));
    part(0) * 10000 + part(1) * 100 + part(2)
}

fn human(bound: &Bound) -> String {
    chunks(bound)
        .into_iter()
        .take(3)
        .collect::<Vec<_>>()
        .join(".")
}

/// The contents of `platform_check.php`, or `None` when there is nothing to
/// check. `check_extensions` is `config.platform-check === true`.
// Composer: Autoload/AutoloadGenerator.php getPlatformCheck
pub(crate) fn platform_check(
    entries: &[Entry<'_>],
    check_extensions: bool,
    dev_names: &BTreeSet<String>,
    requirements: &PlatformRequirements,
) -> Result<Option<String>, Error> {
    let mut lowest = Bound::zero();
    let mut php_64bit = false;
    let mut extensions: Vec<(String, String)> = Vec::new();
    let mut providers: Vec<(String, Vec<String>)> = Vec::new();

    for entry in entries {
        let p = entry.package;
        for link in p.replaces.iter().chain(&p.provides) {
            if let Some(name) = ext_name(&link.target) {
                match providers.iter_mut().find(|(n, _)| n == name) {
                    Some((_, list)) => list.push(link.constraint.clone()),
                    None => providers.push((name.to_owned(), vec![link.constraint.clone()])),
                }
            }
        }
    }

    for entry in entries {
        let p = entry.package;
        if dev_names.contains(&p.name) {
            continue;
        }
        for link in &p.requires {
            if requirements.is_ignored(&link.target) {
                continue;
            }
            if link.target == "php" || link.target == "php-64bit" {
                let bound = lower_bound(&link.constraint).map_err(|reason| Error::Constraint {
                    package: p.name.clone(),
                    reason,
                })?;
                if bound.is_higher_than(&lowest) {
                    lowest = bound;
                }
            }
            if link.target == "php-64bit" {
                php_64bit = true;
            }
            if !check_extensions {
                continue;
            }
            let Some(name) = ext_name(&link.target) else {
                continue;
            };
            if let Some((_, provided)) = providers.iter().find(|(n, _)| n == name) {
                if provided.iter().any(|c| c.trim() == "*") {
                    continue;
                }
                return Err(Error::Unsupported(format!(
                    "{} provides {} with a constraint; only \"*\" is supported",
                    p.name, link.target
                )));
            }
            let name = if name == "zend-opcache" {
                "zend opcache"
            } else {
                name
            };
            let exported = var_export_str(name);
            let line = if name == "pcntl" || name == "readline" {
                format!(
                    "PHP_SAPI !== 'cli' || extension_loaded({exported}) || $missingExtensions[] = {exported};\n"
                )
            } else {
                format!("extension_loaded({exported}) || $missingExtensions[] = {exported};\n")
            };
            match extensions.iter_mut().find(|(k, _)| *k == exported) {
                Some(slot) => slot.1 = line,
                None => extensions.push((exported, line)),
            }
        }
    }

    extensions.sort_by(|(a, _), (b, _)| smart_strcmp(a, b));

    let mut required_php = String::new();
    if !lowest.is_zero() {
        let op = if lowest.inclusive { ">=" } else { ">" };
        required_php = format!(
            "\nif (!(PHP_VERSION_ID {op} {})) {{\n    $issues[] = 'Your Composer dependencies require a PHP version \"{op} {}\". You are running ' . PHP_VERSION . '.';\n}}\n",
            version_id(&lowest),
            human(&lowest)
        );
    }
    if php_64bit {
        required_php.push_str("\nif (PHP_INT_SIZE !== 8) {\n    $issues[] = 'Your Composer dependencies require a 64-bit build of PHP.';\n}\n");
    }
    let mut required_extensions = String::new();
    if !extensions.is_empty() {
        let lines: String = extensions.into_iter().map(|(_, l)| l).collect();
        required_extensions = format!(
            "\n$missingExtensions = array();\n{lines}\nif ($missingExtensions) {{\n    $issues[] = 'Your Composer dependencies require the following PHP extensions to be installed: ' . implode(', ', $missingExtensions) . '.';\n}}\n"
        );
    }
    if required_php.is_empty() && required_extensions.is_empty() {
        return Ok(None);
    }
    Ok(Some(format!(
        r"<?php

// platform_check.php @generated by Composer

$issues = array();
{required_php}{required_extensions}
if ($issues) {{
    if (!headers_sent()) {{
        header('HTTP/1.1 500 Internal Server Error');
    }}
    if (!ini_get('display_errors')) {{
        if (PHP_SAPI === 'cli' || PHP_SAPI === 'phpdbg') {{
            fwrite(STDERR, 'Composer detected issues in your platform:' . PHP_EOL.PHP_EOL . implode(PHP_EOL, $issues) . PHP_EOL.PHP_EOL);
        }} elseif (!headers_sent()) {{
            echo 'Composer detected issues in your platform:' . PHP_EOL.PHP_EOL . str_replace('You are running '.PHP_VERSION.'.', '', implode(PHP_EOL, $issues)) . PHP_EOL.PHP_EOL;
        }}
    }}
    throw new \RuntimeException(
        'Composer detected issues in your platform: ' . implode(' ', $issues)
    );
}}
"
    )))
}

#[cfg(test)]
mod tests {
    use super::{PlatformRequirements, platform_check};
    use crate::autoloads::Entry;
    use crate::package::{Link, Package};
    use std::collections::BTreeSet;

    fn pkg(name: &str, requires: &[(&str, &str)], provides: &[(&str, &str)]) -> Package {
        let links = |l: &[(&str, &str)]| {
            l.iter()
                .map(|(t, c)| Link {
                    target: (*t).into(),
                    constraint: (*c).into(),
                })
                .collect()
        };
        Package {
            name: name.into(),
            requires: links(requires),
            provides: links(provides),
            ..Package::default()
        }
    }

    fn entries(packages: &[Package]) -> Vec<Entry<'_>> {
        packages
            .iter()
            .map(|p| Entry {
                package: p,
                install_path: Some(String::new()),
                is_root: false,
            })
            .collect()
    }

    #[test]
    fn highest_php_lower_bound_wins() {
        let packages = [
            pkg("a/a", &[("php", "^8.2")], &[]),
            pkg("b/b", &[("php", ">=7.4"), ("ext-json", "*")], &[]),
            pkg("dev/x", &[("php", ">=9")], &[]),
        ];
        let dev: BTreeSet<String> = ["dev/x".to_owned()].into();
        let out = platform_check(
            &entries(&packages),
            false,
            &dev,
            &PlatformRequirements::Check,
        )
        .unwrap()
        .unwrap();
        assert!(out.contains("if (!(PHP_VERSION_ID >= 80200)) {\n    $issues[] = 'Your Composer dependencies require a PHP version \">= 8.2.0\". You are running ' . PHP_VERSION . '.';\n}\n\nif ($issues) {"));
        assert!(!out.contains("missingExtensions"));
    }

    #[test]
    fn nothing_to_check() {
        let packages = [pkg("a/a", &[("psr/log", "^3")], &[])];
        assert!(
            platform_check(
                &entries(&packages),
                true,
                &BTreeSet::new(),
                &PlatformRequirements::Check
            )
            .unwrap()
            .is_none()
        );
        let packages = [pkg("a/a", &[("php", "^8.2")], &[])];
        let ignore = PlatformRequirements::Ignore(vec!["ph*".into()]);
        assert!(
            platform_check(&entries(&packages), false, &BTreeSet::new(), &ignore)
                .unwrap()
                .is_none()
        );
        let upper_only = PlatformRequirements::Ignore(vec!["php+".into()]);
        assert!(
            platform_check(&entries(&packages), false, &BTreeSet::new(), &upper_only)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn extensions_when_platform_check_is_true() {
        let packages = [
            pkg(
                "a/a",
                &[
                    ("php", ">8.1"),
                    ("php-64bit", "*"),
                    ("ext-zend-opcache", "*"),
                    ("ext-pcntl", "*"),
                    ("ext-mbstring", "*"),
                ],
                &[],
            ),
            pkg("p/mb", &[], &[("ext-mbstring", "*")]),
            pkg("a/b", &[("ext-ctype", "*")], &[]),
        ];
        let out = platform_check(
            &entries(&packages),
            true,
            &BTreeSet::new(),
            &PlatformRequirements::Check,
        )
        .unwrap()
        .unwrap();
        assert!(out.contains("PHP_VERSION_ID > 80100"));
        assert!(out.contains("\"> 8.1.0\""));
        assert!(out.contains("PHP_INT_SIZE !== 8"));
        assert!(out.contains("\n$missingExtensions = array();\nextension_loaded('ctype') || $missingExtensions[] = 'ctype';\nPHP_SAPI !== 'cli' || extension_loaded('pcntl') || $missingExtensions[] = 'pcntl';\nextension_loaded('zend opcache') || $missingExtensions[] = 'zend opcache';\n\nif ($missingExtensions)"));
        assert!(!out.contains("'mbstring'"));

        let constrained = [
            pkg("a/a", &[("ext-x", "*")], &[]),
            pkg("p/x", &[], &[("ext-x", "^1")]),
        ];
        assert!(
            platform_check(
                &entries(&constrained),
                true,
                &BTreeSet::new(),
                &PlatformRequirements::Check
            )
            .is_err()
        );
        let bad = [pkg("a/a", &[("php", "~>1")], &[])];
        assert!(
            platform_check(
                &entries(&bad),
                false,
                &BTreeSet::new(),
                &PlatformRequirements::Check
            )
            .is_err()
        );
    }
}
