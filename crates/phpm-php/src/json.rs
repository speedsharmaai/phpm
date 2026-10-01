use serde_json::{Map, Number, Value};
use std::fmt::Write as _;

const INDENT: &str = "    ";

/// `json_encode($value, JSON_UNESCAPED_SLASHES | JSON_PRETTY_PRINT | JSON_UNESCAPED_UNICODE)`
/// for a value decoded with `json_decode($json, true)`.
///
/// Objects are PHP arrays after decoding, so an empty object encodes as `[]`
/// and an object whose keys are exactly `"0"` to `"n-1"` in order encodes as a
/// list.
pub fn encode_pretty(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value, 0, false);
    out
}

/// `json_encode($value, JSON_PRETTY_PRINT)`: slashes and non-ASCII escaped,
/// as PHP does by default.
pub fn encode_pretty_escaped(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value, 0, true);
    out
}

/// Like [`encode_pretty`], but indented with `indent` instead of four
/// spaces, the way Composer's `JsonFile::write` re-indents its usual
/// 4-space `json_encode` output to the indent it detected on read.
// Composer: Json/JsonFile.php encode
pub fn encode_pretty_indented(value: &Value, indent: &str) -> String {
    let mut out = String::new();
    write_value(&mut out, value, 0, false);
    if indent == INDENT {
        return out;
    }
    reindent(&out, indent)
}

/// The leading run of spaces or tabs just before a `"` that starts a line,
/// the first time that happens; four spaces if it never does.
// Composer: Json/JsonFile.php detectIndenting
pub fn detect_indent(json: &str) -> String {
    for line in json.split('\n') {
        let rest = line.trim_start_matches([' ', '\t']);
        let indent = &line[..line.len() - rest.len()];
        if !indent.is_empty() && rest.starts_with('"') {
            return indent.to_owned();
        }
    }
    INDENT.to_owned()
}

/// Every run of 4 or more spaces at the start of a line, replaced with that
/// many groups of `indent`.
fn reindent(s: &str, indent: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for line in s.split_inclusive('\n') {
        let spaces = line.bytes().take_while(|&b| b == b' ').count();
        if spaces >= 4 {
            out.extend(std::iter::repeat_n(indent, spaces / 4));
            out.push_str(&line[spaces..]);
        } else {
            out.push_str(line);
        }
    }
    out
}

fn write_value(out: &mut String, value: &Value, level: usize, escaped: bool) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => write_number(out, n),
        Value::String(s) => write_string(out, s, escaped),
        Value::Array(items) => write_list(out, items.iter(), items.len(), level, escaped),
        Value::Object(map) if is_list(map) => {
            write_list(out, map.values(), map.len(), level, escaped);
        }
        Value::Object(map) => write_object(out, map, level, escaped),
    }
}

fn is_list(map: &Map<String, Value>) -> bool {
    map.keys().enumerate().all(|(i, k)| *k == i.to_string())
}

fn write_list<'a>(
    out: &mut String,
    items: impl Iterator<Item = &'a Value>,
    len: usize,
    level: usize,
    escaped: bool,
) {
    if len == 0 {
        out.push_str("[]");
        return;
    }
    out.push_str("[\n");
    for (i, item) in items.enumerate() {
        if i > 0 {
            out.push_str(",\n");
        }
        push_indent(out, level + 1);
        write_value(out, item, level + 1, escaped);
    }
    out.push('\n');
    push_indent(out, level);
    out.push(']');
}

fn write_object(out: &mut String, map: &Map<String, Value>, level: usize, escaped: bool) {
    out.push_str("{\n");
    for (i, (key, item)) in map.iter().enumerate() {
        if i > 0 {
            out.push_str(",\n");
        }
        push_indent(out, level + 1);
        write_string(out, key, escaped);
        out.push_str(": ");
        write_value(out, item, level + 1, escaped);
    }
    out.push('\n');
    push_indent(out, level);
    out.push('}');
}

fn push_indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str(INDENT);
    }
}

fn write_number(out: &mut String, n: &Number) {
    if let Some(i) = n.as_i64() {
        out.push_str(&i.to_string());
    } else if let Some(f) = n.as_f64() {
        out.push_str(&format_float(f));
    }
}

