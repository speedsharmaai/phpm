//! Differential test against composer/semver 3.4.4 (the copy inside
//! Composer 2.10.3) running on the real `php`. Ignored by default; CI runs it
//! on Linux. Skips when `php` is not on PATH.

use phpm_lock::version::{normalize, normalize_branch, parse_numeric_alias_prefix};
use proptest::prelude::*;
use std::io::Write;
use std::process::{Command, Stdio};

// composer/semver 3.4.4 src/VersionParser.php, MIT, the methods under test.
const PARSER: &str = r#"
class P {
    private static $modifierRegex = '[._-]?(?:(stable|beta|b|RC|alpha|a|patch|pl|p)((?:[.-]?\d+)*+)?)?([.-]?dev)?';
    private static $stabilitiesRegex = 'stable|RC|beta|alpha|dev';
    public function normalize($version) {
        $version = trim((string) $version);
        if (preg_match('{^([^,\s]++) ++as ++([^,\s]++)$}', $version, $match)) { $version = $match[1]; }
        if (preg_match('{@(?:' . self::$stabilitiesRegex . ')$}i', $version, $match)) { $version = substr($version, 0, strlen($version) - strlen($match[0])); }
        if (\in_array($version, array('master', 'trunk', 'default'), true)) { $version = 'dev-' . $version; }
        if (stripos($version, 'dev-') === 0) { return 'dev-' . substr($version, 4); }
        if (preg_match('{^([^,\s+]++)\+[^\s]++$}', $version, $match)) { $version = $match[1]; }
        if (preg_match('{^v?(\d{1,5}+)(\.\d++)?(\.\d++)?(\.\d++)?' . self::$modifierRegex . '$}i', $version, $matches)) {
            $version = $matches[1] . (!empty($matches[2]) ? $matches[2] : '.0') . (!empty($matches[3]) ? $matches[3] : '.0') . (!empty($matches[4]) ? $matches[4] : '.0');
            $index = 5;
        } elseif (preg_match('{^v?(\d{4}(?:[.:-]?\d{2}){1,6}(?:[.:-]?\d{1,3}){0,2})' . self::$modifierRegex . '$}i', $version, $matches)) {
            $version = (string) preg_replace('{\D}', '.', $matches[1]);
            $index = 2;
        }
        if (isset($index)) {
            if (!empty($matches[$index])) {
                if ('stable' === $matches[$index]) { return $version; }
                $version .= '-' . $this->expandStability($matches[$index]) . (isset($matches[$index + 1]) && '' !== $matches[$index + 1] ? ltrim($matches[$index + 1], '.-') : '');
            }
            if (!empty($matches[$index + 2])) { $version .= '-dev'; }
            return $version;
        }
        if (preg_match('{(.*?)[.-]?dev$}i', $version, $match)) {
            $normalized = $this->normalizeBranch($match[1]);
            if (strpos($normalized, 'dev-') === false) { return $normalized; }
        }
        return null;
    }
    public function parseNumericAliasPrefix($branch) {
        if (preg_match('{^(?P<version>(\d++\\.)*\d++)(?:\.x)?-dev$}i', (string) $branch, $matches)) { return $matches['version'] . '.'; }
        return null;
    }
    public function normalizeBranch($name) {
        $name = trim((string) $name);
        if (preg_match('{^v?(\d++)(\.(?:\d++|[xX*]))?(\.(?:\d++|[xX*]))?(\.(?:\d++|[xX*]))?$}i', $name, $matches)) {
            $version = '';
            for ($i = 1; $i < 5; ++$i) { $version .= isset($matches[$i]) ? str_replace(array('*', 'X'), 'x', $matches[$i]) : '.x'; }
            return str_replace('x', '9999999', $version) . '-dev';
        }
        return 'dev-' . $name;
    }
    private function expandStability($stability) {
        $stability = strtolower($stability);
        switch ($stability) {
            case 'a': return 'alpha';
            case 'b': return 'beta';
            case 'p': case 'pl': return 'patch';
            case 'rc': return 'RC';
            default: return $stability;
        }
    }
}
$p = new P();
foreach (json_decode(stream_get_contents(STDIN), true) as $v) {
    echo json_encode([$p->normalize($v), $p->normalizeBranch($v), $p->parseNumericAliasPrefix($v)], JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE), "\n";
}
"#;

fn php(input: &str) -> Option<String> {
    let mut child = Command::new("php")
        .args(["-r", PARSER])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(input.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    assert!(out.status.success(), "php failed on {input}");
    Some(String::from_utf8(out.stdout).expect("php prints utf-8"))
}

fn version_like() -> impl Strategy<Value = String> {
    prop_oneof![
        r"[vV]?[0-9]{1,6}(\.[0-9xX*]{1,3}){0,4}([._-]?(stable|STABLE|beta|b|RC|rc|alpha|a|A|patch|pl|p|dev|DEV)([.-]?[0-9]{1,2}){0,2})?([.-]?(dev|Dev))?(\+[a-z0-9.]{1,4})?(@(dev|stable|beta|RC))?( as [0-9.]{1,5})?",
        r"[0-9]{4}([.:-]?[0-9]{2}){0,7}([.:-]?[0-9]{1,3}){0,3}(-p[0-9]|-dev|\.x-dev)?",
        r"(dev-|DEV-|Dev-)?[a-z0-9./+_ -]{0,10}(-dev|\.dev|dev)?",
        r"[ \t]?(master|trunk|default|main)[ \n]?",
        "\\PC{0,8}",
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    #[ignore = "needs php on PATH"]
    fn matches_php_version_parser(inputs in prop::collection::vec(version_like(), 1..64)) {
        let Some(out) = php(&serde_json::to_string(&inputs).unwrap()) else { return Ok(()) };
        prop_assert_eq!(out.lines().count(), inputs.len());
        for (input, line) in inputs.iter().zip(out.lines()) {
            let ours = serde_json::json!([
                normalize(input).ok(),
                normalize_branch(input),
                parse_numeric_alias_prefix(input),
            ]);
            let theirs: serde_json::Value = serde_json::from_str(line).unwrap();
            prop_assert_eq!(ours, theirs, "input {:?}", input);
        }
    }
}
