use crate::paths::find_shortest_path_code;
use phpm_php::{PhpArray, PhpKey, PhpValue, var_export, var_export_str};
use std::fmt::Write as _;

/// PHP `strtr($s, $pairs)`: longest key first at each offset, replaced text
/// is never rescanned.
pub(crate) fn strtr(s: &str, pairs: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = pairs.iter().filter(|(k, _)| !k.is_empty()).collect();
    sorted.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    'outer: while !rest.is_empty() {
        for (from, to) in &sorted {
            if let Some(after) = rest.strip_prefix(from.as_str()) {
                out.push_str(to);
                rest = after;
                continue 'outer;
            }
        }
        let mut chars = rest.chars();
        if let Some(c) = chars.next() {
            out.push(c);
        }
        rest = chars.as_str();
    }
    out
}

/// `ltrim(preg_replace('/^ */m', '    $0$0', $v))` then
/// `preg_replace('/ +$/m', '', $v)`.
fn reindent(value: &str) -> String {
    let lines: Vec<String> = value
        .split('\n')
        .map(|line| {
            let spaces = line.len() - line.trim_start_matches(' ').len();
            let indented = format!(
                "    {}{}{}",
                " ".repeat(spaces),
                " ".repeat(spaces),
                &line[spaces..]
            );
            indented.trim_end_matches(' ').to_owned()
        })
        .collect();
    lines
        .join("\n")
        .trim_start_matches([' ', '\t', '\n', '\r', '\0', '\x0B'])
        .to_owned()
}

/// The `ClassLoader` state `autoload_static.php` copies, built the way
/// `getStaticFile` builds it from the other autoload files.
#[derive(Debug, Default)]
pub(crate) struct Loader {
    files: Option<PhpArray>,
    prefix_lengths_psr4: PhpArray,
    prefix_dirs_psr4: PhpArray,
    fallback_dirs_psr4: PhpArray,
    prefixes_psr0: PhpArray,
    fallback_dirs_psr0: PhpArray,
    class_map: PhpArray,
}

fn list(values: &[String]) -> PhpArray {
    values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            (
                PhpKey::Int(i64::try_from(i).unwrap_or(i64::MAX)),
                PhpValue::String(v.clone()),
            )
        })
        .collect()
}

fn falsy(prefix: &str) -> bool {
    prefix.is_empty() || prefix == "0"
}

fn first_byte_key(prefix: &str) -> PhpKey {
    PhpKey::from(&prefix[..1])
}

fn nested(map: &mut PhpArray, outer: PhpKey) -> &mut PhpArray {
    let slot = map
        .entry(outer)
        .or_insert_with(|| PhpValue::Array(PhpArray::new()));
    if !matches!(slot, PhpValue::Array(_)) {
        *slot = PhpValue::Array(PhpArray::new());
    }
    match slot {
        PhpValue::Array(inner) => inner,
        _ => unreachable!("just made it an array"),
    }
}

impl Loader {
    pub(crate) fn set_files(&mut self, files: &[(String, String)]) {
        self.files = Some(
            files
                .iter()
                .map(|(k, v)| (PhpKey::from(k.as_str()), PhpValue::String(v.clone())))
                .collect(),
        );
    }

    // Composer: Autoload/ClassLoader.php set
    pub(crate) fn set_psr0(&mut self, prefix: &str, paths: &[String]) {
        if falsy(prefix) {
            self.fallback_dirs_psr0 = list(paths);
        } else {
            nested(&mut self.prefixes_psr0, first_byte_key(prefix))
                .insert(PhpKey::from(prefix), PhpValue::Array(list(paths)));
        }
    }

    // Composer: Autoload/ClassLoader.php setPsr4
    pub(crate) fn set_psr4(&mut self, prefix: &str, paths: &[String]) {
        if falsy(prefix) {
            self.fallback_dirs_psr4 = list(paths);
        } else {
            let length = i64::try_from(prefix.len()).unwrap_or(i64::MAX);
            nested(&mut self.prefix_lengths_psr4, first_byte_key(prefix))
                .insert(PhpKey::from(prefix), PhpValue::Int(length));
            self.prefix_dirs_psr4
                .insert(PhpKey::from(prefix), PhpValue::Array(list(paths)));
        }
    }

    pub(crate) fn add_class(&mut self, class: &str, path: &str) {
        self.class_map
            .insert(PhpKey::from(class), PhpValue::String(path.to_owned()));
    }

    fn maps(self) -> Vec<(&'static str, PhpArray)> {
        let mut maps = Vec::new();
        if let Some(files) = self.files {
            maps.push(("files", files));
        }
        for (name, map) in [
            ("prefixLengthsPsr4", self.prefix_lengths_psr4),
            ("prefixDirsPsr4", self.prefix_dirs_psr4),
            ("fallbackDirsPsr4", self.fallback_dirs_psr4),
            ("prefixesPsr0", self.prefixes_psr0),
            ("fallbackDirsPsr0", self.fallback_dirs_psr0),
            ("classMap", self.class_map),
        ] {
            if !map.is_empty() {
                maps.push((name, map));
            }
        }
        maps
    }
}

fn absolute_code(dir: &str) -> String {
    let exported = var_export_str(&format!("{}/", dir.trim_end_matches(['\\', '/'])));
    format!(" => {}", &exported[..exported.len() - 1])
}

