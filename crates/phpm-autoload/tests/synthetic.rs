//! A hand-made project with every autoload feature that needs no class
//! scanning, against what Composer 2.10.3's `dump-autoload` wrote for it.

use phpm_autoload::{Options, Project, generate};
use phpm_lock::{ComposerJson, Lock};
use phpm_testkit::assert_bytes_eq;
use std::path::Path;

fn data(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/synthetic")
        .join(name);
    std::fs::read_to_string(path).expect("test data")
}

#[test]
fn matches_composer_dump_autoload() {
    let composer = ComposerJson::parse(&data("composer.json")).unwrap();
    let lock = Lock::parse(&data("composer.lock")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    let root = root.strip_prefix("//?/").unwrap_or(&root).to_owned();
    let project = Project {
        composer_json: &composer,
        lock: &lock,
        root_dir: &root,
    };
    let options = Options {
        dev_mode: true,
        ..Options::default()
    }
    .with_config(&composer);
    let out = generate(&project, &options).unwrap();
    assert_eq!(out.suffix, "0123456789abcdef0123456789abcdef");
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/synthetic");
    let mut names: Vec<&str> = out.files.iter().map(|(n, _)| n.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "autoload.php",
            "composer/ClassLoader.php",
            "composer/LICENSE",
            "composer/autoload_classmap.php",
            "composer/autoload_files.php",
            "composer/autoload_namespaces.php",
            "composer/autoload_psr4.php",
            "composer/autoload_real.php",
            "composer/autoload_static.php",
            "composer/include_paths.php",
            "composer/platform_check.php",
        ]
    );
    for (name, content) in &out.files {
        if name.ends_with("ClassLoader.php") || name.ends_with("LICENSE") {
            continue;
        }
        let expected = std::fs::read(golden.join(name)).expect("golden file");
        assert_bytes_eq(content.as_bytes(), &expected);
    }
    assert!(out.remove.is_empty());

    out.write().unwrap();
    let vendor = dir.path().join("vendor");
    assert!(vendor.join("composer/autoload_static.php").exists());
    std::fs::write(vendor.join("composer/autoload_files.php"), "stale").unwrap();
    let again = generate(&project, &options).unwrap();
    assert_eq!(again.suffix, out.suffix);
    again.write().unwrap();
    assert_eq!(
        std::fs::read_to_string(vendor.join("composer/autoload_files.php")).unwrap(),
        out.file("composer/autoload_files.php").unwrap()
    );
}
