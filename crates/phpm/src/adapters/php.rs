//! The PHP string functions plugins use on package names, ASCII-only as PHP
//! 8 runs them.

pub(crate) fn lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

pub(crate) fn ucfirst(s: &str) -> String {
    let mut out = s.to_owned();
    if let Some(first) = out.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    out
}

pub(crate) fn lcfirst(s: &str) -> String {
    let mut out = s.to_owned();
    if let Some(first) = out.get_mut(..1) {
        first.make_ascii_lowercase();
    }
    out
}

// php-src: ext/standard/string.c ucwords, default delimiters
pub(crate) fn ucwords(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut after_delimiter = true;
    for c in s.chars() {
        out.push(if after_delimiter {
            c.to_ascii_uppercase()
        } else {
            c
        });
        after_delimiter = matches!(c, ' ' | '\t' | '\r' | '\n' | '\x0c' | '\x0b');
    }
    out
}

/// `str_replace` with several needles, applied one after another.
pub(crate) fn replace_each(s: &str, from: &[&str], to: &str) -> String {
    from.iter().fold(s.to_owned(), |acc, f| acc.replace(f, to))
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// `preg_replace('/(?<=\w)([A-Z])/', '_\1', $s)`.
pub(crate) fn underscore_capitals(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + 4);
    for (i, c) in s.char_indices() {
        if i > 0 && c.is_ascii_uppercase() && is_word(bytes[i - 1]) {
            out.push('_');
        }
        out.push(c);
    }
    out
}

/// Where an alternative of a pattern may match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum At {
    Start,
    End,
    Anywhere,
}

/// `preg_replace` of a pattern made of literal alternatives (`/^a-|-b$|c/`)
/// with the empty string, every match, leftmost first.
pub(crate) fn strip(s: &str, alternatives: &[(&str, At)], case_insensitive: bool) -> String {
    let matches_at = |i: usize, text: &str| {
        s.get(i..i + text.len()).is_some_and(|slice| {
            if case_insensitive {
                slice.eq_ignore_ascii_case(text)
            } else {
                slice == text
            }
        })
    };
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let hit = alternatives.iter().find(|(text, at)| {
            let anchored = match at {
                At::Start => i == 0,
                At::End => i + text.len() == s.len(),
                At::Anywhere => true,
            };
            anchored && matches_at(i, text)
        });
        if let Some((text, _)) = hit {
            i += text.len();
            continue;
        }
        let Some(c) = s[i..].chars().next() else {
            break;
        };
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// `preg_replace('/[^a-z0-9_]/i', '', $s)`.
pub(crate) fn keep_word_chars(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

/// `preg_replace_callback('/(-[a-z])/', fn ($m) => strtoupper($m[0][1]), $s)`.
pub(crate) fn camel_dashes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '-'
            && let Some(next) = chars.peek().copied()
            && next.is_ascii_lowercase()
        {
            out.push(next.to_ascii_uppercase());
            chars.next();
            continue;
        }
        out.push(c);
    }
    out
}

/// PHP's `empty()` for a decoded JSON value.
pub(crate) fn empty(v: Option<&serde_json::Value>) -> bool {
    use serde_json::Value;
    match v {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => !b,
        Some(Value::String(s)) => s.is_empty() || s == "0",
        Some(Value::Number(n)) => n.as_f64() == Some(0.0),
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::Object(o)) => o.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        At, camel_dashes, empty, keep_word_chars, lcfirst, replace_each, strip, ucfirst, ucwords,
        underscore_capitals,
    };
    use serde_json::json;

    #[test]
    fn changes_case_like_php() {
        assert_eq!(ucfirst("abc"), "Abc");
        assert_eq!(ucfirst(""), "");
        assert_eq!(lcfirst("ABC"), "aBC");
        assert_eq!(ucwords("my plugin\tname x"), "My Plugin\tName X");
        assert_eq!(ucwords("a_b-c"), "A_b-c");
        assert_eq!(ucwords("é x"), "é X");
    }

    #[test]
    fn replaces_like_str_replace() {
        assert_eq!(replace_each("a-b_c", &["-", "_"], " "), "a b c");
    }

    #[test]
    fn splits_capitals_after_word_characters() {
        assert_eq!(underscore_capitals("MyPluginName"), "My_Plugin_Name");
        assert_eq!(underscore_capitals("ABC"), "A_B_C");
        assert_eq!(underscore_capitals("a-Bc"), "a-Bc");
        assert_eq!(underscore_capitals("x_Y"), "x__Y");
    }

    #[test]
    fn strips_literal_alternatives() {
        let oc = [("oc-", At::Start), ("-plugin", At::End)];
        assert_eq!(strip("oc-thing-plugin", &oc, false), "thing");
        assert_eq!(strip("my-oc-plugin-x", &oc, false), "my-oc-plugin-x");
        let fork = [
            ("fork-cms-", At::Start),
            ("-module", At::Anywhere),
            ("ForkCMS", At::Anywhere),
            ("Module", At::End),
        ];
        assert_eq!(strip("fork-cms-a-module-b-module", &fork, false), "a-b");
        assert_eq!(strip("SyDes-x", &[("sydes-", At::Start)], true), "x");
        assert_eq!(
            strip("é-cockpit-", &[("cockpit-", At::Anywhere)], true),
            "é-"
        );
    }

    #[test]
    fn keeps_word_characters_only() {
        assert_eq!(keep_word_chars("my-vendor.name_1"), "myvendorname_1");
    }

    #[test]
    fn camel_cases_dash_lowercase_pairs() {
        assert_eq!(camel_dashes("my-plugin-name"), "myPluginName");
        assert_eq!(camel_dashes("a-B-"), "a-B-");
    }

    #[test]
    fn empty_matches_php() {
        assert!(empty(None));
        assert!(empty(Some(&json!(null))));
        assert!(empty(Some(&json!("0"))));
        assert!(empty(Some(&json!(""))));
        assert!(empty(Some(&json!(0))));
        assert!(empty(Some(&json!(0.0))));
        assert!(empty(Some(&json!([]))));
        assert!(empty(Some(&json!({}))));
        assert!(empty(Some(&json!(false))));
        assert!(!empty(Some(&json!("00"))));
        assert!(!empty(Some(&json!(true))));
        assert!(!empty(Some(&json!([0]))));
        assert!(!empty(Some(&json!({"a": 1}))));
    }
}