/// A float as PHP prints it with `serialize_precision = -1` (the shortest
/// round-trip digits, `zend_gcvt` with 17 as the exponent threshold).
///
/// `json_encode` uses this directly: `1.0` becomes `1`, `1e25` becomes
/// `1.0e+25`.
pub fn format_float(f: f64) -> String {
    let mut out = String::new();
    if f.is_sign_negative() {
        out.push('-');
    }
    let f = f.abs();
    if f == 0.0 {
        out.push('0');
        return out;
    }
    let (digits, decpt) = shortest_digits(f);

    if if decpt < 0 { decpt < -3 } else { decpt > 17 } {
        let (first, rest) = digits.split_at(1);
        out.push_str(first);
        out.push('.');
        out.push_str(if rest.is_empty() { "0" } else { rest });
        let e = decpt - 1;
        out.push('e');
        out.push(if e < 0 { '-' } else { '+' });
        out.push_str(&e.unsigned_abs().to_string());
    } else if decpt <= 0 {
        out.push_str("0.");
        for _ in decpt..0 {
            out.push('0');
        }
        out.push_str(&digits);
    } else {
        let decpt = decpt.unsigned_abs() as usize;
        if digits.len() <= decpt {
            out.push_str(&digits);
            for _ in digits.len()..decpt {
                out.push('0');
            }
        } else {
            out.push_str(&digits[..decpt]);
            out.push('.');
            out.push_str(&digits[decpt..]);
        }
    }
    out
}

fn split_sci(sci: &str) -> (String, i32) {
    let (mantissa, exp) = sci.split_once('e').unwrap_or((sci, "0"));
    let digits = mantissa.chars().filter(char::is_ascii_digit).collect();
    (digits, exp.parse::<i32>().unwrap_or(0) + 1)
}

// zend_dtoa mode 0: the shortest digits that round-trip, closest to the exact
// value, ties to even. Rust's shortest formatting breaks ties upwards.
fn shortest_digits(f: f64) -> (String, i32) {
    let (shortest, decpt) = split_sci(&format!("{f:e}"));
    let (exact, exact_decpt) = split_sci(&format!("{f:.767e}"));
    let n = shortest.len();
    let exact = exact.as_bytes();
    let head = &exact[..n];
    let tail = &exact[n..];
    let round_up = match tail.first() {
        Some(b'6'..=b'9') => true,
        Some(b'5') => tail[1..].iter().any(|&d| d != b'0') || head[n - 1] % 2 == 1,
        _ => false,
    };
    let mut digits = head.to_vec();
    let mut decpt_out = exact_decpt;
    if round_up {
        let mut i = n;
        loop {
            if i == 0 {
                digits.insert(0, b'1');
                digits.truncate(n);
                decpt_out += 1;
                break;
            }
            i -= 1;
            if digits[i] == b'9' {
                digits[i] = b'0';
            } else {
                digits[i] += 1;
                break;
            }
        }
    }
    let candidate = String::from_utf8(digits).unwrap_or_default();
    let trimmed = candidate.trim_end_matches('0');
    let trimmed = if trimmed.is_empty() { "0" } else { trimmed };
    let roundtrips = format!("0.{trimmed}e{decpt_out}").parse::<f64>() == Ok(f);
    if roundtrips && trimmed.len() <= n {
        (trimmed.to_owned(), decpt_out)
    } else {
        (shortest, decpt)
    }
}

