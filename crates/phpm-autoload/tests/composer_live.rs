//! Runs the real Composer on every fixture and compares every autoload file
//! byte for byte, dev and `--no-dev`, with each fixture's own
//! `optimize-autoloader` config. Needs `composer`, `php` and the network,
//! so it is ignored; run it with `just golden`. `PHPM_BLESS=1`
//! (`just golden-bless`) refreshes the committed golden files of the small
//! fixtures.

use phpm_autoload::{Options, PlatformRequirements, Project, generate};
use phpm_lock::{COMPOSER_VERSION, ComposerJson, Lock};
use phpm_testkit::first_difference;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

const COMMITTED: [&str; 2] = ["wicketyaari", "ytmate"];

/// Fixtures that need `--ignore-platform-reqs` on this machine (monica
/// wants ext-intl), so Composer writes no `platform_check.php` for them.
const IGNORE_PLATFORM: [&str; 1] = ["monica"];

const FILES: [&str; 11] = [
    "autoload.php",
    "composer/autoload_real.php",
    "composer/autoload_static.php",
    "composer/autoload_psr4.php",
    "composer/autoload_namespaces.php",
    "composer/autoload_classmap.php",
    "composer/autoload_files.php",
    "composer/include_paths.php",
    "composer/platform_check.php",
    "composer/ClassLoader.php",
    "composer/LICENSE",
];

#[expect(clippy::print_stderr, reason = "just golden prints timings")]
fn live(fixture: &str, dev_mode: bool) {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = manifest.join("../../fixtures").join(fixture);
    let work = tempfile::tempdir().expect("live test setup");
    let root = work.path().canonicalize().expect("live test setup");
    for file in ["composer.json", "composer.lock"] {
        std::fs::copy(source.join(file), root.join(file)).expect("live test setup");
    }

    let version = Command::new("composer")
        .arg("--version")
        .output()
        .expect("composer on PATH");
    assert!(
        String::from_utf8_lossy(&version.stdout).contains(COMPOSER_VERSION),
        "golden files need Composer {COMPOSER_VERSION}"
    );

    let ignore_platform = IGNORE_PLATFORM.contains(&fixture);
    let mut install = Command::new("composer");
    install
        .args([
            "install",
            "--no-scripts",
            "--no-plugins",
            "--no-interaction",
            "-q",
        ])
        .current_dir(&root)
        .env_remove("COMPOSER_ROOT_VERSION");
    if ignore_platform {
        install.arg("--ignore-platform-reqs");
    }
    if !dev_mode {
        install.arg("--no-dev");
    }
    let started = Instant::now();
    let status = install.status().expect("live test setup");
    let composer_time = started.elapsed();
    assert!(status.success(), "composer install failed for {fixture}");

    let composer = ComposerJson::parse(
        &std::fs::read_to_string(root.join("composer.json")).expect("live test setup"),
    )
    .expect("live test setup");
    let lock =
        Lock::parse(&std::fs::read_to_string(root.join("composer.lock")).expect("live test setup"))
            .expect("live test setup");
    let root_dir = root.to_string_lossy().replace('\\', "/");
    let project = Project {
        composer_json: &composer,
        lock: &lock,
        root_dir: &root_dir,
    };
    let options = Options {
        dev_mode,
        platform: if ignore_platform {
            PlatformRequirements::IgnoreAll
        } else {
            PlatformRequirements::Check
        },
        ..Options::default()
    }
    .with_config(&composer);
    let started = Instant::now();
    let ours = generate(&project, &options).expect("phpm generates the autoloader");
    let our_time = started.elapsed();
    eprintln!(
        "{fixture} dev={dev_mode} optimize={}: phpm {our_time:?}, composer install {composer_time:?}, {} warnings",
        options.optimize,
        ours.warnings.len()
    );

    let vendor = root.join("vendor");
    let mut differ = Vec::new();
    for name in FILES {
        let theirs = std::fs::read(vendor.join(name)).ok();
        let mine = ours.file(name);
        match (mine, theirs.as_deref()) {
            (None, None) => {}
            (Some(a), Some(b)) if a == b => {}
            (Some(a), Some(b)) => differ.push(format!(
                "{name}: first difference at byte {} ({} vs {} bytes)",
                first_difference(a, b),
                a.len(),
                b.len()
            )),
            (a, b) => differ.push(format!(
                "{name}: phpm {} it, Composer {} it",
                if a.is_some() { "writes" } else { "skips" },
                if b.is_some() { "writes" } else { "skips" }
            )),
        }
    }

    if std::env::var_os("PHPM_BLESS").is_some() && COMMITTED.contains(&fixture) {
        let golden = manifest
            .join("tests/golden")
            .join(fixture)
            .join(if dev_mode { "dev" } else { "no-dev" });
        let _ = std::fs::remove_dir_all(&golden);
        std::fs::create_dir_all(golden.join("composer")).expect("live test setup");
        for name in FILES {
            if name.ends_with("ClassLoader.php") || name.ends_with("LICENSE") {
                continue;
            }
            if let Ok(bytes) = std::fs::read(vendor.join(name)) {
                std::fs::write(golden.join(name), bytes).expect("live test setup");
            }
        }
    }
    drop(work);
    assert!(
        differ.is_empty(),
        "{fixture} dev={dev_mode}:\n{}",
        differ.join("\n")
    );
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
    composer_live_autoload_wicketyaari_dev: "wicketyaari", true;
    composer_live_autoload_wicketyaari_no_dev: "wicketyaari", false;
    composer_live_autoload_ytmate_dev: "ytmate", true;
    composer_live_autoload_ytmate_no_dev: "ytmate", false;
    composer_live_autoload_laravel_skeleton_dev: "laravel-skeleton", true;
    composer_live_autoload_laravel_skeleton_no_dev: "laravel-skeleton", false;
    composer_live_autoload_symfony_demo_dev: "symfony-demo", true;
    composer_live_autoload_symfony_demo_no_dev: "symfony-demo", false;
    composer_live_autoload_monica_dev: "monica", true;
    composer_live_autoload_monica_no_dev: "monica", false;
}
