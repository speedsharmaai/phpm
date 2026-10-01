//! composer/semver's constraints: parsing, matching and bounds.

use crate::version::normalize;
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
pub fn version_compare(a: &str, b: &str) -> Ordering {
    php_version_compare(a.as_bytes(), b.as_bytes()).cmp(&0)
}

/// A bound as composer/semver's `Bound` holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bound {
    pub version: String,
    pub inclusive: bool,
}

impl Bound {
    pub fn zero() -> Self {
        Self {
            version: "0.0.0.0-dev".to_owned(),
            inclusive: true,
        }
    }

    pub fn is_zero(&self) -> bool {
        self.version == "0.0.0.0-dev" && self.inclusive
    }

    // composer/semver: Constraint/Bound.php compareTo
    pub fn is_higher_than(&self, other: &Self) -> bool {
        if self == other {
            return false;
        }
        match version_compare(&self.version, &other.version) {
            Ordering::Greater => true,
            Ordering::Less => false,
            Ordering::Equal => other.inclusive,
        }
    }

    pub fn is_lower_than(&self, other: &Self) -> bool {
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

/// A comparison operator in a constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Op {
    fn parse(op: &str) -> Self {
        match op {
            "<>" | "!=" => Self::Ne,
            "<" => Self::Lt,
            "<=" => Self::Le,
            ">" => Self::Gt,
            ">=" => Self::Ge,
            _ => Self::Eq,
        }
    }

    fn without_equal(self) -> Self {
        match self {
            Self::Le => Self::Lt,
            Self::Ge => Self::Gt,
            other => other,
        }
    }

    fn holds(self, ord: Ordering) -> bool {
        match self {
            Self::Eq => ord == Ordering::Equal,
            Self::Ne => ord != Ordering::Equal,
            Self::Lt => ord == Ordering::Less,
            Self::Le => ord != Ordering::Greater,
            Self::Gt => ord == Ordering::Greater,
            Self::Ge => ord != Ordering::Less,
        }
    }
}

/// A parsed constraint, shaped like composer/semver's object tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Constraint {
    Any,
    Single { op: Op, version: String },
    Multi { conjunctive: bool, items: Vec<Self> },
}

fn is_branch(version: &str) -> bool {
    version.starts_with("dev-")
}

// composer/semver: Constraint/Constraint.php versionCompare
fn compare_versions(a: &str, b: &str, op: Op) -> bool {
    let (a_branch, b_branch) = (is_branch(a), is_branch(b));
    if op == Op::Ne && (a_branch || b_branch) {
        return a != b;
    }
    if a_branch && b_branch {
        return op == Op::Eq && a == b;
    }
    if a_branch || b_branch {
        return false;
    }
    op.holds(version_compare(a, b))
}

// composer/semver: Constraint/Constraint.php matchSpecific
fn match_specific(op: Op, version: &str, p_op: Op, p_version: &str) -> bool {
    if op == Op::Ne || p_op == Op::Ne {
        if op == Op::Ne && p_op != Op::Ne && p_op != Op::Eq && is_branch(p_version) {
            return false;
        }
        if p_op == Op::Ne && op != Op::Ne && op != Op::Eq && is_branch(version) {
            return false;
        }
        if op != Op::Eq && p_op != Op::Eq {
            return true;
        }
        return compare_versions(p_version, version, Op::Ne);
    }
    if op != Op::Eq && op.without_equal() == p_op.without_equal() {
        return !(is_branch(version) || is_branch(p_version));
    }
    let (v1, v2, cmp) = if op == Op::Eq {
        (version, p_version, p_op)
    } else {
        (p_version, version, op)
    };
    if compare_versions(v1, v2, cmp) {
        let strict = |o: Op| matches!(o, Op::Lt | Op::Gt);
        return !(strict(p_op)
            && !strict(op)
            && version_compare(p_version, version) == Ordering::Equal);
    }
    false
}

impl Constraint {
    /// `$this->matches($provider)`.
    pub fn matches(&self, provider: &Self) -> bool {
        match (self, provider) {
            (Self::Any, _) | (_, Self::Any) => true,
            (
                Self::Multi {
                    conjunctive: false,
                    items,
                },
                _,
            ) => items.iter().any(|c| c.matches(provider)),
            (
                Self::Multi {
                    conjunctive: true, ..
                },
                Self::Multi {
                    conjunctive: false, ..
                },
            )
            | (Self::Single { .. }, Self::Multi { .. }) => provider.matches(self),
            (
                Self::Multi {
                    conjunctive: true,
                    items,
                },
                _,
            ) => items.iter().all(|c| provider.matches(c)),
            (
                Self::Single { op, version },
                Self::Single {
                    op: p_op,
                    version: p_version,
                },
            ) => match_specific(*op, version, *p_op, p_version),
        }
    }

