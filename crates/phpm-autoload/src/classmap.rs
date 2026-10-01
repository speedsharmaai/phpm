use crate::Error;
use crate::autoloads::{Autoloads, key_string};
use crate::paths::{preg_quote, real_path};
use crate::scan::{find_classes, raw_bytes};
use indexmap::IndexMap;
use phpm_lock::normalize_path;
use phpm_php::{is_absolute_path, smart_strcmp};
use rayon::prelude::*;
use regex::bytes::{Regex, RegexBuilder};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const EXTENSIONS: [&str; 3] = ["php", "inc", "hh"];
const VCS_DIRS: [&str; 9] = [
    ".svn",
    "_svn",
    "CVS",
    "_darcs",
    ".arch-params",
    ".monotone",
    ".bzr",
    ".git",
    ".hg",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rule {
    Classmap,
    Psr0,
    Psr4,
}

impl Rule {
    fn name(self) -> &'static str {
        match self {
            Self::Classmap => "classmap",
            Self::Psr0 => "psr-0",
            Self::Psr4 => "psr-4",
        }
    }
}

/// One `scanPaths()` call.
struct Job {
    path: String,
    rule: Rule,
    namespace: String,
    excluded: Option<Regex>,
}

struct Candidate {
    file_path: String,
    real_path: String,
}

/// Class name to file path, first found wins, as composer/class-map-generator
/// collects it.
#[derive(Debug, Default)]
pub(crate) struct ClassMap {
    map: IndexMap<String, String>,
    ambiguous: IndexMap<String, Vec<String>>,
    violations: IndexMap<String, Vec<String>>,
}

/// The constant prefix of a PCRE pattern, escapes kept.
fn constant_prefix(pattern: &str) -> &str {
    const SPECIAL: &[u8] = b".+*?[^]$(){}=!<>|:\\#-";
    const ESCAPABLE: &[u8] = b".+*?[^]$(){}=!<>|:#-";
    let b = pattern.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if !SPECIAL.contains(&b[i]) {
            i += 1;
        } else if b[i] == b'\\' && b.get(i + 1).is_some_and(|c| ESCAPABLE.contains(c)) {
            i += 2;
        } else {
            break;
        }
    }
    &pattern[..i]
}

/// A PCRE pattern made of `preg_quote` output, `.+?`, `[^/]+?` and
/// `($|/)` pieces, in the regex crate's syntax.
fn pcre_to_regex(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('0') if chars.peek() == Some(&'0') => {
                chars.next();
                chars.next();
                out.push_str("\\x00");
            }
            Some(e) if e.is_ascii_punctuation() => out.push_str(&regex::escape(&e.to_string())),
            Some(e) => {
                out.push('\\');
                out.push(e);
            }
            None => out.push_str("\\\\"),
        }
    }
    out
}

fn compile(patterns: &[String]) -> Option<Regex> {
    if patterns.is_empty() {
        return None;
    }
    let joined: Vec<String> = patterns.iter().map(|p| pcre_to_regex(p)).collect();
    RegexBuilder::new(&format!("({})", joined.join("|")))
        .unicode(false)
        .build()
        .ok()
}

fn fs_path(base: &str, path: &str) -> PathBuf {
    if is_absolute_path(path) {
        PathBuf::from(path)
    } else {
        Path::new(base).join(path)
    }
}

// Composer: Autoload/AutoloadGenerator.php buildExclusionRegex
fn exclusion_regex(base: &str, dir: &str, excluded: &[String]) -> Option<Regex> {
    if excluded.is_empty() {
        return None;
    }
    let fs = fs_path(base, dir);
    let mut kept: Vec<String> = excluded.to_vec();
    if fs.exists()
        && let Some(real) = real_path(&fs)
    {
        let dir_match = preg_quote(&real);
        let abs = if is_absolute_path(dir) {
            dir.to_owned()
        } else {
            format!("{base}/{dir}")
        };
        let dir_normalized = preg_quote(&normalize_path(&abs));
        let is_symlink = dir_match != dir_normalized;
        kept.retain(|pattern| {
            let prefix = constant_prefix(pattern);
            let related = |d: &str| prefix.starts_with(d) || d.starts_with(prefix);
            related(&dir_match) || (is_symlink && related(&dir_normalized))
        });
    }
    compile(&kept)
}

fn extension(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rfind('.').map_or("", |i| &name[i + 1..])
}

