//! Class discovery: `PhpFileParser::findClasses` from
//! composer/class-map-generator 1.7.3 on top of a port of
//! `php_strip_whitespace()`.

mod strip;

use regex::bytes::{Regex, RegexBuilder};
use std::sync::LazyLock;

pub use strip::strip_whitespace;

/// The `.php`, `.inc` and `.hh` files Symfony Finder yields for `dir`, in
/// its (readdir) order, as Composer's class map generator walks them.
pub fn finder_files(dir: &str) -> Result<Vec<String>, crate::Error> {
    crate::classmap::finder_paths(dir)
}

fn ci(pattern: &str) -> Regex {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .unicode(false)
        .build()
        .expect("valid pattern")
}

static PRECHECK: LazyLock<Regex> = LazyLock::new(|| ci(r"\b(?:class|interface|trait|enum)\s"));
static PRECHECK_NO_ENUM: LazyLock<Regex> = LazyLock::new(|| ci(r"\b(?:class|interface|trait)\s"));

const LABEL: &str = r"[a-zA-Z_\x7f-\xff][a-zA-Z0-9_\x7f-\xff]*";

fn declarations(types: &str) -> Regex {
    ci(&format!(
        r"\b(?P<type>{types})\s+(?P<name>[a-zA-Z_\x7f-\xff:][a-zA-Z0-9_\x7f-\xff:\-]*)|\b(?P<ns>namespace)(?P<nsname>\s+{LABEL}(?:\s*\\\s*{LABEL})*)?\s*[\{{;]"
    ))
}

static DECLARATIONS: LazyLock<Regex> = LazyLock::new(|| declarations("class|interface|trait|enum"));
static DECLARATIONS_NO_ENUM: LazyLock<Regex> =
    LazyLock::new(|| declarations("class|interface|trait"));

const RAW_BYTE_BASE: u32 = 0x10_FF00;

/// Bytes as a `String`, each byte of invalid UTF-8 kept as a private-use
/// character so class names like `\xA9` survive to the output.
pub(crate) fn lossless(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        out.push_str(chunk.valid());
        for &b in chunk.invalid() {
            out.push(char::from_u32(RAW_BYTE_BASE + u32::from(b)).unwrap_or('\u{FFFD}'));
        }
    }
    out
}

/// The bytes [`lossless`] encoded.
pub(crate) fn raw_bytes(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        let code = u32::from(c);
        if (RAW_BYTE_BASE..=RAW_BYTE_BASE + 0xFF).contains(&code) {
            out.push(u8::try_from(code - RAW_BYTE_BASE).unwrap_or(b'?'));
        } else {
            let mut buf = [0; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    }
    out
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn strcspn(s: &[u8], start: usize, reject: &[u8]) -> usize {
    s.get(start..).map_or(0, |rest| {
        rest.iter().take_while(|c| !reject.contains(c)).count()
    })
}

// composer/class-map-generator: src/PhpFileCleaner.php
struct Cleaner<'a> {
    s: &'a [u8],
    index: usize,
    max_matches: usize,
    types: &'static [&'static [u8]],
    reject: Vec<u8>,
}

