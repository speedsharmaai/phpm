//! Install one project with Composer and with phpm, at the same path one
//! after the other, and compare what each left behind.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use phpm_diffvendor::{Difference, Ignore, compare};
use serde_json::Value;

use crate::corpus::Project;
use crate::record::{Mode, Outcome, PhpmPath, Record, Run, phpm_path, tail};

/// Directories compared besides the vendor and bin dirs, when either side has them:
/// where installers, recipes and scripts write.
const EXTRA_DIRS: &[&str] = &["web", "public", "config", "bootstrap"];
const FIRST_DIFFERENCES: usize = 20;

#[derive(Debug, Clone)]
pub struct Config {
    /// Program and leading arguments for each tool.
    pub composer: Vec<String>,
    pub phpm: Vec<String>,
    /// Prefix for `<owner>/<name>.git`.
    pub git_base: String,
    pub fixtures: PathBuf,
    pub timeout: Duration,
    /// Scratch space for one project at a time; emptied before each.
    pub work: PathBuf,
    /// Composer's and phpm's caches, shared by every project of a run.
    pub caches: PathBuf,
    pub keep: bool,
    /// Tries per fetch, `retry_pause` times the try number apart.
    pub fetch_attempts: u32,
    pub retry_pause: Duration,
}

/// `f` until it succeeds, at most `attempts` times; the last error otherwise.
pub fn retry<T>(
    attempts: u32,
    pause: Duration,
    mut f: impl FnMut() -> Result<T, String>,
) -> Result<T, String> {
    let mut tried = 1;
    loop {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) if tried >= attempts => return Err(e),
            Err(_) => {
                std::thread::sleep(pause * tried);
                tried += 1;
            }
        }
    }
}

struct Finished {
    exit: Option<i32>,
    timed_out: bool,
    seconds: f64,
    stderr: String,
}

impl Finished {
    fn run(&self) -> Run {
        let ok = self.exit == Some(0) && !self.timed_out;
        Run {
            exit: self.exit,
            seconds: (self.seconds * 1000.0).round() / 1000.0,
            timed_out: self.timed_out,
            stderr_tail: (!ok).then(|| tail(&self.stderr, 20, 2000)),
        }
    }
}

/// Leave out every `GIT_*` variable, so a run inside a git hook (where
/// `GIT_DIR` and `GIT_INDEX_FILE` point at this repository) never touches it.
pub fn without_git_env(cmd: &mut Command) -> &mut Command {
    for k in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
    ] {
        cmd.env_remove(k);
    }
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(k);
        }
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0")
}

fn exec(
    argv: &[String],
    cwd: &Path,
    env: &[(&str, &Path)],
    logs: &Path,
    timeout: Duration,
) -> io::Result<Finished> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty command"))?;
    let err_file = logs.with_extension("err");
    let mut cmd = Command::new(program);
    without_git_env(&mut cmd)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(fs::File::create(logs.with_extension("out"))?)
        .stderr(fs::File::create(&err_file)?)
        .env("COMPOSER_NO_INTERACTION", "1");
    for var in [
        "COMPOSER",
        "COMPOSER_VENDOR_DIR",
        "COMPOSER_BIN_DIR",
        "COMPOSER_ROOT_VERSION",
    ] {
        cmd.env_remove(var);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let started = Instant::now();
    let mut child = cmd.spawn()?;
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait()? {
            break (Some(status), false);
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            break (None, true);
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let seconds = started.elapsed().as_secs_f64();
    Ok(Finished {
        exit: status.and_then(|s| s.code()),
        timed_out,
        seconds,
        stderr: String::from_utf8_lossy(&fs::read(&err_file)?).into_owned(),
    })
}

fn git(args: &[&str], cwd: &Path) -> Result<(), String> {
    let out = without_git_env(&mut Command::new("git"))
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "git {}: {}",
            args.first().copied().unwrap_or_default(),
            tail(&String::from_utf8_lossy(&out.stderr), 5, 500)
        ))
    }
}