/// `autoload_static.php`. `target_dir` is `vendor/composer`, `vendor` and
/// `base` the runtime values of `$vendorDir` and `$baseDir`.
// Composer: Autoload/AutoloadGenerator.php getStaticFile
pub(crate) fn static_file(
    suffix: &str,
    loader: Loader,
    target_dir: &str,
    vendor: &str,
    base: &str,
) -> String {
    let vendor_code = find_shortest_path_code(target_dir, vendor, true, true).code;
    let base_code = find_shortest_path_code(target_dir, base, true, true).code;
    let pairs = [
        (absolute_code(vendor), format!(" => {vendor_code} . '/")),
        (
            absolute_code(&format!("phar://{vendor}")),
            format!(" => 'phar://' . {vendor_code} . '/"),
        ),
        (absolute_code(base), format!(" => {base_code} . '/")),
        (
            absolute_code(&format!("phar://{base}")),
            format!(" => 'phar://' . {base_code} . '/"),
        ),
    ];
    let mut file = format!(
        "<?php\n\n// autoload_static.php @generated by Composer\n\nnamespace Composer\\Autoload;\n\nclass ComposerStaticInit{suffix}\n{{\n"
    );
    let mut initializer = String::new();
    for (prop, value) in loader.maps() {
        let exported = strtr(&var_export(&PhpValue::Array(value)), &pairs);
        let _ = write!(
            file,
            "    public static ${prop} = {};\n\n",
            reindent(&exported)
        );
        if prop != "files" {
            let _ = writeln!(
                initializer,
                "            $loader->{prop} = ComposerStaticInit{suffix}::${prop};"
            );
        }
    }
    let _ = writeln!(
        file,
        "    public static function getInitializer(ClassLoader $loader)\n    {{\n        return \\Closure::bind(function () use ($loader) {{\n{initializer}\n        }}, null, ClassLoader::class);\n    }}\n}}"
    );
    file
}

#[cfg(test)]
mod tests {
    use super::{Loader, reindent, static_file, strtr};

    #[test]
    fn strtr_prefers_longest_and_never_rescans() {
        let pairs = [
            ("ab".to_owned(), "x".to_owned()),
            ("abc".to_owned(), "ab".to_owned()),
            ("b".to_owned(), "B".to_owned()),
        ];
        assert_eq!(strtr("abcab b", &pairs), "abx B");
        assert_eq!(strtr("é", &pairs), "é");
    }

    #[test]
    fn reindents_like_composer() {
        assert_eq!(
            reindent("array (\n  'a' => \n  array (\n    0 => 'x',\n  ),\n)"),
            "array (\n        'a' =>\n        array (\n            0 => 'x',\n        ),\n    )"
        );
    }

    #[test]
    fn writes_static_file() {
        let mut loader = Loader::default();
        loader.set_files(&[("abc".into(), "/p/vendor/a/b/f.php".into())]);
        loader.set_psr4("App\\", &["/p/app".into()]);
        loader.set_psr4("", &["/p/fallback".into()]);
        loader.set_psr0("Old_", &["/p/vendor/o/old/lib.phar/src".into()]);
        loader.set_psr0("0", &["/elsewhere".into()]);
        loader.add_class("A\\B", "phar:///p/x.phar/B.php");
        let out = static_file("abc123", loader, "/p/vendor/composer", "/p/vendor", "/p");
        let expected = r"<?php

// autoload_static.php @generated by Composer

namespace Composer\Autoload;

class ComposerStaticInitabc123
{
    public static $files = array (
        'abc' => __DIR__ . '/..' . '/a/b/f.php',
    );

    public static $prefixLengthsPsr4 = array (
        'A' =>
        array (
            'App\\' => 4,
        ),
    );

    public static $prefixDirsPsr4 = array (
        'App\\' =>
        array (
            0 => __DIR__ . '/../..' . '/app',
        ),
    );

    public static $fallbackDirsPsr4 = array (
        0 => __DIR__ . '/../..' . '/fallback',
    );

    public static $prefixesPsr0 = array (
        'O' =>
        array (
            'Old_' =>
            array (
                0 => __DIR__ . '/..' . '/o/old/lib.phar/src',
            ),
        ),
    );

    public static $fallbackDirsPsr0 = array (
        0 => '/elsewhere',
    );

    public static $classMap = array (
        'A\\B' => 'phar://' . __DIR__ . '/../..' . '/x.phar/B.php',
    );

    public static function getInitializer(ClassLoader $loader)
    {
        return \Closure::bind(function () use ($loader) {
            $loader->prefixLengthsPsr4 = ComposerStaticInitabc123::$prefixLengthsPsr4;
            $loader->prefixDirsPsr4 = ComposerStaticInitabc123::$prefixDirsPsr4;
            $loader->fallbackDirsPsr4 = ComposerStaticInitabc123::$fallbackDirsPsr4;
            $loader->prefixesPsr0 = ComposerStaticInitabc123::$prefixesPsr0;
            $loader->fallbackDirsPsr0 = ComposerStaticInitabc123::$fallbackDirsPsr0;
            $loader->classMap = ComposerStaticInitabc123::$classMap;

        }, null, ClassLoader::class);
    }
}
";
        assert_eq!(out, expected);
    }
}