    /// Whether the normalized `version` satisfies this constraint.
    pub fn matches_version(&self, version: &str) -> bool {
        self.matches(&Self::Single {
            op: Op::Eq,
            version: version.to_owned(),
        })
    }

    /// `getLowerBound()` and `getUpperBound()`; `None` is positive infinity.
    // composer/semver: Constraint/Constraint.php, MultiConstraint.php extractBounds
    pub fn bounds(&self) -> (Bound, Option<Bound>) {
        match self {
            Self::Any => (Bound::zero(), None),
            Self::Single { op, version } => {
                if is_branch(version) {
                    return (Bound::zero(), None);
                }
                let at = |inclusive| Bound {
                    version: version.clone(),
                    inclusive,
                };
                match op {
                    Op::Eq => (at(true), Some(at(true))),
                    Op::Lt => (Bound::zero(), Some(at(false))),
                    Op::Le => (Bound::zero(), Some(at(true))),
                    Op::Gt => (at(false), None),
                    Op::Ge => (at(true), None),
                    Op::Ne => (Bound::zero(), None),
                }
            }
            Self::Multi { conjunctive, items } => {
                let mut result: Option<(Bound, Option<Bound>)> = None;
                for item in items {
                    let (lower, upper) = item.bounds();
                    let Some((low, high)) = &mut result else {
                        result = Some((lower, upper));
                        continue;
                    };
                    let lower_wins = if *conjunctive {
                        lower.is_higher_than(low)
                    } else {
                        lower.is_lower_than(low)
                    };
                    if lower_wins {
                        *low = lower;
                    }
                    let upper_wins = match (&upper, &*high) {
                        (None, None) => false,
                        (None, Some(_)) => !*conjunctive,
                        (Some(_), None) => *conjunctive,
                        (Some(u), Some(h)) => {
                            if *conjunctive {
                                u.is_lower_than(h)
                            } else {
                                u.is_higher_than(h)
                            }
                        }
                    };
                    if upper_wins {
                        *high = upper;
                    }
                }
                result.unwrap_or_else(|| (Bound::zero(), None))
            }
        }
    }

