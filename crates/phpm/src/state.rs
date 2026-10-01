//! The no-op fast path: a state file outside `vendor/`, so `vendor/` stays
//! byte-identical to Composer's.
//!
//! `<cache>/state/v1/<sha256(project dir)>` holds a digest of everything that
//! changes phpm's output (composer.json and composer.lock bytes, flags, the
//! Composer env vars phpm reads, git HEAD, the phpm version) and the size,
//! mtime and inode of the files phpm wrote last. If all of that still
//! matches, the install is a no-op and needs only a few `stat` calls.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// SHA-256 over length-prefixed parts, so `("ab", "c")` and `("a", "bc")` differ.
#[derive(Debug, Default)]
pub(crate) struct Inputs {
    hasher: Sha256,
}

impl Inputs {
    pub(crate) fn add(&mut self, label: &str, bytes: &[u8]) -> &mut Self {
        for part in [label.as_bytes(), bytes] {
            self.hasher.update((part.len() as u64).to_le_bytes());
            self.hasher.update(part);
        }
        self
    }

    pub(crate) fn finish(self) -> String {
        hex(&self.hasher.finalize())
    }
}

/// What a file looked like right after phpm wrote it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stamp {
    len: u64,
    mtime_ns: u128,
    inode: u64,
}

impl Stamp {
    pub(crate) fn of(path: &Path) -> Option<Self> {
        let meta = fs::metadata(path).ok()?;
        let mtime_ns = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos();
        #[cfg(unix)]
        let inode = std::os::unix::fs::MetadataExt::ino(&meta);
        #[cfg(not(unix))]
        let inode = 0;
        Some(Self {
            len: meta.len(),
            mtime_ns,
            inode,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct State {
    pub(crate) inputs: String,
    pub(crate) files: Vec<(PathBuf, Stamp)>,
}

const HEADER: &str = "phpm-state 1";

impl State {
    /// Stamp `files` as they are now; `None` if one of them is missing.
    pub(crate) fn capture(inputs: String, files: &[PathBuf]) -> Option<Self> {
        let files = files
            .iter()
            .map(|p| Stamp::of(p).map(|s| (p.clone(), s)))
            .collect::<Option<Vec<_>>>()?;
        Some(Self { inputs, files })
    }

    pub(crate) fn is_current(&self, inputs: &str) -> bool {
        self.inputs == inputs
            && self
                .files
                .iter()
                .all(|(path, stamp)| Stamp::of(path) == Some(*stamp))
    }

    fn render(&self) -> String {
        let mut out = format!("{HEADER}\ninputs {}\n", self.inputs);
        for (path, s) in &self.files {
            let _ = writeln!(
                out,
                "file {} {} {} {}",
                s.len,
                s.mtime_ns,
                s.inode,
                path.to_string_lossy()
            );
        }
        out
    }

    fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        if lines.next()? != HEADER {
            return None;
        }
        let inputs = lines.next()?.strip_prefix("inputs ")?.to_owned();
        let mut files = Vec::new();
        for line in lines {
            let mut parts = line.strip_prefix("file ")?.splitn(4, ' ');
            let len = parts.next()?.parse().ok()?;
            let mtime_ns = parts.next()?.parse().ok()?;
            let inode = parts.next()?.parse().ok()?;
            let path = PathBuf::from(parts.next()?);
            files.push((
                path,
                Stamp {
                    len,
                    mtime_ns,
                    inode,
                },
            ));
        }
        Some(Self { inputs, files })
    }

    pub(crate) fn load(path: &Path) -> Option<Self> {
        Self::parse(&fs::read_to_string(path).ok()?)
    }

    pub(crate) fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        fs::write(&tmp, self.render())?;
        fs::rename(&tmp, path)
    }
}

/// Where the state for the project at `root` lives under the phpm cache.
pub(crate) fn state_path(cache_dir: &Path, root: &Path) -> PathBuf {
    let mut h = Sha256::new();
    h.update(root.to_string_lossy().as_bytes());
    cache_dir.join("state").join("v1").join(hex(&h.finalize()))
}

/// Enough of git's state to notice a new commit, branch switch or tag, which
/// change the root version Composer writes into `installed.php`. Read from
/// files, never by running git.
pub(crate) fn git_fingerprint(root: &Path) -> String {
    let Some(dot_git) = root
        .ancestors()
        .map(|d| d.join(".git"))
        .find(|p| fs::symlink_metadata(p).is_ok())
    else {
        return String::new();
    };
    let git_dir = if dot_git.is_file() {
        let text = fs::read_to_string(&dot_git).unwrap_or_default();
        match text.trim().strip_prefix("gitdir: ") {
            Some(dir) => dot_git
                .parent()
                .map_or_else(|| PathBuf::from(dir), |p| p.join(dir)),
            None => return text,
        }
    } else {
        dot_git
    };
    let common = fs::read_to_string(git_dir.join("commondir"))
        .map_or_else(|_| git_dir.clone(), |c| git_dir.join(c.trim()));
    let head = fs::read_to_string(git_dir.join("HEAD")).unwrap_or_default();
    let mut out = head.clone();
    if let Some(reference) = head.trim().strip_prefix("ref: ") {
        for dir in [&git_dir, &common] {
            if let Ok(value) = fs::read_to_string(dir.join(reference)) {
                out.push_str(&value);
                break;
            }
        }
    }
    for p in [
        common.join("packed-refs"),
        common.join("refs/tags"),
        common.join("refs/heads"),
    ] {
        if let Some(s) = Stamp::of(&p) {
            let _ = write!(out, "\n{}:{}:{}", s.len, s.mtime_ns, s.inode);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Inputs, Stamp, State, git_fingerprint, state_path};
    use std::fs;
    use std::path::Path;

    fn digest(parts: &[(&str, &str)]) -> String {
        let mut i = Inputs::default();
        for (l, b) in parts {
            i.add(l, b.as_bytes());
        }
        i.finish()
    }

    #[test]
    fn inputs_are_length_prefixed() {
        assert_ne!(digest(&[("a", "bc")]), digest(&[("ab", "c")]));
        assert_eq!(digest(&[("a", "b")]), digest(&[("a", "b")]));
        assert_eq!(digest(&[]).len(), 64);
    }

    #[test]
    fn round_trips_and_detects_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("installed json");
        fs::write(&file, b"{}").unwrap();
        let state = State::capture("abc".into(), std::slice::from_ref(&file)).unwrap();
        let path = state_path(tmp.path(), Path::new("/project"));
        state.save(&path).unwrap();
        let loaded = State::load(&path).unwrap();
        assert_eq!(loaded, state);
        assert!(loaded.is_current("abc"));
        assert!(!loaded.is_current("xyz"));
        fs::write(&file, b"{ }").unwrap();
        assert!(!loaded.is_current("abc"));
        fs::remove_file(&file).unwrap();
        assert!(!loaded.is_current("abc"));
        assert!(State::capture("abc".into(), &[file]).is_none());
    }

    #[test]
    fn rejects_foreign_or_broken_files() {
        assert!(State::parse("").is_none());
        assert!(State::parse("phpm-state 2\ninputs a\n").is_none());
        assert!(State::parse("phpm-state 1\nnope\n").is_none());
        assert!(State::parse("phpm-state 1\ninputs a\nfile x 1 2 /p\n").is_none());
        assert!(State::parse("phpm-state 1\ninputs a\nother\n").is_none());
        assert!(State::load(Path::new("/nonexistent/phpm/state")).is_none());
        let ok = State::parse("phpm-state 1\ninputs a\nfile 1 2 3 /p q\n").unwrap();
        assert_eq!(ok.files[0].0, Path::new("/p q"));
    }

    #[test]
    fn state_paths_are_per_project() {
        let a = state_path(Path::new("/c"), Path::new("/p/one"));
        let b = state_path(Path::new("/c"), Path::new("/p/two"));
        assert_ne!(a, b);
        assert!(a.starts_with("/c/state/v1"));
    }

    #[test]
    fn stamps_missing_files_as_none() {
        assert!(Stamp::of(Path::new("/nonexistent/file")).is_none());
    }

    #[test]
    fn git_fingerprint_follows_head() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("app");
        fs::create_dir_all(root.join("sub")).unwrap();
        assert_eq!(git_fingerprint(&root.join("sub")), "");

        let git = root.join(".git");
        fs::create_dir_all(git.join("refs/heads")).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(git.join("refs/heads/main"), "aaa\n").unwrap();
        let one = git_fingerprint(&root.join("sub"));
        assert!(one.contains("aaa"));
        fs::write(git.join("refs/heads/main"), "bbb\n").unwrap();
        let two = git_fingerprint(&root);
        assert!(two.contains("bbb"));
        assert_ne!(one, two);

        let wt = tmp.path().join("wt");
        fs::create_dir_all(&wt).unwrap();
        let wt_git = git.join("worktrees/wt");
        fs::create_dir_all(&wt_git).unwrap();
        fs::write(wt_git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(wt_git.join("commondir"), "../..\n").unwrap();
        fs::write(wt.join(".git"), format!("gitdir: {}\n", wt_git.display())).unwrap();
        assert!(git_fingerprint(&wt).contains("bbb"));
        fs::write(wt.join(".git"), "garbage").unwrap();
        assert_eq!(git_fingerprint(&wt), "garbage");
    }
}
