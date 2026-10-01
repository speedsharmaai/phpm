//! Runs the real `phpm` binary against projects whose dists come from a local
//! HTTP server, so nothing here touches the network.

#![allow(clippy::unwrap_used, reason = "test helpers panic on setup failures")]

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use zip::write::SimpleFileOptions;

struct Server {
    base: String,
    hits: Arc<Mutex<BTreeMap<String, usize>>>,
}

impl Server {
    fn start(files: BTreeMap<String, Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let hits = Arc::new(Mutex::new(BTreeMap::new()));
        let log = Arc::clone(&hits);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut sock) = stream else { continue };
                let mut buf = Vec::new();
                let mut chunk = [0_u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&buf).into_owned();
                let path = head.split(' ').nth(1).unwrap_or("/").to_owned();
                *log.lock().unwrap().entry(path.clone()).or_insert(0) += 1;
                let (status, body) = match files.get(&path) {
                    Some(body) => ("200 OK", body.clone()),
                    None => ("404 Not Found", Vec::new()),
                };
                let mut out = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .into_bytes();
                out.extend_from_slice(&body);
                let _ = sock.write_all(&out);
            }
        });
        Self { base, hits }
    }

    fn total_hits(&self) -> usize {
        self.hits.lock().unwrap().values().sum()
    }
}

