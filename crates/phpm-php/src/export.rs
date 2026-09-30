use indexmap::IndexMap;
use std::fmt::Write as _;

/// A PHP array key. Numeric strings in canonical form become integers, as
/// they do in PHP.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PhpKey {
    Int(i64),
    String(String),
}

impl From<&str> for PhpKey {
    fn from(s: &str) -> Self {
        match s.parse::<i64>() {
            Ok(i) if i.to_string() == s => Self::Int(i),
            _ => Self::String(s.to_owned()),
        }
    }
}

impl From<String> for PhpKey {
    fn from(s: String) -> Self {
        match Self::from(s.as_str()) {
            Self::Int(i) => Self::Int(i),
            Self::String(_) => Self::String(s),
        }
    }
}

impl From<i64> for PhpKey {
    fn from(i: i64) -> Self {
        Self::Int(i)
    }
}

/// An ordered PHP array.
pub type PhpArray = IndexMap<PhpKey, PhpValue>;

/// The values `FilesystemRepository::dumpToPhpCode` knows how to write.
#[derive(Debug, Clone, PartialEq)]
pub enum PhpValue {
    Null,
    Bool(bool),
    String(String),
    Array(PhpArray),
}

impl From<&str> for PhpValue {
    fn from(s: &str) -> Self {
        Self::String(s.to_owned())
    }
}

impl From<String> for PhpValue {
    fn from(s: String) -> Self {
        Self::String(s)
    }
}

impl From<Option<String>> for PhpValue {
    fn from(s: Option<String>) -> Self {
        s.map_or(Self::Null, Self::String)
    }
}

impl From<bool> for PhpValue {
    fn from(b: bool) -> Self {
        Self::Bool(b)
    }
}

impl From<PhpArray> for PhpValue {
    fn from(a: PhpArray) -> Self {
        Self::Array(a)
    }
}

/// `var_export($s, true)` for a string.
pub fn var_export_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\0' => out.push_str("' . \"\\0\" . '"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// `var_export` of an array key.
fn export_key(key: &PhpKey) -> String {
    match key {
        PhpKey::Int(i) => i.to_string(),
        PhpKey::String(s) => var_export_str(s),
    }
}

// Composer: Util/Filesystem.php isAbsolutePath
pub fn is_absolute_path(path: &str) -> bool {
    path.starts_with('/') || path.as_bytes().get(1) == Some(&b':') || path.starts_with("\\\\")
}

// Composer: Repository/FilesystemRepository.php dumpToPhpCode
pub fn dump_to_php_code(array: &PhpArray) -> String {
    let mut out = String::new();
    dump_level(&mut out, array, 0);
    out
}

fn dump_level(out: &mut String, array: &PhpArray, level: usize) {
    out.push_str("array(\n");
    let level = level + 1;
    for (key, value) in array {
        out.push_str(&"    ".repeat(level));
        out.push_str(&export_key(key));
        out.push_str(" => ");
        match value {
            PhpValue::Array(inner) if inner.is_empty() => out.push_str("array(),\n"),
            PhpValue::Array(inner) => dump_level(out, inner, level),
            PhpValue::String(s) if *key == PhpKey::String("install_path".to_owned()) => {
                if is_absolute_path(s) {
                    let _ = writeln!(out, "{},", var_export_str(s));
                } else {
                    let _ = writeln!(out, "__DIR__ . {},", var_export_str(&format!("/{s}")));
                }
            }
            PhpValue::String(s) => {
                let _ = writeln!(out, "{},", var_export_str(s));
            }
            PhpValue::Bool(b) => out.push_str(if *b { "true,\n" } else { "false,\n" }),
            PhpValue::Null => out.push_str("null,\n"),
        }
    }
    out.push_str(&"    ".repeat(level - 1));
    out.push(')');
    if level > 1 {
        out.push_str(",\n");
    }
}

#[cfg(test)]
mod tests {
    use super::{PhpArray, PhpKey, PhpValue, dump_to_php_code, is_absolute_path, var_export_str};

    fn arr(items: Vec<(&str, PhpValue)>) -> PhpArray {
        items
            .into_iter()
            .map(|(k, v)| (PhpKey::from(k), v))
            .collect()
    }

    #[test]
    fn exports_strings_like_var_export() {
        assert_eq!(var_export_str("abc"), "'abc'");
        assert_eq!(var_export_str("it's"), "'it\\'s'");
        assert_eq!(var_export_str("a\\b"), "'a\\\\b'");
        assert_eq!(var_export_str("a\0b"), "'a' . \"\\0\" . 'b'");
        assert_eq!(var_export_str("line\nbreak é"), "'line\nbreak é'");
        assert_eq!(var_export_str(""), "''");
    }

    #[test]
    fn canonical_numeric_keys_become_ints() {
        assert_eq!(PhpKey::from("0"), PhpKey::Int(0));
        assert_eq!(PhpKey::from("-3"), PhpKey::Int(-3));
        assert_eq!(PhpKey::from("03"), PhpKey::String("03".into()));
        assert_eq!(PhpKey::from("+3"), PhpKey::String("+3".into()));
        assert_eq!(PhpKey::from("-0"), PhpKey::String("-0".into()));
        assert_eq!(
            PhpKey::from("9223372036854775808".to_owned()),
            PhpKey::String("9223372036854775808".into())
        );
        assert_eq!(PhpKey::from(String::from("7")), PhpKey::Int(7));
        assert_eq!(PhpKey::from(5_i64), PhpKey::Int(5));
    }

    #[test]
    fn detects_absolute_paths_like_composer() {
        assert!(is_absolute_path("/usr/lib"));
        assert!(is_absolute_path("C:/x"));
        assert!(is_absolute_path("\\\\server\\share"));
        assert!(!is_absolute_path("../monolog/monolog"));
        assert!(!is_absolute_path(""));
    }

    #[test]
    fn dumps_installed_php_shape() {
        let aliases: PhpArray = [(PhpKey::Int(0), PhpValue::from("9999999-dev"))]
            .into_iter()
            .collect();
        let pkg = arr(vec![
            ("pretty_version", "dev-main".into()),
            ("reference", PhpValue::Null),
            ("install_path", "../a/b".into()),
            ("aliases", aliases.into()),
            ("empty", PhpArray::new().into()),
            ("dev_requirement", true.into()),
            ("other", false.into()),
        ]);
        let root = arr(vec![
            ("install_path", "/abs/it's".into()),
            ("versions", arr(vec![("a/b", pkg.into())]).into()),
        ]);
        let expected = "array(
    'install_path' => '/abs/it\\'s',
    'versions' => array(
        'a/b' => array(
            'pretty_version' => 'dev-main',
            'reference' => null,
            'install_path' => __DIR__ . '/../a/b',
            'aliases' => array(
                0 => '9999999-dev',
            ),
            'empty' => array(),
            'dev_requirement' => true,
            'other' => false,
        ),
    ),
)";
        assert_eq!(dump_to_php_code(&root), expected);
    }

    #[test]
    fn empty_top_level_array() {
        assert_eq!(dump_to_php_code(&PhpArray::new()), "array(\n)");
    }

    #[test]
    fn value_conversions() {
        assert_eq!(PhpValue::from(None::<String>), PhpValue::Null);
        assert_eq!(
            PhpValue::from(Some("x".to_owned())),
            PhpValue::String("x".into())
        );
        assert_eq!(PhpValue::from("x".to_owned()), PhpValue::String("x".into()));
    }
}
