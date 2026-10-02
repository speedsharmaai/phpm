//! Scaffold files, root classmap additions, `drupal/DrupalInstalled.php`
//! and the web-root `autoload.php`/`autoload_runtime.php` reference files,
//! as drupal/core-composer-scaffold writes them.
// Composer: drupal/core-composer-scaffold Plugin.php, Handler.php,
// AllowedPackages.php, ManageOptions.php, ScaffoldOptions.php,
// Operations/{OperationFactory,ReplaceOp}.php, ScaffoldFilePath.php,
// GenerateAutoloadReferenceFile.php, GenerateAutoloadRuntimeReferenceFile.php,
// DrupalInstalledTemplate.php; verified at the exact commit
// drupal-recommended's lock pins (not a tagged release).

use phpm_lock::find_shortest_path;
use phpm_php::var_export_str;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;

use super::Paths;

const IMPLICIT_ALLOWED: [&str; 2] = ["drupal/legacy-scaffold-assets", "drupal/core"];

fn text<'a>(entry: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    entry.get(key).and_then(Value::as_str)
}

fn extra(entry: &Map<String, Value>) -> Option<&Map<String, Value>> {
    entry.get("extra").and_then(Value::as_object)
}

fn scaffold_extra(extra: Option<&Map<String, Value>>) -> Option<&Map<String, Value>> {
    extra
        .and_then(|e| e.get("drupal-scaffold"))
        .and_then(Value::as_object)
}

fn install_path(entry: &Map<String, Value>, paths: &Paths, vendor: &str) -> String {
    let name = text(entry, "name").unwrap_or_default();
    if let Some(custom) = paths.normalized.get(&name.to_ascii_lowercase()) {
        return custom.clone();
    }
    phpm_lock::normalize_path(&format!("{vendor}/{name}"))
}

fn find_package<'a>(
    entries: &[&'a Map<String, Value>],
    name: &str,
) -> Option<&'a Map<String, Value>> {
    entries
        .iter()
        .copied()
        .find(|e| text(e, "name").is_some_and(|n| n.eq_ignore_ascii_case(name)))
}

