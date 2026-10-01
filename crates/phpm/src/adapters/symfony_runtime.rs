//! `vendor/autoload_runtime.php` as symfony/runtime's plugin writes it.
// Composer: symfony/runtime Internal/ComposerPlugin.php updateAutoloadFile;
// behaviour identical every stable tag v7.0.0-v8.1.0 (checked: 4 content
// changes in that span, each a comment or a `\sprintf` namespace prefix).

use phpm_php::{PhpArray, PhpKey, PhpValue, var_export, var_export_str};
use serde_json::{Map, Value};

const TEMPLATE: &str = include_str!("../../vendor/symfony-runtime/autoload_runtime.template");
const DEFAULT_CLASS: &str = "Symfony\\Component\\Runtime\\SymfonyRuntime";

/// Where the project and its vendor directory are, both absolute.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Context<'a> {
    pub(crate) root: &'a str,
    pub(crate) vendor: &'a str,
}

/// `vendor/autoload_runtime.php`'s bytes. `Ok(None)` when `extra.runtime` is
/// literally `false` (the plugin does nothing this run, and an existing file
/// is left alone). `Err` when phpm cannot reproduce the input.
pub(crate) fn generate(
    root_extra: Option<&Map<String, Value>>,
    at: Context<'_>,
) -> Result<Option<Vec<u8>>, String> {
    let runtime = root_extra.and_then(|e| e.get("runtime"));
    if runtime == Some(&Value::Bool(false)) {
        return Ok(None);
    }
    let mut options = match runtime {
        None => PhpArray::new(),
        Some(v) => phpm_php::array_from_json(v).map_err(|e| format!("extra.runtime: {e}"))?,
    };

    let template = match options.shift_remove(&PhpKey::from("autoload_template")) {
        None => TEMPLATE.to_owned(),
        Some(PhpValue::String(rel)) => {
            let full = if phpm_php::is_absolute_path(&rel) {
                rel.clone()
            } else {
                format!("{}/{rel}", at.root)
            };
            std::fs::read_to_string(&full)
                .map_err(|_| format!("extra.runtime.autoload_template {rel} not found"))?
        }
        Some(_) => return Err("extra.runtime.autoload_template is not a string".into()),
    };

    let class = match options.shift_remove(&PhpKey::from("class")) {
        None => DEFAULT_CLASS.to_owned(),
        Some(PhpValue::String(s)) => s,
        Some(_) => return Err("extra.runtime.class is not a string".into()),
    };

    let project_dir = match options.shift_remove(&PhpKey::from("project_dir")) {
        None => at.root.to_owned(),
        Some(PhpValue::String(sub)) => std::fs::canonicalize(format!("{}/{sub}", at.root))
            .map(|p| phpm_lock::normalize_path(&p.to_string_lossy()))
            .map_err(|_| format!("extra.runtime.project_dir {sub} does not exist"))?,
        Some(_) => return Err("extra.runtime.project_dir is not a string".into()),
    };

    let relative = make_path_relative(&project_dir, at.vendor);
    let (nesting, remainder) = strip_nesting(&relative);
    let project_dir_code = project_dir_code(nesting, &remainder);

    // `substr(var_export($extra, true), 7, -1)`: strip the "array (" / ")"
    // shell var_export always wraps a top-level array in, so the options
    // keep their own place in `$extra` and `project_dir` is appended last.
    let exported = var_export(&PhpValue::Array(options));
    let inner = exported
        .strip_prefix("array (")
        .and_then(|s| s.strip_suffix(')'))
        .ok_or("var_export of extra.runtime did not have the expected shape")?;
    let runtime_options = format!("[{inner}  'project_dir' => {project_dir_code},\n]");

    let code = template
        .replacen("%project_dir%", &project_dir_code, 1)
        .replacen("%runtime_class%", &var_export_str(&class), 1)
        .replacen("%runtime_options%", &runtime_options, 1);
    Ok(Some(code.into_bytes()))
}

// Composer: symfony/filesystem Path-splitting half of ::makePathRelative
fn split_path(p: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for seg in p.trim_matches('/').split('/') {
        match seg {
            ".." => {
                out.pop();
            }
            "." | "" => {}
            s => out.push(s),
        }
    }
    out
}

