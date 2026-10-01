//! installed.json and installed.php against files Composer 2.10.3 wrote for
//! the small fixtures. Regenerate with `just golden-bless`.

use phpm_lock::{ComposerJson, InstallContext, Lock, NO_VERSION_SET, root_version};
use phpm_testkit::assert_bytes_eq;
use std::path::{Path, PathBuf};

fn repo_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn check(fixture: &str, dev_mode: bool) {
    let dir = repo_path(&format!("fixtures/{fixture}"));
    let composer = ComposerJson::parse(
        &std::fs::read_to_string(dir.join("fixture.json")).expect("golden test setup"),
    )
    .expect("golden test setup");
    let lock =
        Lock::parse(&std::fs::read_to_string(dir.join("fixture.lock")).expect("golden test setup"))
            .expect("golden test setup");
    let outside_any_repo = tempfile::tempdir().expect("golden test setup");
    let version =
        root_version(composer.data(), outside_any_repo.path(), None).expect("golden test setup");
    assert_eq!(version.pretty, NO_VERSION_SET);
    let ctx = InstallContext {
        composer_json: &composer,
        lock: &lock,
        root_version: &version,
        root_dir: "/project",
        dev_mode,
        install_paths: &phpm_lock::InstallPaths::new(),
        installed_json_indent: None,
        unchanged_installed: &std::collections::BTreeMap::default(),
    };
    let files = phpm_lock::installed_files(&ctx).expect("golden test setup");

    let golden = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(fixture)
        .join(if dev_mode { "dev" } else { "no-dev" });
    assert_bytes_eq(
        files.installed_json.as_bytes(),
        &std::fs::read(golden.join("installed.json")).expect("golden test setup"),
    );
    assert_bytes_eq(
        files.installed_php.as_bytes(),
        &std::fs::read(golden.join("installed.php")).expect("golden test setup"),
    );
}

#[test]
fn wicketyaari_dev() {
    check("wicketyaari", true);
}

#[test]
fn wicketyaari_no_dev() {
    check("wicketyaari", false);
}

#[test]
fn ytmate_dev() {
    check("ytmate", true);
}

#[test]
fn ytmate_no_dev() {
    check("ytmate", false);
}
