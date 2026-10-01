//! `path` repository packages, installed the way Composer's `PathDownloader`
//! installs them: a relative or absolute symlink by default, or a mirror of
//! the files `ArchivableFilesFinder` keeps.
//!
//! On Windows Composer makes a junction; phpm tries a directory symlink and
//! mirrors when that fails, which is what Composer does when junctions are
//! not available. Only the unix paths are tested.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use phpm_lock::find_shortest_path_with;
use serde_json::{Map, Value};

use crate::error::Error;
use crate::fsutil::path_string;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Strategy {
    Symlink,
    Mirror,
}

/// A locked `path` dist and its `transport-options`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathDist {
    pub(crate) url: String,
    pub(crate) symlink: Option<bool>,
    pub(crate) relative: bool,
}

impl PathDist {
    pub(crate) fn from_lock(url: &str, entry: &Map<String, Value>) -> Self {
        let options = entry.get("transport-options").and_then(Value::as_object);
        let flag = |k: &str| options.and_then(|o| o.get(k)).and_then(Value::as_bool);
        Self {
            url: url.to_owned(),
            symlink: flag("symlink"),
            relative: flag("relative").unwrap_or(true),
        }
    }
}

// Composer: Downloader/PathDownloader.php computeAllowedStrategies
fn strategies(dist: &PathDist, mirror_env: Option<&str>) -> (Strategy, Vec<Strategy>) {
    let mut current = Strategy::Symlink;
    let mut allowed = vec![Strategy::Symlink, Strategy::Mirror];
    if mirror_env.is_some_and(|v| !v.is_empty() && v != "0") {
        current = Strategy::Mirror;
    }
    match dist.symlink {
        Some(true) => {
            current = Strategy::Symlink;
            allowed = vec![Strategy::Symlink];
        }
        Some(false) => {
            current = Strategy::Mirror;
            allowed = vec![Strategy::Mirror];
        }
        None => {}
    }
    (current, allowed)
}

fn io_err(path: &Path) -> impl Fn(io::Error) -> Error + '_ {
    move |e| Error::io(path, &e)
}

/// Install `name` from its path dist into `dest`; returns what was done.
pub(crate) fn install(
    name: &str,
    dist: &PathDist,
    root: &Path,
    dest: &Path,
    mirror_env: Option<&str>,
) -> Result<Strategy, Error> {
    let source = root.join(&dist.url);
    let real = fs::canonicalize(&source)
        .ok()
        .filter(|p| p.is_dir())
        .ok_or_else(|| {
            Error::install(format!(
                "Source path \"{}\" is not found for package {name}",
                dist.url
            ))
        })?;
    if let Ok(existing) = fs::canonicalize(dest) {
        if existing == real {
            return Ok(Strategy::Symlink);
        }
        if existing.starts_with(&real) {
            return Err(Error::install(format!(
                "Package {name} cannot install to \"{}\" inside its source at \"{}\"",
                existing.display(),
                real.display()
            )));
        }
    }
    let (mut current, allowed) = strategies(dist, mirror_env);
    remove(dest)?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(io_err(parent))?;
    }
    if current == Strategy::Symlink {
        let target = if dist.relative {
            let from = path_string(dest);
            let to = path_string(&real);
            find_shortest_path_with(&from, &to, false, true).unwrap_or(to)
        } else {
            path_string(&real)
        };
        if let Err(e) = link_dir(&format!("{target}/"), &real, dest) {
            if !allowed.contains(&Strategy::Mirror) {
                return Err(Error::install(format!(
                    "Symlink from \"{}\" to \"{}\" failed! {e}",
                    real.display(),
                    dest.display()
                )));
            }
            current = Strategy::Mirror;
        }
    }
    if current == Strategy::Mirror {
        mirror(&real, dest)?;
    }
    Ok(current)
}

#[cfg(unix)]
fn link_dir(target: &str, _real: &Path, dest: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, dest)
}

#[cfg(windows)]
fn link_dir(_target: &str, real: &Path, dest: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(real, dest)
}

fn remove(path: &Path) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path).map_err(io_err(path)),
        Ok(_) => fs::remove_file(path).map_err(io_err(path)),
        Err(_) => Ok(()),
    }
}

/// Composer's `Finder::ignoreVCS` list; it only skips directories.
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

#[derive(Debug)]
enum Item {
    Link(PathBuf),
    Dir,
    File,
}

