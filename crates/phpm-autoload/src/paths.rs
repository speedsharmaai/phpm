use phpm_lock::{find_shortest_path, normalize_path};
use phpm_php::{is_absolute_path, var_export_str};
use std::path::Path;

/// A PHP path expression and the string it evaluates to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Code {
    pub(crate) code: String,
    pub(crate) value: String,
}

/// PHP `dirname` on a normalised path.
pub(crate) fn dirname(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.starts_with('/') { "/" } else { "." }.to_owned();
    }
    match trimmed.rfind('/') {
        None => ".".to_owned(),
        Some(i) => {
            let parent = trimmed[..i].trim_end_matches('/');
            if parent.is_empty() {
                "/".to_owned()
            } else {
                parent.to_owned()
            }
        }
    }
}

fn is_drive_root(path: &str) -> bool {
    let b = path.as_bytes();
    (b.len() == 2 || (b.len() == 3 && b[2] == b'/')) && b[0].is_ascii_alphabetic() && b[1] == b':'
}

fn literal(to: String) -> Code {
    Code {
        code: var_export_str(&to),
        value: to,
    }
}

// Composer: Util/Filesystem.php findShortestPathCode
pub(crate) fn find_shortest_path_code(
    from: &str,
    to: &str,
    directories: bool,
    static_code: bool,
) -> Code {
    let from = normalize_path(from);
    let to = normalize_path(to);
    if from == to {
        return Code {
            code: if directories { "__DIR__" } else { "__FILE__" }.to_owned(),
            value: to,
        };
    }
    let mut common = to.clone();
    while !format!("{from}/").starts_with(&format!("{common}/"))
        && common != "/"
        && !is_drive_root(&common)
        && common != "."
    {
        common = dirname(&common).replace('\\', "/");
    }
    if !from.starts_with(&common) || common == "." {
        return literal(to);
    }
    let common = format!("{}/", common.trim_end_matches('/'));
    if let Some(sub) = to.strip_prefix(&format!("{from}/")) {
        return Code {
            code: format!("__DIR__ . {}", var_export_str(&format!("/{sub}"))),
            value: to,
        };
    }
    let depth =
        from.get(common.len()..).unwrap_or("").matches('/').count() + usize::from(directories);
    if common == "/" && depth > 1 {
        return literal(to);
    }
    let mut code = if static_code {
        format!("__DIR__ . '{}'", "/..".repeat(depth))
    } else {
        format!("{}__DIR__{}", "dirname(".repeat(depth), ")".repeat(depth))
    };
    let mut value = from;
    for _ in 0..depth {
        value = dirname(&value);
    }
    let rel = to.get(common.len()..).unwrap_or("");
    if !rel.is_empty() {
        code.push('.');
        code.push_str(&var_export_str(&format!("/{rel}")));
        value = format!("{value}/{rel}");
    }
    Code { code, value }
}

fn is_phar(path: &str) -> bool {
    path.match_indices(".phar")
        .any(|(i, m)| matches!(path.as_bytes().get(i + m.len()), None | Some(b'/' | b'\\')))
}

/// The project and vendor directories every autoload path is written
/// relative to, both real paths with forward slashes.
#[derive(Debug, Clone)]
pub(crate) struct Dirs {
    pub(crate) base: String,
    pub(crate) vendor: String,
}

impl Dirs {
    // Composer: Autoload/AutoloadGenerator.php getPathCode
    pub(crate) fn path_code(&self, path: &str) -> Code {
        let path = if is_absolute_path(path) {
            normalize_path(path)
        } else {
            normalize_path(&format!("{}/{path}", self.base))
        };
        let (prefix, rel, value) = if format!("{path}/").starts_with(&format!("{}/", self.vendor)) {
            let rel = path[self.vendor.len()..].to_owned();
            let value = format!("{}{rel}", self.vendor);
            ("$vendorDir . ", rel, value)
        } else {
            let shortest = find_shortest_path(&self.base, &path, true).unwrap_or(path);
            let rel = normalize_path(&shortest);
            if is_absolute_path(&rel) {
                ("", rel.clone(), rel)
            } else {
                let rel = format!("/{rel}");
                let value = format!("{}{rel}", self.base);
                ("$baseDir . ", rel, value)
            }
        };
        let mut code = format!("{prefix}{}", var_export_str(&rel));
        let mut value = value;
        if is_phar(&rel) {
            code = format!("'phar://' . {code}");
            value = format!("phar://{value}");
        }
        Code { code, value }
    }
}

/// `realpath()` with forward slashes, `None` when the path does not exist.
pub(crate) fn real_path(path: &Path) -> Option<String> {
    let real = std::fs::canonicalize(path).ok()?;
    let s = real.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    Some(s.replace('\\', "/"))
}

