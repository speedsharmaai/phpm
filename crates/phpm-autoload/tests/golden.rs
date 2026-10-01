//! Autoload files against what Composer 2.10.3 wrote for the small
//! fixtures (`just golden-bless`). There is no vendor/ here, so the test
//! rebuilds a stand-in: one PHP file per path in the golden class map,
//! declaring the classes Composer found there, plus every classmap
//! directory the lock names. Real lexing is covered by `just golden`.

use phpm_autoload::{Options, Project, generate};
use phpm_lock::{ComposerJson, Lock};
use phpm_testkit::assert_bytes_eq;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

const FILES: [&str; 9] = [
    "autoload.php",
    "composer/autoload_real.php",
    "composer/autoload_static.php",
    "composer/autoload_psr4.php",
    "composer/autoload_namespaces.php",
    "composer/autoload_classmap.php",
    "composer/autoload_files.php",
    "composer/include_paths.php",
    "composer/platform_check.php",
];

fn unexport(quoted: &str) -> String {
    let inner = &quoted[1..quoted.len() - 1];
    inner
        .replace("\\\\", "\u{0}")
        .replace("\\'", "'")
        .replace('\u{0}', "\\")
}

/// `class => path` from a golden `autoload_classmap.php`.
fn golden_class_map(text: &str, root: &Path) -> BTreeMap<String, Vec<String>> {
    let mut by_path: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in text.lines().filter(|l| l.starts_with("    '")) {
        let (class, path) = line
            .trim()
            .trim_end_matches(',')
            .split_once(" => ")
            .expect("class map line");
        let (base, rel) = path.split_once(" . ").expect("path code");
        let dir = match base {
            "$vendorDir" => root.join("vendor"),
            "$baseDir" => root.to_path_buf(),
            other => panic!("unexpected base {other}"),
        };
        let class = unexport(class);
        if class == "Composer\\InstalledVersions" {
            continue;
        }
        let file = format!("{}{}", dir.to_string_lossy(), unexport(rel));
        by_path.entry(file).or_default().push(class);
    }
    by_path
}

fn stand_in_vendor(root: &Path, lock: &Lock, golden: &Path) {
    let classmap = std::fs::read_to_string(golden.join("composer/autoload_classmap.php"))
        .expect("golden class map");
    for (file, classes) in golden_class_map(&classmap, root) {
        let mut body = String::from("<?php\n");
        for class in classes {
            let (namespace, name) = class.rsplit_once('\\').unwrap_or(("", &class));
            let _ = writeln!(body, "namespace {namespace} {{ class {name} {{}} }}");
        }
        let path = Path::new(&file);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("stand-in vendor");
        std::fs::write(path, body).expect("stand-in vendor");
    }
    let packages = lock
        .packages()
        .expect("lock")
        .into_iter()
        .chain(lock.packages_dev().expect("lock"));
    for package in packages {
        let name = package["name"].as_str().expect("name");
        let Some(Value::Array(entries)) = package.get("autoload").and_then(|a| a.get("classmap"))
        else {
            continue;
        };
        for entry in entries.iter().filter_map(Value::as_str) {
            let path = root.join("vendor").join(name).join(entry);
            if path.exists() {
                continue;
            }
            if Path::new(entry).extension().is_some() {
                std::fs::create_dir_all(path.parent().expect("parent")).expect("stand-in vendor");
                std::fs::write(&path, "").expect("stand-in vendor");
            } else {
                std::fs::create_dir_all(&path).expect("stand-in vendor");
            }
        }
    }
}

fn check(fixture: &str, dev_mode: bool) {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = manifest.join("../../fixtures").join(fixture);
    let golden = manifest
        .join("tests/golden")
        .join(fixture)
        .join(if dev_mode { "dev" } else { "no-dev" });
    let composer = ComposerJson::parse(
        &std::fs::read_to_string(source.join("fixture.json")).expect("fixture"),
    )
    .expect("fixture");
    let lock = Lock::parse(&std::fs::read_to_string(source.join("fixture.lock")).expect("fixture"))
        .expect("fixture");

    let work = tempfile::tempdir().expect("temp dir");
    let real = work.path().canonicalize().expect("real path");
    let root_dir = real.to_string_lossy().replace('\\', "/");
    let root_dir = root_dir
        .strip_prefix("//?/")
        .unwrap_or(&root_dir)
        .to_owned();
    stand_in_vendor(Path::new(&root_dir), &lock, &golden);
    let project = Project {
        composer_json: &composer,
        lock: &lock,
        root_dir: &root_dir,
        install_paths: &phpm_lock::InstallPaths::new(),
    };
    let options = Options {
        dev_mode,
        ..Options::default()
    }
    .with_config(&composer);
    let out = generate(&project, &options).expect("generate");
    for name in FILES {
        match std::fs::read(golden.join(name)) {
            Ok(expected) => assert_bytes_eq(out.file(name).expect(name), &expected),
            Err(_) => assert!(out.file(name).is_none(), "{name} should not be written"),
        }
    }
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
