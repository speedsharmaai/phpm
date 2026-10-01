//! Git source installs, for a lock entry with no usable dist: clone the
//! repository and check out the pinned commit, shelling out to a real
//! `git`, the way Composer's `GitDownloader` does.
//!
//! Composer: Downloader/GitDownloader.php doInstall, updateToCommit. phpm
//! installs once rather than keeping an editable checkout across updates,
//! so this is `clone --no-checkout`, add the `composer` remote, then
//! `checkout`/`reset --hard <reference>`, with a `fetch` in between only
//! when the commit is not already reachable (a detached/rewritten ref a
//! plain clone's branches do not include). `.git` is kept, matching
//! Composer: a source install is a real checkout, not a stripped dist tree.

use std::path::Path;
use std::process::{Command, Output};

use reqwest::Url;

use crate::auth::Auth;
use crate::error::{Error, Result};

/// `source.url`, the pinned commit `source.reference` and the lock's own
/// `version` (pretty) from a lock entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitSource {
    pub url: String,
    pub reference: String,
    pub pretty_version: String,
}

// Composer: Downloader/GitDownloader.php updateToCommit,
// `Preg::replace('{(?:^dev-|(?:\.x)?-dev$)}i', '', $prettyVersion)`.
fn branch_name(pretty_version: &str) -> String {
    let stripped = if pretty_version.len() >= 4 && pretty_version[..4].eq_ignore_ascii_case("dev-")
    {
        &pretty_version[4..]
    } else {
        pretty_version
    };
    let lower = stripped.to_ascii_lowercase();
    if let Some(stem) = lower.strip_suffix(".x-dev") {
        stripped[..stem.len()].to_owned()
    } else if let Some(stem) = lower.strip_suffix("-dev") {
        stripped[..stem.len()].to_owned()
    } else {
        stripped.to_owned()
    }
}

fn fail(url: &str, reason: impl Into<String>) -> Error {
    Error::Download {
        url: url.to_owned(),
        reason: reason.into(),
    }
}

fn spawn(
    header: Option<&(String, String)>,
    dir: Option<&Path>,
    args: &[&str],
) -> std::io::Result<Output> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        // Composer: Util/Git.php cleanEnv
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE");
    if let Some((name, value)) = header {
        // Config injected through the environment never appears in argv,
        // unlike a `-c http.extraHeader=...` flag a process listing would show.
        cmd.env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "http.extraHeader")
            .env("GIT_CONFIG_VALUE_0", format!("{name}: {value}"));
    }
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    cmd.output()
}

fn git(
    header: Option<&(String, String)>,
    dir: Option<&Path>,
    args: &[&str],
    url: &str,
) -> Result<()> {
    let out =
        spawn(header, dir, args).map_err(|e| fail(url, format!("git {}: {e}", args.join(" "))))?;
    if out.status.success() {
        return Ok(());
    }
    Err(fail(
        url,
        format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    ))
}

/// Clones `source.url` into `target` and checks it out at
/// `source.reference`, replacing whatever was there.
pub fn install(auth: &Auth, source: &GitSource, target: &Path) -> Result<()> {
    if target.exists() {
        std::fs::remove_dir_all(target).map_err(|e| fail(&source.url, e.to_string()))?;
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| fail(&source.url, e.to_string()))?;
    }
    let header = Url::parse(&source.url)
        .ok()
        .and_then(|u| auth.git_credential(&u));
    let target_str = target.to_string_lossy().into_owned();
    git(
        header.as_ref(),
        None,
        &["clone", "--no-checkout", "--", &source.url, &target_str],
        &source.url,
    )?;
    // Composer: GitDownloader doInstall adds a second remote next to the
    // one `clone` makes, so a later `fetch composer` never disturbs origin.
    let _ = git(
        header.as_ref(),
        Some(target),
        &["remote", "add", "composer", "--", &source.url],
        &source.url,
    );
    // Composer checks out the branch the version names (so HEAD stays
    // symbolic, e.g. "dev-master" installs with HEAD on "master"), falling
    // back to the bare commit for a tag or any other non-branch version.
    let branch = branch_name(&source.pretty_version);
    let checkout_ref = ["checkout", source.reference.as_str(), "--"];
    let reset = ["reset", "--hard", source.reference.as_str(), "--"];
    let try_checkout = |target: &Path| -> bool {
        if !branch.is_empty()
            && git(
                header.as_ref(),
                Some(target),
                &["checkout", branch.as_str(), "--"],
                &source.url,
            )
            .is_ok()
        {
            return true;
        }
        git(header.as_ref(), Some(target), &checkout_ref, &source.url).is_ok()
    };
    if !try_checkout(target) || git(header.as_ref(), Some(target), &reset, &source.url).is_err() {
        git(
            header.as_ref(),
            Some(target),
            &["fetch", "composer", source.reference.as_str()],
            &source.url,
        )?;
        if !try_checkout(target) {
            git(header.as_ref(), Some(target), &checkout_ref, &source.url)?;
        }
        git(header.as_ref(), Some(target), &reset, &source.url)?;
    }
    if let Some(push_url) = github_push_url(&source.url) {
        // Composer: GitDownloader setPushUrl, default github-protocols
        // includes ssh, so a github.com origin gets a separate push URL.
        let _ = git(
            header.as_ref(),
            Some(target),
            &["remote", "set-url", "--push", "origin", "--", &push_url],
            &source.url,
        );
    }
    Ok(())
}

