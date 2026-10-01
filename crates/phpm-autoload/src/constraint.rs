//! The lower bound of a version constraint, which is all `platform_check.php`
//! needs from composer/semver.

use phpm_lock::version::normalize;
use regex::bytes::{Captures, Regex, RegexBuilder};
use std::cmp::Ordering;
use std::sync::LazyLock;

fn is_digit(c: u8) -> bool {
    c.is_ascii_digit()
}

fn is_non_digit(c: u8) -> bool {
    !c.is_ascii_digit() && c != b'.'
}

// php-src: ext/standard/versioning.c php_canonicalize_version
fn canonicalize(version: &[u8]) -> Vec<u8> {
    let Some((&first, rest)) = version.split_first() else {
        return Vec::new();
    };
    let mut out = vec![first];
    let mut last = first;
    for &c in rest {
        let prev = out.last().copied().unwrap_or(0);
        if matches!(c, b'-' | b'_' | b'+') {
            if prev != b'.' {
                out.push(b'.');
            }
        } else if (is_non_digit(last) && is_digit(c)) || (is_digit(last) && is_non_digit(c)) {
            if prev != b'.' {
                out.push(b'.');
            }
            out.push(c);
        } else if !c.is_ascii_alphanumeric() {
            if prev != b'.' {
                out.push(b'.');
            }
        } else {
            out.push(c);
        }
        last = c;
    }
    out
}

// php-src: ext/standard/versioning.c compare_special_version_forms
fn special_form_order(form: &[u8]) -> i32 {
    const FORMS: [(&[u8], i32); 10] = [
        (b"dev", 0),
        (b"alpha", 1),
        (b"a", 1),
        (b"beta", 2),
        (b"b", 2),
        (b"RC", 3),
        (b"rc", 3),
        (b"#", 4),
        (b"pl", 5),
        (b"p", 5),
    ];
    FORMS
        .iter()
        .find(|(name, _)| form.starts_with(name))
        .map_or(-1, |&(_, order)| order)
}

fn compare_forms(a: &[u8], b: &[u8]) -> i32 {
    (special_form_order(a) - special_form_order(b)).signum()
}

fn strtol(digits: &[u8]) -> i64 {
    digits
        .iter()
        .take_while(|c| c.is_ascii_digit())
        .fold(0_i64, |n, c| {
            n.saturating_mul(10).saturating_add(i64::from(c - b'0'))
        })
}

fn at(v: &[u8], i: usize) -> u8 {
    v.get(i).copied().unwrap_or(0)
}

// php-src: ext/standard/versioning.c php_version_compare
fn php_version_compare(a: &[u8], b: &[u8]) -> i32 {
    if a.is_empty() || b.is_empty() {
        return i32::from(!a.is_empty()) - i32::from(!b.is_empty());
    }
    let v1 = if a[0] == b'#' {
        a.to_vec()
    } else {
        canonicalize(a)
    };
    let v2 = if b[0] == b'#' {
        b.to_vec()
    } else {
        canonicalize(b)
    };
    let (mut p1, mut p2) = (0, 0);
    let (mut n1, mut n2) = (Some(0), Some(0));
    let mut compare = 0;
    while at(&v1, p1) != 0 && at(&v2, p2) != 0 && n1.is_some() && n2.is_some() {
        n1 = v1[p1..].iter().position(|&c| c == b'.').map(|i| p1 + i);
        n2 = v2[p2..].iter().position(|&c| c == b'.').map(|i| p2 + i);
        let e1 = &v1[p1..n1.unwrap_or(v1.len())];
        let e2 = &v2[p2..n2.unwrap_or(v2.len())];
        compare = match (is_digit(at(e1, 0)), is_digit(at(e2, 0))) {
            (true, true) => match strtol(e1).cmp(&strtol(e2)) {
                Ordering::Less => -1,
                Ordering::Equal => 0,
                Ordering::Greater => 1,
            },
            (false, false) => compare_forms(e1, e2),
            (true, false) => compare_forms(b"#N#", e2),
            (false, true) => compare_forms(e1, b"#N#"),
        };
        if compare != 0 {
            break;
        }
        if let Some(n) = n1 {
            p1 = n + 1;
        }
        if let Some(n) = n2 {
            p2 = n + 1;
        }
    }
    if compare == 0 {
        if n1.is_some() {
            compare = if is_digit(at(&v1, p1)) {
                1
            } else {
                php_version_compare(&v1[p1.min(v1.len())..], b"#N#")
            };
        } else if n2.is_some() {
            compare = if is_digit(at(&v2, p2)) {
                -1
            } else {
                php_version_compare(b"#N#", &v2[p2.min(v2.len())..])
            };
        }
    }
    compare
}

