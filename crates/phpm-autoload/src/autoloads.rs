use crate::package::Package;
use crate::paths::{preg_quote, real_path};
use indexmap::IndexMap;
use phpm_php::{PhpArray, PhpKey, PhpValue, smart_strcmp};
use std::path::Path;

/// One row of Composer's package map: the package and where it is
/// installed, `Some("")` for the root package and `None` for metapackages.
#[derive(Debug, Clone)]
pub(crate) struct Entry<'a> {
    pub(crate) package: &'a Package,
    pub(crate) install_path: Option<String>,
    pub(crate) is_root: bool,
}

/// What `parseAutoloads` returns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Autoloads {
    pub(crate) psr0: Vec<(PhpKey, Vec<String>)>,
    pub(crate) psr4: Vec<(PhpKey, Vec<String>)>,
    pub(crate) classmap: Vec<String>,
    pub(crate) files: IndexMap<String, String>,
    pub(crate) exclude: Vec<String>,
}

pub(crate) fn key_string(key: &PhpKey) -> String {
    match key {
        PhpKey::Int(i) => i.to_string(),
        PhpKey::String(s) => s.clone(),
    }
}

/// PHP's `(string)` cast of a scalar.
fn php_string(value: &PhpValue) -> Option<String> {
    match value {
        PhpValue::String(s) => Some(s.clone()),
        PhpValue::Int(i) => Some(i.to_string()),
        PhpValue::Bool(true) => Some("1".to_owned()),
        PhpValue::Bool(false) | PhpValue::Null => Some(String::new()),
        PhpValue::Array(_) => None,
    }
}

/// PHP `empty()` on a string.
fn empty(s: &str) -> bool {
    s.is_empty() || s == "0"
}

fn append(target: &mut PhpArray, value: PhpValue) {
    let next = target
        .keys()
        .filter_map(|k| match k {
            PhpKey::Int(i) => Some(*i + 1),
            PhpKey::String(_) => None,
        })
        .max()
        .unwrap_or(0)
        .max(0);
    target.insert(PhpKey::Int(next), value);
}

fn to_array(value: PhpValue) -> PhpArray {
    match value {
        PhpValue::Array(a) => a,
        scalar => [(PhpKey::Int(0), scalar)].into_iter().collect(),
    }
}

// php-src: ext/standard/array.c php_array_merge_recursive
fn merge_into(dest: &mut PhpArray, src: &PhpArray) {
    for (key, value) in src {
        match key {
            PhpKey::Int(_) => append(dest, value.clone()),
            PhpKey::String(_) => match dest.get_mut(key) {
                Some(existing) => {
                    let mut merged = to_array(std::mem::replace(existing, PhpValue::Null));
                    match value {
                        PhpValue::Array(inner) => merge_into(&mut merged, inner),
                        scalar => append(&mut merged, scalar.clone()),
                    }
                    *existing = PhpValue::Array(merged);
                }
                None => {
                    dest.insert(key.clone(), value.clone());
                }
            },
        }
    }
}

/// PHP `array_merge_recursive($a, $b)`.
pub(crate) fn array_merge_recursive(a: &PhpArray, b: &PhpArray) -> PhpArray {
    let mut out = PhpArray::new();
    merge_into(&mut out, a);
    merge_into(&mut out, b);
    out
}

fn readable(path: &str) -> bool {
    std::fs::metadata(path).is_ok()
}