/// `preg_replace('{(?<!:)[\\\\/]{2,}}', '/', $path)`
fn collapse_slashes(path: &str) -> String {
    let b = path.as_bytes();
    let slash = |i: usize| matches!(b.get(i), Some(b'/' | b'\\'));
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if slash(i) && slash(i + 1) && (i == 0 || b[i - 1] != b':') {
            while slash(i) {
                i += 1;
            }
            out.push(b'/');
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| path.to_owned())
}

/// A file Finder yields: the path as Composer sees it and, when known
/// without asking the OS, its real path.
struct Found {
    path: String,
    real: Option<String>,
}

/// Symfony Finder's `files()->followLinks()->in($dir)` with default
/// ignores, in readdir order: dot files and VCS directories skipped.
fn finder(base: &str, dir: &str, out: &mut Vec<Found>) -> Result<(), Error> {
    let root = if dir == "/" {
        dir
    } else {
        dir.trim_end_matches('/')
    };
    let fs = fs_path(base, root);
    let real = real_path(&fs);
    walk(&fs, root, real.as_deref(), out)
}

pub(crate) fn finder_paths(dir: &str) -> Result<Vec<String>, Error> {
    let mut out = Vec::new();
    finder("/", dir, &mut out)?;
    Ok(out.into_iter().map(|f| f.path).collect())
}

fn walk(fs: &Path, path: &str, real: Option<&str>, out: &mut Vec<Found>) -> Result<(), Error> {
    let scan_error = |reason: String| Error::Scan {
        path: path.to_owned(),
        reason,
    };
    let entries = std::fs::read_dir(fs).map_err(|e| scan_error(e.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|e| scan_error(e.to_string()))?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| scan_error("a file name is not UTF-8".to_owned()))?;
        let join = |dir: &str| {
            if dir.ends_with('/') {
                format!("{dir}{name}")
            } else {
                format!("{dir}/{name}")
            }
        };
        let child = join(path);
        let child_fs = entry.path();
        let file_type = entry.file_type().map_err(|e| scan_error(e.to_string()))?;
        let (is_dir, child_real) = if file_type.is_symlink() {
            (child_fs.is_dir(), real_path(&child_fs))
        } else {
            (file_type.is_dir(), real.map(join))
        };
        let dotted = name.starts_with('.') && name.len() > 1;
        if is_dir {
            if dotted || VCS_DIRS.contains(&name) {
                continue;
            }
            walk(&child_fs, &child, child_real.as_deref(), out)?;
        } else if !dotted && EXTENSIONS.iter().any(|e| name.ends_with(&format!(".{e}"))) {
            out.push(Found {
                path: child,
                real: child_real,
            });
        }
    }
    Ok(())
}

fn candidates(base: &str, job: &Job) -> Result<Vec<Candidate>, Error> {
    let fs = fs_path(base, &job.path);
    let mut files = Vec::new();
    if fs.is_file() {
        files.push(Found {
            path: job.path.clone(),
            real: None,
        });
    } else if fs.is_dir() {
        finder(base, &job.path, &mut files)?;
    } else if job.path.contains('*') {
        return Err(Error::Unsupported(format!("classmap glob {}", job.path)));
    } else {
        return Err(Error::Scan {
            path: job.path.clone(),
            reason: "it does not appear to be a file nor a folder".to_owned(),
        });
    }
    let mut out = Vec::with_capacity(files.len());
    for Found { path: file, real } in files {
        if !EXTENSIONS.contains(&extension(&file)) {
            continue;
        }
        if file.contains("://") {
            return Err(Error::Unsupported(format!("stream wrapper path {file}")));
        }
        let file_path = if is_absolute_path(&file) {
            collapse_slashes(&file)
        } else {
            normalize_path(&format!("{base}/{file}"))
        };
        let real = real
            .or_else(|| real_path(Path::new(&file_path)))
            .ok_or_else(|| Error::Scan {
                path: file_path.clone(),
                reason: "realpath failed".to_owned(),
            })?;
        if let Some(excluded) = &job.excluded
            && (excluded.is_match(real.as_bytes()) || excluded.is_match(file_path.as_bytes()))
        {
            continue;
        }
        out.push(Candidate {
            file_path,
            real_path: real,
        });
    }
    Ok(out)
}

fn short_path(cwd: &str, path: &str) -> String {
    let path = normalize_path(path);
    match path.strip_prefix(cwd) {
        Some(rest) => format!(".{rest}"),
        None => path,
    }
}