/// The project's tree at its pin, with a fixture's files over it when the
/// project needs one.
fn prepare(p: &Project, cfg: &Config, src: &Path) -> Result<(), String> {
    fs::create_dir_all(src).map_err(|e| format!("{}: {e}", src.display()))?;
    if let Some(repo) = &p.repo {
        let commit = p.commit.as_deref().ok_or("no commit pinned")?;
        let url = format!("{}{repo}.git", cfg.git_base);
        git(&["init", "-q"], src)?;
        retry(cfg.fetch_attempts, cfg.retry_pause, || {
            git(&["fetch", "-q", "--depth", "1", &url, commit], src)
        })?;
        git(
            &[
                "-c",
                "advice.detachedHead=false",
                "checkout",
                "-q",
                "FETCH_HEAD",
            ],
            src,
        )?;
    }
    if let Some(f) = &p.fixture {
        let dir = cfg.fixtures.join(f);
        for (from, to) in [
            ("fixture.json", "composer.json"),
            ("fixture.lock", "composer.lock"),
        ] {
            fs::copy(dir.join(from), src.join(to))
                .map_err(|e| format!("{}: {e}", dir.join(from).display()))?;
        }
    }
    if p.repo.is_none() && p.fixture.is_none() {
        return Err("neither a repository nor a fixture".to_owned());
    }
    Ok(())
}

/// Copy a tree, keeping file modes and symlinks as links.
pub fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for item in fs::read_dir(from)? {
        let item = item?;
        let (src, dst) = (item.path(), to.join(item.file_name()));
        let kind = item.file_type()?;
        if kind.is_symlink() {
            link(&fs::read_link(&src)?, &dst, &src)?;
        } else if kind.is_dir() {
            copy_tree(&src, &dst)?;
        } else {
            fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn link(target: &Path, dst: &Path, _src: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, dst)
}

#[cfg(windows)]
fn link(target: &Path, dst: &Path, src: &Path) -> io::Result<()> {
    if fs::metadata(src).is_ok_and(|m| m.is_dir()) {
        std::os::windows::fs::symlink_dir(target, dst)
    } else {
        std::os::windows::fs::symlink_file(target, dst)
    }
}

fn count_packages(lock: &Value) -> usize {
    ["packages", "packages-dev"]
        .iter()
        .filter_map(|k| lock.get(k).and_then(Value::as_array))
        .map(Vec::len)
        .sum()
}

/// The vendor dir, the bin dir when it sits outside it, and the extra dirs.
fn compared_dirs(composer_json: &Value) -> Vec<String> {
    let config = |k: &str| {
        composer_json
            .get("config")
            .and_then(|c| c.get(k))
            .and_then(Value::as_str)
            .map(|s| s.trim_end_matches('/').to_owned())
    };
    let vendor = config("vendor-dir").unwrap_or_else(|| "vendor".to_owned());
    let mut dirs = vec![vendor.clone()];
    if let Some(bin) = config("bin-dir")
        && !bin.starts_with(&format!("{vendor}/"))
        && bin != vendor
    {
        dirs.push(bin);
    }
    for d in EXTRA_DIRS {
        if !dirs.iter().any(|x| x == d) {
            dirs.push((*d).to_owned());
        }
    }
    dirs
}

fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn describe(dir: &str, d: &Difference) -> String {
    let at = |p: &Path| format!("{dir}/{}", slash(p));
    match d {
        Difference::OnlyInLeft(p) => format!("{}: only from composer", at(p)),
        Difference::OnlyInRight(p) => format!("{}: only from phpm", at(p)),
        Difference::KindDiffers(p) => format!("{}: file kind differs", at(p)),
        Difference::ContentDiffers { path, offset } => {
            format!("{}: bytes differ at offset {offset}", at(path))
        }
        Difference::ModeDiffers { path, left, right } => {
            format!(
                "{}: mode {left:o} from composer, {right:o} from phpm",
                at(path)
            )
        }
        Difference::SymlinkTargetDiffers(p) => format!("{}: link target differs", at(p)),
    }
}

/// Every difference between the two trees under `dirs`, and the dirs that
/// existed on either side.
/// `autoload_real.php` with the `APCu` prefix blanked: Composer picks a random
/// one per dump unless `apcu-autoloader-prefix` is set.
// Composer: Autoload/AutoloadGenerator.php getAutoloadRealFile, bin2hex(random_bytes(10))
fn mask_apcu_prefix(text: &[u8]) -> Vec<u8> {
    const CALL: &[u8] = b"$loader->setApcuPrefix('";
    let mut out = text.to_vec();
    let mut from = 0;
    while let Some(i) = out[from..].windows(CALL.len()).position(|w| w == CALL) {
        let start = from + i + CALL.len();
        let end = start + 20;
        let random = out
            .get(start..end)
            .is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit))
            && out.get(end) == Some(&b'\'');
        if random {
            out[start..end].fill(b'0');
        }
        from = start;
    }
    out
}

