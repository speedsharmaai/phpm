use crate::error::Error;
use crate::package::pretty_alias;
use crate::version::{normalize, normalize_branch};
use phpm_php::strnatcasecmp;
use regex::bytes::{Regex, RegexBuilder};
use serde_json::{Map, Value};
use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;

/// Composer's version for a root package it could not version.
pub const NO_VERSION_SET: &str = "1.0.0+no-version-set";

/// The root package's version as `RootPackageLoader` settles it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootVersion {
    pub pretty: String,
    pub normalized: String,
    /// The commit, when the version came from git.
    pub reference: Option<String>,
}

fn bytes_regex(pattern: &str) -> Regex {
    RegexBuilder::new(pattern)
        .unicode(false)
        .build()
        .expect("valid pattern")
}

static CURRENT_BRANCH: LazyLock<Regex> = LazyLock::new(|| {
    bytes_regex(
        r"^(?:\* ) *(\(no branch\)|\(detached from \S+\)|\(HEAD detached at \S+\)|\S+) *([a-f0-9]+) .*$",
    )
});
static REMOTE_HEAD: LazyLock<Regex> = LazyLock::new(|| bytes_regex(r"^ *.+/HEAD "));
static ANY_BRANCH: LazyLock<Regex> = LazyLock::new(|| {
    bytes_regex(r"^(?:\* )? *((?:remotes/(?:origin|upstream)/)?[^\s/]+) *([a-f0-9]+) .*$")
});
static REMOTE_PREFIX: LazyLock<Regex> = LazyLock::new(|| bytes_regex(r"^remotes/\S+/"));

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Runs a VCS command the way Composer does for version guessing: in `dir`,
/// with a clean git environment. `None` on a non-zero exit or spawn failure.
fn run(dir: &Path, program: &str, args: &[&str]) -> Option<String> {
    // Composer: Util/Git.php cleanEnv
    let out = Command::new(program)
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LANGUAGE", "C")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("DYLD_LIBRARY_PATH")
        .output()
        .ok()?;
    out.status.success().then(|| text(&out.stdout))
}

// Composer: Util/ProcessExecutor.php splitLines
fn split_lines(output: &str) -> Vec<&str> {
    let trimmed = crate::version::php_trim(output);
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect()
}