/// PHP `version_compare($a, $b)`.
pub(crate) fn version_compare(a: &str, b: &str) -> Ordering {
    php_version_compare(a.as_bytes(), b.as_bytes()).cmp(&0)
}

/// A lower bound as composer/semver's `Bound` holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Bound {
    pub(crate) version: String,
    pub(crate) inclusive: bool,
}

impl Bound {
    pub(crate) fn zero() -> Self {
        Self {
            version: "0.0.0.0-dev".to_owned(),
            inclusive: true,
        }
    }

    pub(crate) fn is_zero(&self) -> bool {
        self.version == "0.0.0.0-dev" && self.inclusive
    }

    // composer/semver: Constraint/Bound.php compareTo
    pub(crate) fn is_higher_than(&self, other: &Self) -> bool {
        if self == other {
            return false;
        }
        match version_compare(&self.version, &other.version) {
            Ordering::Greater => true,
            Ordering::Less => false,
            Ordering::Equal => other.inclusive,
        }
    }

    fn is_lower_than(&self, other: &Self) -> bool {
        if self == other {
            return false;
        }
        match version_compare(&self.version, &other.version) {
            Ordering::Less => true,
            Ordering::Greater => false,
            Ordering::Equal => !other.inclusive,
        }
    }
}

fn re(pattern: &str) -> Regex {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .unicode(false)
        .build()
        .expect("valid pattern")
}

const MODIFIER: &str =
    r"[._-]?(?:(stable|beta|b|RC|alpha|a|patch|pl|p)((?:[.-]?\d+)*)?)?([.-]?dev)?";

fn version_regex() -> String {
    format!(
        r"v?(\d+)(?:\.(\d+))?(?:\.(\d+))?(?:\.(\d+))?(?:{MODIFIER}|\.([xX*][.-]?dev))(?:\+[^\s]+)?"
    )
}

static OR_SPLIT: LazyLock<Regex> = LazyLock::new(|| re(r"\s*\|\|?\s*"));
static AS_ALIAS: LazyLock<Regex> = LazyLock::new(|| re(r"^([^,\s]+) +as +([^,\s]+)$"));
static STABILITY_FLAG: LazyLock<Regex> =
    LazyLock::new(|| re(r"^([^,\s]*?)@(stable|RC|beta|alpha|dev)$"));
static REFERENCE: LazyLock<Regex> = LazyLock::new(|| re(r"^(dev-[^,\s@]+?|[^,\s@]+?\.x-dev)#.+$"));
static ANY: LazyLock<Regex> = LazyLock::new(|| re(r"^(v)?[xX*](\.[xX*])*$"));
static TILDE: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"^~>?{}$", version_regex())));
static CARET: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"^\^{}$", version_regex())));
static WILDCARD: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"^v?(\d+)(?:\.(\d+))?(?:\.(\d+))?(?:\.[xX*])+$")
        .unicode(false)
        .build()
        .expect("valid pattern")
});
static HYPHEN: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"^(?P<from>{}) +- +(?P<to>{})$",
        version_regex(),
        version_regex()
    ))
});
static BASIC: LazyLock<Regex> = LazyLock::new(|| re(r"^(<>|!=|>=?|<=?|==?)?\s*(.*)"));
static MODIFIER_END: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"-{MODIFIER}$")));
static STABILITY: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"{MODIFIER}(?:\+.*)?$")));
static X_DEV_NAME: LazyLock<Regex> = LazyLock::new(|| re(r"^[0-9a-zA-Z\-./]+$"));