fn zip(top: &str, files: &[(&str, u32, &str)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    w.add_directory(
        format!("{top}/"),
        SimpleFileOptions::default().unix_permissions(0o755),
    )
    .unwrap();
    for (name, mode, body) in files {
        w.start_file(
            format!("{top}/{name}"),
            SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(*mode),
        )
        .unwrap();
        w.write_all(body.as_bytes()).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn package(base: &str, name: &str, extra: &Value) -> Value {
    let mut p = json!({
        "name": name,
        "version": "1.0.0",
        "dist": {
            "type": "zip",
            "url": format!("{base}/{name}.zip"),
            "reference": format!("ref-{}", name.replace('/', "-")),
            "shasum": ""
        },
        "type": "library",
    });
    if let (Value::Object(p), Value::Object(extra)) = (&mut p, extra) {
        for (k, v) in extra {
            p.insert(k.clone(), v.clone());
        }
    }
    p
}

struct Project {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    cache: PathBuf,
    home: PathBuf,
    server: Server,
}

impl Project {
    /// a/lib (psr-4), b/tool (a PHP bin shipped 0644), c/dev (dev, a shell
    /// bin) and m/meta (a metapackage).
    fn new() -> Self {
        Self::with(|_, _| {})
    }

    fn with(edit: impl FnOnce(&mut Value, &mut Value)) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let base_dir = tmp.path().canonicalize().unwrap();
        let root = base_dir.join("app");
        std::fs::create_dir_all(&root).unwrap();
        let mut files = BTreeMap::new();
        files.insert(
            "/a/lib.zip".to_owned(),
            zip(
                "a-lib-1",
                &[
                    ("src/Lib.php", 0o644, "<?php\nnamespace A;\nclass Lib {}\n"),
                    ("composer.json", 0o644, "{}"),
                ],
            ),
        );
        files.insert(
            "/b/tool.zip".to_owned(),
            zip(
                "b-tool-1",
                &[(
                    "bin/tool",
                    0o644,
                    "#!/usr/bin/env php\n<?php\necho 'tool';\n",
                )],
            ),
        );
        files.insert(
            "/c/dev.zip".to_owned(),
            zip("c-dev-1", &[("run", 0o755, "#!/bin/sh\necho dev\n")]),
        );
        let server = Server::start(files);
        let base = server.base.clone();
        let mut composer = json!({
            "name": "acme/app",
            "require": {"a/lib": "^1.0", "b/tool": "^1.0", "m/meta": "^1.0"},
            "require-dev": {"c/dev": "^1.0"},
            "autoload": {"psr-4": {"App\\": "src/"}},
        });
        let mut lock = json!({
            "content-hash": "0123456789abcdef0123456789abcdef",
            "packages": [
                package(&base, "a/lib", &json!({"autoload": {"psr-4": {"A\\": "src/"}}})),
                package(&base, "b/tool", &json!({"bin": ["bin/tool"]})),
                {"name": "m/meta", "version": "1.0.0", "type": "metapackage", "require": {"a/lib": "^1.0"}},
            ],
            "packages-dev": [package(&base, "c/dev", &json!({"bin": "run"}))],
            "aliases": [],
            "minimum-stability": "stable",
            "stability-flags": {},
            "prefer-stable": false,
            "prefer-lowest": false,
            "platform": {},
            "platform-dev": {},
            "plugin-api-version": "2.9.0"
        });
        edit(&mut composer, &mut lock);
        std::fs::write(root.join("composer.json"), composer.to_string()).unwrap();
        std::fs::write(root.join("composer.lock"), lock.to_string()).unwrap();
        Self {
            cache: base_dir.join("cache"),
            home: base_dir.join("home"),
            root,
            server,
            _tmp: tmp,
        }
    }

    fn phpm(&self, args: &[&str]) -> Output {
        self.phpm_env(args, &[])
    }

    fn phpm_env(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_phpm"));
        cmd.args(args)
            .current_dir(&self.root)
            .env("PHPM_CACHE_DIR", &self.cache)
            .env("COMPOSER_HOME", &self.home)
            .env("HOME", &self.home)
            .env_remove("COMPOSER")
            .env_remove("COMPOSER_AUTH")
            .env_remove("COMPOSER_VENDOR_DIR")
            .env_remove("COMPOSER_BIN_DIR")
            .env_remove("COMPOSER_ROOT_VERSION");
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }

    fn vendor(&self, rel: &str) -> PathBuf {
        self.root.join("vendor").join(rel)
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn ok(out: &Output) -> String {
    assert!(out.status.success(), "{}", stderr(out));
    stderr(out)
}

#[cfg(unix)]
fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn installed(p: &Project) -> Value {
    serde_json::from_slice(&std::fs::read(p.vendor("composer/installed.json")).unwrap()).unwrap()
}

#[test]
fn prints_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_phpm"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout, format!("phpm {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn usage_errors_exit_2() {
    let out = Command::new(env!("CARGO_BIN_EXE_phpm")).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("Usage"), "{}", stderr(&out));
    let out = Command::new(env!("CARGO_BIN_EXE_phpm"))
        .args(["install", "--link-mode", "symlink"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
#[cfg_attr(windows, ignore = "Windows installs are Phase 03")]
fn installs_then_does_nothing_until_something_changes() {
    let p = Project::new();
    let first = ok(&p.phpm(&["install"]));
    assert!(first.contains("Installed 3 packages"), "{first}");
    assert_eq!(p.server.total_hits(), 3);
    assert!(p.vendor("a/lib/src/Lib.php").is_file());
    assert!(p.vendor("c/dev/run").is_file());
    assert!(!p.vendor("m").exists());
    assert!(p.vendor("autoload.php").is_file());
    assert!(p.vendor("composer/autoload_static.php").is_file());
    assert!(p.vendor("composer/InstalledVersions.php").is_file());
    let json = installed(&p);
    assert_eq!(json["dev"], true);
    assert_eq!(json["dev-package-names"], json!(["c/dev"]));
    let proxy = std::fs::read_to_string(p.vendor("bin/tool")).unwrap();
    assert!(proxy.contains("return include __DIR__ . '/..'.'/b/tool/bin/tool';"));
    let sh = std::fs::read_to_string(p.vendor("bin/run")).unwrap();
    assert!(sh.starts_with("#!/usr/bin/env sh\n"));
    #[cfg(unix)]
    {
        assert_eq!(
            mode(&p.vendor("b/tool/bin/tool")),
            mode(&p.vendor("bin/tool"))
        );
        assert_eq!(mode(&p.vendor("bin/tool")) & 0o100, 0o100);
    }

    let again = ok(&p.phpm(&["install"]));
    assert_eq!(again, "Nothing to install, update or remove\n");

    std::fs::remove_file(p.vendor("bin/tool")).unwrap();
    let repaired = ok(&p.phpm(&["install", "-v"]));
    assert!(
        repaired.contains("Nothing to install, update or remove"),
        "{repaired}"
    );
    assert!(p.vendor("bin/tool").is_file());

    std::fs::remove_dir_all(p.root.join("vendor")).unwrap();
    let warm = ok(&p.phpm(&[
        "install",
        "--no-interaction",
        "--no-progress",
        "--prefer-dist",
    ]));
    assert!(warm.contains("Installed 3 packages"), "{warm}");
    assert!(!warm.contains("Downloaded"), "{warm}");
    assert_eq!(p.server.total_hits(), 3, "a warm install must not download");

    let quiet = p.phpm(&["install", "-q"]);
    assert!(quiet.status.success());
    assert!(quiet.stderr.is_empty());
}

#[test]
#[cfg_attr(windows, ignore = "Windows installs are Phase 03")]
fn no_dev_removes_dev_packages_and_their_bins() {
    let p = Project::new();
    ok(&p.phpm(&["install"]));
    let out = ok(&p.phpm(&["install", "--no-dev"]));
    assert!(out.contains("Removed 1 package"), "{out}");
    assert!(!p.vendor("c").exists());
    assert!(!p.vendor("bin/run").exists());
    assert!(p.vendor("bin/tool").exists());
    let json = installed(&p);
    assert_eq!(json["dev"], false);
    assert_eq!(json["dev-package-names"], json!([]));
    ok(&p.phpm(&["install"]));
    assert!(p.vendor("c/dev/run").is_file());
}

#[test]
#[cfg_attr(windows, ignore = "Windows installs are Phase 03")]
fn refuses_plugins_and_scripts_without_the_flags() {
    let p = Project::with(|composer, lock| {
        composer["scripts"] = json!({"post-install-cmd": ["@php -v"]});
        lock["packages"][0]["type"] = json!("composer-plugin");
    });
    let out = p.phpm(&["install"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("a/lib"), "{}", stderr(&out));
    assert!(stderr(&out).contains("--no-plugins"));
    let out = p.phpm(&["install", "--no-plugins"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("post-install-cmd"),
        "{}",
        stderr(&out)
    );
    assert!(!p.root.join("vendor").exists());
    ok(&p.phpm(&["install", "--no-plugins", "--no-scripts"]));
}

#[test]
#[cfg_attr(windows, ignore = "Windows installs are Phase 03")]
fn reports_missing_inputs_and_failed_downloads() {
    let p = Project::new();
    std::fs::remove_file(p.root.join("composer.lock")).unwrap();
    let out = p.phpm(&["install"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("composer.lock not found"),
        "{}",
        stderr(&out)
    );
    std::fs::remove_file(p.root.join("composer.json")).unwrap();
    let out = p.phpm(&["install"]);
    assert_eq!(out.status.code(), Some(2));

    let p = Project::with(|_, lock| {
        let url = lock["packages"][0]["dist"]["url"]
            .as_str()
            .unwrap()
            .replace("lib", "gone");
        lock["packages"][0]["dist"]["url"] = json!(url);
    });
    let out = p.phpm(&["install"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("404"), "{}", stderr(&out));
}

#[test]
#[cfg_attr(windows, ignore = "Windows installs are Phase 03")]
fn honours_custom_file_names_and_dirs() {
    let p = Project::with(|composer, _| {
        composer["config"] = json!({"vendor-dir": "deps", "bin-dir": "tools"});
    });
    std::fs::rename(p.root.join("composer.json"), p.root.join("app.json")).unwrap();
    std::fs::rename(p.root.join("composer.lock"), p.root.join("app.lock")).unwrap();
    let out = p.phpm_env(&["install", "-d", "."], &[("COMPOSER", "app.json")]);
    ok(&out);
    assert!(p.root.join("deps/a/lib/src/Lib.php").is_file());
    assert!(p.root.join("tools/tool").is_file());
    let proxy = std::fs::read_to_string(p.root.join("tools/tool")).unwrap();
    assert!(
        proxy.contains("'/../deps/autoload.php'") || proxy.contains("'/..'.'/deps/autoload.php'"),
        "{proxy}"
    );

    let out = p.phpm_env(
        &["install"],
        &[("COMPOSER", "app.json"), ("COMPOSER_VENDOR_DIR", "other")],
    );
    ok(&out);
    assert!(p.root.join("other/a/lib/src/Lib.php").is_file());
}

#[cfg(unix)]
#[test]
fn hardlink_mode_leaves_the_store_untouched() {
    let p = Project::new();
    ok(&p.phpm(&["install", "--link-mode", "hardlink"]));
    let stored = walk(&p.cache)
        .into_iter()
        .find(|f| f.ends_with("bin/tool"))
        .unwrap();
    assert_eq!(mode(&stored), 0o644);
    assert_eq!(mode(&p.vendor("b/tool/bin/tool")) & 0o100, 0o100);
    ok(&p.phpm(&["install", "--link-mode", "copy"]));
    assert!(p.vendor("a/lib/src/Lib.php").is_file());
}

#[cfg(unix)]
fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}