fn is_feature_branch(config: &Map<String, Value>, branch: &str) -> bool {
    let custom: Vec<&str> = config
        .get("non-feature-branches")
        .and_then(Value::as_array)
        .map(|v| v.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let pattern = format!(
        r"^({}|master|main|latest|next|current|support|tip|trunk|default|develop|\d+\..+)$",
        custom.join("|")
    );
    RegexBuilder::new(&pattern)
        .unicode(false)
        .build()
        .map_or(true, |re| !re.is_match(branch.as_bytes()))
}

fn mentions_self_version(v: &Value) -> bool {
    match v {
        Value::String(s) => s == "self.version",
        Value::Array(items) => items.iter().any(mentions_self_version),
        Value::Object(map) => map
            .iter()
            .any(|(k, v)| k == "self.version" || mentions_self_version(v)),
        _ => false,
    }
}

struct Guess {
    version: String,
    pretty: String,
    commit: Option<String>,
}

/// The version Composer 2.10 gives the root package: `version` from
/// composer.json, else `COMPOSER_ROOT_VERSION` (passed in, never read here),
/// else a guess from git in `root_dir`, else `1.0.0+no-version-set`.
///
/// Mercurial, Fossil and Subversion checkouts are refused rather than guessed.
// Composer: Package/Loader/RootPackageLoader.php load
pub fn root_version(
    composer_json: &Map<String, Value>,
    root_dir: &Path,
    env_root_version: Option<&str>,
) -> Result<RootVersion, Error> {
    if let Some(version) = composer_json.get("version").filter(|v| !v.is_null()) {
        let pretty = match version {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let normalized = match composer_json
            .get("version_normalized")
            .and_then(Value::as_str)
        {
            Some(n) if n != crate::version::DEFAULT_BRANCH_ALIAS => n.to_owned(),
            _ => normalize(&pretty)?,
        };
        return Ok(RootVersion {
            pretty,
            normalized,
            reference: None,
        });
    }
    if let Some(env) = env_root_version.filter(|v| !v.is_empty()) {
        let pretty = from_env(env);
        let normalized = normalize(&pretty)?;
        return Ok(RootVersion {
            pretty,
            normalized,
            reference: None,
        });
    }
    if let Some(guess) = guess_git_version(composer_json, root_dir) {
        let (pretty, version) = postprocess(guess.pretty, guess.version);
        return Ok(RootVersion {
            pretty,
            normalized: version,
            reference: guess.commit,
        });
    }
    for (marker, vcs) in [
        (".hg", "Mercurial"),
        (".svn", "Subversion"),
        (".fslckout", "Fossil"),
        ("_FOSSIL_", "Fossil"),
    ] {
        if root_dir.ancestors().any(|dir| dir.join(marker).exists()) {
            return Err(Error::UnsupportedVcs(vcs.to_owned()));
        }
    }
    Ok(RootVersion {
        pretty: NO_VERSION_SET.to_owned(),
        normalized: "1.0.0.0".to_owned(),
        reference: None,
    })
}

// Composer: Package/Version/VersionGuesser.php getRootVersionFromEnv
fn from_env(version: &str) -> String {
    let b = version.as_bytes();
    let Some(stem) = version
        .len()
        .checked_sub(4)
        .filter(|&n| b[n..].eq_ignore_ascii_case(b"-dev"))
        .map(|n| &version[..n])
    else {
        return version.to_owned();
    };
    let numeric = !stem.is_empty()
        && stem
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()));
    if numeric {
        format!("{stem}.x-dev")
    } else {
        version.to_owned()
    }
}

// Composer: Package/Version/VersionGuesser.php postprocess
fn postprocess(pretty: String, version: String) -> (String, String) {
    if version.ends_with("-dev") && version.contains(".9999999") {
        (pretty_alias(&version), version)
    } else {
        (pretty, version)
    }
}

