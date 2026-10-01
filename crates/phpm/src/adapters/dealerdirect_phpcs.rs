//! `vendor/squizlabs/php_codesniffer/CodeSniffer.conf`'s `installed_paths`
//! as dealerdirect/phpcodesniffer-composer-installer writes it.
// Composer: dealerdirect/phpcodesniffer-composer-installer src/Plugin.php
// onDependenciesChangedEvent and its helpers; verified v1.2.1 only (every
// release changed the file).

use phpm_lock::{constraint, find_shortest_path};
use phpm_php::var_export_str;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::Paths;

const PACKAGE_NAME: &str = "squizlabs/php_codesniffer";
const PACKAGE_TYPE: &str = "phpcodesniffer-standard";

fn text<'a>(entry: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    entry.get(key).and_then(Value::as_str)
}

fn install_path(entry: &Map<String, Value>, paths: &Paths, vendor: &str) -> String {
    let name = text(entry, "name").unwrap_or_default();
    if let Some(custom) = paths.normalized.get(&name.to_ascii_lowercase()) {
        return custom.clone();
    }
    phpm_lock::normalize_path(&format!("{vendor}/{name}"))
}

// Composer: dealerdirect Plugin::getMinDepth
fn min_depth(phpcs: &Map<String, Value>) -> u32 {
    let version = text(phpcs, "version").unwrap_or_default();
    let normalized = phpm_lock::version::normalize(version).unwrap_or_default();
    let before_3 = constraint::version_compare(&normalized, "3.0.0.0") == std::cmp::Ordering::Less;
    u32::from(before_3)
}

// Composer: dealerdirect Plugin::getMaxDepth
fn max_depth(root_extra: Option<&Map<String, Value>>, min: u32) -> Result<u32, String> {
    let Some(value) = root_extra.and_then(|e| e.get("phpcodesniffer-search-depth")) else {
        return Ok(3);
    };
    let Some(n) = value.as_i64() else {
        return Err("extra.phpcodesniffer-search-depth is not an integer".into());
    };
    let max =
        u32::try_from(n).map_err(|_| "extra.phpcodesniffer-search-depth is negative".to_owned())?;
    if max <= min {
        return Err(format!(
            "extra.phpcodesniffer-search-depth must be larger than {min}"
        ));
    }
    Ok(max)
}

/// `ruleset.xml` files under `root`, at `min..=max` path segments deep,
/// skipping unreadable and VCS directories, as Symfony Finder's search does.
fn rulesets(root: &Path, min: u32, max: u32, out: &mut Vec<PathBuf>) {
    fn walk(dir: &Path, depth: u32, min: u32, max: u32, out: &mut Vec<PathBuf>) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = read.flatten().collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue; // Finder's ignoreVCS skips dotted (.git, .svn, ...) dirs
            }
            if file_type.is_dir() {
                if depth < max {
                    walk(&entry.path(), depth + 1, min, max, out);
                }
            } else if file_type.is_file() && depth >= min && name == "ruleset.xml" {
                out.push(entry.path());
            }
        }
    }
    walk(root, 0, min, max, out);
}

/// The directory a found `ruleset.xml` contributes to `installed_paths`.
// Composer: dealerdirect Plugin::updateInstalledPaths, the `dirname()` step
fn standards_dir(ruleset: &Path, cwd: &str) -> PathBuf {
    let containing = ruleset.parent().unwrap_or(ruleset);
    if containing.to_string_lossy() == cwd {
        containing.to_path_buf()
    } else {
        containing.parent().unwrap_or(containing).to_path_buf()
    }
}

/// `CodeSniffer.conf`'s bytes, or `None` when the plugin would not write it
/// (no `phpcodesniffer-standard` package found, so nothing changed).
pub(crate) fn generate(
    packages: &[&Map<String, Value>],
    root_type: Option<&str>,
    root_extra: Option<&Map<String, Value>>,
    paths: &Paths,
    root: &str,
    vendor: &str,
) -> Result<Option<Vec<u8>>, String> {
    let Some(phpcs) = packages
        .iter()
        .find(|p| text(p, "name") == Some(PACKAGE_NAME))
    else {
        return Err(format!("{PACKAGE_NAME} is not installed"));
    };
    let min = min_depth(phpcs);
    let max = max_depth(root_extra, min)?;
    let phpcs_install = install_path(phpcs, paths, vendor);

    let mut search_roots: Vec<PathBuf> = Vec::new();
    if root_type == Some(PACKAGE_TYPE) {
        search_roots.push(PathBuf::from(root));
    }
    for entry in packages {
        if text(entry, "type") == Some(PACKAGE_TYPE) {
            search_roots.push(PathBuf::from(install_path(entry, paths, vendor)));
        }
    }
    if search_roots.is_empty() {
        return Ok(None);
    }

    let mut found: Vec<PathBuf> = Vec::new();
    for search_root in &search_roots {
        rulesets(search_root, min, max, &mut found);
    }
    let mut installed_paths: BTreeSet<String> = BTreeSet::new();
    for ruleset in &found {
        let dir = standards_dir(ruleset, root);
        let dir = dir.to_string_lossy();
        let Some(relative) = find_shortest_path(&phpcs_install, &dir, true) else {
            return Err(format!("{dir} has no path relative to {phpcs_install}"));
        };
        installed_paths.insert(relative);
    }
    if installed_paths.is_empty() {
        return Ok(None);
    }

    let mut sorted: Vec<&str> = installed_paths.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    let joined = sorted.join(",");
    let code = format!(
        "<?php\n $phpCodeSnifferConfig = array (\n  'installed_paths' => {},\n);\n?>",
        var_export_str(&joined)
    );
    Ok(Some(code.into_bytes()))
}