// Composer: Downloader/GitDownloader.php setPushUrl, default github-domains.
fn github_push_url(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("git://github.com/"))?;
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, repo) = rest.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some(format!("git@github.com:{owner}/{repo}.git"))
}

#[cfg(test)]
mod tests {
    use super::{GitSource, install};
    use crate::auth::Auth;
    use crate::testutil::TempDir;
    use std::path::Path;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }

    fn head(dir: &Path) -> String {
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    fn repo_with_two_commits() -> (TempDir, String, String) {
        let dir = TempDir::new("git-origin");
        git(dir.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(dir.path().join("a.txt"), b"one").unwrap();
        git(dir.path(), &["add", "a.txt"]);
        git(dir.path(), &["commit", "-q", "-m", "one"]);
        let first = head(dir.path());
        std::fs::write(dir.path().join("a.txt"), b"two").unwrap();
        git(dir.path(), &["commit", "-q", "-am", "two"]);
        let second = head(dir.path());
        (dir, first, second)
    }

    #[test]
    fn clones_and_checks_out_the_pinned_commit() {
        let (origin, first, _second) = repo_with_two_commits();
        let target = TempDir::new("git-target");
        let dest = target.path().join("pkg");
        let source = GitSource {
            url: format!("file://{}", origin.path().display()),
            reference: first.clone(),
            pretty_version: String::new(),
        };
        install(&Auth::default(), &source, &dest).unwrap();
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"one");
        assert!(dest.join(".git").is_dir());
        assert_eq!(head(&dest), first);
    }

    #[test]
    fn replaces_an_existing_checkout() {
        let (origin, first, second) = repo_with_two_commits();
        let target = TempDir::new("git-target");
        let dest = target.path().join("pkg");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("stale.txt"), b"old").unwrap();
        let source = GitSource {
            url: format!("file://{}", origin.path().display()),
            reference: second,
            pretty_version: String::new(),
        };
        install(&Auth::default(), &source, &dest).unwrap();
        assert!(!dest.join("stale.txt").exists());
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"two");
        let _ = first;
    }

    #[test]
    fn fetches_a_commit_not_on_any_local_branch() {
        // A plain clone only brings in commits reachable from some ref; a
        // dangling commit (its only branch deleted) needs an explicit fetch
        // by sha, same as Composer's own fallback path in updateToCommit.
        let dir = TempDir::new("git-origin");
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "base"]);
        git(dir.path(), &["checkout", "-q", "-b", "doomed"]);
        git(
            dir.path(),
            &["commit", "-q", "--allow-empty", "-m", "orphaned"],
        );
        let orphaned = head(dir.path());
        git(dir.path(), &["checkout", "-q", "main"]);
        git(dir.path(), &["branch", "-D", "doomed"]);

        let target = TempDir::new("git-target");
        let dest = target.path().join("pkg");
        let source = GitSource {
            url: format!("file://{}", dir.path().display()),
            reference: orphaned.clone(),
            pretty_version: String::new(),
        };
        install(&Auth::default(), &source, &dest).unwrap();
        assert_eq!(head(&dest), orphaned);
    }

    #[test]
    fn reports_a_missing_repository() {
        let target = TempDir::new("git-target");
        let dest = target.path().join("pkg");
        let source = GitSource {
            url: "file:///does/not/exist".to_owned(),
            reference: "abc123".to_owned(),
            pretty_version: String::new(),
        };
        let err = install(&Auth::default(), &source, &dest).unwrap_err();
        assert!(err.to_string().contains("does/not/exist"), "{err}");
    }

    #[test]
    fn keeps_head_symbolic_on_a_dev_branch_version() {
        // Composer: a "dev-master" package installs with HEAD still on
        // "refs/heads/master" (it checks out the branch, then resets it to
        // the pinned commit), not detached, unlike a tagged version.
        let (origin, first, second) = repo_with_two_commits();
        let target = TempDir::new("git-target");
        let dest = target.path().join("pkg");
        let source = GitSource {
            url: format!("file://{}", origin.path().display()),
            reference: first.clone(),
            pretty_version: "dev-main".to_owned(),
        };
        install(&Auth::default(), &source, &dest).unwrap();
        let head_file = std::fs::read_to_string(dest.join(".git/HEAD")).unwrap();
        assert_eq!(head_file.trim(), "ref: refs/heads/main");
        assert_eq!(head(&dest), first);
        let _ = second;
    }

    #[test]
    fn builds_a_github_push_url_like_composer() {
        assert_eq!(
            super::github_push_url("https://github.com/a/b"),
            Some("git@github.com:a/b.git".to_owned())
        );
        assert_eq!(
            super::github_push_url("https://github.com/a/b.git"),
            Some("git@github.com:a/b.git".to_owned())
        );
        assert_eq!(super::github_push_url("https://codeberg.org/a/b"), None);
        assert_eq!(super::github_push_url("https://github.com/a"), None);
    }

    #[test]
    fn derives_the_checkout_branch_like_composer() {
        assert_eq!(super::branch_name("dev-master"), "master");
        assert_eq!(super::branch_name("DEV-Main"), "Main");
        assert_eq!(super::branch_name("2.x-dev"), "2");
        assert_eq!(super::branch_name("1.0.x-dev"), "1.0");
        assert_eq!(super::branch_name("feature-dev"), "feature");
        assert_eq!(super::branch_name("v1.2.3"), "v1.2.3");
    }
}