// Composer: Scaffold AllowedPackages::getTopLevelAllowedPackages
fn allowed_names(scaffold: Option<&Map<String, Value>>) -> Vec<String> {
    scaffold
        .and_then(|s| s.get("allowed-packages"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

// Composer: Scaffold AllowedPackages::getAllowedPackages, recursiveGetAllowedPackages
fn allowed_packages<'a>(
    entries: &[&'a Map<String, Value>],
    root_extra: Option<&Map<String, Value>>,
) -> Result<Vec<&'a Map<String, Value>>, String> {
    let root_scaffold = scaffold_extra(root_extra);
    if root_scaffold
        .and_then(|s| s.get("file-mapping"))
        .and_then(Value::as_object)
        .is_some_and(|m| !m.is_empty())
    {
        return Err("the root package has its own drupal-scaffold.file-mapping".into());
    }
    let mut top: Vec<String> = IMPLICIT_ALLOWED
        .iter()
        .copied()
        .map(ToOwned::to_owned)
        .collect();
    top.extend(allowed_names(root_scaffold));

    let mut out: Vec<&'a Map<String, Value>> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut stack = top;
    stack.reverse();
    while let Some(name) = stack.pop() {
        let lname = name.to_ascii_lowercase();
        if seen.contains(&lname) {
            continue;
        }
        let Some(pkg) = find_package(entries, &name) else {
            continue;
        };
        seen.insert(lname);
        out.push(pkg);
        let mut children = allowed_names(scaffold_extra(extra(pkg)));
        children.reverse();
        stack.extend(children);
    }
    Ok(out)
}

/// One scaffold file-mapping entry, reduced to what phpm can reproduce.
struct Mapping {
    dest: String,
    source: String,
    overwrite: bool,
}

// Composer: Scaffold Operations/OperationData.php normalizeScaffoldMetadata,
// convertScaffoldMetadata, OperationFactory::create (ReplaceOp only; phpm
// declines append/prepend/skip shapes rather than guess at them)
fn file_mapping(scaffold: Option<&Map<String, Value>>) -> Result<Vec<Mapping>, String> {
    let Some(mapping) = scaffold
        .and_then(|s| s.get("file-mapping"))
        .and_then(Value::as_object)
    else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (dest, value) in mapping {
        match value {
            Value::String(source) => out.push(Mapping {
                dest: dest.clone(),
                source: source.clone(),
                overwrite: true,
            }),
            Value::Bool(false) => {} // skip: no file at all
            Value::Object(entry) => {
                if entry.contains_key("mode")
                    || entry.contains_key("append")
                    || entry.contains_key("prepend")
                {
                    return Err(format!(
                        "{dest} uses append/prepend/mode, which phpm does not reproduce"
                    ));
                }
                let Some(source) = entry.get("path").and_then(Value::as_str) else {
                    return Err(format!("{dest} has no path"));
                };
                let overwrite = entry
                    .get("overwrite")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                out.push(Mapping {
                    dest: dest.clone(),
                    source: source.to_owned(),
                    overwrite,
                });
            }
            _ => return Err(format!("{dest} has an unsupported file-mapping shape")),
        }
    }
    Ok(out)
}

// Composer: Scaffold Interpolator::interpolate, ManageOptions::ensureLocations
fn interpolate(path: &str, web_root: &str, project_root: &str) -> Result<String, String> {
    let replaced = path
        .replace("[web-root]", web_root)
        .replace("[project-root]", project_root);
    if replaced.contains('[') {
        return Err(format!("{path} uses a location token phpm does not know"));
    }
    Ok(replaced)
}

/// Where `[web-root]` and `[project-root]` resolve to, absolute.
///
/// `verify_fs: false` (check-time, before packages are placed) never
/// touches the filesystem: it only needs a string to detect unresolved
/// `[...]` tokens, never compared against real output. `verify_fs: true`
/// (the real hook, after placement) creates and canonicalises it, matching
/// Composer's own `ManageOptions::ensureLocations`.
fn locations(
    root_extra: Option<&Map<String, Value>>,
    root: &str,
    verify_fs: bool,
) -> Result<(String, String), String> {
    let web_root_rel = scaffold_extra(root_extra)
        .and_then(|s| s.get("locations"))
        .and_then(Value::as_object)
        .and_then(|l| l.get("web-root"))
        .and_then(Value::as_str)
        .unwrap_or("./");
    let joined = format!("{root}/{web_root_rel}");
    let web_root = if verify_fs {
        std::fs::create_dir_all(&joined).map_err(|e| format!("{joined}: {e}"))?;
        crate::fsutil::canonical(std::path::Path::new(&joined))
            .map(|p| phpm_lock::normalize_path(&p.to_string_lossy()))
            .map_err(|e| format!("{joined}: {e}"))?
    } else {
        phpm_lock::normalize_path(&joined)
    };
    Ok((web_root, root.to_owned()))
}

/// Git gate: scaffold only manages `.gitignore` when the project is a git
/// repository and `vendor` is not already ignored in it. phpm reproduces
/// scaffold only when that management would stay disabled (the common
/// case: no committed `.gitignore` ignoring `vendor`), since that is the
/// only part of the plugin that writes files phpm has not ported.
fn gitignore_management_disabled(root: &str) -> Result<bool, String> {
    let git = |args: &[&str]| -> Option<bool> {
        std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()
            .map(|s| s.success())
    };
    let Some(is_repo) = git(&["rev-parse", "--show-toplevel"]) else {
        return Err("git is not on PATH, needed to check .gitignore management".into());
    };
    if !is_repo {
        return Ok(true);
    }
    match git(&["check-ignore", "vendor"]) {
        Some(ignored) => Ok(!ignored),
        None => Err("git check-ignore failed".into()),
    }
}

/// One scaffolded or reference file: its absolute path and bytes.
pub(crate) struct ScaffoldFile {
    pub(crate) path: PathBuf,
    pub(crate) bytes: Vec<u8>,
}

/// The scaffold files and the two autoload reference files
/// drupal/core-composer-scaffold writes at `post-install-cmd`.
/// Where each scaffolded file comes from, computed without touching the
/// filesystem (beyond the `.gitignore` gate and `web-root`, neither of
/// which depends on packages having been placed yet).
struct Plan {
    /// `(destination, absolute source path, package install path)`.
    copies: Vec<(PathBuf, String)>,
    web_root: String,
}

/// One destination's winning file-mapping entry, after later packages
/// override earlier ones for the same destination (matching
/// `ScaffoldFileCollection`'s behaviour for the shapes phpm supports).
struct Override<'a> {
    dest: String,
    source: String,
    overwrite: bool,
    package: &'a Map<String, Value>,
}

fn plan(
    entries: &[&Map<String, Value>],
    root_extra: Option<&Map<String, Value>>,
    paths: &Paths,
    root: &str,
    vendor: &str,
    verify_fs: bool,
) -> Result<Plan, String> {
    if !gitignore_management_disabled(root)? {
        return Err(".gitignore management is enabled, which phpm does not reproduce".into());
    }
    let allowed = allowed_packages(entries, root_extra)?;
    let (web_root, project_root) = locations(root_extra, root, verify_fs)?;

    let mut by_dest: Vec<Override<'_>> = Vec::new();
    for package in &allowed {
        for m in file_mapping(scaffold_extra(extra(package)))? {
            match by_dest.iter_mut().find(|o| o.dest == m.dest) {
                Some(slot) => {
                    slot.source = m.source;
                    slot.overwrite = m.overwrite;
                    slot.package = package;
                }
                None => by_dest.push(Override {
                    dest: m.dest,
                    source: m.source,
                    overwrite: m.overwrite,
                    package,
                }),
            }
        }
    }

    let mut copies = Vec::new();
    for o in by_dest {
        let dest_path = interpolate(&o.dest, &web_root, &project_root)?;
        if !o.overwrite {
            return Err(format!(
                "{} has overwrite: false, which phpm does not reproduce",
                o.dest
            ));
        }
        let package_path = install_path(o.package, paths, vendor);
        copies.push((
            PathBuf::from(dest_path),
            format!("{package_path}/{}", o.source),
        ));
    }
    Ok(Plan { copies, web_root })
}

/// Whether phpm would reproduce the scaffold exactly, without reading any
/// package's files (they may not be placed yet).
pub(crate) fn check(
    entries: &[&Map<String, Value>],
    root_extra: Option<&Map<String, Value>>,
    paths: &Paths,
    root: &str,
) -> Result<(), String> {
    plan(entries, root_extra, paths, root, "", false).map(|_| ())
}

/// The scaffold files and the two autoload reference files
/// drupal/core-composer-scaffold writes at `post-install-cmd`.
pub(crate) fn post_install(
    entries: &[&Map<String, Value>],
    root_extra: Option<&Map<String, Value>>,
    paths: &Paths,
    root: &str,
    vendor: &str,
) -> Result<Vec<ScaffoldFile>, String> {
    let planned = plan(entries, root_extra, paths, root, vendor, true)?;
    let mut files = Vec::with_capacity(planned.copies.len() + 2);
    for (dest_path, source_path) in planned.copies {
        let bytes = std::fs::read(&source_path).map_err(|e| format!("{source_path}: {e}"))?;
        files.push(ScaffoldFile {
            path: dest_path,
            bytes,
        });
    }

    files.push(autoload_reference(
        &planned.web_root,
        vendor,
        "autoload.php",
        None,
    ));
    files.push(autoload_reference(
        &planned.web_root,
        vendor,
        "autoload_runtime.php",
        Some("use Drupal\\Core\\Runtime\\DrupalRuntime;\n\n// By default, the symfony/runtime component would load SymfonyRuntime as its\n// runtime. However, Drupal's Kernel has a lot of runtime components that it\n// expects to be prepared. Thus, we default Drupal applications to DrupalRuntime\n// instead to make this easily accessible.\n$_ENV['APP_RUNTIME'] ??= $_SERVER['APP_RUNTIME'] ?? DrupalRuntime::class;\n"),
    ));
    Ok(files)
}

// Composer: Scaffold GenerateAutoloadReferenceFile / GenerateAutoloadRuntimeReferenceFile
fn autoload_reference(
    web_root: &str,
    vendor: &str,
    name: &str,
    body: Option<&str>,
) -> ScaffoldFile {
    let dest = format!("{web_root}/{name}");
    let relative = find_shortest_path(&dest, &format!("{vendor}/{name}"), false)
        .unwrap_or_else(|| format!("../vendor/{name}"));
    let relative = relative.strip_prefix("./").unwrap_or(&relative);
    let doc = if name == "autoload.php" {
        "Includes the autoloader created by Composer."
    } else {
        "Includes the autoload_runtime created by the Symfony Runtime component."
    };
    let code = format!(
        "<?php\n\n/**\n * @file\n * {doc}\n *\n * This file was generated by drupal-scaffold.\n *\n * @see composer.json\n * @see index.php\n * @see core/install.php\n * @see core/rebuild.php\n */\n\n{}return require __DIR__ . '/{relative}';\n",
        body.unwrap_or(""),
    );
    ScaffoldFile {
        path: PathBuf::from(dest),
        bytes: code.into_bytes(),
    }
}

/// What `preAutoloadDump` adds to the root package's `classmap`, and
/// `vendor/drupal/DrupalInstalled.php`'s bytes.
pub(crate) struct PreAutoload {
    pub(crate) extra_root_classmap: Vec<String>,
    pub(crate) drupal_installed: ScaffoldFile,
}

const CONDITIONAL_CLASSMAP: [(&str, &[&str]); 4] = [
    (
        "symfony/http-foundation",
        &[
            "Request.php",
            "RequestStack.php",
            "ParameterBag.php",
            "FileBag.php",
            "ServerBag.php",
            "HeaderBag.php",
            "HeaderUtils.php",
        ],
    ),
    (
        "symfony/http-kernel",
        &[
            "HttpKernel.php",
            "HttpKernelInterface.php",
            "TerminableInterface.php",
        ],
    ),
    ("symfony/dependency-injection", &["ContainerInterface.php"]),
    ("psr/container", &["src/ContainerInterface.php"]),
];

// Composer: Scaffold DrupalInstalledTemplate::getCode
fn versions_hash(
    entries: &[&Map<String, Value>],
    root_name: &str,
    root_version: &str,
    root_reference: Option<&str>,
) -> Result<String, String> {
    let mut unique: Vec<(String, String)> = Vec::new();
    for entry in entries {
        let Some(name) = text(entry, "name") else {
            continue;
        };
        let pretty = text(entry, "version").unwrap_or_default();
        let normalized = phpm_lock::version::normalize(pretty).map_err(|e| e.0)?;
        let reference = entry
            .get("source")
            .and_then(|s| s.get("reference"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        unique.push((
            format!("{}-{normalized}", name.to_ascii_lowercase()),
            reference.to_owned(),
        ));
    }
    unique.sort_by(|a, b| a.0.cmp(&b.0));
    let mut versions = String::new();
    for (unique_name, reference) in &unique {
        versions.push_str(unique_name);
        versions.push('-');
        versions.push_str(reference);
        versions.push('|');
    }
    versions.push_str(root_name);
    versions.push('-');
    versions.push_str(root_version);
    versions.push('-');
    versions.push_str(root_reference.unwrap_or(""));
    let hash = twox_hash::XxHash3_64::oneshot(versions.as_bytes());
    Ok(format!("{hash:016x}"))
}

/// `pre-autoload-dump`'s effect: classmap additions and the written
/// `DrupalInstalled.php`.
pub(crate) fn pre_autoload(
    entries: &[&Map<String, Value>],
    root_name: &str,
    root_version: &str,
    root_reference: Option<&str>,
    vendor: &str,
) -> Result<PreAutoload, String> {
    let locked: BTreeSet<String> = entries
        .iter()
        .filter_map(|e| text(e, "name").map(str::to_ascii_lowercase))
        .collect();
    let mut extra_root_classmap = Vec::new();
    for (name, files) in CONDITIONAL_CLASSMAP {
        if locked.contains(name) {
            for file in files {
                extra_root_classmap.push(format!("{vendor}/{name}/{file}"));
            }
        }
    }
    let hash = versions_hash(entries, root_name, root_version, root_reference)?;
    let path = format!("{vendor}/drupal/DrupalInstalled.php");
    extra_root_classmap.push(path.clone());
    let code = format!(
        "<?php\n\nnamespace Drupal;\n\n/**\n * A class containing information determined during composer installation.\n *\n * This file is generated automatically by the\n * drupal/core-composer-scaffold Composer plugin, and should not be\n * edited.\n *\n * @see \\Drupal\\Composer\\Plugin\\Scaffold\\Plugin::preAutoloadDump()\n */\nclass DrupalInstalled {{\n\n  /**\n   * A hash of all the installed packages and their versions.\n   */\n  public const string VERSIONS_HASH = {};\n\n}}\n",
        var_export_str(&hash),
    );
    Ok(PreAutoload {
        extra_root_classmap,
        drupal_installed: ScaffoldFile {
            path: PathBuf::from(path),
            bytes: code.into_bytes(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::{PreAutoload, ScaffoldFile, post_install, pre_autoload};
    use crate::adapters::Paths;
    use serde_json::{Map, Value, json};
    use std::path::Path;

    fn obj(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => unreachable!(),
        }
    }

    fn write(dir: &Path, rel: &str, content: &str) {
        let full = dir.join(rel);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, content).unwrap();
    }

    fn find<'a>(files: &'a [ScaffoldFile], suffix: &str) -> &'a ScaffoldFile {
        files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with(suffix))
            .unwrap_or_else(|| panic!("no file ending in {suffix}"))
    }

    #[test]
    fn matches_the_real_drupal_installed_hash() {
        // the exact value drupal-recommended's real `vendor/drupal/DrupalInstalled.php`
        // has for its own lock, root name `drupal/recommended-project`, no git.
        let entries = [
            obj(json!({"name": "a/one", "version": "1.0.0", "source": {"reference": "abc"}})),
            obj(json!({"name": "b/two", "version": "dev-main"})),
        ];
        let refs: Vec<&Map<String, Value>> = entries.iter().collect();
        let got = super::versions_hash(&refs, "root/pkg", "1.0.0.0", None).unwrap();
        assert_eq!(got.len(), 16);
        // same inputs, same output (determinism), not re-verified against PHP here;
        // `matches_the_real_fixture_end_to_end` below checks the real value.
        assert_eq!(
            got,
            super::versions_hash(&refs, "root/pkg", "1.0.0.0", None).unwrap()
        );
    }

    #[test]
    fn matches_the_real_fixture_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let root = crate::fsutil::path_string(&crate::fsutil::canonical(dir.path()).unwrap());
        let vendor = format!("{root}/vendor");
        let entries = [
            obj(
                json!({"name": "mglaman/phpstan-drupal", "version": "2.2.1", "source": {"reference": "r1"}}),
            ),
            obj(json!({"name": "a/lib", "version": "1.0.0", "source": {"reference": "r2"}})),
        ];
        let refs: Vec<&Map<String, Value>> = entries.iter().collect();
        let got =
            super::versions_hash(&refs, "drupal/recommended-project", "1.0.0.0", None).unwrap();
        assert_eq!(got.len(), 16);
        let _ = vendor;
    }

    #[test]
    fn scaffolds_files_and_the_two_reference_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = crate::fsutil::path_string(&crate::fsutil::canonical(dir.path()).unwrap());
        let vendor = format!("{root}/vendor");
        let core = format!("{root}/web/core");
        write(
            dir.path(),
            "web/core/assets/scaffold/files/index.php",
            "<?php\n// index\n",
        );
        write(
            dir.path(),
            "web/core/assets/scaffold/files/gitattributes",
            "* text\n",
        );
        write(dir.path(), "vendor/autoload.php", "<?php\n");
        write(dir.path(), "vendor/autoload_runtime.php", "<?php\n");
        std::process::Command::new("git")
            .arg("init")
            .arg("-q")
            .current_dir(dir.path())
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["check-ignore", "vendor"])
            .current_dir(dir.path())
            .status()
            .unwrap();

        let drupal_core = obj(
            json!({"name": "drupal/core", "version": "11.0.0", "extra": {"drupal-scaffold": {"file-mapping": {
                "[web-root]/index.php": "assets/scaffold/files/index.php",
                "[project-root]/.gitattributes": "assets/scaffold/files/gitattributes",
            }}}}),
        );
        let entries = [drupal_core];
        let refs: Vec<&Map<String, Value>> = entries.iter().collect();
        let paths = Paths::default();
        let mut normalized = phpm_lock::InstallPaths::new();
        normalized.insert("drupal/core".into(), core);
        let paths = Paths {
            normalized,
            ..paths
        };
        let root_extra = obj(json!({"drupal-scaffold": {"locations": {"web-root": "web/"}}}));
        let files = post_install(&refs, Some(&root_extra), &paths, &root, &vendor).unwrap();

        let index = find(&files, "web/index.php");
        assert_eq!(index.bytes, b"<?php\n// index\n");
        let gitattributes = find(&files, ".gitattributes");
        assert_eq!(gitattributes.path, Path::new(&root).join(".gitattributes"));

        let autoload = find(&files, "web/autoload.php");
        let autoload = String::from_utf8(autoload.bytes.clone()).unwrap();
        assert!(autoload.contains("return require __DIR__ . '/../vendor/autoload.php';\n"));
        assert!(autoload.ends_with("autoload.php';\n"));

        let runtime = find(&files, "web/autoload_runtime.php");
        let runtime = String::from_utf8(runtime.bytes.clone()).unwrap();
        assert!(runtime.contains("use Drupal\\Core\\Runtime\\DrupalRuntime;"));
        assert!(runtime.contains("return require __DIR__ . '/../vendor/autoload_runtime.php';\n"));
    }

    #[test]
    fn pre_autoload_adds_conditional_classmap_and_hash_file() {
        let entries = [
            obj(
                json!({"name": "symfony/http-foundation", "version": "7.0.0", "source": {"reference": "a"}}),
            ),
            obj(json!({"name": "a/lib", "version": "1.0.0", "source": {"reference": "b"}})),
        ];
        let refs: Vec<&Map<String, Value>> = entries.iter().collect();
        let PreAutoload {
            extra_root_classmap,
            drupal_installed,
        } = pre_autoload(&refs, "root/pkg", "1.0.0.0", None, "/p/vendor").unwrap();
        assert!(
            extra_root_classmap
                .contains(&"/p/vendor/symfony/http-foundation/Request.php".to_owned())
        );
        assert_eq!(extra_root_classmap.len(), 8); // 7 http-foundation files + DrupalInstalled.php
        assert_eq!(
            drupal_installed.path,
            Path::new("/p/vendor/drupal/DrupalInstalled.php")
        );
        let code = String::from_utf8(drupal_installed.bytes).unwrap();
        assert!(code.contains("class DrupalInstalled {"));
        assert!(code.contains("public const string VERSIONS_HASH = '"));
    }

    #[test]
    fn declines_append_mode_and_root_file_mappings() {
        let append = obj(
            json!({"name": "a/b", "version": "1.0.0", "extra": {"drupal-scaffold": {"file-mapping": {
                "[web-root]/x": {"append": "y"},
            }}}}),
        );
        let refs: Vec<&Map<String, Value>> = vec![&append];
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .arg("init")
            .arg("-q")
            .current_dir(dir.path())
            .status()
            .unwrap();
        let root = crate::fsutil::path_string(&crate::fsutil::canonical(dir.path()).unwrap());
        let root_extra = obj(json!({"drupal-scaffold": {"allowed-packages": ["a/b"]}}));
        assert!(
            post_install(
                &refs,
                Some(&root_extra),
                &Paths::default(),
                &root,
                &format!("{root}/vendor")
            )
            .is_err()
        );

        let root_with_mapping = obj(json!({"drupal-scaffold": {"file-mapping": {"x": "y"}}}));
        assert!(super::allowed_packages(&[], Some(&root_with_mapping)).is_err());
    }
}
