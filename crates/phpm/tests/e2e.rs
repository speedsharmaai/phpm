//! `composer install` and `phpm install` on every fixture, in sibling temp
//! directories outside this repository, then the whole `vendor/` compared
//! byte for byte and mode for mode. Needs `composer`, `php` and the network,
//! so it is ignored; run it with `just e2e`.
//!
//! The `e2e_app_` tests install each fixture inside its real application with
//! scripts and plugins on, so the Composer fallback runs. Both tools install
//! at the same path in turn, because plugins write absolute paths. Run them
//! with `just e2e-apps`.

#![allow(clippy::unwrap_used, reason = "test helpers panic on setup failures")]

use std::path::Path;
use std::process::Command;

use phpm_diffvendor::{Ignore, compare};

fn prepare(fixture: &str, dir: &Path) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(fixture);
    for (from, to) in [
        ("fixture.json", "composer.json"),
        ("fixture.lock", "composer.lock"),
    ] {
        std::fs::copy(source.join(from), dir.join(to)).unwrap();
    }
}

fn run(cmd: &mut Command, what: &str) {
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{what} failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn e2e(fixture: &str, flags: &[&str]) {
    let work = tempfile::tempdir().unwrap();
    let root = work.path().canonicalize().unwrap();
    let (composer_dir, phpm_dir) = (root.join("composer"), root.join("phpm"));
    for dir in [&composer_dir, &phpm_dir] {
        std::fs::create_dir_all(dir).unwrap();
        prepare(fixture, dir);
    }

    run(
        Command::new("composer")
            .args([
                "install",
                "--no-scripts",
                "--no-plugins",
                "--no-interaction",
                "--no-progress",
                "-q",
            ])
            .args(flags)
            .current_dir(&composer_dir)
            .env_remove("COMPOSER_ROOT_VERSION"),
        "composer install",
    );
    assert!(composer_dir.join("vendor/autoload.php").is_file());
    for pass in ["first install", "again from the store"] {
        let _ = std::fs::remove_dir_all(phpm_dir.join("vendor"));
        run(
            Command::new(env!("CARGO_BIN_EXE_phpm"))
                .args(["install", "--no-scripts", "--no-plugins", "-q"])
                .args(flags)
                .current_dir(&phpm_dir)
                .env_remove("COMPOSER_ROOT_VERSION")
                .env("COMPOSER_DISABLE_NETWORK", "1"),
            "phpm install",
        );
        let diffs = compare(
            &composer_dir.join("vendor"),
            &phpm_dir.join("vendor"),
            &Ignore::default(),
        )
        .unwrap();
        let shown: Vec<String> = diffs.iter().take(20).map(ToString::to_string).collect();
        assert!(
            diffs.is_empty(),
            "{fixture} {flags:?} ({pass}): {} differences\n{}",
            diffs.len(),
            shown.join("\n")
        );
    }
}

macro_rules! e2e_tests {
    ($($name:ident: $fixture:literal $(, $flag:literal)*;)*) => {$(
        #[test]
        #[ignore = "runs composer install over the network"]
        fn $name() {
            e2e($fixture, &[$($flag),*]);
        }
    )*};
}

e2e_tests! {
    e2e_laravel_skeleton: "laravel-skeleton";
    e2e_laravel_skeleton_no_dev: "laravel-skeleton", "--no-dev";
    e2e_laravel_skeleton_authoritative: "laravel-skeleton", "--classmap-authoritative";
    e2e_ytmate: "ytmate";
    e2e_ytmate_no_dev: "ytmate", "--no-dev";
    e2e_wicketyaari: "wicketyaari";
}

/// `fixtures/apps.txt`: the repository and commit each fixture belongs to.
fn app(fixture: &str) -> (String, String) {
    let list = include_str!("../../../fixtures/apps.txt");
    let line = list
        .lines()
        .filter(|l| !l.starts_with('#'))
        .find(|l| l.split_whitespace().next() == Some(fixture))
        .unwrap();
    let parts: Vec<&str> = line.split_whitespace().collect();
    (parts[1].to_owned(), parts[2].to_owned())
}

fn checkout(fixture: &str, dir: &Path) {
    let (repo, commit) = app(fixture);
    std::fs::create_dir_all(dir).unwrap();
    let git = |args: &[&str]| run(Command::new("git").args(args).current_dir(dir), "git");
    git(&["init", "-q"]);
    git(&[
        "fetch",
        "-q",
        "--depth",
        "1",
        &format!("https://github.com/{repo}"),
        &commit,
    ]);
    git(&["checkout", "-q", "FETCH_HEAD"]);
    prepare(fixture, dir);
}

fn copy_tree(from: &Path, to: &Path) {
    run(Command::new("cp").arg("-Rp").arg(from).arg(to), "cp");
}

/// What a working install of each app answers to.
fn smoke(dir: &Path, check: &[&str]) {
    let out = Command::new(check[0])
        .args(&check[1..])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{check:?} failed in the phpm install:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn e2e_app(fixture: &str, flags: &[&str], trees: &[&str], check: &[&str]) {
    let work = tempfile::tempdir().unwrap();
    let root = work.path().canonicalize().unwrap();
    let (pristine, app_dir, composer_dir) = (
        root.join("pristine"),
        root.join("app"),
        root.join("composer"),
    );
    checkout(fixture, &pristine);

    copy_tree(&pristine, &app_dir);
    run(
        Command::new("composer")
            .args(["install", "--no-interaction", "--no-progress"])
            .args(flags)
            .current_dir(&app_dir)
            .env_remove("COMPOSER_ROOT_VERSION"),
        "composer install",
    );
    std::fs::rename(&app_dir, &composer_dir).unwrap();

    copy_tree(&pristine, &app_dir);
    run(
        Command::new(env!("CARGO_BIN_EXE_phpm"))
            .args(["install"])
            .args(flags)
            .current_dir(&app_dir)
            .env_remove("COMPOSER_ROOT_VERSION")
            .env_remove("PHPM_COMPOSER"),
        "phpm install",
    );
    smoke(&app_dir, check);

    let ignore = Ignore::names(&[".git", "var", "node_modules"]);
    for tree in trees {
        let diffs = compare(&composer_dir.join(tree), &app_dir.join(tree), &ignore).unwrap();
        let shown: Vec<String> = diffs.iter().take(20).map(ToString::to_string).collect();
        assert!(
            diffs.is_empty(),
            "{fixture} {tree}/: {} differences\n{}",
            diffs.len(),
            shown.join("\n")
        );
    }
}

macro_rules! e2e_app_tests {
    ($($name:ident: $fixture:literal, [$($flag:literal),*], [$($tree:literal),*], [$($check:literal),*];)*) => {$(
        #[test]
        #[ignore = "clones the app and runs composer install over the network"]
        fn $name() {
            e2e_app($fixture, &[$($flag),*], &[$($tree),*], &[$($check),*]);
        }
    )*};
}

e2e_app_tests! {
    e2e_app_laravel_skeleton: "laravel-skeleton", [], ["vendor", "bootstrap"], ["php", "artisan", "--version"];
    e2e_app_symfony_demo: "symfony-demo", [], ["vendor", "config", "public"], ["php", "bin/console", "about"];
    e2e_app_monica: "monica", ["--ignore-platform-reqs"], ["vendor", "bootstrap"], ["php", "artisan", "--version"];
    e2e_app_drupal_recommended: "drupal-recommended", [], ["vendor", "web", "recipes"],
        ["php", "-r", "require 'vendor/autoload.php'; echo Drupal::VERSION;"];
    e2e_app_bedrock: "bedrock", [], ["vendor", "web"],
        ["php", "-r", "require 'vendor/autoload.php'; require 'web/wp/wp-includes/version.php'; echo $wp_version;"];
}