// Composer: Package/Archiver/ArchivableFilesFinder.php
fn archivable(real: &Path) -> Result<Vec<(PathBuf, Item)>, Error> {
    let filter = ExcludeFilter::from_gitattributes(real);
    let prefix = path_string(real);
    let mut out = Vec::new();
    let mut stack = vec![real.to_owned()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(io_err(&dir))?;
        for entry in entries {
            let entry = entry.map_err(io_err(&dir))?;
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).map_err(io_err(&path))?;
            let is_dir = meta.is_dir();
            if is_dir && VCS_DIRS.iter().any(|v| entry.file_name() == *v) {
                continue;
            }
            if is_dir {
                stack.push(path.clone());
            }
            let Ok(resolved) = fs::canonicalize(&path) else {
                continue;
            };
            let resolved = path_string(&resolved);
            if meta.is_symlink() && !resolved.starts_with(&prefix) {
                continue;
            }
            let rel = resolved.strip_prefix(&prefix).unwrap_or(&resolved);
            if filter.excludes(rel) {
                continue;
            }
            let item = if meta.is_symlink() {
                Item::Link(fs::read_link(&path).map_err(io_err(&path))?)
            } else if is_dir {
                let empty = fs::read_dir(&path).map_err(io_err(&path))?.next().is_none();
                if !empty {
                    continue;
                }
                Item::Dir
            } else {
                Item::File
            };
            let rel_path = path.strip_prefix(real).unwrap_or(&path).to_owned();
            out.push((rel_path, item));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

// Composer: symfony/filesystem Filesystem::mirror and copy
fn mirror(real: &Path, dest: &Path) -> Result<(), Error> {
    fs::create_dir_all(dest).map_err(io_err(dest))?;
    for (rel, item) in archivable(real)? {
        let target = dest.join(&rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(io_err(parent))?;
        }
        match item {
            Item::Link(to) => copy_link(&to, &target)?,
            Item::Dir => fs::create_dir_all(&target).map_err(io_err(&target))?,
            Item::File => copy_file(&real.join(&rel), &target)?,
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_link(to: &Path, target: &Path) -> Result<(), Error> {
    std::os::unix::fs::symlink(to, target).map_err(io_err(target))
}

#[cfg(windows)]
fn copy_link(to: &Path, target: &Path) -> Result<(), Error> {
    std::os::windows::fs::symlink_file(to, target).map_err(io_err(target))
}

fn copy_file(from: &Path, to: &Path) -> Result<(), Error> {
    let bytes = fs::read(from).map_err(io_err(from))?;
    fs::write(to, bytes).map_err(io_err(to))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let exec = fs::metadata(from)
            .map_err(io_err(from))?
            .permissions()
            .mode()
            & 0o111;
        let mode = fs::metadata(to).map_err(io_err(to))?.permissions().mode() & 0o7777;
        fs::set_permissions(to, fs::Permissions::from_mode(mode | exec)).map_err(io_err(to))?;
    }
    Ok(())
}

/// `.gitattributes` export-ignore rules, as `GitExcludeFilter` reads them.
#[derive(Debug, Default)]
struct ExcludeFilter {
    rules: Vec<Rule>,
}

#[derive(Debug)]
struct Rule {
    anchored: bool,
    tokens: Option<Vec<Tok>>,
    negate: bool,
}

impl ExcludeFilter {
    fn from_gitattributes(dir: &Path) -> Self {
        let Ok(text) = fs::read_to_string(dir.join(".gitattributes")) else {
            return Self::default();
        };
        let rules = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                match parts.as_slice() {
                    [path, "export-ignore"] => Some(Rule::new(path)),
                    [path, "-export-ignore"] => Some(Rule::new(&format!("!{path}"))),
                    _ => None,
                }
            })
            .collect();
        Self { rules }
    }

    // Composer: Package/Archiver/BaseExcludeFilter.php filter
    fn excludes(&self, rel: &str) -> bool {
        let mut exclude = false;
        for rule in &self.rules {
            if rule.matches(rel) {
                exclude = !rule.negate;
            }
        }
        exclude
    }
}

/// One element of the regex `Glob::toRegex` builds.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Lit(u8),
    /// `[^/]*`
    Star,
    /// `[^/]`
    One,
    /// `(?=[^\.])`
    NoDot,
    /// `/(?:(?=[^\.])[^/]++/)*`, the last `/` optional at the end of the glob.
    GlobStar {
        trailing: bool,
    },
    Class {
        ranges: Vec<(u8, u8)>,
    },
    Alt(Vec<Vec<Tok>>),
    /// `(?=$|/)`
    EndOrSlash,
}

impl Rule {
    // Composer: Package/Archiver/BaseExcludeFilter.php generatePattern
    fn new(rule: &str) -> Self {
        let negate = rule.starts_with('!');
        let rule = rule.trim_start_matches('!');
        let first_slash = rule.find('/');
        let (anchored, prefix_slash) = match first_slash {
            Some(0) => (true, true),
            None => (false, true),
            Some(i) if i == rule.len() - 1 => (false, true),
            Some(_) => (false, false),
        };
        let tokens = glob_tokens(rule.trim_matches('/')).map(|mut t| {
            if prefix_slash {
                t.insert(0, Tok::Lit(b'/'));
            }
            t.push(Tok::EndOrSlash);
            t
        });
        Self {
            anchored,
            tokens,
            negate,
        }
    }

    fn matches(&self, path: &str) -> bool {
        let Some(tokens) = &self.tokens else {
            return false;
        };
        let s = path.as_bytes();
        if self.anchored {
            return run(tokens, s, 0);
        }
        (0..=s.len()).any(|start| run(tokens, s, start))
    }
}

/// symfony/finder `Glob::toRegex` as tokens; `None` when PCRE would reject
/// the regex (Composer then ignores the rule).
fn glob_tokens(glob: &str) -> Option<Vec<Tok>> {
    let g = glob.as_bytes();
    let mut stack: Vec<(Vec<Vec<Tok>>, Vec<Tok>)> = Vec::new();
    let mut out: Vec<Tok> = Vec::new();
    let mut first = true;
    let mut escaping = false;
    let mut i = 0;
    while i < g.len() {
        let c = g[i];
        if first && c != b'.' {
            out.push(Tok::NoDot);
        }
        first = c == b'/';
        if first
            && g.get(i + 1) == Some(&b'*')
            && g.get(i + 2) == Some(&b'*')
            && matches!(g.get(i + 3), None | Some(b'/'))
        {
            let trailing = g.get(i + 3).is_none();
            out.push(Tok::GlobStar { trailing });
            i += 3 + usize::from(!trailing);
            escaping = false;
            continue;
        }
        match c {
            b'*' if !escaping => out.push(Tok::Star),
            b'?' if !escaping => out.push(Tok::One),
            b'{' if !escaping => {
                stack.push((Vec::new(), std::mem::take(&mut out)));
            }
            b'}' if !stack.is_empty() => {
                if escaping {
                    out.push(Tok::Lit(b'}'));
                } else if let Some((mut branches, before)) = stack.pop() {
                    branches.push(std::mem::replace(&mut out, before));
                    out.push(Tok::Alt(branches));
                }
            }
            b',' if !stack.is_empty() && !escaping => {
                if let Some((branches, _)) = stack.last_mut() {
                    branches.push(std::mem::take(&mut out));
                }
            }
            b'\\' if !escaping => {
                escaping = true;
                i += 1;
                continue;
            }
            b'[' if !escaping => {
                let (tok, len) = class(&g[i..])?;
                out.push(tok);
                i += len;
                escaping = false;
                continue;
            }
            _ => out.push(Tok::Lit(c)),
        }
        escaping = false;
        i += 1;
    }
    if !stack.is_empty() {
        return None;
    }
    Some(out)
}

/// A `[...]` passed through to PCRE; `toRegex` escapes `^`, so there is no negation.
fn class(g: &[u8]) -> Option<(Tok, usize)> {
    let mut i = 1;
    let mut ranges = Vec::new();
    let mut first = true;
    loop {
        let c = *g.get(i)?;
        if c == b']' && !first {
            return Some((Tok::Class { ranges }, i + 1));
        }
        first = false;
        if g.get(i + 1) == Some(&b'-') && g.get(i + 2).is_some_and(|&e| e != b']') {
            ranges.push((c, g[i + 2]));
            i += 3;
        } else {
            ranges.push((c, c));
            i += 1;
        }
    }
}

fn run(toks: &[Tok], s: &[u8], at: usize) -> bool {
    let Some((tok, rest)) = toks.split_first() else {
        return true;
    };
    match tok {
        Tok::Lit(c) => s.get(at) == Some(c) && run(rest, s, at + 1),
        Tok::One => s.get(at).is_some_and(|&c| c != b'/') && run(rest, s, at + 1),
        Tok::NoDot => s.get(at).is_some_and(|&c| c != b'.') && run(rest, s, at),
        Tok::Star => {
            let max = s[at..].iter().take_while(|&&c| c != b'/').count();
            (0..=max).rev().any(|k| run(rest, s, at + k))
        }
        Tok::Class { ranges } => {
            s.get(at)
                .is_some_and(|&c| ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&c)))
                && run(rest, s, at + 1)
        }
        Tok::Alt(branches) => branches.iter().any(|b| {
            let mut joined = b.clone();
            joined.extend_from_slice(rest);
            run(&joined, s, at)
        }),
        Tok::EndOrSlash => (at == s.len() || s[at] == b'/') && run(rest, s, at),
        Tok::GlobStar { trailing } => {
            if s.get(at) != Some(&b'/') {
                return false;
            }
            segments(rest, s, at + 1, *trailing)
        }
    }
}