fn group(caps: &Captures<'_>, i: usize) -> String {
    caps.get(i)
        .map(|m| String::from_utf8_lossy(m.as_bytes()).into_owned())
        .unwrap_or_default()
}

fn non_empty(caps: &Captures<'_>, i: usize) -> bool {
    caps.get(i).is_some_and(|m| !m.as_bytes().is_empty())
}

// composer/semver: VersionParser.php parseStability
fn is_stable(version: &str) -> bool {
    let version = version.split_once('#').map_or(version, |(v, _)| v);
    if version.starts_with("dev-") || version.ends_with("-dev") {
        return false;
    }
    let lower = version.to_ascii_lowercase();
    let Some(caps) = STABILITY.captures(lower.as_bytes()) else {
        return true;
    };
    if non_empty(&caps, 3) {
        return false;
    }
    !matches!(
        group(&caps, 1).as_str(),
        "beta" | "b" | "alpha" | "a" | "rc"
    )
}

fn normalized(version: &str) -> Result<String, String> {
    normalize(version).map_err(|e| e.to_string())
}

fn manipulate(parts: [&str; 4], position: usize) -> String {
    let mut out: Vec<&str> = parts.to_vec();
    for (i, part) in out.iter_mut().enumerate() {
        if i + 1 > position {
            *part = "0";
        }
    }
    out.join(".")
}

// composer/semver: VersionParser.php parseConstraint (lower bound only)
fn single_lower_bound(constraint: &str) -> Result<Bound, String> {
    let mut constraint = constraint.to_owned();
    let mut stability_modifier: Option<String> = None;
    if let Some(caps) = AS_ALIAS.captures(constraint.as_bytes()) {
        constraint = group(&caps, 1);
    }
    if let Some(caps) = STABILITY_FLAG.captures(constraint.as_bytes()) {
        let modifier = group(&caps, 2);
        let base = group(&caps, 1);
        constraint = if base.is_empty() {
            "*".to_owned()
        } else {
            base
        };
        if modifier != "stable" {
            stability_modifier = Some(modifier);
        }
    }
    if let Some(caps) = REFERENCE.captures(constraint.as_bytes()) {
        constraint = group(&caps, 1);
    }
    if ANY.is_match(constraint.as_bytes()) {
        return Ok(Bound::zero());
    }
    for regex in [&*TILDE, &*CARET] {
        if let Some(caps) = regex.captures(constraint.as_bytes()) {
            if constraint.starts_with("~>") {
                return Err(format!("Invalid operator \"~>\" in {constraint}"));
            }
            let suffix = if non_empty(&caps, 5) || non_empty(&caps, 7) || non_empty(&caps, 8) {
                ""
            } else {
                "-dev"
            };
            let version = normalized(&format!("{}{suffix}", &constraint[1..]))?;
            return Ok(Bound {
                version,
                inclusive: true,
            });
        }
    }
    if let Some(caps) = WILDCARD.captures(constraint.as_bytes()) {
        let position = if non_empty(&caps, 3) {
            3
        } else if non_empty(&caps, 2) {
            2
        } else {
            1
        };
        let parts = [group(&caps, 1), group(&caps, 2), group(&caps, 3)];
        let low = format!(
            "{}-dev",
            manipulate([&parts[0], &parts[1], &parts[2], ""], position)
        );
        if low == "0.0.0.0-dev" {
            return Ok(Bound::zero());
        }
        return Ok(Bound {
            version: low,
            inclusive: true,
        });
    }
    if let Some(caps) = HYPHEN.captures(constraint.as_bytes()) {
        let suffix = if non_empty(&caps, 6) || non_empty(&caps, 8) || non_empty(&caps, 9) {
            ""
        } else {
            "-dev"
        };
        let version = format!(
            "{}{suffix}",
            normalized(&String::from_utf8_lossy(&caps["from"]))?
        );
        return Ok(Bound {
            version,
            inclusive: true,
        });
    }
    if let Some(caps) = BASIC.captures(constraint.as_bytes()) {
        let raw = group(&caps, 2);
        let raw = raw.as_str();
        let mut version = match normalize(raw) {
            Ok(v) => v,
            Err(e) => {
                if raw.ends_with("-dev") && X_DEV_NAME.is_match(raw.as_bytes()) {
                    normalized(&format!("dev-{}", &raw[..raw.len() - 4]))?
                } else {
                    return Err(e.to_string());
                }
            }
        };
        let op = group(&caps, 1);
        let op = if op.is_empty() { "=" } else { op.as_str() };
        if op != "==" && op != "=" && stability_modifier.is_some() && is_stable(&version) {
            version = format!("{version}-{}", stability_modifier.unwrap_or_default());
        } else if (op == "<" || op == ">=")
            && !MODIFIER_END.is_match(raw.to_ascii_lowercase().as_bytes())
            && !raw.starts_with("dev-")
        {
            version.push_str("-dev");
        }
        if version.starts_with("dev-") {
            return Ok(Bound::zero());
        }
        return Ok(match op {
            "=" | "==" | ">=" => Bound {
                version,
                inclusive: true,
            },
            ">" => Bound {
                version,
                inclusive: false,
            },
            _ => Bound::zero(),
        });
    }
    Err(format!("Could not parse version constraint {constraint}"))
}