// Composer: Package/Version/VersionGuesser.php guessGitVersion
fn guess_git_version(config: &Map<String, Value>, dir: &Path) -> Option<Guess> {
    let mut commit: Option<String> = None;
    let mut version: Option<String> = None;
    let mut pretty: Option<String> = None;
    let mut detached = false;

    if let Some(output) = run(
        dir,
        "git",
        &["branch", "-a", "--no-color", "--no-abbrev", "-v"],
    ) {
        let mut branches: Vec<String> = Vec::new();
        let mut feature = false;
        for line in split_lines(&output) {
            if let Some(caps) = CURRENT_BRANCH.captures(line.as_bytes()) {
                let name = text(&caps[1]);
                let hash = text(&caps[2]);
                if name == "(no branch)"
                    || name.starts_with("(detached ")
                    || name.starts_with("(HEAD detached at")
                {
                    version = Some(format!("dev-{hash}"));
                    pretty.clone_from(&version);
                    feature = true;
                    detached = true;
                } else {
                    version = Some(normalize_branch(&name));
                    pretty = Some(format!("dev-{name}"));
                    feature = is_feature_branch(config, &name);
                }
                commit = Some(hash);
            }
            if !line.is_empty()
                && !REMOTE_HEAD.is_match(line.as_bytes())
                && let Some(caps) = ANY_BRANCH.captures(line.as_bytes())
            {
                branches.push(text(&caps[1]));
            }
        }
        if feature && let Some(current) = version.clone() {
            let (v, p) = guess_feature_version(config, &current, branches, dir);
            version = Some(v);
            pretty = Some(p);
        }
    }

    if (version.is_none() || detached)
        && let Some(tag) = run(dir, "git", &["describe", "--exact-match", "--tags"])
    {
        let tag = crate::version::php_trim(&tag).to_owned();
        if let Ok(normalized) = normalize(&tag) {
            version = Some(normalized);
            pretty = Some(tag);
        }
    }

    if commit.is_none() {
        let head = run(
            dir,
            "git",
            &[
                "rev-list",
                "--no-commit-header",
                "--format=%H",
                "-n1",
                "HEAD",
                "--no-show-signature",
            ],
        )
        .or_else(|| {
            run(dir, "git", &["rev-list", "--format=%H", "-n1", "HEAD"]).map(|out| {
                out.lines()
                    .filter(|l| !l.starts_with("commit "))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        });
        commit = head
            .map(|h| crate::version::php_trim(&h).to_owned())
            .filter(|h| !h.is_empty());
    }

    Some(Guess {
        version: version?,
        pretty: pretty?,
        commit,
    })
}

// Composer: Package/Version/VersionGuesser.php guessFeatureVersion
fn guess_feature_version(
    config: &Map<String, Value>,
    version: &str,
    mut branches: Vec<String>,
    dir: &Path,
) -> (String, String) {
    let mut found = (version.to_owned(), version.to_owned());
    let aliased = config
        .get("extra")
        .and_then(|e| e.get("branch-alias"))
        .and_then(|a| a.get(version))
        .is_some_and(|v| !v.is_null());
    if aliased && !mentions_self_version(&Value::Object(config.clone())) {
        return found;
    }
    let branch = version.strip_prefix("dev-").unwrap_or(version);
    if !is_feature_branch(config, branch) {
        return found;
    }
    branches.sort_by(|a, b| {
        let (ar, br) = (a.starts_with("remotes/"), b.starts_with("remotes/"));
        if ar == br {
            strnatcasecmp(b, a)
        } else if ar {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Less
        }
    });
    let mut length = usize::MAX;
    for candidate in &branches {
        let candidate_version = text(&REMOTE_PREFIX.replace(candidate.as_bytes(), &b""[..]));
        if candidate == branch || is_feature_branch(config, &candidate_version) {
            continue;
        }
        let Some(output) = run(dir, "git", &["rev-list", &format!("{candidate}..{branch}")]) else {
            continue;
        };
        if output.len() <= length {
            length = output.len();
            found = (
                normalize_branch(&candidate_version),
                format!("dev-{candidate_version}"),
            );
            if length == 0 {
                break;
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::{
        NO_VERSION_SET, RootVersion, from_env, is_feature_branch, mentions_self_version,
        root_version, split_lines,
    };
    use crate::error::Error;
    use serde_json::{Map, Value, json};
    use std::path::Path;
    use std::process::Command;

    fn obj(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(map) => map,
            other => panic!("not an object: {other}"),
        }
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "tag.gpgsign=false",
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

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "one"]);
        dir
    }

    fn head(dir: &Path) -> String {
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    #[test]
    fn version_in_composer_json_wins() {
        let dir = tempfile::tempdir().unwrap();
        let v = root_version(&obj(json!({"version": "2.1"})), dir.path(), Some("9.0.0")).unwrap();
        assert_eq!(
            v,
            RootVersion {
                pretty: "2.1".into(),
                normalized: "2.1.0.0".into(),
                reference: None
            }
        );
        let v = root_version(&obj(json!({"version": 3})), dir.path(), None).unwrap();
        assert_eq!(v.normalized, "3.0.0.0");
        assert!(root_version(&obj(json!({"version": "bad version"})), dir.path(), None).is_err());
    }

    #[test]
    fn environment_version_comes_next() {
        let dir = tempfile::tempdir().unwrap();
        let v = root_version(&Map::new(), dir.path(), Some("1.2-dev")).unwrap();
        assert_eq!(v.pretty, "1.2.x-dev");
        assert_eq!(v.normalized, "1.2.9999999.9999999-dev");
        assert_eq!(from_env("dev-main"), "dev-main");
        assert_eq!(from_env("3-DEV"), "3.x-dev");
        assert_eq!(from_env("1..2-dev"), "1..2-dev");
        assert_eq!(from_env("-dev"), "-dev");
    }

    #[test]
    fn no_vcs_gives_composer_default() {
        let dir = tempfile::tempdir().unwrap();
        let v = root_version(&Map::new(), dir.path(), None).unwrap();
        assert_eq!(
            v,
            RootVersion {
                pretty: NO_VERSION_SET.into(),
                normalized: "1.0.0.0".into(),
                reference: None
            }
        );
    }

    #[test]
    fn other_vcs_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".hg")).unwrap();
        assert!(matches!(
            root_version(&Map::new(), dir.path(), None),
            Err(Error::UnsupportedVcs(_))
        ));
    }

    #[test]
    fn git_main_branch() {
        let dir = repo();
        let v = root_version(&Map::new(), dir.path(), None).unwrap();
        assert_eq!(v.pretty, "dev-main");
        assert_eq!(v.normalized, "dev-main");
        assert_eq!(v.reference, Some(head(dir.path())));
    }

    #[test]
    fn git_numeric_branch_prints_as_x_dev() {
        let dir = repo();
        git(dir.path(), &["switch", "-q", "-c", "2.1"]);
        let v = root_version(&Map::new(), dir.path(), None).unwrap();
        assert_eq!(v.pretty, "2.1.x-dev");
        assert_eq!(v.normalized, "2.1.9999999.9999999-dev");
    }

    #[test]
    fn git_feature_branch_takes_nearest_version_branch() {
        let dir = repo();
        git(dir.path(), &["switch", "-q", "-c", "feature-x"]);
        git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "two"]);
        let v = root_version(&Map::new(), dir.path(), None).unwrap();
        assert_eq!(v.pretty, "dev-main");
        assert_eq!(v.normalized, "dev-main");
        assert_eq!(v.reference, Some(head(dir.path())));

        let aliased = obj(json!({"extra": {"branch-alias": {"dev-feature-x": "3.x-dev"}}}));
        let v = root_version(&aliased, dir.path(), None).unwrap();
        assert_eq!(v.pretty, "dev-feature-x");

        let non_feature = obj(json!({"non-feature-branches": ["feature-.*"]}));
        let v = root_version(&non_feature, dir.path(), None).unwrap();
        assert_eq!(v.pretty, "dev-feature-x");
    }

    #[test]
    fn git_detached_head_on_a_tag() {
        let dir = repo();
        git(dir.path(), &["tag", "v1.4.0"]);
        git(dir.path(), &["checkout", "-q", "--detach", "v1.4.0"]);
        let v = root_version(&Map::new(), dir.path(), None).unwrap();
        assert_eq!(v.pretty, "v1.4.0");
        assert_eq!(v.normalized, "1.4.0.0");
        assert_eq!(v.reference, Some(head(dir.path())));
    }

    #[test]
    fn git_detached_head_without_tag() {
        let dir = repo();
        git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "two"]);
        git(dir.path(), &["checkout", "-q", "--detach", "HEAD"]);
        let v = root_version(&Map::new(), dir.path(), None).unwrap();
        assert_eq!(v.pretty, "dev-main");
    }

    #[test]
    fn git_repo_without_commits_falls_through() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        let v = root_version(&Map::new(), dir.path(), None).unwrap();
        assert_eq!(v.pretty, NO_VERSION_SET);
    }

    #[test]
    fn helpers() {
        assert_eq!(split_lines("  a\r\nb\n"), ["a", "b"]);
        assert!(split_lines(" \n").is_empty());
        assert!(!is_feature_branch(&Map::new(), "main"));
        assert!(!is_feature_branch(&Map::new(), "1.x"));
        assert!(is_feature_branch(&Map::new(), "topic"));
        assert!(is_feature_branch(
            &obj(json!({"non-feature-branches": ["("]})),
            "topic"
        ));
        assert!(mentions_self_version(
            &json!({"require": {"a": ["self.version"]}})
        ));
        assert!(mentions_self_version(&json!({"self.version": 1})));
        assert!(!mentions_self_version(&json!({"a": 1})));
    }
}
