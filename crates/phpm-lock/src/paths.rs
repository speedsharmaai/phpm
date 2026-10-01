use phpm_php::is_absolute_path;
use regex::Regex;
use std::sync::LazyLock;

static PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i-u)^(?:[0-9a-z]{2,}:(?://(?:[a-z]:)?)?|[a-z]:)").expect("valid pattern")
});
static DRIVE_ROOT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i-u)^[A-Z]:/?$").expect("valid pattern"));

// Composer: Util/Filesystem.php normalizePath
pub fn normalize_path(path: &str) -> String {
    let mut path = path.replace('\\', "/");
    let mut absolute = "";
    if path.starts_with("//") && path.len() > 2 {
        absolute = "//";
        path = path[2..].to_owned();
    }
    let mut prefix = String::new();
    if let Some(m) = PREFIX.find(&path) {
        m.as_str().clone_into(&mut prefix);
        path = path[m.end()..].to_owned();
    }
    if let Some(rest) = path.strip_prefix('/') {
        absolute = "/";
        path = rest.to_owned();
    }

    let mut parts: Vec<&str> = Vec::new();
    let mut up = false;
    for chunk in path.split('/') {
        if chunk == ".." && (!absolute.is_empty() || up) {
            parts.pop();
            up = !(parts.is_empty() || parts.last() == Some(&".."));
        } else if chunk != "." && !chunk.is_empty() {
            parts.push(chunk);
            up = chunk != "..";
        }
    }

    let b = prefix.as_bytes();
    let n = b.len();
    if n >= 2
        && b[n - 1] == b':'
        && b[n - 2].is_ascii_alphabetic()
        && (n == 2 || prefix[..n - 2].ends_with("://"))
    {
        prefix = format!(
            "{}{}",
            &prefix[..n - 2],
            prefix[n - 2..].to_ascii_uppercase()
        );
    }
    format!("{prefix}{absolute}{}", parts.join("/"))
}

fn dirname(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.starts_with('/') {
            "/".to_owned()
        } else {
            ".".to_owned()
        };
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

fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed.rsplit('/').next().unwrap_or("")
}

/// `Filesystem::findShortestPath($from, $to, true)` for absolute paths;
/// `None` when either is relative (Composer throws).
// Composer: Util/Filesystem.php findShortestPath
pub fn find_shortest_path(from: &str, to: &str, directories: bool) -> Option<String> {
    find_shortest_path_with(from, to, directories, false)
}

/// `findShortestPath` with its `$preferRelative` argument.
pub fn find_shortest_path_with(
    from: &str,
    to: &str,
    directories: bool,
    prefer_relative: bool,
) -> Option<String> {
    if !is_absolute_path(from) || !is_absolute_path(to) {
        return None;
    }
    let mut from = normalize_path(from);
    let to = normalize_path(to);
    if directories {
        from = format!("{}/dummy_file", from.trim_end_matches('/'));
    }
    if dirname(&from) == dirname(&to) {
        return Some(format!("./{}", basename(&to)));
    }

    let mut common = to.clone();
    while !format!("{from}/").starts_with(&format!("{common}/"))
        && common != "/"
        && !DRIVE_ROOT.is_match(&common)
    {
        common = dirname(&common).replace('\\', "/");
    }
    if !from.starts_with(&common) {
        return Some(to);
    }

    let common = format!("{}/", common.trim_end_matches('/'));
    let depth = from.get(common.len()..).unwrap_or("").matches('/').count();
    if !prefer_relative && common == "/" && depth > 1 {
        return Some(to);
    }
    let result = format!(
        "{}{}",
        "../".repeat(depth),
        to.get(common.len()..).unwrap_or("")
    );
    Some(if result.is_empty() {
        "./".to_owned()
    } else {
        result
    })
}

#[cfg(test)]
mod tests {
    use super::{basename, dirname, find_shortest_path, normalize_path};

    #[test]
    fn normalizes_like_composer() {
        let cases = [
            ("../foo", "../foo"),
            ("c:\\foo\\bar", "C:/foo/bar"),
            ("C:\\foo\\bar", "C:/foo/bar"),
            ("/foo/../bar", "/bar"),
            ("/foo/./bar/", "/foo/bar"),
            ("foo/../../bar", "../bar"),
            ("../../foo", "../../foo"),
            ("/../foo", "/foo"),
            ("//server/share/x", "//server/share/x"),
            ("phar://c:/Foo", "phar://C:/Foo"),
            ("file:///a/../b", "file:///b"),
            ("a/./b//c", "a/b/c"),
            ("", ""),
        ];
        for (input, expected) in cases {
            assert_eq!(normalize_path(input), expected, "{input}");
        }
    }

    #[test]
    fn php_dirname_and_basename() {
        assert_eq!(dirname("/a/b/c"), "/a/b");
        assert_eq!(dirname("/a"), "/");
        assert_eq!(dirname("/"), "/");
        assert_eq!(dirname("a"), ".");
        assert_eq!(dirname(""), ".");
        assert_eq!(dirname("/a//b"), "/a");
        assert_eq!(basename("/a/b/"), "b");
    }

    #[test]
    fn prefers_relative_paths_when_asked() {
        assert_eq!(
            super::find_shortest_path_with("/foo/a/b", "/bar/c", false, true).as_deref(),
            Some("../../bar/c")
        );
        assert_eq!(
            super::find_shortest_path_with("/foo/a/b", "/bar/c", false, false).as_deref(),
            Some("/bar/c")
        );
    }

    #[test]
    fn shortest_paths_like_composer() {
        let cases = [
            (
                "/foo/vendor/composer",
                "/foo/vendor/monolog/monolog",
                "../monolog/monolog",
            ),
            ("/foo/vendor/composer", "/foo", "../../"),
            ("/foo/vendor/composer", "/foo/vendor/composer/x", "./x"),
            ("/foo/vendor/composer", "/foo/vendor/composer", "./"),
            ("/foo/vendor/composer", "/bar/baz", "/bar/baz"),
            ("/a/b", "/a/c", "../c"),
            ("C:/a/vendor/composer", "D:/x", "D:/x"),
        ];
        for (from, to, expected) in cases {
            assert_eq!(
                find_shortest_path(from, to, true).as_deref(),
                Some(expected),
                "{from} -> {to}"
            );
        }
        assert_eq!(
            find_shortest_path("/a/b", "/a/c", false).as_deref(),
            Some("./c")
        );
        assert_eq!(
            find_shortest_path("/foo/bar", "/foo/baz/x", false).as_deref(),
            Some("baz/x")
        );
        assert_eq!(find_shortest_path("rel", "/x", true), None);
    }
}