impl ClassMap {
    /// Scans the classmap rules, and the PSR-0/PSR-4 directories when
    /// optimizing, the way `AutoloadGenerator::dump` drives
    /// `ClassMapGenerator`.
    pub(crate) fn scan(
        autoloads: &Autoloads,
        optimize: bool,
        base: &str,
        vendor: &str,
    ) -> Result<Self, Error> {
        let excluded = &autoloads.exclude;
        let mut jobs: Vec<Job> = autoloads
            .classmap
            .iter()
            .map(|dir| Job {
                path: dir.clone(),
                rule: Rule::Classmap,
                namespace: String::new(),
                excluded: exclusion_regex(base, dir, excluded),
            })
            .collect();
        if optimize {
            type Groups<'a> = Vec<(Rule, &'a Vec<String>)>;
            let mut namespaces: Vec<(String, Groups<'_>)> = Vec::new();
            for (rule, list) in [(Rule::Psr4, &autoloads.psr4), (Rule::Psr0, &autoloads.psr0)] {
                for (namespace, paths) in list {
                    let namespace = key_string(namespace);
                    match namespaces.iter_mut().find(|(n, _)| *n == namespace) {
                        Some((_, groups)) => groups.push((rule, paths)),
                        None => namespaces.push((namespace, vec![(rule, paths)])),
                    }
                }
            }
            namespaces.sort_by(|(a, _), (b, _)| smart_strcmp(b, a));
            for (namespace, groups) in namespaces {
                for (rule, paths) in groups {
                    for dir in paths {
                        let dir = normalize_path(&if is_absolute_path(dir) {
                            dir.clone()
                        } else {
                            format!("{base}/{dir}")
                        });
                        if !Path::new(&dir).is_dir() {
                            continue;
                        }
                        let excluded = if vendor.contains(&format!("{dir}/")) {
                            let mut with_vendor = excluded.clone();
                            with_vendor.push(format!("{vendor}/"));
                            exclusion_regex(base, &dir, &with_vendor)
                        } else {
                            exclusion_regex(base, &dir, excluded)
                        };
                        jobs.push(Job {
                            path: dir,
                            rule,
                            namespace: namespace.clone(),
                            excluded,
                        });
                    }
                }
            }
        }

        let found: Vec<Vec<Candidate>> = jobs
            .par_iter()
            .map(|job| candidates(base, job))
            .collect::<Result<_, _>>()?;
        let mut unique: Vec<(&str, &str)> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        for c in found.iter().flatten() {
            if seen.insert(c.real_path.as_str()) {
                unique.push((c.real_path.as_str(), c.file_path.as_str()));
            }
        }
        let parsed: HashMap<&str, Result<Vec<String>, String>> = unique
            .par_iter()
            .map(|&(real, file)| {
                let classes = std::fs::read(file)
                    .map(|source| find_classes(&source, true))
                    .map_err(|e| e.to_string());
                (real, classes)
            })
            .collect();

        let mut map = Self::default();
        let mut scanned: HashSet<&str> = HashSet::new();
        for (job, found) in jobs.iter().zip(&found) {
            for c in found {
                if scanned.contains(c.real_path.as_str()) {
                    continue;
                }
                let classes = match parsed.get(c.real_path.as_str()) {
                    Some(Ok(classes)) => classes.clone(),
                    Some(Err(reason)) => {
                        return Err(Error::Scan {
                            path: c.file_path.clone(),
                            reason: reason.clone(),
                        });
                    }
                    None => Vec::new(),
                };
                let classes = if job.rule == Rule::Classmap {
                    scanned.insert(c.real_path.as_str());
                    classes
                } else {
                    let valid = map.filter_by_namespace(classes, &c.file_path, job, base);
                    if !valid.is_empty() {
                        scanned.insert(c.real_path.as_str());
                    }
                    valid
                };
                for class in classes {
                    match map.map.get(&class) {
                        None => map.add_class(&class, &c.file_path),
                        Some(existing) if *existing != c.file_path => {
                            map.ambiguous
                                .entry(class)
                                .or_default()
                                .push(c.file_path.clone());
                        }
                        Some(_) => {}
                    }
                }
            }
        }
        let vendor_prefix = vendor.trim_end_matches('/');
        map.violations.retain(|path, _| {
            path != vendor_prefix && !path.starts_with(&format!("{vendor_prefix}/"))
        });
        Ok(map)
    }