#[cfg(test)]
mod tests {
    use super::{generate, standards_dir};
    use crate::adapters::Paths;
    use serde_json::{Map, Value, json};
    use std::path::Path;

    fn obj(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => unreachable!(),
        }
    }

    #[test]
    fn standards_dir_goes_up_one_level_unless_it_is_the_project_root() {
        assert_eq!(
            standards_dir(Path::new("/p/vendor/a/b/Standard/ruleset.xml"), "/p"),
            Path::new("/p/vendor/a/b")
        );
        assert_eq!(
            standards_dir(Path::new("/p/ruleset.xml"), "/p"),
            Path::new("/p")
        );
    }

    #[test]
    fn matches_the_real_drupal_recommended_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_string_lossy().replace('\\', "/");
        let vendor = format!("{root}/vendor");
        for (pkg, rulesets) in [
            (
                "drupal/coder",
                &["coder_sniffer/Drupal", "coder_sniffer/DrupalPractice"][..],
            ),
            (
                "sirbrillig/phpcs-variable-analysis",
                &["VariableAnalysis"][..],
            ),
            ("slevomat/coding-standard", &["SlevomatCodingStandard"][..]),
        ] {
            for sub in rulesets {
                let dir = Path::new(&vendor).join(pkg).join(sub);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join("ruleset.xml"), "<ruleset/>").unwrap();
            }
            std::fs::create_dir_all(Path::new(&vendor).join(pkg).join(".git")).unwrap();
            std::fs::write(
                Path::new(&vendor).join(pkg).join(".git/ruleset.xml"),
                "ignored",
            )
            .unwrap();
        }
        std::fs::create_dir_all(Path::new(&vendor).join(super::PACKAGE_NAME)).unwrap();

        let packages = [
            obj(
                json!({"name": "squizlabs/php_codesniffer", "version": "4.0.4", "type": "library"}),
            ),
            obj(
                json!({"name": "drupal/coder", "version": "9.0.1", "type": "phpcodesniffer-standard"}),
            ),
            obj(
                json!({"name": "sirbrillig/phpcs-variable-analysis", "version": "v2.13.0", "type": "phpcodesniffer-standard"}),
            ),
            obj(
                json!({"name": "slevomat/coding-standard", "version": "8.31.1", "type": "phpcodesniffer-standard"}),
            ),
        ];
        let refs: Vec<&Map<String, Value>> = packages.iter().collect();
        let code = generate(&refs, None, None, &Paths::default(), &root, &vendor)
            .unwrap()
            .unwrap();
        assert_eq!(
            String::from_utf8(code).unwrap(),
            "<?php\n $phpCodeSnifferConfig = array (\n  'installed_paths' => '../../drupal/coder/coder_sniffer,../../sirbrillig/phpcs-variable-analysis,../../slevomat/coding-standard',\n);\n?>"
        );
    }

    #[test]
    fn nothing_to_write_when_no_standards_are_found() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_string_lossy().replace('\\', "/");
        let vendor = format!("{root}/vendor");
        let packages = [obj(
            json!({"name": "squizlabs/php_codesniffer", "version": "4.0.4", "type": "library"}),
        )];
        let refs: Vec<&Map<String, Value>> = packages.iter().collect();
        assert_eq!(
            generate(&refs, None, None, &Paths::default(), &root, &vendor),
            Ok(None)
        );
    }

    #[test]
    fn declines_without_phpcs_or_a_bad_search_depth() {
        let packages: [Map<String, Value>; 0] = [];
        let refs: Vec<&Map<String, Value>> = packages.iter().collect();
        assert!(generate(&refs, None, None, &Paths::default(), "/p", "/p/vendor").is_err());

        let phpcs = [obj(
            json!({"name": "squizlabs/php_codesniffer", "version": "4.0.4", "type": "library"}),
        )];
        let refs: Vec<&Map<String, Value>> = phpcs.iter().collect();
        let bad = obj(json!({"phpcodesniffer-search-depth": "x"}));
        assert!(
            generate(
                &refs,
                None,
                Some(&bad),
                &Paths::default(),
                "/p",
                "/p/vendor"
            )
            .is_err()
        );
        let too_small = obj(json!({"phpcodesniffer-search-depth": 0}));
        assert!(
            generate(
                &refs,
                None,
                Some(&too_small),
                &Paths::default(),
                "/p",
                "/p/vendor"
            )
            .is_err()
        );
    }

    #[test]
    fn min_depth_follows_the_locked_phpcs_version() {
        let old = obj(
            json!({"name": "squizlabs/php_codesniffer", "version": "2.9.2", "type": "library"}),
        );
        assert_eq!(super::min_depth(&old), 1);
        let new = obj(
            json!({"name": "squizlabs/php_codesniffer", "version": "4.0.4", "type": "library"}),
        );
        assert_eq!(super::min_depth(&new), 0);
    }
}