/// Whether a content difference is only Composer's random `APCu` prefix.
fn only_apcu_prefix(composer: &Path, phpm: &Path, rel: &Path) -> bool {
    if rel.file_name().is_none_or(|n| n != "autoload_real.php") {
        return false;
    }
    match (fs::read(composer.join(rel)), fs::read(phpm.join(rel))) {
        (Ok(a), Ok(b)) => mask_apcu_prefix(&a) == mask_apcu_prefix(&b),
        _ => false,
    }
}

/// The dirs that existed on either side, every difference between them, and
/// the differences left out because Composer itself is not deterministic there.
fn compare_trees(
    composer: &Path,
    phpm: &Path,
    dirs: &[String],
) -> io::Result<(Vec<String>, Vec<String>, Vec<String>)> {
    let mut compared = Vec::new();
    let mut diffs = Vec::new();
    let mut normalized = Vec::new();
    for dir in dirs {
        let (c, p) = (composer.join(dir), phpm.join(dir));
        match (c.is_dir(), p.is_dir()) {
            (false, false) => continue,
            (true, false) => diffs.push(format!("{dir}/: only from composer")),
            (false, true) => diffs.push(format!("{dir}/: only from phpm")),
            (true, true) => {
                for d in compare(&c, &p, &Ignore::default())? {
                    match &d {
                        Difference::ContentDiffers { path, .. }
                            if only_apcu_prefix(&c, &p, path) =>
                        {
                            normalized.push(format!(
                                "{dir}/{}: Composer's random APCu prefix",
                                slash(path)
                            ));
                        }
                        _ => diffs.push(describe(dir, &d)),
                    }
                }
            }
        }
        compared.push(dir.clone());
    }
    Ok((compared, diffs, normalized))
}

/// Files Composer writes from its package map, whose order on a first
/// install follows the order packages finished installing, not the
/// `installed.json` order every later dump uses.
const AUTOLOAD_FILES: &[&str] = &[
    "include_paths.php",
    "autoload_files.php",
    "autoload_static.php",
    "autoload_psr4.php",
    "autoload_namespaces.php",
    "autoload_classmap.php",
];

/// Whether a difference is in one of those files under the vendor dir.
fn in_autoloader(diff: &str, vendor: &str) -> bool {
    diff.split_once(": ")
        .and_then(|(path, _)| path.strip_prefix(&format!("{vendor}/composer/")))
        .is_some_and(|name| AUTOLOAD_FILES.contains(&name))
}

/// `composer dump-autoload` that only rewrites the autoloader.
fn redump_argv(composer: &[String]) -> Vec<String> {
    let args = [
        "dump-autoload",
        "--no-interaction",
        "--no-scripts",
        "--no-plugins",
        "--ignore-platform-reqs",
    ];
    composer
        .iter()
        .cloned()
        .chain(args.iter().map(|s| (*s).to_owned()))
        .collect()
}