    // composer/class-map-generator: src/ClassMapGenerator.php filterByNamespace
    fn filter_by_namespace(
        &mut self,
        classes: Vec<String>,
        file_path: &str,
        job: &Job,
        cwd: &str,
    ) -> Vec<String> {
        let sub = file_path.get(job.path.len() + 1..).unwrap_or("");
        let real_sub_path = sub.rfind('.').map_or(sub, |i| &sub[..i]);
        let base_ns = job.namespace.as_str();
        let mut valid = Vec::new();
        let mut rejected = Vec::new();
        for class in classes {
            let sub_path = if job.rule == Rule::Psr0 {
                if !base_ns.is_empty() && !class.starts_with(base_ns) {
                    rejected.push(class);
                    continue;
                }
                match class.rfind('\\') {
                    Some(i) => format!(
                        "{}{}",
                        class[..=i].replace('\\', "/"),
                        class[i + 1..].replace('_', "/")
                    ),
                    None => class.replace('_', "/"),
                }
            } else {
                let sub_namespace = if base_ns.is_empty() {
                    class.as_str()
                } else {
                    class.get(base_ns.len()..).unwrap_or("")
                };
                sub_namespace.replace('\\', "/")
            };
            if sub_path == real_sub_path {
                valid.push(class);
            } else {
                rejected.push(class);
            }
        }
        if valid.is_empty() {
            let short = short_path(cwd, file_path);
            let short_base = short_path(cwd, &job.path);
            let key = file_path
                .replace('\\', "/")
                .trim_end_matches('/')
                .to_owned();
            for class in rejected {
                self.violations.entry(key.clone()).or_default().push(format!(
                    "Class {class} located in {short} does not comply with {} autoloading standard (rule: {base_ns} => {short_base}). Skipping.",
                    job.rule.name()
                ));
            }
        }
        valid
    }

    pub(crate) fn add_class(&mut self, class: &str, path: &str) {
        self.violations.shift_remove(&path.replace('\\', "/"));
        self.map.insert(class.to_owned(), path.to_owned());
    }

    pub(crate) fn sort(&mut self) {
        if self.map.keys().all(|k| raw_bytes(k).len() == k.len()) {
            self.map.sort_by(|a, _, b, _| smart_strcmp(a, b));
        } else {
            self.map.sort_by_cached_key(|k, _| raw_bytes(k));
        }
    }

    pub(crate) fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.map.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Ambiguous classes and PSR violations outside vendor, worded as
    /// Composer prints them.
    // Composer: Autoload/AutoloadGenerator.php dump, ambiguous class warnings
    pub(crate) fn warnings(self) -> Vec<String> {
        let mut out = Vec::new();
        for (class, paths) in &self.ambiguous {
            let paths: Vec<&String> = paths
                .iter()
                .filter(|p| !is_test_path(&p.replace('\\', "/")))
                .collect();
            let first = self.map.get(class).map_or("", String::as_str);
            let joined = paths
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join("\", \"");
            match paths.len() {
                0 => {}
                1 => out.push(format!(
                    "Warning: Ambiguous class resolution, \"{class}\" was found in both \"{first}\" and \"{joined}\", the first will be used."
                )),
                n => out.push(format!(
                    "Warning: Ambiguous class resolution, \"{class}\" was found {}x: in \"{first}\" and \"{joined}\", the first will be used.",
                    n + 1
                )),
            }
        }
        out.extend(self.violations.into_values().flatten());
        out
    }
}