/// `preg_split('{(?<!^|as|[=>< ,]) *(?<!-)[, ](?!-) *(?!,|as|$)}', $s)`.
fn split_and(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let spaces_from = |i: usize| b[i..].iter().take_while(|&&c| c == b' ').count();
    let mut parts = Vec::new();
    let mut last = 0;
    let mut start = 1;
    'scan: while start < b.len() {
        let blocked = matches!(b[start - 1], b'=' | b'>' | b'<' | b' ' | b',')
            || (start >= 2 && &b[start - 2..start] == b"as");
        if !blocked {
            for lead in (0..=spaces_from(start)).rev() {
                let sep = start + lead;
                if sep >= b.len() || !matches!(b[sep], b',' | b' ') || b[sep - 1] == b'-' {
                    continue;
                }
                if b.get(sep + 1) == Some(&b'-') {
                    continue;
                }
                for trail in (0..=spaces_from(sep + 1)).rev() {
                    let end = sep + 1 + trail;
                    if end < b.len() && b[end] != b',' && !b[end..].starts_with(b"as") {
                        parts.push(&s[last..start]);
                        last = end;
                        start = end.max(start + 1);
                        continue 'scan;
                    }
                }
            }
        }
        start += 1;
    }
    parts.push(&s[last..]);
    parts
}

/// The lower bound of a constraint as `parseConstraints($c)->getLowerBound()`
/// returns it.
// composer/semver: VersionParser.php parseConstraints, Constraint/MultiConstraint.php extractBounds
pub(crate) fn lower_bound(constraints: &str) -> Result<Bound, String> {
    let mut or_bound: Option<Bound> = None;
    for or in OR_SPLIT.split(constraints.trim().as_bytes()) {
        let or = String::from_utf8_lossy(or);
        let mut and_bound: Option<Bound> = None;
        for and in split_and(&or) {
            let bound = single_lower_bound(and)?;
            match &and_bound {
                Some(current) if !bound.is_higher_than(current) => {}
                _ => and_bound = Some(bound),
            }
        }
        let bound = and_bound.unwrap_or_else(Bound::zero);
        match &or_bound {
            Some(current) if !bound.is_lower_than(current) => {}
            _ => or_bound = Some(bound),
        }
    }
    Ok(or_bound.unwrap_or_else(Bound::zero))
}

#[cfg(test)]
mod tests {
    use super::{Bound, lower_bound, split_and, version_compare};
    use std::cmp::Ordering;

