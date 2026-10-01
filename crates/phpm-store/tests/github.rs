//! Real GitHub downloads compared against a real `composer install`.
//!
//! `cargo nextest run -p phpm-store --run-ignored only`. Needs network and
//! `composer` on PATH; a `github-oauth` token in auth.json avoids rate limits.

#![allow(clippy::unwrap_used, reason = "test helpers panic on setup failures")]

use std::path::{Path, PathBuf};
use std::process::Command;

use phpm_diffvendor::{Ignore, compare};
use phpm_store::{Auth, Dist, FetchOptions, Fetcher, LinkMode, Package, Store, place, prune};
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

fn locked_packages(lock: &Path) -> Vec<Package> {
    let lock: Value = serde_json::from_slice(&std::fs::read(lock).unwrap()).unwrap();
    ["packages", "packages-dev"]
        .iter()
        .filter_map(|k| lock[k].as_array())
        .flatten()
        .filter(|p| p["type"] != "metapackage")
        .map(|p| {
            let dist = &p["dist"];
            let text = |v: &Value| v.as_str().map(str::to_owned);
            Package::new(
                p["name"].as_str().unwrap(),
                Dist {
                    kind: text(&dist["type"]).unwrap(),
                    url: text(&dist["url"]).unwrap(),
                    reference: text(&dist["reference"]),
                    shasum: text(&dist["shasum"]),
                },
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "network: downloads from GitHub and runs composer"]
async fn vendor_matches_composer_on_wicketyaari() {
    let src = fixture("wicketyaari");
    let tmp = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("github-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let composer_dir = tmp.join("composer");
    std::fs::create_dir_all(&composer_dir).unwrap();
    for f in ["composer.json", "composer.lock"] {
        std::fs::copy(src.join(f), composer_dir.join(f)).unwrap();
    }
    let status = Command::new("composer")
        .args([
            "install",
            "--no-scripts",
            "--no-plugins",
            "--no-interaction",
            "--no-progress",
            "--ignore-platform-reqs",
            "--quiet",
        ])
        .current_dir(&composer_dir)
        .status()
        .expect("composer must be on PATH");
    assert!(status.success());

    let packages = locked_packages(&src.join("composer.lock"));
    let store = Store::new(&tmp.join("cache"));
    let fetcher = Fetcher::new(FetchOptions {
        auth: Auth::load(None).unwrap(),
        ..FetchOptions::default()
    })
    .unwrap();
    let fetched = store.fetch_missing(&fetcher, &packages).await.unwrap();
    assert_eq!(fetched.len(), packages.len());

    let vendor = tmp.join("phpm/vendor");
    let placements: Vec<_> = packages
        .iter()
        .map(|p| store.placement(p).unwrap())
        .collect();
    place(&vendor, &placements, LinkMode::platform_default()).unwrap();
    let keep: Vec<PathBuf> = packages.iter().map(|p| p.install_path.clone()).collect();
    assert!(prune(&vendor, &keep).unwrap().is_empty());

    let mut failures = Vec::new();
    for p in &packages {
        let diffs = compare(
            &composer_dir.join("vendor").join(&p.install_path),
            &vendor.join(&p.install_path),
            &Ignore::default(),
        )
        .unwrap();
        failures.extend(diffs.iter().map(|d| format!("{}: {d}", p.name)));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let _ = std::fs::remove_dir_all(&tmp);
}