// Composer: symfony/filesystem Filesystem::makePathRelative($endPath, $startPath)
fn make_path_relative(end_path: &str, start_path: &str) -> String {
    let end = split_path(end_path);
    let start = split_path(start_path);
    let mut index = 0;
    while index < start.len() && index < end.len() && start[index] == end[index] {
        index += 1;
    }
    let depth = start.len() - index;
    let traverser = "../".repeat(depth);
    let remainder = end[index..].join("/");
    let mut relative = if remainder.is_empty() {
        traverser
    } else {
        format!("{traverser}{remainder}/")
    };
    if relative.ends_with('/') && std::path::Path::new(end_path).is_file() {
        relative.pop();
    }
    if relative.is_empty() {
        "./".clone_into(&mut relative);
    }
    relative
}

/// Strips leading `../` segments, counting them.
fn strip_nesting(relative: &str) -> (usize, String) {
    let mut nesting = 0;
    let mut rest = relative;
    while let Some(stripped) = rest.strip_prefix("../") {
        nesting += 1;
        rest = stripped;
    }
    (nesting, rest.to_owned())
}

/// The `__DIR__`/`dirname(__DIR__, N)` expression `ComposerPlugin` writes
/// for the project directory, as PHP source.
fn project_dir_code(nesting: usize, remainder: &str) -> String {
    let suffix = || var_export_str(&format!("/{remainder}"));
    if nesting == 0 {
        // the hack about `__DIR__` is required because Composer pre-processes
        // plugins: this is literally `__DIR__ . '/<remainder>'`.
        format!("__DIR__ . {}", suffix())
    } else if remainder.is_empty() {
        format!("dirname(__DIR__, {nesting})")
    } else {
        format!("dirname(__DIR__, {nesting}) . {}", suffix())
    }
}

#[cfg(test)]
mod tests {
    use super::{Context, generate, make_path_relative, project_dir_code, strip_nesting};
    use serde_json::{Map, Value, json};

