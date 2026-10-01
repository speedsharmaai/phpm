//! Differential tests against the real `php` binary. Ignored by default; CI
//! runs them on Linux with `--run-ignored only`. They skip when `php` is not
//! on PATH. `strnatcasecmp` is only compared on ASCII input: PHP folds high
//! bytes with the C library's locale-dependent `toupper`.

use phpm_php::{
    PhpArray, PhpKey, PhpValue, encode_pretty, smart_strcmp, strnatcasecmp, strnatcmp, var_export,
    var_export_str,
};
use proptest::prelude::*;
use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

fn php(script: &str, input: &str) -> Option<String> {
    let mut child = Command::new("php")
        .args(["-d", "serialize_precision=-1", "-r", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(input.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    assert!(out.status.success(), "php failed on {input}");
    Some(String::from_utf8(out.stdout).expect("php prints utf-8"))
}

const ENCODE: &str = r#"
foreach (json_decode(stream_get_contents(STDIN), true) as $v) {
    echo json_encode($v, JSON_UNESCAPED_SLASHES | JSON_PRETTY_PRINT | JSON_UNESCAPED_UNICODE), "\0";
}"#;

const EXPORT: &str = r#"
foreach (json_decode(stream_get_contents(STDIN), true) as $v) {
    echo var_export($v, true), "\0";
}"#;

const COMPARE: &str = r#"
foreach (json_decode(stream_get_contents(STDIN), true) as [$a, $b]) {
    $ascii = !preg_match('/[\x80-\xff]/', $a . $b);
    echo strnatcmp($a, $b) <=> 0, ' ', $ascii ? strnatcasecmp($a, $b) <=> 0 : '-', ' ', $a <=> $b, "\0";
}"#;

fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z0-9 ./\\\\'\"-]{0,12}",
        "[0-9. eE+-]{0,8}",
        any::<String>(),
        prop::collection::vec(
            prop_oneof![
                Just('\u{2028}'),
                Just('\u{2029}'),
                Just('\u{0}'),
                Just('\u{7f}'),
                Just('é'),
                Just('😀'),
                prop::char::range('\u{0}', '\u{1f}'),
            ],
            0..6
        )
        .prop_map(|v| v.into_iter().collect()),
    ]
}

fn json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        any::<f64>()
            .prop_filter("finite", |f| f.is_finite())
            .prop_map(Value::from),
        text().prop_map(Value::String),
    ];
    leaf.prop_recursive(4, 32, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..5).prop_map(Value::Array),
            prop::collection::vec((prop_oneof!["[0-3]", text()], inner), 0..5)
                .prop_map(|kv| Value::Object(kv.into_iter().collect())),
        ]
    })
}

fn without_floats() -> impl Strategy<Value = Value> {
    json_value().prop_filter("no floats", |v| !has_float(v))
}

fn has_float(v: &Value) -> bool {
    match v {
        Value::Number(n) => !n.is_i64(),
        Value::Array(items) => items.iter().any(has_float),
        Value::Object(map) => map.values().any(has_float),
        _ => false,
    }
}

fn to_php(v: &Value) -> PhpValue {
    match v {
        Value::Null => PhpValue::Null,
        Value::Bool(b) => PhpValue::Bool(*b),
        Value::Number(n) => PhpValue::Int(n.as_i64().expect("floats filtered out")),
        Value::String(s) => PhpValue::String(s.clone()),
        Value::Array(items) => PhpValue::Array(
            items
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    (
                        PhpKey::Int(i64::try_from(i).expect("small index")),
                        to_php(v),
                    )
                })
                .collect::<PhpArray>(),
        ),
        Value::Object(map) => PhpValue::Array(
            map.iter()
                .map(|(k, v)| (PhpKey::from(k.as_str()), to_php(v)))
                .collect::<PhpArray>(),
        ),
    }
}

fn split(out: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = out.split('\0').collect();
    parts.pop();
    parts
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    #[ignore = "needs php on PATH"]
    fn matches_php_json_encode(values in prop::collection::vec(json_value(), 1..16)) {
        let input = serde_json::to_string(&values).unwrap();
        let Some(out) = php(ENCODE, &input) else { return Ok(()) };
        let ours: Vec<String> = values.iter().map(encode_pretty).collect();
        prop_assert_eq!(ours, split(&out));
    }

    #[test]
    #[ignore = "needs php on PATH"]
    fn matches_php_var_export(values in prop::collection::vec(text(), 1..32)) {
        let input = serde_json::to_string(&values).unwrap();
        let Some(out) = php(EXPORT, &input) else { return Ok(()) };
        let ours: Vec<String> = values.iter().map(|s| var_export_str(s)).collect();
        prop_assert_eq!(ours, split(&out));
    }

    #[test]
    #[ignore = "needs php on PATH"]
    fn matches_php_var_export_arrays(values in prop::collection::vec(without_floats(), 1..16)) {
        let input = serde_json::to_string(&values).unwrap();
        let Some(out) = php(EXPORT, &input) else { return Ok(()) };
        let ours: Vec<String> = values.iter().map(|v| var_export(&to_php(v))).collect();
        prop_assert_eq!(ours, split(&out));
    }

    #[test]
    #[ignore = "needs php on PATH"]
    fn matches_php_string_comparisons(pairs in prop::collection::vec((text(), text()), 1..32)) {
        let input = serde_json::to_string(&pairs).unwrap();
        let Some(out) = php(COMPARE, &input) else { return Ok(()) };
        let sign = |o: std::cmp::Ordering| o as i8;
        let ours: Vec<String> = pairs
            .iter()
            .map(|(a, b)| {
                let folded = if a.is_ascii() && b.is_ascii() {
                    sign(strnatcasecmp(a, b)).to_string()
                } else {
                    "-".to_owned()
                };
                format!("{} {folded} {}", sign(strnatcmp(a, b)), sign(smart_strcmp(a, b)))
            })
            .collect();
        prop_assert_eq!(ours, split(&out));
    }
}