impl Cleaner<'_> {
    fn peek(&self, c: u8) -> bool {
        self.index + 1 < self.s.len() && self.s[self.index + 1] == c
    }

    fn skip_to_php(&mut self) {
        while self.index < self.s.len() {
            if self.s[self.index] == b'<' && self.peek(b'?') {
                self.index += 2;
                break;
            }
            self.index += 1;
        }
    }

    fn skip_string(&mut self, delimiter: u8) {
        self.index += 1;
        while self.index < self.s.len() {
            self.index += strcspn(self.s, self.index, &[b'\\', delimiter]);
            if self.index >= self.s.len() {
                break;
            }
            if self.s[self.index] == b'\\' && (self.peek(b'\\') || self.peek(delimiter)) {
                self.index += 2;
                continue;
            }
            if self.s[self.index] == delimiter {
                self.index += 1;
                break;
            }
            self.index += 1;
        }
    }

    fn skip_comment(&mut self) {
        self.index += 2;
        while self.index < self.s.len() {
            self.index += strcspn(self.s, self.index, b"*");
            if self.peek(b'/') {
                self.index += 2;
                break;
            }
            self.index += 1;
        }
    }

    fn skip_to_newline(&mut self) {
        self.index += strcspn(self.s, self.index, b"\r\n");
    }

    /// `{<<<[ \t]*+(['"]?)(LABEL)\1(?:\r\n|\n|\r)}A`: (length, label).
    fn heredoc_start(&self) -> Option<(usize, &[u8])> {
        let s = self.s;
        let mut i = self.index + 3;
        if !s.get(self.index..)?.starts_with(b"<<<") {
            return None;
        }
        while matches!(s.get(i), Some(b' ' | b'\t')) {
            i += 1;
        }
        let quote = match s.get(i) {
            Some(&q @ (b'\'' | b'"')) => {
                i += 1;
                Some(q)
            }
            _ => None,
        };
        let start = i;
        match s.get(i) {
            Some(&c) if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 => i += 1,
            _ => return None,
        }
        while matches!(s.get(i), Some(&c) if c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80) {
            i += 1;
        }
        let label = &s[start..i];
        if let Some(q) = quote {
            if s.get(i) != Some(&q) {
                return None;
            }
            i += 1;
        }
        match (s.get(i), s.get(i + 1)) {
            (Some(b'\r'), Some(b'\n')) => i += 2,
            (Some(b'\n' | b'\r'), _) => i += 1,
            _ => return None,
        }
        Some((i - self.index, label))
    }

    fn skip_heredoc(&mut self, delimiter: &[u8]) {
        let s = self.s;
        while self.index < s.len() {
            match s[self.index] {
                b'\t' | b' ' => {
                    self.index += 1;
                    continue;
                }
                c if c == delimiter[0] => {
                    let end = self.index + delimiter.len();
                    if s[self.index..].starts_with(delimiter)
                        && !matches!(s.get(end), Some(&c) if c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80)
                    {
                        self.index = end;
                        return;
                    }
                }
                _ => {}
            }
            self.skip_to_newline();
            self.index += s.get(self.index..).map_or(0, |rest| {
                rest.iter()
                    .take_while(|c| matches!(c, b'\r' | b'\n'))
                    .count()
            });
        }
    }

    /// `{.\b(?<![\$:>])TYPE\s++[a-zA-Z_\x7f-\xff:][a-zA-Z0-9_\x7f-\xff:\-]*+}Ais`
    /// matched at `index - 1`.
    fn declaration_at(&self, ty: &[u8]) -> Option<&[u8]> {
        let s = self.s;
        let before = *s.get(self.index.checked_sub(1)?)?;
        if is_word(before) || matches!(before, b'$' | b':' | b'>') {
            return None;
        }
        let mut i = self.index + ty.len();
        let ws_start = i;
        while matches!(
            s.get(i),
            Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c')
        ) {
            i += 1;
        }
        if i == ws_start {
            return None;
        }
        match s.get(i) {
            Some(&c) if c.is_ascii_alphabetic() || c == b'_' || c >= 0x7f || c == b':' => i += 1,
            _ => return None,
        }
        while matches!(s.get(i), Some(&c) if c.is_ascii_alphanumeric() || c == b'_' || c >= 0x7f || c == b':' || c == b'-')
        {
            i += 1;
        }
        Some(&s[self.index - 1..i])
    }

    fn clean(mut self) -> Vec<u8> {
        let s = self.s;
        let mut clean = Vec::with_capacity(s.len());
        'outer: while self.index < s.len() {
            self.skip_to_php();
            clean.extend_from_slice(b"<?");
            while self.index < s.len() {
                let c = s[self.index];
                if c == b'?' && self.peek(b'>') {
                    clean.extend_from_slice(b"?>");
                    self.index += 2;
                    continue 'outer;
                }
                if c == b'"' || c == b'\'' {
                    self.skip_string(c);
                    clean.extend_from_slice(b"null");
                    continue;
                }
                if c == b'<'
                    && self.peek(b'<')
                    && let Some((len, label)) = self.heredoc_start()
                {
                    let label = label.to_vec();
                    self.index += len;
                    self.skip_heredoc(&label);
                    clean.extend_from_slice(b"null");
                    continue;
                }
                if c == b'/' {
                    if self.peek(b'/') {
                        self.skip_to_newline();
                        continue;
                    }
                    if self.peek(b'*') {
                        self.skip_comment();
                        continue;
                    }
                }
                if self.max_matches == 1
                    && let Some(ty) = self.types.iter().find(|t| t[0] == c)
                    && s[self.index..].starts_with(ty)
                    && let Some(found) = self.declaration_at(ty)
                {
                    clean.extend_from_slice(found);
                    return clean;
                }
                self.index += 1;
                let skip = strcspn(s, self.index, &self.reject);
                clean.push(c);
                clean.extend_from_slice(&s[self.index..self.index + skip]);
                self.index += skip;
            }
        }
        clean
    }
}

/// What the class map scan records for one file: [`find_classes`] as
/// Composer running on PHP 8.1+ sees it. Callers caching scan results use
/// this so they agree with the scan.
pub fn file_classes(source: &[u8]) -> Vec<String> {
    find_classes(source, true)
}

const TYPES: [&[u8]; 4] = [b"class", b"interface", b"trait", b"enum"];