/// PHP `preg_quote` without a delimiter.
pub(crate) fn preg_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '.' | '\\' | '+' | '*' | '?' | '[' | '^' | ']' | '$' | '(' | ')' | '{' | '}' | '='
            | '!' | '<' | '>' | '|' | ':' | '-' | '#' => {
                out.push('\\');
                out.push(c);
            }
            '\0' => out.push_str("\\000"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Dirs, dirname, find_shortest_path_code, is_phar, preg_quote, real_path};

    #[test]
    fn php_dirname() {
        assert_eq!(dirname("/a/b/c"), "/a/b");
        assert_eq!(dirname("/a"), "/");
        assert_eq!(dirname("/"), "/");
        assert_eq!(dirname("a"), ".");
        assert_eq!(dirname("a//b/"), "a");
    }

    #[test]
    fn shortest_path_code_like_composer() {
        let cases = [
            (
                "/p/vendor/composer",
                "/p/vendor",
                true,
                false,
                "dirname(__DIR__)",
            ),
            ("/p/vendor", "/p", true, false, "dirname(__DIR__)"),
            (
                "/p/lib/deps",
                "/p",
                true,
                false,
                "dirname(dirname(__DIR__))",
            ),
            (
                "/p/vendor",
                "/p/vendor/composer",
                true,
                false,
                "__DIR__ . '/composer'",
            ),
            ("/p/vendor", "/p/vendor", true, false, "__DIR__"),
            ("/p/vendor", "/p/vendor", false, false, "__FILE__"),
            (
                "/p/vendor/composer",
                "/p/vendor",
                true,
                true,
                "__DIR__ . '/..'",
            ),
            ("/p/vendor/composer", "/p", true, true, "__DIR__ . '/../..'"),
            ("/p/vendor", "/q/x", true, false, "'/q/x'"),
            (
                "/p/vendor",
                "/p/src",
                true,
                false,
                "dirname(__DIR__).'/src'",
            ),
            ("C:/p/vendor", "D:/x", true, false, "'D:/x'"),
        ];
        for (from, to, dirs, stat, expected) in cases {
            assert_eq!(
                find_shortest_path_code(from, to, dirs, stat).code,
                expected,
                "{from} -> {to}"
            );
        }
        assert_eq!(
            find_shortest_path_code("/p/lib/deps", "/p", true, false).value,
            "/p"
        );
        assert_eq!(
            find_shortest_path_code("/p/vendor", "/p/src", true, false).value,
            "/p/src"
        );
    }

    #[test]
    fn path_codes_like_composer() {
        let dirs = Dirs {
            base: "/p".into(),
            vendor: "/p/vendor".into(),
        };
        let code = |p: &str| dirs.path_code(p);
        assert_eq!(code("/p/vendor/a/b/src").code, "$vendorDir . '/a/b/src'");
        assert_eq!(code("/p/vendor/a/b/src").value, "/p/vendor/a/b/src");
        assert_eq!(code("app/").code, "$baseDir . '/app'");
        assert_eq!(code("app/").value, "/p/app");
        assert_eq!(code(".").code, "$baseDir . '/'");
        assert_eq!(code("../other").code, "$baseDir . '/../other'");
        assert_eq!(code("/elsewhere/x").code, "$baseDir . '/../elsewhere/x'");
        let deep = Dirs {
            base: "/p/q".into(),
            vendor: "/p/q/vendor".into(),
        };
        assert_eq!(deep.path_code("/elsewhere/x").code, "'/elsewhere/x'");
        assert_eq!(deep.path_code("/elsewhere/x").value, "/elsewhere/x");
        assert_eq!(code("/p/vendor").code, "$vendorDir . ''");
        let phar = code("/p/vendor/a/b/lib.phar/x.php");
        assert_eq!(phar.code, "'phar://' . $vendorDir . '/a/b/lib.phar/x.php'");
        assert_eq!(phar.value, "phar:///p/vendor/a/b/lib.phar/x.php");
        assert!(!is_phar("/a/lib.phard"));
        assert!(is_phar("/a/x.phar"));
    }

    #[test]
    fn quotes_like_preg_quote() {
        assert_eq!(preg_quote("/a.b-c/d#e"), "/a\\.b\\-c/d\\#e");
        assert_eq!(preg_quote("x\0"), "x\\000");
    }

    #[test]
    fn real_paths_use_forward_slashes() {
        let dir = tempfile::tempdir().unwrap();
        let real = real_path(dir.path()).unwrap();
        assert!(!real.contains('\\'));
        assert!(real_path(&dir.path().join("missing")).is_none());
    }
}
