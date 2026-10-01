//! Differential tests of the `php_strip_whitespace()` port against the real
//! `php`. Ignored by default; CI runs them on Linux. They skip when `php` is
//! not on PATH. `PHPM_STRIP_DIR=<dir>` also compares every `.php` file
//! under that directory.

use phpm_autoload::scan::{finder_files, strip_whitespace};
use proptest::prelude::*;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

const STRIP: &str = r#"<?php
foreach (array_slice($argv, 1) as $file) {
    echo base64_encode(php_strip_whitespace($file)), "\n";
}
"#;

/// What PHP strips each file to, run from a script file so the CLI skips
/// `#!` lines the way it does under Composer.
fn php_strip(files: &[PathBuf]) -> Option<Vec<Vec<u8>>> {
    let dir = tempfile::tempdir().ok()?;
    let script = dir.path().join("strip.php");
    std::fs::write(&script, STRIP).ok()?;
    let mut out = Vec::new();
    for chunk in files.chunks(200) {
        let result = Command::new("php")
            .args(["-n", "-d", "short_open_tag=0"])
            .arg(&script)
            .args(chunk)
            .output()
            .ok()?;
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        for line in String::from_utf8(result.stdout).ok()?.lines() {
            out.push(decode(line));
        }
    }
    Some(out)
}

fn decode(b64: &str) -> Vec<u8> {
    let table = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        _ => 63,
    };
    let mut out = Vec::new();
    let bytes: Vec<u8> = b64.bytes().filter(|&c| c != b'=').collect();
    for chunk in bytes.chunks(4) {
        let mut n: u32 = 0;
        for (i, &c) in chunk.iter().enumerate() {
            n |= u32::from(table(c)) << (18 - 6 * i);
        }
        let take = chunk.len().saturating_sub(1);
        for i in 0..take {
            out.push(((n >> (16 - 8 * i)) & 0xff) as u8);
        }
    }
    out
}

fn compare(sources: &[Vec<u8>]) -> Result<(), TestCaseError> {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut files = Vec::new();
    for (i, source) in sources.iter().enumerate() {
        let path = dir.path().join(format!("{i}.php"));
        std::fs::File::create(&path)
            .and_then(|mut f| f.write_all(source))
            .expect("write source");
        files.push(path);
    }
    let Some(expected) = php_strip(&files) else {
        return Ok(());
    };
    for (source, expected) in sources.iter().zip(expected) {
        let ours = strip_whitespace(source, false);
        prop_assert_eq!(
            String::from_utf8_lossy(&ours),
            String::from_utf8_lossy(&expected),
            "source: {:?}",
            String::from_utf8_lossy(source)
        );
    }
    Ok(())
}

const FRAGMENTS: &[&str] = &[
    "<?php ",
    "<?php\n",
    "<?PHP\r\n",
    "<?=",
    "<?",
    "?>",
    "?>\n",
    "<",
    "class ",
    "Foo",
    " ",
    "  ",
    "\n",
    "\r\n",
    "\t",
    "/*",
    "*/",
    "/**",
    "/** ",
    "//",
    "#",
    "#[",
    "'",
    "\"",
    "\\",
    "$a",
    "$",
    "{",
    "}",
    "{$",
    "${",
    "<<<EOT\n",
    "<<<'EOT'\n",
    "<<< \"EOT\"\n",
    "\nEOT",
    "\n  EOT",
    "EOT;",
    "EOT",
    "->",
    "?->",
    "??",
    "(int)",
    "( string )",
    "yield from ",
    "yield\n/* x */from(",
    "`",
    "[",
    "]",
    "namespace ",
    ";",
    "0",
    "__halt_compiler();",
    "b'",
    "#!/bin/php\n",
    "x",
    "é",
    "--",
    "-",
    ">",
    "=",
    "<<",
    "<<=",
    "*",
    "/",
    "?",
    "\r",
    "1.5",
    "fn",
    "&",
    "...",
];

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    #[ignore = "needs php on PATH"]
    fn matches_php_strip_whitespace(
        sources in prop::collection::vec(
            prop::collection::vec(prop::sample::select(FRAGMENTS), 0..40)
                .prop_map(|parts| parts.concat().into_bytes()),
            1..24,
        )
    ) {
        compare(&sources)?;
    }
}

fn php_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            php_files(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e == "php" || e == "inc" || e == "hh")
        {
            out.push(path);
        }
    }
}

#[test]
#[ignore = "needs php on PATH and PHPM_STRIP_DIR"]
fn matches_php_strip_whitespace_on_a_tree() {
    let Some(dir) = std::env::var_os("PHPM_STRIP_DIR") else {
        return;
    };
    let mut files = Vec::new();
    php_files(Path::new(&dir), &mut files);
    let Some(expected) = php_strip(&files) else {
        return;
    };
    let mut differ = Vec::new();
    for (file, expected) in files.iter().zip(expected) {
        let source = std::fs::read(file).expect("read source");
        if strip_whitespace(&source, false) != expected {
            differ.push(file.display().to_string());
        }
    }
    assert!(
        differ.is_empty(),
        "{} of {} files differ: {differ:#?}",
        differ.len(),
        files.len()
    );
}

const FINDER: &str = r#"<?php
$it = new RecursiveIteratorIterator(
    new RecursiveDirectoryIterator($argv[1], FilesystemIterator::SKIP_DOTS | FilesystemIterator::FOLLOW_SYMLINKS),
    RecursiveIteratorIterator::SELF_FIRST
);
$vcs = ['.svn', '_svn', 'CVS', '_darcs', '.arch-params', '.monotone', '.bzr', '.git', '.hg'];
foreach ($it as $file) {
    $rel = substr($file->getPathname(), strlen($argv[1]) + 1);
    $parts = explode('/', $rel);
    if (array_intersect($parts, $vcs) || preg_match('#(^|/)\..+(/|$)#', $rel)) {
        continue;
    }
    if (!$file->isDir() && preg_match('/\.(?:php|inc|hh)$/', $file->getFilename())) {
        echo $file->getPathname(), "\n";
    }
}
"#;

#[test]
#[ignore = "needs php on PATH"]
fn matches_php_readdir_order() {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path().canonicalize().expect("real path");
    let names = [
        "zeta", "Alpha", "mid", "10", "9", "_x", "beta", "Gamma", "delta", "a-b",
    ];
    for (i, sub) in ["", "src/", "src/Deep/", ".hidden/", ".git/", "lib/"]
        .iter()
        .enumerate()
    {
        for (j, name) in names.iter().enumerate() {
            let pick = names[(j * 7 + i * 3) % names.len()];
            let ext = ["php", "inc", "hh", "txt"][(i + j) % 4];
            let path = root.join(format!("{sub}{pick}{name}.{ext}"));
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, "<?php").expect("write");
        }
    }
    let scripts = tempfile::tempdir().expect("temp dir");
    let script = scripts.path().join("finder.php");
    std::fs::write(&script, FINDER).expect("write script");
    let Ok(out) = Command::new("php").arg(&script).arg(&root).output() else {
        return;
    };
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let php: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    let ours = finder_files(&root.to_string_lossy()).expect("walk");
    assert_eq!(ours, php);
}