/// `{/(test|fixture|example|stub)s?/}i`
fn is_test_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    ["test", "fixture", "example", "stub"].iter().any(|word| {
        lower.match_indices(&format!("/{word}")).any(|(i, m)| {
            let rest = &lower[i + m.len()..];
            rest.starts_with('/') || rest.starts_with("s/")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ClassMap, collapse_slashes, constant_prefix, extension, is_test_path, pcre_to_regex,
    };
    use crate::autoloads::Autoloads;
    use crate::paths::{preg_quote, real_path};
    use phpm_php::PhpKey;
    use std::path::Path;

    fn write(root: &Path, rel: &str, body: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn helpers_match_php() {
        assert_eq!(constant_prefix("/a/b\\.c/.+?x"), "/a/b\\.c/");
        assert_eq!(constant_prefix("/a\\-b/[^/]+?"), "/a\\-b/");
        assert_eq!(
            pcre_to_regex("/a\\-b\\<c\\>/x\\000($|/)"),
            "/a\\-b<c>/x\\x00($|/)"
        );
        assert_eq!(extension("/a/b.c/file.php"), "php");
        assert_eq!(extension("/a/b.c/file"), "");
        assert_eq!(collapse_slashes("/a//b///c"), "/a/b/c");
        assert_eq!(collapse_slashes("phar://x//y"), "phar://x/y");
        assert!(is_test_path("/a/Tests/b.php"));
        assert!(is_test_path("/a/stub/b.php"));
        assert!(!is_test_path("/a/testing/b.php"));
    }

    #[test]
    fn scans_classmap_and_psr_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = real_path(dir.path()).unwrap();
        write(
            dir.path(),
            "vendor/a/lib/src/Foo.php",
            "<?php namespace A; class Foo {}",
        );
        write(
            dir.path(),
            "vendor/a/lib/src/Sub/Bar.php",
            "<?php namespace A\\Sub; class Bar {} class Extra {}",
        );
        write(
            dir.path(),
            "vendor/a/lib/src/Wrong.php",
            "<?php namespace B; class Wrong {}",
        );
        write(
            dir.path(),
            "vendor/a/lib/src/.hidden/H.php",
            "<?php namespace A\\H; class H {}",
        );
        write(
            dir.path(),
            "vendor/a/lib/src/.git/G.php",
            "<?php class G {}",
        );
        write(dir.path(), "vendor/a/lib/src/notes.txt", "class Nope {}");
        write(
            dir.path(),
            "vendor/a/lib/old/Old/Thing.php",
            "<?php class Old_Thing {}",
        );
        write(
            dir.path(),
            "vendor/a/lib/stubs/S.inc",
            "<?php class Foo {} class Stubbed {}",
        );
        write(
            dir.path(),
            "vendor/a/lib/stubs/skip/Skip.php",
            "<?php class Skip {}",
        );
        write(
            dir.path(),
            "app/Mine.php",
            "<?php namespace App; class Mine {} class Misplaced {}",
        );
        write(
            dir.path(),
            "app/Bad.php",
            "<?php namespace App; class NotBad {}",
        );
        let vendor = format!("{root}/vendor");
        let autoloads = Autoloads {
            psr4: vec![
                (PhpKey::from("App\\"), vec!["app/".into()]),
                (PhpKey::from("A\\"), vec![format!("{vendor}/a/lib/src")]),
            ],
            psr0: vec![(PhpKey::from("Old_"), vec![format!("{vendor}/a/lib/old")])],
            classmap: vec![format!("{vendor}/a/lib/stubs")],
            exclude: vec![format!(
                "{}/stubs/skip($|/)",
                preg_quote(&format!("{vendor}/a/lib"))
            )],
            ..Autoloads::default()
        };
        let mut map = ClassMap::scan(&autoloads, true, &root, &vendor).unwrap();
        map.sort();
        let entries: Vec<(String, String)> = map
            .entries()
            .map(|(c, p)| (c.to_owned(), p.trim_start_matches(&root).to_owned()))
            .collect();
        let expected: Vec<(String, String)> = [
            ("A\\Foo", "/vendor/a/lib/src/Foo.php"),
            ("A\\Sub\\Bar", "/vendor/a/lib/src/Sub/Bar.php"),
            ("App\\Mine", "/app/Mine.php"),
            ("B\\Wrong", "/vendor/a/lib/src/Wrong.php"),
            ("Foo", "/vendor/a/lib/stubs/S.inc"),
            ("Old_Thing", "/vendor/a/lib/old/Old/Thing.php"),
            ("Stubbed", "/vendor/a/lib/stubs/S.inc"),
        ]
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect();
        assert_eq!(entries, expected);
        let warnings = map.warnings();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].starts_with(
                "Class App\\NotBad located in ./app/Bad.php does not comply with psr-4"
            )
        );

        let without_o = ClassMap::scan(&autoloads, false, &root, &vendor).unwrap();
        assert_eq!(without_o.entries().count(), 2);
    }

    #[test]
    fn reports_ambiguity_and_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = real_path(dir.path()).unwrap();
        write(dir.path(), "lib/a/One.php", "<?php class Dup {}");
        write(dir.path(), "lib/b/Two.php", "<?php class Dup {}");
        write(dir.path(), "lib/c/Three.php", "<?php class Dup {}");
        let autoloads = Autoloads {
            classmap: vec![
                "lib/a".into(),
                "lib/b/Two.php".into(),
                "lib/c".into(),
                "lib/a".into(),
            ],
            ..Autoloads::default()
        };
        let map = ClassMap::scan(&autoloads, false, &root, &format!("{root}/vendor")).unwrap();
        assert_eq!(
            map.entries()
                .next()
                .map(|(_, p)| p.ends_with("lib/a/One.php")),
            Some(true)
        );
        let warnings = map.warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("was found 3x"), "{warnings:?}");

        let missing = Autoloads {
            classmap: vec!["nope".into()],
            ..Autoloads::default()
        };
        assert!(ClassMap::scan(&missing, false, &root, &root).is_err());
        let glob = Autoloads {
            classmap: vec!["lib/*".into()],
            ..Autoloads::default()
        };
        assert!(ClassMap::scan(&glob, false, &root, &root).is_err());
    }
}