    /// `IgnoreListPlatformRequirementFilter::filterConstraint` for `name+`:
    /// anything at or above the constraint's upper end also matches.
    #[must_use]
    pub fn without_upper_bound(self) -> Self {
        match self.bounds().1 {
            Some(end) => Self::Multi {
                conjunctive: false,
                items: vec![
                    self,
                    Self::Single {
                        op: Op::Ge,
                        version: end.version,
                    },
                ],
            },
            None => self,
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

// composer/semver: VersionParser.php manipulateVersionString
fn manipulate(parts: [&str; 4], position: usize, increment: i64) -> Option<String> {
    let mut out: Vec<String> = parts.iter().map(|p| (*p).to_owned()).collect();
    let mut position = position;
    for i in (1..=4).rev() {
        if i > position {
            "0".clone_into(&mut out[i - 1]);
        } else if i == position && increment != 0 {
            let value = php_intval(&out[i - 1]) + increment;
            if value < 0 {
                "0".clone_into(&mut out[i - 1]);
                position -= 1;
                if i == 1 {
                    return None;
                }
            } else {
                out[i - 1] = value.to_string();
            }
        }
    }
    Some(out.join("."))
}

fn php_intval(s: &str) -> i64 {
    s.bytes()
        .take_while(u8::is_ascii_digit)
        .fold(0_i64, |n, c| {
            n.saturating_mul(10).saturating_add(i64::from(c - b'0'))
        })
}

fn range(low: String, high: Option<String>) -> Vec<Constraint> {
    let mut out = vec![Constraint::Single {
        op: Op::Ge,
        version: low,
    }];
    out.extend(high.map(|version| Constraint::Single {
        op: Op::Lt,
        version,
    }));
    out
}

fn high(parts: [&str; 4], position: usize) -> Option<String> {
    manipulate(parts, position, 1).map(|v| format!("{v}-dev"))
}

fn tilde_or_caret(
    constraint: &str,
    caps: &Captures<'_>,
    tilde: bool,
) -> Result<Vec<Constraint>, String> {
    let parts = [
        group(caps, 1),
        group(caps, 2),
        group(caps, 3),
        group(caps, 4),
    ];
    let parts = [
        parts[0].as_str(),
        parts[1].as_str(),
        parts[2].as_str(),
        parts[3].as_str(),
    ];
    let position = if tilde {
        let p = (1..=4).rev().find(|&i| non_empty(caps, i)).unwrap_or(1);
        let p = if non_empty(caps, 8) { p + 1 } else { p };
        (p - 1).max(1)
    } else if parts[0] != "0" || parts[1].is_empty() {
        1
    } else if parts[1] != "0" || parts[2].is_empty() {
        2
    } else {
        3
    };
    let suffix = if non_empty(caps, 5) || non_empty(caps, 7) || non_empty(caps, 8) {
        ""
    } else {
        "-dev"
    };
    let low = normalized(&format!("{}{suffix}", &constraint[1..]))?;
    Ok(range(low, high(parts, position)))
}

// composer/semver: VersionParser.php parseConstraint
fn parse_single(input: &str) -> Result<Vec<Constraint>, String> {
    let mut constraint = input.to_owned();
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
    if let Some(caps) = ANY.captures(constraint.as_bytes()) {
        if non_empty(&caps, 1) || non_empty(&caps, 2) {
            return Ok(vec![Constraint::Single {
                op: Op::Ge,
                version: "0.0.0.0-dev".to_owned(),
            }]);
        }
        return Ok(vec![Constraint::Any]);
    }
    if let Some(caps) = TILDE.captures(constraint.as_bytes()) {
        if constraint.starts_with("~>") {
            return Err(format!(
                "Could not parse version constraint {constraint}: Invalid operator \"~>\", you probably meant to use the \"~\" operator"
            ));
        }
        return tilde_or_caret(&constraint, &caps, true);
    }
    if let Some(caps) = CARET.captures(constraint.as_bytes()) {
        return tilde_or_caret(&constraint, &caps, false);
    }
    if let Some(caps) = WILDCARD.captures(constraint.as_bytes()) {
        let position = (1..=3).rev().find(|&i| non_empty(&caps, i)).unwrap_or(1);
        let parts = [group(&caps, 1), group(&caps, 2), group(&caps, 3)];
        let parts = [parts[0].as_str(), parts[1].as_str(), parts[2].as_str(), ""];
        let low = format!("{}-dev", manipulate(parts, position, 0).unwrap_or_default());
        let high = high(parts, position);
        if low == "0.0.0.0-dev" {
            return Ok(high
                .map(|version| Constraint::Single {
                    op: Op::Lt,
                    version,
                })
                .into_iter()
                .collect());
        }
        return Ok(range(low, high));
    }
    if let Some(caps) = HYPHEN.captures(constraint.as_bytes()) {
        let low_suffix = if non_empty(&caps, 6) || non_empty(&caps, 8) || non_empty(&caps, 9) {
            ""
        } else {
            "-dev"
        };
        let low = format!("{}{low_suffix}", normalized(&group(&caps, 1))?);
        let to = group(&caps, 10);
        let filled = |i: usize| non_empty(&caps, i);
        let upper = if (filled(12) && filled(13)) || filled(15) || filled(17) || filled(18) {
            Constraint::Single {
                op: Op::Le,
                version: normalized(&to)?,
            }
        } else {
            normalized(&to)?;
            let parts = [
                group(&caps, 11),
                group(&caps, 12),
                group(&caps, 13),
                group(&caps, 14),
            ];
            let parts = [
                parts[0].as_str(),
                parts[1].as_str(),
                parts[2].as_str(),
                parts[3].as_str(),
            ];
            let position = if filled(12) { 2 } else { 1 };
            Constraint::Single {
                op: Op::Lt,
                version: high(parts, position).unwrap_or_default(),
            }
        };
        return Ok(vec![
            Constraint::Single {
                op: Op::Ge,
                version: low,
            },
            upper,
        ]);
    }
    if let Some(caps) = BASIC.captures(constraint.as_bytes()) {
        let raw = group(&caps, 2);
        let mut version = match normalize(&raw) {
            Ok(v) => v,
            Err(e) => {
                if raw.ends_with("-dev") && X_DEV_NAME.is_match(raw.as_bytes()) {
                    normalized(&format!("dev-{}", &raw[..raw.len() - 4]))?
                } else {
                    return Err(format!(
                        "Could not parse version constraint {constraint}: {e}"
                    ));
                }
            }
        };
        let op_text = group(&caps, 1);
        let op = Op::parse(&op_text);
        if op != Op::Eq
            && is_stable(&version)
            && let Some(modifier) = &stability_modifier
        {
            version = format!("{version}-{modifier}");
        } else if (op == Op::Lt || op == Op::Ge)
            && !MODIFIER_END.is_match(raw.to_ascii_lowercase().as_bytes())
            && !raw.starts_with("dev-")
        {
            version.push_str("-dev");
        }
        return Ok(vec![Constraint::Single { op, version }]);
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

/// `VersionParser::parseConstraints`.
pub fn parse(constraints: &str) -> Result<Constraint, String> {
    let mut groups = Vec::new();
    for or in OR_SPLIT.split(constraints.trim().as_bytes()) {
        let or = String::from_utf8_lossy(or);
        let mut items = Vec::new();
        for and in split_and(&or) {
            items.extend(parse_single(and)?);
        }
        groups.push(if items.len() == 1 {
            items.swap_remove(0)
        } else {
            Constraint::Multi {
                conjunctive: true,
                items,
            }
        });
    }
    Ok(if groups.len() == 1 {
        groups.swap_remove(0)
    } else {
        Constraint::Multi {
            conjunctive: false,
            items: groups,
        }
    })
}

/// The lower bound of a constraint as `parseConstraints($c)->getLowerBound()`
/// returns it.
pub fn lower_bound(constraints: &str) -> Result<Bound, String> {
    Ok(parse(constraints)?.bounds().0)
}

#[cfg(test)]
mod tests {
    use super::{Bound, Constraint, Op, lower_bound, parse, split_and, version_compare};
    use crate::version::normalize;
    use serde_json::Value;
    use std::cmp::Ordering;

    const CASES: &str = include_str!("../tests/data/semver-constraints.jsonl");

    fn recorded(v: &Value) -> Option<Bound> {
        v.as_array().map(|b| Bound {
            version: b[0].as_str().unwrap().to_owned(),
            inclusive: b[1].as_bool().unwrap(),
        })
    }

    #[test]
    fn matches_composer_semver_on_every_recorded_case() {
        let rows: Vec<Vec<Value>> = CASES
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let versions: Vec<String> = rows.iter().find(|r| r[0] == "#versions").unwrap()[1]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                let v = v.as_str().unwrap();
                if v.starts_with("dev-") {
                    v.to_owned()
                } else {
                    normalize(v).unwrap()
                }
            })
            .collect();
        let mut checked = 0;
        for row in &rows {
            let first = row[0].as_str().unwrap();
            if first == "#pair" {
                let a = parse(row[1].as_str().unwrap()).unwrap();
                let b = parse(row[2].as_str().unwrap()).unwrap();
                assert_eq!(a.matches(&b), row[3].as_bool().unwrap(), "{row:?}");
                checked += 1;
                continue;
            }
            if first == "#versions" {
                continue;
            }
            let parsed = parse(first).unwrap();
            let (low, high) = parsed.bounds();
            assert_eq!(Some(low), recorded(&row[1]), "lower bound of {first}");
            assert_eq!(high, recorded(&row[2]), "upper bound of {first}");
            let expected = row[3].as_str().unwrap().as_bytes();
            for (v, want) in versions.iter().zip(expected) {
                assert_eq!(
                    parsed.matches_version(v),
                    *want == b'1',
                    "{first} matches {v}"
                );
                checked += 1;
            }
        }
        assert!(checked > 2500, "{checked}");
    }

    #[test]
    fn drops_the_upper_bound_for_plus_requirements() {
        let c = parse("^7.4 || ^8.0").unwrap().without_upper_bound();
        assert!(c.matches_version("9.1.0.0"));
        assert!(c.matches_version("8.1.0.0"));
        assert!(!c.matches_version("7.3.0.0"));
        let open = parse(">=8.1").unwrap();
        assert_eq!(open.clone().without_upper_bound(), open);
        assert!(Constraint::Any.matches_version("1.0.0.0"));
        let ne = Constraint::Single {
            op: Op::Ne,
            version: "dev-x".into(),
        };
        assert!(ne.matches_version("dev-y"));
        assert!(!ne.matches(&Constraint::Single {
            op: Op::Ge,
            version: "dev-y".into()
        }));
    }

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