fn write_string(out: &mut String, s: &str, escaped: bool) {
    out.push('"');
    for c in s.chars() {
        match c {
            '/' if escaped => out.push_str("\\/"),
            c if escaped && !c.is_ascii() => {
                let mut units = [0_u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::{
        detect_indent, encode_pretty, encode_pretty_escaped, encode_pretty_indented, format_float,
    };
    use serde_json::{Value, json};

    #[test]
    fn detects_the_indent_composer_wrote_before() {
        assert_eq!(detect_indent("{\n        \"a\": 1\n}"), "        ");
        assert_eq!(detect_indent("{\n\t\"a\": 1\n}"), "\t");
        assert_eq!(detect_indent("[\n    1\n]"), "    ");
        assert_eq!(detect_indent("not json"), "    ");
        assert_eq!(detect_indent(""), "    ");
        assert_eq!(detect_indent("{}"), "    ");
    }

    #[test]
    fn reindents_every_nesting_level() {
        let v = parse(r#"{"a":[1,{"b":2}]}"#);
        assert_eq!(encode_pretty_indented(&v, "    "), encode_pretty(&v));
        assert_eq!(
            encode_pretty_indented(&v, "        "),
            "{\n        \"a\": [\n                1,\n                {\n                        \"b\": 2\n                }\n        ]\n}"
        );
        assert_eq!(
            encode_pretty_indented(&v, "\t"),
            "{\n\t\"a\": [\n\t\t1,\n\t\t{\n\t\t\t\"b\": 2\n\t\t}\n\t]\n}"
        );
    }

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn scalars_encode_like_php() {
        assert_eq!(encode_pretty(&json!(null)), "null");
        assert_eq!(encode_pretty(&json!(true)), "true");
        assert_eq!(encode_pretty(&json!(false)), "false");
        assert_eq!(encode_pretty(&json!(-42)), "-42");
        assert_eq!(encode_pretty(&json!("a/b")), "\"a/b\"");
    }

    #[test]
    fn empty_object_becomes_empty_list() {
        assert_eq!(encode_pretty(&parse("{}")), "[]");
        assert_eq!(encode_pretty(&parse("[]")), "[]");
        assert_eq!(encode_pretty(&parse(r#"{"a":{}}"#)), "{\n    \"a\": []\n}");
    }

    #[test]
    fn sequential_numeric_keys_become_a_list() {
        assert_eq!(
            encode_pretty(&parse(r#"{"0":"a","1":"b"}"#)),
            "[\n    \"a\",\n    \"b\"\n]"
        );
        assert_eq!(
            encode_pretty(&parse(r#"{"1":"a","0":"b"}"#)),
            "{\n    \"1\": \"a\",\n    \"0\": \"b\"\n}"
        );
        assert_eq!(
            encode_pretty(&parse(r#"{"1":"a"}"#)),
            "{\n    \"1\": \"a\"\n}"
        );
        assert_eq!(
            encode_pretty(&parse(r#"{"00":"a"}"#)),
            "{\n    \"00\": \"a\"\n}"
        );
        assert_eq!(
            encode_pretty(&parse(r#"{"+0":"a"}"#)),
            "{\n    \"+0\": \"a\"\n}"
        );
    }

    #[test]
    fn nests_with_four_space_indent() {
        let v = parse(r#"{"a":[1,[2,{"b":null}]],"c":{"d":"e"}}"#);
        let expected = "{\n    \"a\": [\n        1,\n        [\n            2,\n            {\n                \"b\": null\n            }\n        ]\n    ],\n    \"c\": {\n        \"d\": \"e\"\n    }\n}";
        assert_eq!(encode_pretty(&v), expected);
    }

    #[test]
    fn escapes_like_php_with_unescaped_unicode_and_slashes() {
        let v = json!("x/y\u{2028}\u{2029}\u{7f}\u{1}\u{1f} é 😀 \"\\ \u{8}\u{c}\n\r\t");
        assert_eq!(
            encode_pretty(&v),
            "\"x/y\\u2028\\u2029\u{7f}\\u0001\\u001f é 😀 \\\"\\\\ \\b\\f\\n\\r\\t\""
        );
    }

    #[test]
    fn default_flags_escape_slashes_and_unicode() {
        let v = json!({"k/é": ["Pest\\Plugins\\Bail", "a/b", "é 😀\u{2028}"]});
        assert_eq!(
            encode_pretty_escaped(&v),
            "{\n    \"k\\/\\u00e9\": [\n        \"Pest\\\\Plugins\\\\Bail\",\n        \"a\\/b\",\n        \"\\u00e9 \\ud83d\\ude00\\u2028\"\n    ]\n}"
        );
        assert_eq!(encode_pretty_escaped(&json!([])), "[]");
    }

    #[test]
    fn keys_are_escaped_too() {
        assert_eq!(
            encode_pretty(&parse(r#"{"a\"b\\":1}"#)),
            "{\n    \"a\\\"b\\\\\": 1\n}"
        );
    }

    #[test]
    fn floats_follow_serialize_precision_minus_one() {
        let cases = [
            (1.0, "1"),
            (0.1, "0.1"),
            (1e25, "1.0e+25"),
            (1.5e-7, "1.5e-7"),
            (123_456_789_012_345_680.0, "1.2345678901234568e+17"),
            (1e15, "1000000000000000"),
            (1e16, "10000000000000000"),
            (1e17, "1.0e+17"),
            (0.0001, "0.0001"),
            (0.00001, "1.0e-5"),
            (-0.0, "-0"),
            (0.0, "0"),
            (2.5, "2.5"),
            (0.5, "0.5"),
            (-1.25, "-1.25"),
            (3.0e-5, "3.0e-5"),
            (1.797_693_134_862_315_7e308, "1.7976931348623157e+308"),
            (5e-324, "5.0e-324"),
            (100.0, "100"),
            (12345.678, "12345.678"),
            (-1_077_910_613_244_010.2, "-1077910613244010.2"),
            (9.5, "9.5"),
            (0.3, "0.3"),
            (999_999_999_999_999_900_000.0, "9.999999999999999e+20"),
            (1e23, "1.0e+23"),
        ];
        for (f, expected) in cases {
            assert_eq!(format_float(f), expected, "{f:e}");
        }
    }

    #[test]
    fn integers_beyond_i64_are_floats() {
        assert_eq!(
            encode_pretty(&parse("18446744073709551615")),
            "1.8446744073709552e+19"
        );
        assert_eq!(
            encode_pretty(&parse("-9223372036854775809")),
            "-9.223372036854776e+18"
        );
        assert_eq!(encode_pretty(&parse("1.0")), "1");
    }
}
