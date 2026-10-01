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
    /// `(path, body)` of every POST.
    posts: Arc<Mutex<Vec<(String, String)>>>,
}

impl Server {
    fn start(files: BTreeMap<String, Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let hits = Arc::new(Mutex::new(BTreeMap::new()));
        let log = Arc::clone(&hits);
        let posts = Arc::new(Mutex::new(Vec::new()));
        let post_log = Arc::clone(&posts);
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
                if head.starts_with("POST ") {
                    let len: usize = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    let start = buf
                        .windows(4)
                        .position(|w| w == b"\r\n\r\n")
                        .map_or(buf.len(), |i| i + 4);
                    let mut body = buf[start..].to_vec();
                    while body.len() < len {
                        match sock.read(&mut chunk) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => body.extend_from_slice(&chunk[..n]),
                        }
                    }
                    post_log
                        .lock()
                        .unwrap()
                        .push((path.clone(), String::from_utf8_lossy(&body).into_owned()));
                }
                let (status, body) = match files.get(&path) {
                    Some(body) => ("200 OK", body.clone()),
                    None => ("404 Not Found", Vec::new()),
                };
                let fresh = if path.contains("fresh") {
                    "Cache-Control: public, max-age=900\r\n"
                } else {
                    ""
                };
                let mut out = format!(
                    "HTTP/1.1 {status}\r\n{fresh}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .into_bytes();
                out.extend_from_slice(&body);
                let _ = sock.write_all(&out);
            }
        });
        Self { base, hits, posts }
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
        Self::serving(BTreeMap::new(), edit)
    }

    /// Like `with`, with `extra` files on the test server too.
    fn serving(
        extra: BTreeMap<String, Vec<u8>>,
        edit: impl FnOnce(&mut Value, &mut Value),
    ) -> Self {
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
        files.extend(extra);
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
fn optimized_autoloaders_match_with_and_without_the_class_cache() {
    let p = Project::new();
    ok(&p.phpm(&["install", "-o"]));
    let classmap = p.vendor("composer/autoload_classmap.php");
    let first = std::fs::read(&classmap).unwrap();
    assert!(
        String::from_utf8_lossy(&first)
            .contains("'A\\\\Lib' => $vendorDir . '/a/lib/src/Lib.php',"),
        "{}",
        String::from_utf8_lossy(&first)
    );
    std::fs::remove_dir_all(p.root.join("vendor")).unwrap();
    let out = ok(&p.phpm(&["install", "-o", "-v"]));
    assert!(out.contains("loaded class scans for 3 packages"), "{out}");
    assert_eq!(std::fs::read(&classmap).unwrap(), first);
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

/// A stand-in `composer` that logs its arguments and the dev-mode variable.
#[cfg(unix)]
fn fake_composer(p: &Project) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let bin = p.home.join("fake-composer");
    let log = p.home.join("composer.log");
    std::fs::create_dir_all(&p.home).unwrap();
    std::fs::write(
        &bin,
        format!(
            "#!/bin/sh\necho \"$*\" >> {log}\n\
             [ \"$1\" = run-script ] && [ \"$2\" = post-install-cmd ] && [ -n \"$NOT_DEFINED\" ] && \
             {{ echo 'Script \"post-install-cmd\" is not defined in this package' >&2; exit 1; }}\n\
             [ \"$1\" = dump-autoload ] && [ -n \"$FAIL_DUMP\" ] && exit 5\n\
             exit 0\n",
            log = log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    (bin, log)
}

#[cfg(unix)]
fn plugin_project() -> Project {
    Project::with(|composer, lock| {
        composer["config"] = json!({"allow-plugins": {"a/*": true}});
        lock["packages"][0]["type"] = json!("composer-plugin");
    })
}

#[test]
#[cfg(unix)]
fn blocks_plugins_composer_would_block() {
    let p = Project::with(|_, lock| {
        lock["packages"][0]["type"] = json!("composer-plugin");
    });
    let out = p.phpm(&["install"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains(
            "a/lib contains a Composer plugin which is blocked by your allow-plugins config"
        ),
        "{}",
        stderr(&out)
    );
    assert!(!p.root.join("vendor").exists());
    ok(&p.phpm(&["install", "--no-plugins"]));
}

#[test]
#[cfg(unix)]
fn falls_back_to_composer_for_plugins() {
    let p = plugin_project();
    let out = p.phpm_env(&["install"], &[("PHPM_COMPOSER", "/nonexistent/composer")]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("composer is not on PATH"),
        "{}",
        stderr(&out)
    );
    assert!(stderr(&out).contains("Nothing was changed"));
    assert!(!p.root.join("vendor").exists());

    let (bin, log) = fake_composer(&p);
    let bin = bin.to_string_lossy().into_owned();
    let out = ok(&p.phpm_env(
        &["install", "--explain"],
        &[("PHPM_COMPOSER", &bin), ("NOT_DEFINED", "1")],
    ));
    assert!(
        out.contains("package a/lib 1.0.0: native, downloaded to the store, then placed; plugin, Composer loads it"),
        "{out}"
    );
    assert!(
        out.contains("package m/meta 1.0.0: native, metapackage, nothing to place"),
        "{out}"
    );
    assert!(out.contains("decision plugins: a/lib allowed"), "{out}");
    assert!(
        out.contains("decision autoload: fallback to Composer, plugins are loaded (a/lib)"),
        "{out}"
    );
    assert!(out.contains("Composer runs the autoload dump"), "{out}");
    assert!(!out.contains("is not defined"), "{out}");
    assert!(p.vendor("a/lib/src/Lib.php").is_file());
    assert!(p.vendor("composer/installed.json").is_file());
    assert!(
        !p.vendor("autoload.php").exists(),
        "composer writes the autoloader, not phpm"
    );
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        "dump-autoload --dev --no-interaction\nrun-script post-install-cmd --dev --no-interaction\n"
    );

    std::fs::remove_dir_all(p.root.join("vendor")).unwrap();
    let out = p.phpm_env(
        &["install", "--no-dev", "-q"],
        &[("PHPM_COMPOSER", &bin), ("FAIL_DUMP", "1")],
    );
    assert_eq!(out.status.code(), Some(5), "{}", stderr(&out));
    assert!(stderr(&out).contains("dump-autoload failed with exit code 5"));
    assert!(
        std::fs::read_to_string(&log)
            .unwrap()
            .ends_with("dump-autoload --no-dev --no-interaction --quiet\n")
    );
}

#[test]
#[cfg(unix)]
fn hands_the_whole_install_to_composer_when_a_plugin_moves_packages() {
    let p = Project::with(|composer, lock| {
        composer["config"] = json!({"allow-plugins": {"composer/installers": true}});
        lock["packages"][0]["name"] = json!("composer/installers");
        lock["packages"][0]["type"] = json!("composer-plugin");
    });
    let (bin, log) = fake_composer(&p);
    let bin = bin.to_string_lossy().into_owned();
    let out = ok(&p.phpm_env(
        &["install", "--explain", "-o", "--no-dev"],
        &[("PHPM_COMPOSER", &bin)],
    ));
    assert!(
        out.contains("package composer/installers 1.0.0: fallback, composer install places it; plugin, loaded by composer install"),
        "{out}"
    );
    assert!(
        out.contains("decision install: fallback to composer install, composer/installers: it changes where packages are installed"),
        "{out}"
    );
    assert!(!p.root.join("vendor").exists());
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        "install --no-dev --optimize-autoloader --no-interaction\n"
    );
    ok(&p.phpm_env(&["install", "-o", "--no-dev"], &[("PHPM_COMPOSER", &bin)]));
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().lines().count(),
        2,
        "no state file after a full fallback"
    );
}

#[test]
#[cfg(unix)]
fn runs_string_scripts_natively_in_composer_order() {
    let p = Project::with(|composer, _| {
        composer["scripts"] = json!({
            "pre-install-cmd": "test -d vendor && echo pre-install:vendor >> events.log || echo pre-install >> events.log",
            "pre-autoload-dump": ["@putenv STAGE=dump", "@log-it pre-autoload"],
            "post-autoload-dump": "test -f vendor/autoload.php && echo \"post-autoload:$STAGE:$COMPOSER_DEV_MODE\" >> events.log",
            "post-install-cmd": ["echo \"post-install:${PATH%%:*}\" >> events.log"],
            "log-it": "echo $1 >> events.log; true",
        });
    });
    let out = ok(&p.phpm(&["install", "--explain"]));
    assert!(
        out.contains("decision pre-install-cmd: native, 1 script"),
        "{out}"
    );
    assert!(
        out.contains("decision pre-autoload-dump: native, 2 scripts"),
        "{out}"
    );
    assert!(
        out.contains("decision post-autoload-dump: native, 1 script"),
        "{out}"
    );
    assert!(out.contains("> @putenv STAGE=dump"), "{out}");
    let bin = p.root.join("vendor/bin").canonicalize().unwrap();
    assert_eq!(
        std::fs::read_to_string(p.root.join("events.log")).unwrap(),
        format!(
            "pre-install\n\npost-autoload:dump:1\npost-install:{}\n",
            bin.display()
        )
    );
}

#[test]
#[cfg(unix)]
fn php_callables_send_their_event_to_composer() {
    let p = Project::with(|composer, _| {
        composer["scripts"] = json!({
            "post-autoload-dump": ["App\\Scripts::dump", "@php -v"],
            "post-install-cmd": "exit 9",
        });
    });
    let (bin, log) = fake_composer(&p);
    let bin = bin.to_string_lossy().into_owned();
    let out = p.phpm_env(&["install"], &[("PHPM_COMPOSER", &bin)]);
    assert_eq!(out.status.code(), Some(9), "{}", stderr(&out));
    assert!(
        stderr(&out).contains(
            "Script exit 9 handling the post-install-cmd event returned with error code 9"
        ),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        "run-script post-autoload-dump --dev --no-interaction\n"
    );
    assert!(p.vendor("a/lib/src/Lib.php").is_file());
    assert!(
        p.vendor("autoload.php").is_file(),
        "phpm still writes the autoloader"
    );
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

/// A `php` that answers the platform probe with PHP 8.4 and only mbstring.
#[cfg(unix)]
fn fake_php(dir: &std::path::Path) -> String {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).unwrap();
    let php = dir.join("php");
    let probe = json!({
        "version": "8.4.13", "debug": false, "zts": false, "int_size": 8, "ipv6": true,
        "extensions": [["mbstring", "8.4.13"]], "libraries": [], "ini": [""],
    });
    std::fs::write(
        &php,
        format!("#!/bin/sh\nwhile read -r _; do :; done\nprintf '%s' '{probe}'\n"),
    )
    .unwrap();
    std::fs::set_permissions(&php, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir.to_string_lossy().into_owned()
}

#[test]
#[cfg(unix)]
fn refuses_a_lock_the_platform_cannot_install() {
    let p = Project::with(|_, lock| {
        lock["packages"][0]["require"] =
            json!({"php": ">=8.1", "ext-mbstring": "*", "ext-nope": "^1"});
    });
    let path = fake_php(&p.home.join("bin"));
    let out = p.phpm_env(&["install"], &[("PATH", &path)]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains("Your lock file does not contain a compatible set of packages"),
        "{err}"
    );
    assert!(err.contains("a/lib 1.0.0 requires ext-nope ^1 -> it is missing from your system. Install or enable PHP's nope extension."), "{err}");
    assert!(!p.root.join("vendor").exists());
    assert_eq!(p.server.total_hits(), 0);

    let out = p.phpm_env(
        &["install", "--ignore-platform-req=ext-*"],
        &[("PATH", &path)],
    );
    ok(&out);
    let again = ok(&p.phpm_env(
        &["install", "--ignore-platform-req=ext-*", "-v"],
        &[("PATH", &path)],
    ));
    assert!(again.contains("vendor/ matches"), "{again}");
    std::fs::OpenOptions::new()
        .append(true)
        .open(p.home.join("bin/php"))
        .unwrap()
        .write_all(b"\n")
        .unwrap();
    let changed = ok(&p.phpm_env(
        &["install", "--ignore-platform-req=ext-*", "-v"],
        &[("PATH", &path)],
    ));
    assert!(
        changed.contains("platform requirements met by"),
        "{changed}"
    );
}

#[test]
#[cfg(unix)]
fn platform_checks_need_php_unless_ignored() {
    let p = Project::with(|_, lock| {
        lock["platform"] = json!({"php": ">=8.1"});
    });
    let empty = p.home.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let out = p.phpm_env(&["install"], &[("PATH", empty.to_str().unwrap())]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("php is not on PATH"),
        "{}",
        stderr(&out)
    );
    ok(&p.phpm_env(
        &["install", "--ignore-platform-reqs"],
        &[("PATH", empty.to_str().unwrap())],
    ));
}

#[test]
#[cfg(unix)]
fn installs_path_packages_and_local_artifacts() {
    let p = Project::with(|composer, lock| {
        composer["require"] = json!({"l/linked": "*", "l/copied": "*", "l/art": "*"});
        lock["packages"] = json!([
            {"name": "l/linked", "version": "1.0.0", "type": "library",
             "dist": {"type": "path", "url": "packages/linked", "reference": "r1"},
             "transport-options": {"relative": true},
             "autoload": {"psr-4": {"Linked\\": "src/"}}},
            {"name": "l/copied", "version": "1.0.0", "type": "library",
             "dist": {"type": "path", "url": "packages/copied", "reference": "r2"},
             "transport-options": {"symlink": false, "relative": true}},
            {"name": "l/art", "version": "1.0.0", "type": "library",
             "dist": {"type": "zip", "url": "artifacts/art.zip", "shasum": ""}},
        ]);
        lock["packages-dev"] = json!([]);
    });
    for pkg in ["linked", "copied"] {
        let dir = p.root.join("packages").join(pkg);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/A.php"), "<?php\n").unwrap();
    }
    std::fs::write(
        p.root.join("packages/copied/.gitattributes"),
        "/src export-ignore\n",
    )
    .unwrap();
    std::fs::create_dir_all(p.root.join("artifacts")).unwrap();
    std::fs::write(
        p.root.join("artifacts/art.zip"),
        zip("art-1", &[("lib/Art.php", 0o644, "<?php\n")]),
    )
    .unwrap();
    let out = ok(&p.phpm(&["install", "-v"]));
    assert!(out.contains("Installed 3 packages"), "{out}");
    assert_eq!(
        std::fs::read_link(p.vendor("l/linked")).unwrap(),
        std::path::Path::new("../../packages/linked/")
    );
    assert!(p.vendor("l/copied").is_dir() && !p.vendor("l/copied/src").exists());
    assert!(p.vendor("l/art/lib/Art.php").is_file());
    assert_eq!(p.server.total_hits(), 0);
    let again = ok(&p.phpm_env(&["install", "-v"], &[("COMPOSER_MIRROR_PATH_REPOS", "1")]));
    assert!(again.contains("Nothing to install"), "{again}");
    std::fs::remove_dir_all(p.root.join("vendor")).unwrap();
    ok(&p.phpm_env(&["install"], &[("COMPOSER_MIRROR_PATH_REPOS", "1")]));
    assert!(p.vendor("l/linked/src/A.php").is_file());
    assert!(
        !std::fs::symlink_metadata(p.vendor("l/linked"))
            .unwrap()
            .is_symlink()
    );
}

/// A composer repository on the test server whose malware list flags a/lib 1.0.0.
fn malware_repo() -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    files.insert(
        "/packages.json".to_owned(),
        json!({
            "metadata-url": "/p2/%package%.json",
            "filter": {"metadata": true, "lists": {"malware": {"enabled": true}}, "summary-url": "/lists/summary.json"},
            "security-advisories": {"metadata": true, "api-url": "/api/security-advisories/"},
        })
        .to_string()
        .into_bytes(),
    );
    files.insert(
        "/lists/summary.json".to_owned(),
        json!({"filter": {"malware": {"a/lib": "1.0.0", "z/other": "*"}}})
            .to_string()
            .into_bytes(),
    );
    files.insert(
        "/p2/a/lib.json".to_owned(),
        json!({"packages": {}, "filter": {"malware": [
            {"constraint": "1.0.0", "url": "https://example.test/a/lib/malware/", "reason": "malware", "id": "PKFE-test", "source": "phpm-test"},
            {"constraint": "2.0.0", "reason": "other version"},
        ]}})
        .to_string()
        .into_bytes(),
    );
    files.insert(
        "/api/security-advisories/".to_owned(),
        json!({"advisories": {"b/tool": [{
            "advisoryId": "PKSA-test", "packageName": "b/tool", "affectedVersions": ">=1.0,<1.1",
            "title": "Test advisory", "cve": "CVE-2026-0001", "link": "https://example.test/adv",
            "reportedAt": "2026-01-01 00:00:00", "sources": [{"name": "GitHub", "remoteId": "GHSA-test"}], "severity": "high",
        }]}})
        .to_string()
        .into_bytes(),
    );
    files
}

fn with_repo(files: BTreeMap<String, Vec<u8>>, policy: &Value) -> Project {
    let p = Project::serving(files, |_, _| {});
    let mut composer: Value =
        serde_json::from_slice(&std::fs::read(p.root.join("composer.json")).unwrap()).unwrap();
    composer["repositories"] = json!([
        {"type": "composer", "url": p.server.base},
        {"packagist.org": false},
    ]);
    composer["config"] = json!({"policy": policy});
    std::fs::write(p.root.join("composer.json"), composer.to_string()).unwrap();
    p
}

fn hits(p: &Project, path: &str) -> usize {
    p.server
        .hits
        .lock()
        .unwrap()
        .get(path)
        .copied()
        .unwrap_or(0)
}

#[test]
fn the_malware_filter_blocks_a_flagged_locked_package() {
    let p = with_repo(malware_repo(), &json!({}));
    let out = p.phpm(&["install"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains("Your lock file does not contain a compatible set of packages. Please run composer update.\n\n  Problem 1\n    - Package a/lib 1.0.0 (in the lock file) was not loaded, because it was flagged as malware reported by phpm-test (see https://example.test/a/lib/malware/) reason: malware. To ignore filters for this package, add the package to the \"policy.malware.ignore\" config. To turn the feature off entirely, you can set \"policy.malware.block\" to false."),
        "{err}"
    );
    assert!(!p.root.join("vendor").exists());
    assert!(
        !p.cache
            .join("pkgs/v1/a~lib")
            .read_dir()
            .is_ok_and(|mut d| d.next().is_some()),
        "a refused package fetched while the lists were checked leaves the store"
    );
    assert_eq!(hits(&p, "/lists/summary.json"), 1);
    assert_eq!(hits(&p, "/p2/a/lib.json"), 1);
    assert_eq!(hits(&p, "/p2/b/tool.json"), 0);

    let again = p.phpm(&["install"]);
    assert_eq!(again.status.code(), Some(2));
    assert_eq!(
        hits(&p, "/packages.json"),
        1,
        "the root file is reused for 600s"
    );

    ok(&p.phpm(&["install", "--no-blocking"]));
    std::fs::remove_dir_all(p.root.join("vendor")).unwrap();
    ok(&p.phpm_env(&["install"], &[("COMPOSER_POLICY_MALWARE_BLOCK", "0")]));
}

#[test]
fn a_fresh_clean_verdict_is_reused_until_the_lists_go_stale() {
    let mut files = malware_repo();
    files.insert(
        "/packages.json".to_owned(),
        json!({
            "metadata-url": "/p2/%package%.json",
            "filter": {"metadata": true, "lists": {"malware": {"enabled": true}}, "summary-url": "/lists/fresh.json"},
        })
        .to_string()
        .into_bytes(),
    );
    files.insert(
        "/lists/fresh.json".to_owned(),
        json!({"filter": {"malware": {"z/other": "*", "b/tool": ">=2"}}})
            .to_string()
            .into_bytes(),
    );
    let p = with_repo(files, &json!({}));
    ok(&p.phpm(&["install"]));
    assert_eq!(hits(&p, "/lists/fresh.json"), 1);
    std::fs::remove_dir_all(p.root.join("vendor")).unwrap();
    let again = ok(&p.phpm(&["install", "-vv"]));
    assert!(again.contains("Installed"), "{again}");
    assert_eq!(
        hits(&p, "/lists/fresh.json"),
        1,
        "the same packages, checked clean within the summary's max-age"
    );
    ok(&p.phpm(&["install", "--no-dev"]));
    assert_eq!(hits(&p, "/lists/fresh.json"), 1, "a subset is covered too");

    let mut lock: Value =
        serde_json::from_slice(&std::fs::read(p.root.join("composer.lock")).unwrap()).unwrap();
    lock["packages"][0]["version"] = json!("1.0.1");
    std::fs::write(p.root.join("composer.lock"), lock.to_string()).unwrap();
    ok(&p.phpm(&["install"]));
    assert_eq!(
        hits(&p, "/lists/fresh.json"),
        2,
        "a version never checked asks the lists again"
    );
}

#[test]
fn flagged_packages_are_checked_on_every_install() {
    let p = with_repo(malware_repo(), &json!({"malware": {"ignore": ["a/lib"]}}));
    ok(&p.phpm(&["install"]));
    std::fs::remove_dir_all(p.root.join("vendor")).unwrap();
    ok(&p.phpm(&["install"]));
    assert_eq!(hits(&p, "/lists/summary.json"), 2);
    assert_eq!(hits(&p, "/p2/a/lib.json"), 2);
}

#[test]
fn ignored_packages_and_sources_install() {
    for policy in [
        json!({"malware": {"ignore": {"a/lib": "reviewed"}}}),
        json!({"malware": {"ignore": {"a/*": {"constraint": "^1.0"}}}}),
        json!({"malware": {"ignore-source": ["phpm-test"]}}),
        json!({"malware": {"block-scope": "update"}}),
        json!({"malware": false}),
    ] {
        let p = with_repo(malware_repo(), &policy);
        let out = ok(&p.phpm(&["install"]));
        assert!(out.contains("Installed"), "{policy}: {out}");
    }
}

#[test]
fn an_unreachable_filter_list_warns_and_installs() {
    let mut files = malware_repo();
    files.insert(
        "/packages.json".to_owned(),
        json!({"filter": {"metadata": true, "lists": {"malware": {"enabled": true}}, "summary-url": "/lists/missing.json"}})
            .to_string()
            .into_bytes(),
    );
    let p = with_repo(files, &json!({}));
    let out = ok(&p.phpm(&["install"]));
    assert!(
        out.contains("Filter list data could not be fetched from some sources (ignored per policy.ignore-unreachable); matches may be incomplete:"),
        "{out}"
    );
    assert!(out.contains("missing.json returned 404"), "{out}");
}

#[test]
fn audit_reports_advisories_and_exits_5() {
    let p = with_repo(malware_repo(), &json!({"malware": {"ignore": ["a/lib"]}}));
    let out = p.phpm(&["install", "--audit"]);
    assert_eq!(out.status.code(), Some(5), "{}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        stdout,
        "Found 1 security vulnerability advisory affecting 1 package.\nRun \"composer audit\" for a full list of advisories.\n"
    );
    let plain = p.phpm(&["install", "--audit", "--audit-format", "plain"]);
    let stdout = String::from_utf8_lossy(&plain.stdout).into_owned();
    assert!(
        stdout.contains("Package: b/tool\nSeverity: high\nAdvisory ID: PKSA-test\nCVE: CVE-2026-0001\nTitle: Test advisory\n"),
        "{stdout}"
    );
    let json_out = p.phpm(&["install", "--audit", "--audit-format", "json"]);
    let doc: Value = serde_json::from_slice(&json_out.stdout).unwrap();
    assert_eq!(doc["advisories"]["b/tool"][0]["advisoryId"], "PKSA-test");

    let mut composer: Value =
        serde_json::from_slice(&std::fs::read(p.root.join("composer.json")).unwrap()).unwrap();
    composer["config"]["audit"] = json!({"ignore": {"GHSA-test": "accepted"}});
    std::fs::write(p.root.join("composer.json"), composer.to_string()).unwrap();
    let ignored = p.phpm(&["install", "--audit"]);
    assert_eq!(ignored.status.code(), Some(0), "{}", stderr(&ignored));
    let stdout = String::from_utf8_lossy(&ignored.stdout).into_owned();
    assert!(
        stdout.starts_with("Found 1 ignored security vulnerability advisory affecting 1 package."),
        "{stdout}"
    );
}

#[test]
fn filter_api_urls_are_posted_to() {
    let mut files = BTreeMap::new();
    files.insert(
        "/packages.json".to_owned(),
        json!({"filter": {"metadata": true, "lists": {"malware": {"enabled": true}, "advisories": {"enabled": true}}, "api-url": "/api/filter"}})
            .to_string()
            .into_bytes(),
    );
    files.insert(
        "/api/filter".to_owned(),
        json!({"filter": {"malware": [{"package": "b/tool", "constraint": "*", "source": "api"}]}})
            .to_string()
            .into_bytes(),
    );
    let p = with_repo(files, &json!({}));
    let out = p.phpm(&["install"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("Package b/tool 1.0.0 (in the lock file) was not loaded, because it was flagged as malware reported by api."),
        "{}",
        stderr(&out)
    );
}

#[test]
#[ignore = "asks the real Packagist; aikido/endpoint-test is a harmless package on its malware list"]
fn live_packagist_blocks_the_aikido_test_package() {
    let p = Project::with(|composer, lock| {
        composer["require"] = json!({"aikido/endpoint-test": "0.0.1"});
        lock["packages"] = json!([{
            "name": "aikido/endpoint-test",
            "version": "0.0.1",
            "dist": {"type": "zip", "url": "https://api.github.com/repos/Aikido-demo-apps/endpoint-php-test/zipball/2e7234b3021c26c1839fd06e2a7625d29f3bb229", "reference": "2e7234b3021c26c1839fd06e2a7625d29f3bb229", "shasum": ""},
            "type": "library",
        }]);
        lock["packages-dev"] = json!([]);
    });
    let out = p.phpm(&["install"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("- Package aikido/endpoint-test 0.0.1 (in the lock file) was not loaded, because it was flagged as malware reported by aikido"),
        "{}",
        stderr(&out)
    );
    assert!(!p.root.join("vendor").exists());
}

/// The POSTs once at least `n` have arrived: a detached phpm sends them
/// after the install has exited.
fn posts(p: &Project, n: usize) -> Vec<(String, String)> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let posts = p.server.posts.lock().unwrap().clone();
        if posts.len() >= n || std::time::Instant::now() > deadline {
            return posts;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn notifies_downloads_once_per_url_for_what_was_installed() {
    let p = Project::with(|_, lock| {
        let url = "/downloads/";
        for (i, entry) in lock["packages"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            if i < 2 {
                entry["notification-url"] = json!(url);
            }
        }
    });
    let mut lock: Value =
        serde_json::from_slice(&std::fs::read(p.root.join("composer.lock")).unwrap()).unwrap();
    for entry in lock["packages"].as_array_mut().unwrap() {
        if entry.get("notification-url").is_some() {
            entry["notification-url"] = json!(format!("{}/downloads/", p.server.base));
        }
    }
    std::fs::write(p.root.join("composer.lock"), lock.to_string()).unwrap();
    ok(&p.phpm(&["install"]));
    let first = posts(&p, 1);
    assert_eq!(first.len(), 1, "{first:?}");
    let posts_seen = first;
    assert_eq!(posts_seen[0].0, "/downloads/");
    let body: Value = serde_json::from_str(&posts_seen[0].1).unwrap();
    assert_eq!(body["downloads"][0]["name"], "a/lib");
    assert_eq!(body["downloads"][0]["version"], "1.0.0.0");
    assert_eq!(body["downloads"][1]["name"], "b/tool");
    assert!(
        body["downloads"][0].get("downloaded").is_none(),
        "only Packagist gets sizes"
    );
    assert!(
        posts_seen[0].1.contains(r#""name":"a\/lib""#),
        "PHP json_encode escapes slashes"
    );

    ok(&p.phpm(&["install"]));
    std::fs::remove_dir_all(p.root.join("vendor/a")).unwrap();
    ok(&p.phpm(&["install"]));
    let second = posts(&p, 2);
    assert_eq!(second.len(), 2, "{second:?}");
    let body: Value = serde_json::from_str(&second[1].1).unwrap();
    assert_eq!(body["downloads"].as_array().unwrap().len(), 1);

    let mut composer: Value =
        serde_json::from_slice(&std::fs::read(p.root.join("composer.json")).unwrap()).unwrap();
    composer["config"] = json!({"notify-on-install": false});
    std::fs::write(p.root.join("composer.json"), composer.to_string()).unwrap();
    std::fs::remove_dir_all(p.root.join("vendor")).unwrap();
    ok(&p.phpm(&["install"]));
    ok(&p.phpm_env(
        &["install", "--no-dev"],
        &[("COMPOSER_DISABLE_NETWORK", "1")],
    ));
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(p.server.posts.lock().unwrap().len(), 2);
}
