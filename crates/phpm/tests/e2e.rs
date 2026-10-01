//! `composer install` and `phpm install` on every fixture, in sibling temp
//! directories outside this repository, then the whole `vendor/` compared
//! byte for byte and mode for mode. Needs `composer`, `php` and the network,
//! so it is ignored; run it with `just e2e`.

#![allow(clippy::unwrap_used, reason = "test helpers panic on setup failures")]

use std::path::Path;
use std::process::Command;

use phpm_diffvendor::{Ignore, compare};

fn prepare(fixture: &str, dir: &Path) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(fixture);
    for file in ["composer.json", "composer.lock"] {
        std::fs::copy(source.join(file), dir.join(file)).unwrap();
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
    run(
        Command::new(env!("CARGO_BIN_EXE_phpm"))
            .args(["install", "--no-scripts", "--no-plugins", "-q"])
            .args(flags)
            .current_dir(&phpm_dir)
            .env_remove("COMPOSER_ROOT_VERSION"),
        "phpm install",
    );

    assert!(composer_dir.join("vendor/autoload.php").is_file());
    let diffs = compare(
        &composer_dir.join("vendor"),
        &phpm_dir.join("vendor"),
        &Ignore::default(),
    )
    .unwrap();
    let shown: Vec<String> = diffs.iter().take(20).map(ToString::to_string).collect();
    assert!(
        diffs.is_empty(),
        "{fixture} {flags:?}: {} differences\n{}",
        diffs.len(),
        shown.join("\n")
    );
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
    e2e_symfony_demo: "symfony-demo";
    e2e_monica: "monica", "--ignore-platform-reqs";
}