fn segments(rest: &[Tok], s: &[u8], at: usize, trailing: bool) -> bool {
    if run(rest, s, at) {
        return true;
    }
    if s.get(at).is_none_or(|&c| c == b'.' || c == b'/') {
        return false;
    }
    let end = at + s[at..].iter().take_while(|&&c| c != b'/').count();
    if s.get(end) == Some(&b'/') && segments(rest, s, end + 1, trailing) {
        return true;
    }
    trailing && run(rest, s, end)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::install;
    use super::{ExcludeFilter, PathDist, Rule, Strategy, strategies};
    use serde_json::json;
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::path::Path;

    fn dist(symlink: Option<bool>, relative: bool) -> PathDist {
        PathDist {
            url: "packages/foo".into(),
            symlink,
            relative,
        }
    }

    #[test]
    fn picks_strategies_like_composer() {
        assert_eq!(strategies(&dist(None, true), None).0, Strategy::Symlink);
        assert_eq!(strategies(&dist(None, true), Some("1")).0, Strategy::Mirror);
        assert_eq!(
            strategies(&dist(None, true), Some("0")).0,
            Strategy::Symlink
        );
        assert_eq!(
            strategies(&dist(Some(true), true), Some("1")),
            (Strategy::Symlink, vec![Strategy::Symlink])
        );
        assert_eq!(
            strategies(&dist(Some(false), true), None),
            (Strategy::Mirror, vec![Strategy::Mirror])
        );
        let entry = json!({"transport-options": {"symlink": false, "relative": false}});
        let d = PathDist::from_lock("x", entry.as_object().unwrap());
        assert_eq!((d.symlink, d.relative), (Some(false), false));
        let d = PathDist::from_lock("x", json!({}).as_object().unwrap());
        assert_eq!((d.symlink, d.relative), (None, true));
    }

    fn excluded(rules: &[&str], path: &str) -> bool {
        let filter = ExcludeFilter {
            rules: rules.iter().map(|r| Rule::new(r)).collect(),
        };
        filter.excludes(path)
    }

    #[test]
    fn matches_export_ignore_rules_like_composer() {
        let cases: [(&[&str], &str, bool); 20] = [
            (&["/tests"], "/tests/a.php", true),
            (&["/tests"], "/src/tests/a.php", false),
            (&["tests"], "/src/tests/a.php", true),
            (&["tests/"], "/src/tests", true),
            (&["*.md"], "/README.md", true),
            (&["*.md"], "/docs/x.md", true),
            (&["*.md", "/docs/other.md"], "/docs/keep.md", true),
            (&["*.md", "!/docs/keep.md"], "/docs/keep.md", false),
            (&["docs/api"], "/mydocs/api/x", true),
            (&["/.github"], "/.github/workflows/ci.yml", true),
            (&["*"], "/.hidden", false),
            (&["/src/**/Test*.php"], "/src/a/b/TestX.php", true),
            (&["/src/**/Test*.php"], "/src/TestX.php", true),
            (&["/src/**"], "/src/a/b", true),
            (&["/{tests,docs}"], "/docs/a", true),
            (&["/{tests,docs}"], "/src/a", false),
            (&["/[ab]*.txt"], "/b1.txt", true),
            (&["/[^ab]*.txt"], "/^1.txt", true),
            (&["/file?.txt"], "/file1.txt", true),
            (&["/[oops"], "/[oops", false),
        ];
        for (rules, path, want) in cases {
            assert_eq!(excluded(rules, path), want, "{rules:?} {path}");
        }
        assert!(excluded(&["/a\\*b"], "/a*b"));
        assert!(!excluded(&["/{a,b"], "/a"));
    }

    #[cfg(unix)]
    fn tree(root: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let pkg = root.join("packages/foo");
        fs::create_dir_all(pkg.join("src")).unwrap();
        fs::create_dir_all(pkg.join("tests")).unwrap();
        fs::create_dir_all(pkg.join(".git")).unwrap();
        fs::create_dir_all(pkg.join("empty")).unwrap();
        fs::write(pkg.join("src/A.php"), "<?php").unwrap();
        fs::write(pkg.join("tests/T.php"), "<?php").unwrap();
        fs::write(pkg.join(".git/HEAD"), "x").unwrap();
        fs::write(
            pkg.join(".gitattributes"),
            "# c\n/tests export-ignore\nbad line here\n",
        )
        .unwrap();
        fs::write(pkg.join("run.sh"), "#!/bin/sh").unwrap();
        fs::set_permissions(pkg.join("run.sh"), fs::Permissions::from_mode(0o750)).unwrap();
        std::os::unix::fs::symlink("src/A.php", pkg.join("link.php")).unwrap();
        std::os::unix::fs::symlink("/etc/hosts", pkg.join("outside")).unwrap();
        std::os::unix::fs::symlink("missing", pkg.join("broken")).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_relative_or_absolute() {
        let tmp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        tree(&root);
        let dest = root.join("vendor/acme/foo");
        let done = install("acme/foo", &dist(None, true), &root, &dest, None).unwrap();
        assert_eq!(done, Strategy::Symlink);
        assert_eq!(
            fs::read_link(&dest).unwrap(),
            Path::new("../../packages/foo/")
        );
        assert_eq!(
            install("acme/foo", &dist(None, true), &root, &dest, None).unwrap(),
            Strategy::Symlink
        );
        fs::remove_file(&dest).unwrap();
        install("acme/foo", &dist(Some(true), false), &root, &dest, Some("")).unwrap();
        assert_eq!(fs::read_link(&dest).unwrap(), root.join("packages/foo/"));
        let err = install(
            "acme/foo",
            &PathDist {
                url: "nope".into(),
                symlink: None,
                relative: true,
            },
            &root,
            &dest,
            None,
        )
        .unwrap_err();
        assert!(
            err.message
                .contains("Source path \"nope\" is not found for package acme/foo"),
            "{err}"
        );
        let inside = root.join("packages/foo/sub");
        fs::create_dir_all(&inside).unwrap();
        let err = install("acme/foo", &dist(None, true), &root, &inside, None).unwrap_err();
        assert!(err.message.contains("inside its source"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn mirrors_what_composer_archives() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        tree(&root);
        let dest = root.join("vendor/acme/foo");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("stale"), "x").unwrap();
        let done = install("acme/foo", &dist(Some(false), true), &root, &dest, None).unwrap();
        assert_eq!(done, Strategy::Mirror);
        let mut names: Vec<String> = walk(&dest);
        names.sort();
        assert_eq!(
            names,
            [
                ".gitattributes",
                "empty/",
                "link.php",
                "run.sh",
                "src/",
                "src/A.php"
            ]
        );
        assert_eq!(
            fs::read_link(dest.join("link.php")).unwrap(),
            Path::new("src/A.php")
        );
        let umask_mode = fs::metadata(dest.join("src/A.php"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let run = fs::metadata(dest.join("run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(run, umask_mode | 0o110);
        let done = install("acme/foo", &dist(None, true), &root, &dest, Some("1")).unwrap();
        assert_eq!(done, Strategy::Mirror);
    }

    #[cfg(unix)]
    fn walk(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        for e in fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let name = e.file_name().to_string_lossy().into_owned();
            let meta = fs::symlink_metadata(e.path()).unwrap();
            if meta.is_dir() {
                out.push(format!("{name}/"));
                out.extend(walk(&e.path()).into_iter().map(|n| format!("{name}/{n}")));
            } else {
                out.push(name);
            }
        }
        out
    }
}