    #[test]
    fn compares_versions_like_php() {
        let cases = [
            ("1.0", "1.0.0", Ordering::Less),
            ("8.2.0.0-dev", "8.2.0.0", Ordering::Less),
            ("8.2.0.0", "8.1.99.0", Ordering::Greater),
            ("1.0-alpha", "1.0-beta", Ordering::Less),
            ("1.0rc1", "1.0", Ordering::Less),
            ("1.0pl1", "1.0", Ordering::Greater),
            ("1.0.", "1.0.0", Ordering::Less),
            ("", "1", Ordering::Less),
            ("1", "", Ordering::Greater),
            ("", "", Ordering::Equal),
            ("1.0-dev", "1.0-dev", Ordering::Equal),
            ("10", "9", Ordering::Greater),
            ("1.x", "1.0", Ordering::Less),
            ("#a", "#b", Ordering::Equal),
        ];
        for (a, b, expected) in cases {
            assert_eq!(version_compare(a, b), expected, "{a} vs {b}");
        }
    }

    fn bound(c: &str) -> (String, bool) {
        let b = lower_bound(c).unwrap();
        (b.version, b.inclusive)
    }

    #[test]
    fn lower_bounds_like_semver() {
        let cases = [
            ("^8.2", "8.2.0.0-dev", true),
            (">=8.1", "8.1.0.0-dev", true),
            (">=7.2.5", "7.2.5.0-dev", true),
            (">8.1", "8.1.0.0", false),
            ("^7.4|^8.0", "7.4.0.0-dev", true),
            ("^8.0 || ^7.4", "7.4.0.0-dev", true),
            (">=8.1 <8.5", "8.1.0.0-dev", true),
            (">=7.1,<9", "7.1.0.0-dev", true),
            ("~8.2.0", "8.2.0.0-dev", true),
            ("8.1.*", "8.1.0.0-dev", true),
            ("8.*", "8.0.0.0-dev", true),
            ("8.2.1", "8.2.1.0", true),
            ("==8.2.1", "8.2.1.0", true),
            ("7.4 - 8.2", "7.4.0.0-dev", true),
            ("*", "0.0.0.0-dev", true),
            ("<8", "0.0.0.0-dev", true),
            ("!=8.0", "0.0.0.0-dev", true),
            ("0.*", "0.0.0.0-dev", true),
            ("dev-main", "0.0.0.0-dev", true),
            (">=8.1@dev", "8.1.0.0-dev", true),
            (">=8.1-beta", "8.1.0.0-beta", true),
            ("1.0 as 2.0", "1.0.0.0", true),
            ("1.x-dev#abc", "1.9999999.9999999.9999999-dev", true),
            (">=8.0-dev", "8.0.0.0-dev", true),
        ];
        for (constraint, version, inclusive) in cases {
            assert_eq!(
                bound(constraint),
                (version.to_owned(), inclusive),
                "{constraint}"
            );
        }
        assert!(lower_bound("~>1.0").is_err());
        assert!(lower_bound("nope nope").is_err());
        assert!(Bound::zero().is_zero());
    }

    #[test]
    fn bounds_compare_like_semver() {
        let a = Bound {
            version: "8.1.0.0".into(),
            inclusive: false,
        };
        let b = Bound {
            version: "8.1.0.0".into(),
            inclusive: true,
        };
        assert!(a.is_higher_than(&b));
        assert!(!b.is_higher_than(&a));
        assert!(!a.is_higher_than(&a));
    }

    #[test]
    fn splits_and_groups_like_composer() {
        assert_eq!(split_and(">=1.0 <2.0"), [">=1.0", "<2.0"]);
        assert_eq!(split_and(">=1.0,<2.0"), [">=1.0", "<2.0"]);
        assert_eq!(split_and(">= 1.0"), [">= 1.0"]);
        assert_eq!(split_and("1.0 - 2.0"), ["1.0 - 2.0"]);
        assert_eq!(split_and("1.0 as 2.0"), ["1.0 as 2.0"]);
        assert_eq!(split_and("^1.0"), ["^1.0"]);
        assert_eq!(split_and("a  b"), ["a", "b"]);
    }
}