    fn obj(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => unreachable!(),
        }
    }

    #[test]
    fn make_path_relative_matches_symfony_filesystem() {
        assert_eq!(make_path_relative("/p", "/p/vendor"), "../");
        assert_eq!(make_path_relative("/p", "/p/some/vendor"), "../../");
        assert_eq!(make_path_relative("/p/sub", "/p/vendor"), "../sub/");
        assert_eq!(make_path_relative("/p/vendor", "/p/vendor"), "./");
        assert_eq!(make_path_relative("/a/b", "/a/c/d"), "../../b/");
    }

    #[test]
    fn project_dir_code_covers_every_nesting_shape() {
        assert_eq!(project_dir_code(0, ""), "__DIR__ . '/'");
        assert_eq!(project_dir_code(0, "./"), "__DIR__ . '/./'");
        assert_eq!(project_dir_code(1, ""), "dirname(__DIR__, 1)");
        assert_eq!(project_dir_code(1, "sub/"), "dirname(__DIR__, 1) . '/sub/'");
        assert_eq!(project_dir_code(2, ""), "dirname(__DIR__, 2)");
    }

    #[test]
    fn strips_leading_up_segments() {
        assert_eq!(strip_nesting("../"), (1, String::new()));
        assert_eq!(strip_nesting("../../"), (2, String::new()));
        assert_eq!(strip_nesting("../sub/"), (1, "sub/".into()));
        assert_eq!(strip_nesting("sub/"), (0, "sub/".into()));
    }

    #[test]
    fn default_options_match_the_common_case() {
        // the exact shape of a real `composer install` with no extra.runtime.
        let code = generate(
            None,
            Context {
                root: "/p",
                vendor: "/p/vendor",
            },
        )
        .unwrap()
        .unwrap();
        let code = String::from_utf8(code).unwrap();
        assert!(
            code.contains(
                "$_SERVER['APP_RUNTIME'] ??= $_ENV['APP_RUNTIME'] ?? 'Symfony\\\\Component\\\\Runtime\\\\SymfonyRuntime';"
            ),
            "{code}"
        );
        assert!(
            code.contains(
                "$_SERVER['APP_RUNTIME_OPTIONS'] += [\n  'project_dir' => dirname(__DIR__, 1),\n]);"
            ),
            "{code}"
        );
    }

    #[test]
    fn literal_false_skips_without_an_error() {
        let extra = obj(json!({"runtime": false}));
        assert_eq!(
            generate(
                Some(&extra),
                Context {
                    root: "/p",
                    vendor: "/p/vendor"
                }
            ),
            Ok(None)
        );
    }

    #[test]
    fn custom_class_and_extra_options_keep_their_order_before_project_dir() {
        let extra = obj(json!({"runtime": {"class": "App\\Kernel", "env": "prod", "debug": true}}));
        let code = generate(
            Some(&extra),
            Context {
                root: "/p",
                vendor: "/p/vendor",
            },
        )
        .unwrap()
        .unwrap();
        let code = String::from_utf8(code).unwrap();
        assert!(code.contains("?? 'App\\\\Kernel';"), "{code}");
        assert!(
            code.contains(
                "[\n  'env' => 'prod',\n  'debug' => true,\n  'project_dir' => dirname(__DIR__, 1),\n]"
            ),
            "{code}"
        );
    }

    #[test]
    fn project_dir_override_resolves_against_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let canon = dir.path().canonicalize().unwrap();
        let root = canon.to_string_lossy().replace('\\', "/");
        std::fs::create_dir(dir.path().join("public")).unwrap();
        let extra = obj(json!({"runtime": {"project_dir": "public"}}));
        let vendor = format!("{root}/vendor");
        let code = generate(
            Some(&extra),
            Context {
                root: &root,
                vendor: &vendor,
            },
        )
        .unwrap()
        .unwrap();
        let code = String::from_utf8(code).unwrap();
        assert!(
            code.contains("'project_dir' => dirname(__DIR__, 1) . '/public/',"),
            "public/ is a sibling of vendor, one level up then down into it: {code}"
        );
        let missing = obj(json!({"runtime": {"project_dir": "nope"}}));
        assert!(
            generate(
                Some(&missing),
                Context {
                    root: &root,
                    vendor: &vendor
                }
            )
            .unwrap_err()
            .contains("nope")
        );
    }

    #[test]
    fn custom_autoload_template_is_read_and_substituted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().replace('\\', "/");
        std::fs::write(
            dir.path().join("my.template"),
            "<?php // %runtime_class% %project_dir%\n",
        )
        .unwrap();
        let extra = obj(json!({"runtime": {"autoload_template": "my.template"}}));
        let vendor = format!("{root}/vendor");
        let code = generate(
            Some(&extra),
            Context {
                root: &root,
                vendor: &vendor,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            String::from_utf8(code).unwrap(),
            "<?php // 'Symfony\\\\Component\\\\Runtime\\\\SymfonyRuntime' dirname(__DIR__, 1)\n"
        );
        let missing = obj(json!({"runtime": {"autoload_template": "nope.template"}}));
        assert!(
            generate(
                Some(&missing),
                Context {
                    root: &root,
                    vendor: &vendor
                }
            )
            .unwrap_err()
            .contains("nope.template")
        );
    }

    #[test]
    fn declines_what_it_cannot_represent() {
        let not_object = obj(json!({"runtime": "x"}));
        assert!(
            generate(
                Some(&not_object),
                Context {
                    root: "/p",
                    vendor: "/p/vendor"
                }
            )
            .is_err()
        );
        let bad_class = obj(json!({"runtime": {"class": 1}}));
        assert!(
            generate(
                Some(&bad_class),
                Context {
                    root: "/p",
                    vendor: "/p/vendor"
                }
            )
            .is_err()
        );
        let bad_template = obj(json!({"runtime": {"autoload_template": 1}}));
        assert!(
            generate(
                Some(&bad_template),
                Context {
                    root: "/p",
                    vendor: "/p/vendor"
                }
            )
            .is_err()
        );
        let bad_project = obj(json!({"runtime": {"project_dir": 1}}));
        assert!(
            generate(
                Some(&bad_project),
                Context {
                    root: "/p",
                    vendor: "/p/vendor"
                }
            )
            .is_err()
        );
        let float = obj(json!({"runtime": {"x": 1.5}}));
        assert!(
            generate(
                Some(&float),
                Context {
                    root: "/p",
                    vendor: "/p/vendor"
                }
            )
            .is_err()
        );
    }
}
