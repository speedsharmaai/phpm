//! Runs the real Composer on every fixture and compares
//! vendor/composer/installed.json, installed.php and InstalledVersions.php
//! byte for byte. Needs `composer`, `php` and the network, so it is ignored;
//! run it with `just golden`. `PHPM_BLESS=1` (`just golden-bless`) also
//! refreshes the committed golden files for the small fixtures.
//!
//! Each fixture is installed in a fresh temp directory outside this git
//! repository, and phpm guesses the root version from that same directory,
//! so both tools see the same VCS state (none).

use phpm_lock::{
    COMPOSER_VERSION, ComposerJson, INSTALLED_VERSIONS_PHP, InstallContext, Lock, root_version,
};
use phpm_testkit::assert_bytes_eq;
use std::path::Path;
use std::process::Command;

const COMMITTED: [&str; 2] = ["wicketyaari", "ytmate"];

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
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
        .output()
        .expect("live test setup");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn live(fixture: &str, dev_mode: bool) {
    live_with(fixture, dev_mode, |_| {});
}

fn live_with(fixture: &str, dev_mode: bool, setup: impl Fn(&Path)) {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = manifest.join("../../fixtures").join(fixture);
    let work = tempfile::tempdir().expect("live test setup");
    let root = work.path().canonicalize().expect("live test setup");
    for (from, to) in [
        ("fixture.json", "composer.json"),
        ("fixture.lock", "composer.lock"),
    ] {
        std::fs::copy(source.join(from), root.join(to)).expect("live test setup");
    }
    setup(&root);

    let version = Command::new("composer")
        .arg("--version")
        .output()
        .expect("composer on PATH");
    assert!(
        String::from_utf8_lossy(&version.stdout).contains(COMPOSER_VERSION),
        "golden files need Composer {COMPOSER_VERSION}"
    );

    let mut install = Command::new("composer");
    install
        .args([
            "install",
            "--no-scripts",
            "--no-plugins",
            "--no-autoloader",
            "--ignore-platform-reqs",
            "--no-interaction",
            "-q",
        ])
        .current_dir(&root)
        .env_remove("COMPOSER_ROOT_VERSION");
    if !dev_mode {
        install.arg("--no-dev");
    }
    let status = install.status().expect("live test setup");
    assert!(status.success(), "composer install failed for {fixture}");

    let composer = ComposerJson::parse(
        &std::fs::read_to_string(root.join("composer.json")).expect("live test setup"),
    )
    .expect("live test setup");
    let lock =
        Lock::parse(&std::fs::read_to_string(root.join("composer.lock")).expect("live test setup"))
            .expect("live test setup");
    let root_version = root_version(composer.data(), &root, None).expect("live test setup");
    assert_eq!(
        root.join(".git").exists(),
        root_version.reference.is_some(),
        "{root_version:?}"
    );
    let root_dir = root.to_string_lossy().replace('\\', "/");
    let ctx = InstallContext {
        composer_json: &composer,
        lock: &lock,
        root_version: &root_version,
        root_dir: &root_dir,
        dev_mode,
        install_paths: &phpm_lock::InstallPaths::new(),
    };
    let ours = phpm_lock::installed_files(&ctx).expect("live test setup");

    let theirs = root.join("vendor/composer");
    let installed_json = std::fs::read(theirs.join("installed.json")).expect("live test setup");
    let installed_php = std::fs::read(theirs.join("installed.php")).expect("live test setup");
    assert_bytes_eq(ours.installed_json.as_bytes(), &installed_json);
    assert_bytes_eq(ours.installed_php.as_bytes(), &installed_php);
    assert_bytes_eq(
        INSTALLED_VERSIONS_PHP.as_bytes(),
        &std::fs::read(theirs.join("InstalledVersions.php")).expect("live test setup"),
    );

    if std::env::var_os("PHPM_BLESS").is_some()
        && COMMITTED.contains(&fixture)
        && !root.join(".git").exists()
    {
        let golden = manifest
            .join("tests/golden")
            .join(fixture)
            .join(if dev_mode { "dev" } else { "no-dev" });
        std::fs::create_dir_all(&golden).expect("live test setup");
        std::fs::write(golden.join("installed.json"), installed_json).expect("live test setup");
        std::fs::write(golden.join("installed.php"), installed_php).expect("live test setup");
    }
}

macro_rules! live_tests {
    ($($name:ident: $fixture:literal, $dev:literal;)*) => {$(
        #[test]
        #[ignore = "runs composer install over the network"]
        fn $name() {
            live($fixture, $dev);
        }
    )*};
}

live_tests! {
    composer_live_wicketyaari_dev: "wicketyaari", true;
    composer_live_wicketyaari_no_dev: "wicketyaari", false;
    composer_live_ytmate_dev: "ytmate", true;
    composer_live_ytmate_no_dev: "ytmate", false;
    composer_live_laravel_skeleton_dev: "laravel-skeleton", true;
    composer_live_laravel_skeleton_no_dev: "laravel-skeleton", false;
    composer_live_symfony_demo_dev: "symfony-demo", true;
    composer_live_symfony_demo_no_dev: "symfony-demo", false;
    composer_live_monica_dev: "monica", true;
    composer_live_monica_no_dev: "monica", false;
}

fn repo(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["add", "composer.json", "composer.lock"]);
    git(dir, &["commit", "-q", "-m", "one"]);
}

#[test]
#[ignore = "runs composer install over the network"]
fn composer_live_git_main_branch() {
    live_with("wicketyaari", true, repo);
}

#[test]
#[ignore = "runs composer install over the network"]
fn composer_live_git_numeric_branch() {
    live_with("wicketyaari", true, |dir| {
        repo(dir);
        git(dir, &["switch", "-q", "-c", "2.3"]);
    });
}

#[test]
#[ignore = "runs composer install over the network"]
fn composer_live_git_feature_branch() {
    live_with("wicketyaari", true, |dir| {
        repo(dir);
        git(dir, &["switch", "-q", "-c", "1.x"]);
        git(dir, &["commit", "-q", "--allow-empty", "-m", "two"]);
        git(dir, &["switch", "-q", "-c", "topic"]);
        git(dir, &["commit", "-q", "--allow-empty", "-m", "three"]);
    });
}

#[test]
#[ignore = "runs composer install over the network"]
fn composer_live_git_detached_tag() {
    live_with("wicketyaari", true, |dir| {
        repo(dir);
        git(dir, &["tag", "v1.2.0"]);
        git(dir, &["checkout", "-q", "--detach", "v1.2.0"]);
    });
}