/// The classes, interfaces, traits and enums a PHP file declares, from its
/// source. `enums` is whether the PHP running Composer is 8.1 or newer.
// composer/class-map-generator: src/PhpFileParser.php findClasses
pub fn find_classes(source: &[u8], enums: bool) -> Vec<String> {
    let contents = strip_whitespace(source, false);
    let (precheck, declarations, types): (&Regex, &Regex, &'static [&'static [u8]]) = if enums {
        (&PRECHECK, &DECLARATIONS, &TYPES)
    } else {
        (&PRECHECK_NO_ENUM, &DECLARATIONS_NO_ENUM, &TYPES[..3])
    };
    let max_matches = precheck.find_iter(&contents).count();
    if max_matches == 0 {
        return Vec::new();
    }
    let mut reject = b"?\"'</".to_vec();
    reject.extend(types.iter().map(|t| t[0]));
    let cleaned = Cleaner {
        s: &contents,
        index: 0,
        max_matches,
        types,
        reject,
    }
    .clean();

    let mut classes = Vec::new();
    let mut namespace = String::new();
    let mut at = 0;
    while let Some(caps) = declarations.captures_at(&cleaned, at) {
        let whole = caps.get(0).map_or(0..0, |m| m.range());
        if whole.start > 0 && matches!(cleaned[whole.start - 1], b'\\' | b'$' | b':' | b'>') {
            at = whole.start + 1;
            continue;
        }
        at = whole.end.max(whole.start + 1);
        let text = |name: &str| {
            caps.name(name)
                .map(|m| lossless(m.as_bytes()))
                .unwrap_or_default()
        };
        if caps.name("ns").is_some() {
            namespace = text("nsname").replace([' ', '\t', '\r', '\n'], "") + "\\";
            continue;
        }
        let mut name = text("name");
        if name == "extends" || name == "implements" {
            continue;
        }
        if name.starts_with(':') {
            name = format!("xhp{}", &name.replace('-', "_").replace(':', "__")[1..]);
        } else if text("type").eq_ignore_ascii_case("enum")
            && let Some(colon) = name.rfind(':')
        {
            name.truncate(colon);
        }
        classes.push(
            format!("{namespace}{name}")
                .trim_start_matches('\\')
                .to_owned(),
        );
    }
    classes
}

#[cfg(test)]
mod tests {
    use super::find_classes;

    fn classes(s: &str) -> Vec<String> {
        find_classes(s.as_bytes(), true)
    }

    #[test]
    fn finds_declarations() {
        assert_eq!(
            classes("<?php namespace A\\B; class C {} interface D {} trait E {} enum F: string {}"),
            ["A\\B\\C", "A\\B\\D", "A\\B\\E", "A\\B\\F"]
        );
        assert_eq!(find_classes(b"<?php enum F {} class G {}", false), ["G"]);
        assert_eq!(
            classes("<?php namespace A { class B {} } namespace { class C {} }"),
            ["A\\B", "C"]
        );
        assert_eq!(classes("<?php namespace A \\ B ; class C {}"), ["A\\B\\C"]);
        assert!(classes("<?php echo 1;").is_empty());
        assert!(classes("").is_empty());
    }

    #[test]
    fn skips_what_is_not_a_declaration() {
        assert_eq!(
            classes("<?php $x = new class extends Y {}; class Z {}"),
            ["Z"]
        );
        assert_eq!(
            classes("<?php $a = 'class Foo'; /* class Bar */ class Baz {}"),
            ["Baz"]
        );
        assert_eq!(
            classes("<?php $a = Foo::class; $b->class; class Real {}"),
            ["Real"]
        );
        assert_eq!(
            classes("<?php $a = <<<EOT\nclass Fake {}\nEOT;\nclass Real {}"),
            ["Real"]
        );
        assert_eq!(
            classes("<?php $a = <<<'EOT'\nclass Fake {}\n  EOT;\nclass Real {}"),
            ["Real"]
        );
        assert_eq!(
            classes("<?php class A {} ?>class Html <?php class B {}"),
            ["A", "B"]
        );
        assert_eq!(classes("<?php\n__halt_compiler(); class Data {}"), ["Data"]);
        assert_eq!(
            classes("<?php class :xhp:my-thing {}"),
            ["xhp_xhp__my_thing"]
        );
    }

    #[test]
    fn keeps_invalid_utf8_names() {
        let found = find_classes(b"<?php class \xa9 {}", true);
        assert_eq!(super::raw_bytes(&found[0]), b"\xa9");
        assert_eq!(super::raw_bytes("plain é"), "plain é".as_bytes());
    }

    #[test]
    fn single_match_shortcut() {
        assert_eq!(
            classes("<?php namespace N; final class Only extends Base {}"),
            ["N\\Only"]
        );
        assert_eq!(classes("<?php $s = \"class\"; class Only {}"), ["Only"]);
    }
}