// Composer: Autoload/AutoloadGenerator.php parseAutoloadsType, exclude-from-classmap branch
fn exclusion_pattern(path: &str, install_path: &str) -> Option<String> {
    let quoted = preg_quote(path.replace('\\', "/").trim_matches('/'));
    let mut collapsed = String::with_capacity(quoted.len());
    for c in quoted.chars() {
        if !(c == '/' && collapsed.ends_with('/')) {
            collapsed.push(c);
        }
    }
    let pattern = collapsed.replace("\\*\\*", ".+?").replace("\\*", "[^/]+?");
    let mut rest = pattern.as_str();
    let mut updir = String::new();
    loop {
        let step = ["\\.\\./", "\\./"]
            .into_iter()
            .find(|p| rest.starts_with(p));
        let Some(step) = step else { break };
        updir.push_str(&step.replace("\\.", "."));
        rest = &rest[step.len()..];
    }
    let resolved = real_path(Path::new(&format!("{install_path}/{updir}")))?;
    Some(format!("{}/{rest}($|/)", preg_quote(&resolved)))
}

fn target_dir_regex(target: &str) -> String {
    preg_quote(&target.replace(['/', '\\'], "<dirsep>")).replace("\\<dirsep\\>", "[\\\\/]")
}

fn strip_target_dir(path: &str, target: &str) -> String {
    let trimmed = path.trim_start_matches(['\\', '/']);
    let pattern = format!("^{}", target_dir_regex(target));
    let stripped = regex::bytes::RegexBuilder::new(&pattern)
        .unicode(false)
        .build()
        .ok()
        .map_or_else(
            || trimmed.to_owned(),
            |re| String::from_utf8_lossy(&re.replace(trimmed.as_bytes(), &b""[..])).into_owned(),
        );
    stripped.trim_start_matches(['\\', '/']).to_owned()
}