fn clean(dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Install `p` both ways in `mode` and say how the results compare.
pub fn run_project(p: &Project, mode: Mode, cfg: &Config) -> Record {
    let mut rec = Record {
        repo: p.repo.clone(),
        commit: p.commit.clone(),
        fixture: p.fixture.clone(),
        stars: p.stars,
        mode,
        os: std::env::consts::OS.to_owned(),
        outcome: Outcome::FetchFailed,
        packages: None,
        compared: Vec::new(),
        differences: 0,
        first_differences: Vec::new(),
        normalized: Vec::new(),
        phpm_path: PhpmPath::Unknown,
        phpm_reasons: Vec::new(),
        composer: None,
        phpm: None,
        error: None,
    };
    if let Err(e) = install_both(p, mode, cfg, &mut rec) {
        rec.error = Some(e);
    }
    if !cfg.keep {
        for d in ["src", "project", "composer", "phpm"] {
            let _ = clean(&cfg.work.join(d));
        }
    }
    rec
}

fn install_both(p: &Project, mode: Mode, cfg: &Config, rec: &mut Record) -> Result<(), String> {
    let io_err = |e: io::Error| e.to_string();
    let (src, project) = (cfg.work.join("src"), cfg.work.join("project"));
    let (composer_out, phpm_out) = (cfg.work.join("composer"), cfg.work.join("phpm"));
    for d in [&src, &project, &composer_out, &phpm_out] {
        clean(d).map_err(io_err)?;
    }
    prepare(p, cfg, &src)?;
    let read = |name: &str| -> Result<Value, String> {
        let bytes = fs::read(src.join(name)).map_err(|_| format!("no {name} at the pin"))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("{name}: {e}"))
    };
    let composer_json = read("composer.json")?;
    rec.packages = Some(count_packages(&read("composer.lock")?));

    let home = cfg.work.join("home");
    fs::create_dir_all(&home).map_err(io_err)?;
    fs::write(
        home.join("config.json"),
        r#"{"config": {"notify-on-install": false}}"#,
    )
    .map_err(io_err)?;
    let composer_cache = cfg.caches.join("composer");
    let phpm_cache = cfg.caches.join("phpm");
    let env: [(&str, &Path); 3] = [
        ("COMPOSER_HOME", &home),
        ("COMPOSER_CACHE_DIR", &composer_cache),
        ("PHPM_CACHE_DIR", &phpm_cache),
    ];
    let common = ["install", "--no-interaction", "--no-progress"];
    let argv = |tool: &[String], extra: &[&str]| -> Vec<String> {
        tool.iter()
            .cloned()
            .chain(
                common
                    .iter()
                    .chain(extra)
                    .chain(mode.flags())
                    .map(|s| (*s).to_owned()),
            )
            .collect()
    };

    copy_tree(&src, &project).map_err(io_err)?;
    let c = exec(
        &argv(&cfg.composer, &[]),
        &project,
        &env,
        &cfg.work.join("composer-log"),
        cfg.timeout,
    )
    .map_err(|e| format!("composer: {e}"))?;
    fs::rename(&project, &composer_out).map_err(io_err)?;
    rec.composer = Some(c.run());
    if !rec.composer.as_ref().is_some_and(Run::ok) {
        rec.outcome = Outcome::ComposerFailed;
        return Ok(());
    }

    copy_tree(&src, &project).map_err(io_err)?;
    let ph = exec(
        &argv(&cfg.phpm, &["--explain"]),
        &project,
        &env,
        &cfg.work.join("phpm-log"),
        cfg.timeout,
    )
    .map_err(|e| format!("phpm: {e}"))?;
    fs::rename(&project, &phpm_out).map_err(io_err)?;
    (rec.phpm_path, rec.phpm_reasons) = phpm_path(&ph.stderr);
    let run = ph.run();
    let ok = run.ok();
    rec.phpm = Some(run);
    if !ok {
        rec.outcome = Outcome::PhpmFailed;
        return Ok(());
    }

    let dirs = compared_dirs(&composer_json);
    let (compared, mut diffs, mut normalized) =
        compare_trees(&composer_out, &phpm_out, &dirs).map_err(io_err)?;
    if !diffs.is_empty() && diffs.iter().all(|d| in_autoloader(d, &dirs[0])) {
        fs::rename(&composer_out, &project).map_err(io_err)?;
        let dump = exec(
            &redump_argv(&cfg.composer),
            &project,
            &env,
            &cfg.work.join("redump-log"),
            cfg.timeout,
        );
        fs::rename(&project, &composer_out).map_err(io_err)?;
        if dump.is_ok_and(|d| d.run().ok()) {
            let (_, again, _) = compare_trees(&composer_out, &phpm_out, &dirs).map_err(io_err)?;
            if again.is_empty() {
                normalized.extend(diffs.drain(..).map(|d| {
                    format!("{d} (Composer's first-install package order; identical after composer dump-autoload)")
                }));
            }
        }
    }
    rec.compared = compared;
    rec.normalized = normalized;
    rec.differences = diffs.len();
    rec.first_differences = diffs.into_iter().take(FIRST_DIFFERENCES).collect();
    rec.outcome = if rec.differences == 0 {
        Outcome::Identical
    } else {
        Outcome::Different
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        compared_dirs, count_packages, describe, in_autoloader, mask_apcu_prefix, redump_argv,
        retry, without_git_env,
    };
    use phpm_diffvendor::Difference;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn autoloader_differences_are_recognised() {
        assert!(in_autoloader(
            "vendor/composer/include_paths.php: bytes differ at offset 3",
            "vendor"
        ));
        assert!(in_autoloader(
            "lib/composer/autoload_files.php: bytes differ at offset 3",
            "lib"
        ));
        assert!(!in_autoloader(
            "vendor/composer/installed.json: bytes differ at offset 3",
            "vendor"
        ));
        assert!(!in_autoloader(
            "vendor/a/composer/autoload_files.php: x",
            "vendor"
        ));
        assert!(!in_autoloader("garbage", "vendor"));
        assert_eq!(
            redump_argv(&["php".to_owned(), "c.phar".to_owned()])[..3],
            ["php", "c.phar", "dump-autoload"]
        );
    }

    #[test]
    fn masks_only_a_random_apcu_prefix() {
        let real = b"x\n        $loader->setApcuPrefix('cd8701d0e42551dfc35f');\n";
        let masked = mask_apcu_prefix(real);
        assert_eq!(
            masked,
            b"x\n        $loader->setApcuPrefix('00000000000000000000');\n"
        );
        let mine = b"$loader->setApcuPrefix('my-own-prefix');";
        assert_eq!(mask_apcu_prefix(mine), mine);
        let short = b"$loader->setApcuPrefix('abc";
        assert_eq!(mask_apcu_prefix(short), short);
    }

    #[test]
    fn retries_until_success_or_the_last_attempt() {
        let mut calls = 0;
        let ok = retry(3, std::time::Duration::ZERO, || {
            calls += 1;
            if calls < 2 {
                Err("flaky".to_owned())
            } else {
                Ok(calls)
            }
        });
        assert_eq!(ok, Ok(2));
        let mut calls = 0;
        let err: Result<(), String> = retry(3, std::time::Duration::ZERO, || {
            calls += 1;
            Err(format!("try {calls}"))
        });
        assert_eq!(err, Err("try 3".to_owned()));
        assert_eq!(
            retry(0, std::time::Duration::ZERO, || Err::<(), _>(
                "x".to_owned()
            )),
            Err("x".to_owned())
        );
    }

    #[test]
    fn git_hook_variables_never_reach_children() {
        let mut cmd = std::process::Command::new("git");
        cmd.env("GIT_DIR", "/repo/.git")
            .env("GIT_INDEX_FILE", "/repo/index");
        without_git_env(&mut cmd);
        let envs: Vec<_> = cmd.get_envs().collect();
        for k in ["GIT_DIR", "GIT_INDEX_FILE"] {
            assert!(envs.contains(&(std::ffi::OsStr::new(k), None)), "{envs:?}");
        }
    }

    #[test]
    fn counts_prod_and_dev_packages() {
        let lock = json!({"packages": [{}, {}], "packages-dev": [{}]});
        assert_eq!(count_packages(&lock), 3);
        assert_eq!(count_packages(&json!({})), 0);
    }

    #[test]
    fn compares_the_configured_vendor_and_bin_dirs() {
        assert_eq!(
            compared_dirs(&json!({})),
            ["vendor", "web", "public", "config", "bootstrap"]
        );
        assert_eq!(
            compared_dirs(&json!({"config": {"vendor-dir": "lib/composer/", "bin-dir": "bin"}})),
            [
                "lib/composer",
                "bin",
                "web",
                "public",
                "config",
                "bootstrap"
            ]
        );
        assert_eq!(
            compared_dirs(&json!({"config": {"bin-dir": "vendor/bin", "vendor-dir": "public"}}))
                .iter()
                .filter(|d| *d == "public")
                .count(),
            1
        );
    }

    #[test]
    fn describes_each_kind_of_difference() {
        let p = PathBuf::from("a/b.php");
        let all = [
            Difference::OnlyInLeft(p.clone()),
            Difference::OnlyInRight(p.clone()),
            Difference::KindDiffers(p.clone()),
            Difference::ContentDiffers {
                path: p.clone(),
                offset: 7,
            },
            Difference::ModeDiffers {
                path: p.clone(),
                left: 0o755,
                right: 0o644,
            },
            Difference::SymlinkTargetDiffers(p),
        ];
        let text: Vec<String> = all.iter().map(|d| describe("vendor", d)).collect();
        assert_eq!(
            text,
            [
                "vendor/a/b.php: only from composer",
                "vendor/a/b.php: only from phpm",
                "vendor/a/b.php: file kind differs",
                "vendor/a/b.php: bytes differ at offset 7",
                "vendor/a/b.php: mode 755 from composer, 644 from phpm",
                "vendor/a/b.php: link target differs",
            ]
        );
    }

    #[cfg(unix)]
    mod unix {
        use super::super::{Config, copy_tree, run_project, without_git_env};
        use crate::corpus::Project;
        use crate::record::{Mode, Outcome, PhpmPath};
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::path::{Path, PathBuf};
        use std::process::Command;
        use std::time::Duration;

        const NATIVE: &str =
            "echo 'decision install: native, phpm fetches and places every package' >&2";

        struct Env {
            _tmp: tempfile::TempDir,
            root: PathBuf,
        }

        impl Env {
            fn new() -> Self {
                let tmp = tempfile::tempdir().unwrap();
                let root = tmp.path().to_path_buf();
                Self { _tmp: tmp, root }
            }

            fn script(&self, name: &str, body: &str) -> Vec<String> {
                let path = self.root.join(name);
                fs::write(&path, format!("#!/bin/sh\nset -e\n{body}\n")).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
                vec![path.to_string_lossy().into_owned()]
            }

            fn config(&self, composer: &str, phpm: &str) -> Config {
                Config {
                    composer: self.script("composer", composer),
                    phpm: self.script("phpm", phpm),
                    git_base: format!("file://{}/repos/", self.root.display()),
                    fixtures: self.root.join("fixtures"),
                    timeout: Duration::from_secs(20),
                    work: self.root.join("work"),
                    caches: self.root.join("caches"),
                    keep: false,
                    fetch_attempts: 2,
                    retry_pause: Duration::ZERO,
                }
            }

            /// A repository `o/<name>` with the given files, and its head commit.
            fn repo(&self, name: &str, files: &[(&str, &str)]) -> String {
                let dir = self.root.join("repos/o").join(format!("{name}.git"));
                fs::create_dir_all(&dir).unwrap();
                for (path, text) in files {
                    fs::write(dir.join(path), text).unwrap();
                }
                let git = |args: &[&str]| {
                    let out = without_git_env(&mut Command::new("git"))
                        .args(["-c", "commit.gpgsign=false"])
                        .args(args)
                        .current_dir(&dir)
                        .env("GIT_AUTHOR_NAME", "t")
                        .env("GIT_AUTHOR_EMAIL", "t@t")
                        .env("GIT_COMMITTER_NAME", "t")
                        .env("GIT_COMMITTER_EMAIL", "t@t")
                        .output()
                        .unwrap();
                    assert!(out.status.success(), "{out:?}");
                    String::from_utf8(out.stdout).unwrap().trim().to_owned()
                };
                git(&["init", "-q"]);
                git(&["config", "uploadpack.allowAnySHA1InWant", "true"]);
                git(&["add", "-A"]);
                git(&["commit", "-q", "-m", "x"]);
                git(&["rev-parse", "HEAD"])
            }

            fn app(&self) -> Project {
                let commit = self.repo(
                    "app",
                    &[
                        ("composer.json", r#"{"name": "o/app"}"#),
                        ("composer.lock", r#"{"packages": [{}], "packages-dev": []}"#),
                    ],
                );
                Project {
                    repo: Some("o/app".to_owned()),
                    commit: Some(commit),
                    stars: Some(5),
                    fixture: None,
                }
            }
        }

        fn writes(file: &str, text: &str) -> String {
            format!("mkdir -p vendor/a && printf '{text}' > vendor/a/{file}")
        }

        #[test]
        fn identical_trees_are_identical() {
            let e = Env::new();
            let cfg = e.config(
                &writes("f.txt", "x"),
                &format!("{}\n{NATIVE}", writes("f.txt", "x")),
            );
            let r = run_project(&e.app(), Mode::Pure, &cfg);
            assert_eq!(r.outcome, Outcome::Identical, "{r:?}");
            assert_eq!(r.phpm_path, PhpmPath::Native);
            assert_eq!(r.packages, Some(1));
            assert_eq!(r.compared, ["vendor"]);
            assert_eq!(r.stars, Some(5));
            assert!(r.composer.unwrap().stderr_tail.is_none());
            assert!(!cfg.work.join("src").exists(), "work dirs are cleaned");
            let config = fs::read_to_string(cfg.work.join("home/config.json")).unwrap();
            assert!(config.contains(r#""notify-on-install": false"#));
        }

        #[test]
        fn both_tools_get_the_same_flags_and_path() {
            let e = Env::new();
            let record = "mkdir -p vendor && echo \"$PWD $*\" > vendor/args";
            let cfg = e.config(record, &format!("{record}\n{NATIVE}"));
            let mut cfg = cfg;
            cfg.keep = true;
            let r = run_project(&e.app(), Mode::Default, &cfg);
            assert_eq!(r.outcome, Outcome::Different);
            let c = fs::read_to_string(cfg.work.join("composer/vendor/args")).unwrap();
            let p = fs::read_to_string(cfg.work.join("phpm/vendor/args")).unwrap();
            assert_eq!(p.replace(" --explain", ""), c);
            assert!(
                c.ends_with("install --no-interaction --no-progress --ignore-platform-reqs\n"),
                "{c}"
            );
        }

        #[test]
        fn different_bytes_and_extra_files_are_listed() {
            let e = Env::new();
            let cfg = e.config(
                &format!(
                    "{}\nmkdir -p public && touch public/x",
                    writes("f.txt", "x")
                ),
                &format!(
                    "{}\n{}\n{NATIVE}",
                    writes("f.txt", "y"),
                    writes("g.txt", "z")
                ),
            );
            let r = run_project(&e.app(), Mode::Pure, &cfg);
            assert_eq!(r.outcome, Outcome::Different);
            assert_eq!(r.differences, 3);
            assert_eq!(
                r.first_differences,
                [
                    "vendor/a/f.txt: bytes differ at offset 0",
                    "vendor/a/g.txt: only from phpm",
                    "public/: only from composer",
                ]
            );
        }

        #[test]
        fn a_random_apcu_prefix_is_not_a_difference() {
            let e = Env::new();
            let app = e.app();
            let real = |prefix: &str| {
                format!(
                    "mkdir -p vendor/composer && echo \"\\$loader->setApcuPrefix('{prefix}');\" > vendor/composer/autoload_real.php"
                )
            };
            let cfg = e.config(
                &real("cd8701d0e42551dfc35f"),
                &format!("{}\n{NATIVE}", real("13248f148288792ea009")),
            );
            let r = run_project(&app, Mode::Pure, &cfg);
            assert_eq!(r.outcome, Outcome::Identical, "{r:?}");
            assert_eq!(
                r.normalized,
                ["vendor/composer/autoload_real.php: Composer's random APCu prefix"]
            );
            let cfg = e.config(
                &real("cd8701d0e42551dfc35f"),
                &format!("{}\n{NATIVE}", real("mine")),
            );
            let r = run_project(&app, Mode::Pure, &cfg);
            assert_eq!(r.outcome, Outcome::Different);
        }

        #[test]
        fn first_install_order_is_checked_against_a_redump() {
            let e = Env::new();
            let app = e.app();
            let order = |first: &str, later: &str| {
                format!(
                    "mkdir -p vendor/composer\nif [ \"$1\" = dump-autoload ]; then echo {later} > vendor/composer/autoload_files.php; else echo {first} > vendor/composer/autoload_files.php; fi"
                )
            };
            let cfg = e.config(
                &order("one-two", "two-one"),
                &format!("{}\n{NATIVE}", order("two-one", "two-one")),
            );
            let r = run_project(&app, Mode::Pure, &cfg);
            assert_eq!(r.outcome, Outcome::Identical, "{r:?}");
            assert_eq!(r.normalized.len(), 1);
            assert!(r.normalized[0].contains("identical after composer dump-autoload"));

            let cfg = e.config(
                &order("one-two", "one-two"),
                &format!("{}\n{NATIVE}", order("two-one", "two-one")),
            );
            let r = run_project(&app, Mode::Pure, &cfg);
            assert_eq!(r.outcome, Outcome::Different);
            assert_eq!(
                r.first_differences,
                ["vendor/composer/autoload_files.php: bytes differ at offset 0"]
            );
        }

        #[test]
        fn composer_failures_skip_phpm() {
            let e = Env::new();
            let cfg = e.config("echo 'dead dist url' >&2; exit 3", "touch ran");
            let r = run_project(&e.app(), Mode::Pure, &cfg);
            assert_eq!(r.outcome, Outcome::ComposerFailed);
            let c = r.composer.unwrap();
            assert_eq!(c.exit, Some(3));
            assert_eq!(c.stderr_tail.as_deref(), Some("dead dist url"));
            assert!(r.phpm.is_none());
        }

        #[test]
        fn phpm_failures_keep_the_explain_reasons() {
            let e = Env::new();
            let cfg = e.config(
                &writes("f.txt", "x"),
                "echo 'decision install: fallback to composer install, composer/installers places packages' >&2; exit 1",
            );
            let r = run_project(&e.app(), Mode::Default, &cfg);
            assert_eq!(r.outcome, Outcome::PhpmFailed);
            assert_eq!(r.phpm_path, PhpmPath::Full);
            assert_eq!(r.phpm_reasons, ["composer/installers places packages"]);
            assert_eq!(r.phpm.unwrap().exit, Some(1));
        }

        #[test]
        fn a_slow_install_times_out() {
            let e = Env::new();
            let mut cfg = e.config("sleep 5", "true");
            cfg.timeout = Duration::from_millis(200);
            let r = run_project(&e.app(), Mode::Pure, &cfg);
            assert_eq!(r.outcome, Outcome::ComposerFailed);
            let c = r.composer.unwrap();
            assert!(c.timed_out);
            assert_eq!(c.exit, None);
        }

        #[test]
        fn unreachable_repositories_and_missing_locks_are_fetch_failures() {
            let e = Env::new();
            let cfg = e.config("true", "true");
            let gone = Project {
                repo: Some("o/gone".to_owned()),
                commit: Some("0".repeat(40)),
                stars: None,
                fixture: None,
            };
            let r = run_project(&gone, Mode::Pure, &cfg);
            assert_eq!(r.outcome, Outcome::FetchFailed);
            assert!(r.error.unwrap().starts_with("git fetch"));

            let commit = e.repo("nolock", &[("composer.json", "{}")]);
            let nolock = Project {
                repo: Some("o/nolock".to_owned()),
                commit: Some(commit),
                stars: None,
                fixture: None,
            };
            let r = run_project(&nolock, Mode::Pure, &cfg);
            assert_eq!(r.error.as_deref(), Some("no composer.lock at the pin"));

            let neither = Project {
                repo: None,
                commit: None,
                stars: None,
                fixture: None,
            };
            let r = run_project(&neither, Mode::Pure, &cfg);
            assert_eq!(
                r.error.as_deref(),
                Some("neither a repository nor a fixture")
            );
        }

        #[test]
        fn fixtures_install_alone_or_over_their_app() {
            let e = Env::new();
            let fx = e.root.join("fixtures/ours");
            fs::create_dir_all(&fx).unwrap();
            fs::write(
                fx.join("fixture.json"),
                r#"{"config": {"vendor-dir": "lib"}}"#,
            )
            .unwrap();
            fs::write(fx.join("fixture.lock"), r#"{"packages": [{}, {}]}"#).unwrap();
            let both = "mkdir -p lib && cp composer.json lib/";
            let cfg = e.config(both, &format!("{both}\n{NATIVE}"));
            let alone = Project {
                repo: None,
                commit: None,
                stars: None,
                fixture: Some("ours".to_owned()),
            };
            let r = run_project(&alone, Mode::Pure, &cfg);
            assert_eq!(r.outcome, Outcome::Identical, "{r:?}");
            assert_eq!((r.packages, r.compared), (Some(2), vec!["lib".to_owned()]));

            let mut over = e.app();
            over.fixture = Some("ours".to_owned());
            let r = run_project(&over, Mode::Pure, &cfg);
            assert_eq!(r.packages, Some(2));
            assert_eq!(r.outcome, Outcome::Identical);
        }

        #[test]
        fn copies_trees_with_modes_and_links() {
            let e = Env::new();
            let from = e.root.join("from");
            fs::create_dir_all(from.join("d")).unwrap();
            fs::write(from.join("d/run"), "x").unwrap();
            fs::set_permissions(from.join("d/run"), fs::Permissions::from_mode(0o750)).unwrap();
            std::os::unix::fs::symlink("d/run", from.join("link")).unwrap();
            let to = e.root.join("to");
            copy_tree(&from, &to).unwrap();
            let mode = fs::metadata(to.join("d/run")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o750);
            assert_eq!(fs::read_link(to.join("link")).unwrap(), Path::new("d/run"));
            assert!(
                phpm_diffvendor::compare(&from, &to, &phpm_diffvendor::Ignore::default())
                    .unwrap()
                    .is_empty()
            );
        }
    }
}
