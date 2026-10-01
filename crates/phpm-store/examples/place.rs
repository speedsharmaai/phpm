//! Place a fixture's packages from the store and time it.
//!
//! `cargo run --release -p phpm-store --example place -- fixtures/laravel-skeleton`
//! The first run fills the store; later runs show the warm placement time.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use phpm_store::{Auth, Dist, FetchOptions, Fetcher, LinkMode, Package, Store, place, prune};
use serde_json::Value;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut fixture: Option<PathBuf> = None;
    let mut vendor: Option<PathBuf> = None;
    let mut mode = LinkMode::platform_default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--vendor" => vendor = args.next().map(PathBuf::from),
            "--mode" => {
                mode = match args.next().as_deref() {
                    Some("clone") => LinkMode::Clone,
                    Some("hardlink") => LinkMode::Hardlink,
                    Some("copy") => LinkMode::Copy,
                    other => return Err(format!("unknown mode {other:?}").into()),
                }
            }
            _ => fixture = Some(PathBuf::from(arg)),
        }
    }
    let fixture =
        fixture.ok_or("usage: place <fixture-dir> [--vendor DIR] [--mode clone|hardlink|copy]")?;
    let vendor = vendor.unwrap_or_else(|| fixture.join("vendor"));
    let packages = locked_packages(&fixture.join("fixture.lock"))?;
    let store = Store::from_env()?;
    let mut out = std::io::stdout().lock();

    let started = Instant::now();
    let fetched = tokio::runtime::Runtime::new()?.block_on(async {
        let fetcher = Fetcher::new(FetchOptions {
            auth: Auth::load(Some(&fixture))?,
            ..FetchOptions::default()
        })?;
        store.fetch_missing(&fetcher, &packages).await
    })?;
    if !fetched.is_empty() {
        writeln!(
            out,
            "fetched {} packages into {} in {:.2?}",
            fetched.len(),
            store.root().display(),
            started.elapsed()
        )?;
    }

    if vendor.exists() {
        std::fs::remove_dir_all(&vendor)?;
    }
    let placements = packages
        .iter()
        .map(|p| store.placement(p))
        .collect::<Result<Vec<_>, _>>()?;
    let keep: Vec<PathBuf> = packages.iter().map(|p| p.install_path.clone()).collect();
    let started = Instant::now();
    let used = place(&vendor, &placements, mode)?;
    prune(&vendor, &keep)?;
    writeln!(
        out,
        "placed {} packages into {} in {:.1?} ({used:?})",
        packages.len(),
        vendor.display(),
        started.elapsed()
    )?;
    Ok(())
}

fn locked_packages(lock: &Path) -> Result<Vec<Package>, Box<dyn std::error::Error>> {
    let lock: Value = serde_json::from_slice(&std::fs::read(lock)?)?;
    let mut out = Vec::new();
    for entry in ["packages", "packages-dev"]
        .iter()
        .filter_map(|k| lock[k].as_array())
        .flatten()
    {
        let text = |v: &Value| v.as_str().map(str::to_owned);
        let name = text(&entry["name"]).ok_or("package without a name")?;
        let dist = &entry["dist"];
        if dist.is_null() && entry["type"] == "metapackage" {
            continue;
        }
        out.push(Package::new(
            name,
            Dist {
                kind: text(&dist["type"]).ok_or("package without dist.type")?,
                url: text(&dist["url"]).ok_or("package without dist.url")?,
                reference: text(&dist["reference"]),
                shasum: text(&dist["shasum"]),
            },
        ));
    }
    Ok(out)
}