/// `parseAutoloadsType` for one type over the package map, in map order.
fn parse_type(entries: &[&Entry<'_>], kind: &str, dev_mode: bool, cwd: &str, out: &mut Autoloads) {
    for entry in entries {
        let Some(install) = &entry.install_path else {
            continue;
        };
        let package = entry.package;
        let autoload = if dev_mode && entry.is_root {
            array_merge_recursive(&package.autoload, &package.dev_autoload)
        } else {
            package.autoload.clone()
        };
        let Some(PhpValue::Array(section)) = autoload.get(&PhpKey::from(kind)) else {
            continue;
        };
        let mut install_path = install.clone();
        if let Some(target) = &package.target_dir
            && !entry.is_root
        {
            install_path.truncate(install_path.len().saturating_sub(target.len() + 1));
        }
        for (namespace, paths) in section {
            let mut namespace = key_string(namespace);
            if kind == "psr-0" || kind == "psr-4" {
                namespace = namespace.trim_start_matches('\\').to_owned();
            }
            let paths: Vec<String> = match paths {
                PhpValue::Array(items) => items.values().filter_map(php_string).collect(),
                scalar => php_string(scalar).into_iter().collect(),
            };
            for mut path in paths {
                if matches!(kind, "files" | "classmap" | "exclude-from-classmap")
                    && let Some(target) = package.target_dir.as_deref().filter(|t| !empty(t))
                    && !readable(&format!("{install_path}/{path}"))
                {
                    path = if entry.is_root {
                        strip_target_dir(&path, target)
                    } else {
                        format!("{target}/{path}")
                    };
                }
                if kind == "exclude-from-classmap" {
                    if empty(&install_path) {
                        install_path = cwd.replace('\\', "/");
                    }
                    if let Some(pattern) = exclusion_pattern(&path, &install_path) {
                        out.exclude.push(pattern);
                    }
                    continue;
                }
                let relative = if empty(&install_path) {
                    if empty(&path) {
                        ".".to_owned()
                    } else {
                        path.clone()
                    }
                } else {
                    format!("{install_path}/{path}")
                };
                match kind {
                    "files" => {
                        let id = format!("{:x}", md5::compute(format!("{}:{path}", package.name)));
                        out.files.insert(id, relative);
                    }
                    "classmap" => out.classmap.push(relative),
                    _ => {
                        let key = PhpKey::from(namespace.clone());
                        let list = if kind == "psr-0" {
                            &mut out.psr0
                        } else {
                            &mut out.psr4
                        };
                        match list.iter_mut().find(|(k, _)| *k == key) {
                            Some((_, dirs)) => dirs.push(relative),
                            None => list.push((key, vec![relative])),
                        }
                    }
                }
            }
        }
    }
}

fn krsort(list: &mut [(PhpKey, Vec<String>)]) {
    list.sort_by(|(a, _), (b, _)| smart_strcmp(&key_string(b), &key_string(a)));
}

/// `parseAutoloads` on a package map already sorted by `PackageSorter`,
/// root last.
// Composer: Autoload/AutoloadGenerator.php parseAutoloads
pub(crate) fn parse_autoloads(sorted: &[Entry<'_>], dev_mode: bool, cwd: &str) -> Autoloads {
    let forward: Vec<&Entry<'_>> = sorted.iter().collect();
    let reverse: Vec<&Entry<'_>> = sorted.iter().rev().collect();
    let mut out = Autoloads::default();
    parse_type(&reverse, "psr-0", dev_mode, cwd, &mut out);
    parse_type(&reverse, "psr-4", dev_mode, cwd, &mut out);
    parse_type(&reverse, "classmap", dev_mode, cwd, &mut out);
    parse_type(&forward, "files", dev_mode, cwd, &mut out);
    parse_type(&forward, "exclude-from-classmap", dev_mode, cwd, &mut out);
    krsort(&mut out.psr0);
    krsort(&mut out.psr4);
    out
}

#[cfg(test)]
mod tests {
    use super::{
        Entry, array_merge_recursive, exclusion_pattern, parse_autoloads, strip_target_dir,
    };
    use crate::package::{Package, to_php};
    use phpm_php::{PhpArray, PhpKey, PhpValue};
    use serde_json::{Value, json};

    fn arr(v: &Value) -> PhpArray {
        match to_php(v) {
            PhpValue::Array(a) => a,
            _ => unreachable!(),
        }
    }

    #[test]
    fn merges_recursively_like_php() {
        let merged = array_merge_recursive(
            &arr(
                &json!({"psr-4": {"App\\": "app/", "X\\": ["x/"]}, "classmap": ["a"], "files": ["f"]}),
            ),
            &arr(
                &json!({"psr-4": {"App\\": "tests/", "X\\": "y/", "T\\": "t/"}, "classmap": ["b"]}),
            ),
        );
        let expected = arr(&json!({
            "psr-4": {"App\\": ["app/", "tests/"], "X\\": ["x/", "y/"], "T\\": "t/"},
            "classmap": ["a", "b"],
            "files": ["f"]
        }));
        assert_eq!(merged, expected);
        let lists = array_merge_recursive(&arr(&json!([1, 2])), &arr(&json!({"k": null, "0": 3})));
        let keys: Vec<PhpKey> = lists.keys().cloned().collect();
        assert_eq!(
            keys,
            [
                PhpKey::Int(0),
                PhpKey::Int(1),
                PhpKey::from("k"),
                PhpKey::Int(2)
            ]
        );
        let null = array_merge_recursive(&arr(&json!({"k": null})), &arr(&json!({"k": "v"})));
        assert_eq!(null, arr(&json!({"k": [null, "v"]})));
    }

    fn package(name: &str, autoload: &Value, dev: &Value) -> Package {
        Package {
            name: name.into(),
            pretty_name: name.into(),
            autoload: arr(autoload),
            dev_autoload: arr(dev),
            ..Package::default()
        }
    }

    #[test]
    fn parses_like_composer() {
        let root = package(
            "__root__",
            &json!({"psr-4": {"App\\": "app/", "\\Lead\\": ""}, "files": ["helpers.php"], "psr-0": {"": "lib/"}}),
            &json!({"psr-4": {"Tests\\": "tests/"}, "classmap": ["database/"]}),
        );
        let lib = package(
            "a/lib",
            &json!({"psr-4": {"A\\": ["src/", "more/"]}, "files": ["boot.php"], "classmap": ["Resources/stubs"], "psr-0": {"A_": "old/"}}),
            &json!({}),
        );
        let meta = package("a/meta", &json!({"psr-4": {"M\\": "m/"}}), &json!({}));
        let sorted = [
            Entry {
                package: &lib,
                install_path: Some("/p/vendor/a/lib".into()),
                is_root: false,
            },
            Entry {
                package: &meta,
                install_path: None,
                is_root: false,
            },
            Entry {
                package: &root,
                install_path: Some(String::new()),
                is_root: true,
            },
        ];
        let a = parse_autoloads(&sorted, true, "/p");
        let psr4: Vec<(String, Vec<String>)> = a
            .psr4
            .iter()
            .map(|(k, v)| (super::key_string(k), v.clone()))
            .collect();
        assert_eq!(
            psr4,
            [
                ("Tests\\".to_owned(), vec!["tests/".to_owned()]),
                ("Lead\\".to_owned(), vec![".".to_owned()]),
                ("App\\".to_owned(), vec!["app/".to_owned()]),
                (
                    "A\\".to_owned(),
                    vec![
                        "/p/vendor/a/lib/src/".to_owned(),
                        "/p/vendor/a/lib/more/".to_owned()
                    ]
                ),
            ]
        );
        assert_eq!(a.psr0.len(), 2);
        assert_eq!(a.classmap, ["database/", "/p/vendor/a/lib/Resources/stubs"]);
        let files: Vec<(&str, &str)> = a
            .files
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(
            files,
            [
                (
                    format!("{:x}", md5::compute("a/lib:boot.php")).as_str(),
                    "/p/vendor/a/lib/boot.php"
                ),
                (
                    format!("{:x}", md5::compute("__root__:helpers.php")).as_str(),
                    "helpers.php"
                ),
            ]
        );
        assert_eq!(parse_autoloads(&sorted, false, "/p").classmap.len(), 1);
    }

    #[test]
    fn target_dir_paths() {
        let mut lib = package(
            "a/old",
            &json!({"psr-0": {"Old_": ""}, "classmap": ["x.php"]}),
            &json!({}),
        );
        lib.target_dir = Some("Old/Lib".into());
        let sorted = [Entry {
            package: &lib,
            install_path: Some("/p/vendor/a/old/Old/Lib".into()),
            is_root: false,
        }];
        let a = parse_autoloads(&sorted, false, "/p");
        assert_eq!(a.psr0[0].1, ["/p/vendor/a/old/"]);
        assert_eq!(a.classmap, ["/p/vendor/a/old/Old/Lib/x.php"]);
        assert_eq!(strip_target_dir("/Old/Lib/x.php", "Old/Lib"), "x.php");
        assert_eq!(strip_target_dir("y.php", "Old/Lib"), "y.php");
    }

    #[test]
    fn exclusion_patterns_like_composer() {
        let dir = tempfile::tempdir().unwrap();
        let root = crate::paths::real_path(dir.path()).unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let quoted = crate::paths::preg_quote(&root);
        assert_eq!(
            exclusion_pattern("/Tests//**/*.php", &root).unwrap(),
            format!("{quoted}/Tests/.+?/[^/]+?\\.php($|/)")
        );
        assert_eq!(
            exclusion_pattern("../x", &format!("{root}/sub")).unwrap(),
            format!("{quoted}/x($|/)")
        );
        assert!(exclusion_pattern("x", &format!("{root}/missing")).is_none());

        let mut p = package(
            "__root__",
            &json!({"exclude-from-classmap": ["/tests/"]}),
            &json!({}),
        );
        p.target_dir = None;
        let sorted = [Entry {
            package: &p,
            install_path: Some(String::new()),
            is_root: true,
        }];
        let a = parse_autoloads(&sorted, false, &root);
        assert_eq!(a.exclude, [format!("{quoted}/tests($|/)")]);
    }
}
