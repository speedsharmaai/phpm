//! `vendor/bin` proxies, the way Composer 2.10's `BinaryInstaller` writes them.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use phpm_lock::{find_shortest_path, normalize_path};
use phpm_php::var_export_str;

use crate::fsutil::{self, Modes};

/// PHP `dirname` on a normalised path.
pub(crate) fn php_dirname(path: &str) -> String {
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

/// PHP `basename` with `/` as the only separator.
pub(crate) fn php_basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed.rsplit('/').next().unwrap_or("")
}

// Composer: Util/ProcessExecutor.php escapeArgument (non-Windows)
fn shell_escape(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\\''"))
}

/// `ProcessExecutor::escape`, the branch Composer takes when it is itself
/// running on Windows: cmd.exe quoting, not POSIX, even though the string
/// this crate uses it for ends up inside a shell-script bin proxy meant for
/// a POSIX shell (git-bash, WSL) — Composer picks the escaping by its own
/// host OS, not the target shell, so matching it byte for byte means doing
/// the same.
// Composer: Util/ProcessExecutor.php escapeArgument (Windows)
#[cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "only reachable through host_shell_escape on windows; tested on every platform"
    )
)]
fn windows_shell_escape(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".to_owned();
    }
    let confusable = |c: char| match c {
        '\n' => Some(' '),
        '\u{ff02}' | '\u{02ba}' | '\u{301d}' | '\u{301e}' | '\u{030e}' => Some('"'),
        '\u{ff1a}' | '\u{0589}' | '\u{2236}' => Some(':'),
        '\u{ff0f}' | '\u{2044}' | '\u{2215}' | '\u{00b4}' => Some('/'),
        _ => None,
    };
    let normalized: String = arg.chars().map(|c| confusable(c).unwrap_or(c)).collect();
    let mut quote = normalized.contains([' ', '\t', ',']);

    // Double every backslash run immediately before a '"', then escape the
    // '"' itself; count how many quotes that touched.
    let mut escaped = String::with_capacity(normalized.len());
    let mut trailing_backslashes = 0u32;
    let mut dquotes = 0u32;
    for c in normalized.chars() {
        if c == '\\' {
            trailing_backslashes += 1;
            escaped.push('\\');
        } else if c == '"' {
            for _ in 0..trailing_backslashes {
                escaped.push('\\');
            }
            escaped.push_str("\\\"");
            dquotes += 1;
            trailing_backslashes = 0;
        } else {
            escaped.push(c);
            trailing_backslashes = 0;
        }
    }

    let meta = dquotes > 0 || has_delimited_run(&escaped, '%') || has_delimited_run(&escaped, '!');
    if !meta && !quote {
        quote = escaped.contains(['^', '&', '|', '<', '>', '(', ')']);
    }
    if quote {
        // Double a trailing backslash run so it does not escape the
        // closing quote, then wrap; a '^' from the meta pass below still
        // lands in front of each of these two added quotes too.
        escaped.push('"');
        for _ in 0..trailing_backslashes {
            escaped.insert(escaped.len() - 1, '\\');
        }
        escaped.insert(0, '"');
    }
    if meta {
        let mut out = String::with_capacity(escaped.len() * 2);
        for c in escaped.chars() {
            if matches!(c, '"' | '^' | '&' | '|' | '<' | '>' | '(' | ')' | '%') {
                out.push('^');
            }
            out.push(c);
        }
        let mut doubled = String::with_capacity(out.len() * 2);
        for c in out.chars() {
            if c == '!' {
                doubled.push('^');
                doubled.push('^');
            }
            doubled.push(c);
        }
        escaped = doubled;
    }
    escaped
}

/// Whether `s` contains `delim`, at least one other character, then `delim`
/// again (Composer: `%[^%]+%` or `![^!]+!`).
#[cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "only reachable through windows_shell_escape on windows"
    )
)]
fn has_delimited_run(s: &str, delim: char) -> bool {
    let mut open: Option<usize> = None;
    for (i, c) in s.char_indices() {
        if c != delim {
            continue;
        }
        if let Some(start) = open
            && i > start + delim.len_utf8()
        {
            return true;
        }
        open = Some(i);
    }
    false
}

#[cfg(windows)]
fn host_shell_escape(arg: &str) -> String {
    windows_shell_escape(arg)
}

#[cfg(not(windows))]
fn host_shell_escape(arg: &str) -> String {
    shell_escape(arg)
}

fn is_drive_root(path: &str) -> bool {
    let b = path.as_bytes();
    (b.len() == 2 || (b.len() == 3 && b[2] == b'/')) && b[0].is_ascii_alphabetic() && b[1] == b':'
}

// Composer: Util/Filesystem.php findShortestPathCode, $directories = false, $staticCode = true
fn shortest_path_code(from: &str, to: &str) -> String {
    let from = normalize_path(from);
    let to = normalize_path(to);
    if from == to {
        return "__FILE__".to_owned();
    }
    let mut common = to.clone();
    while !format!("{from}/").starts_with(&format!("{common}/"))
        && common != "/"
        && !is_drive_root(&common)
        && common != "."
    {
        common = php_dirname(&common).replace('\\', "/");
    }
    if !from.starts_with(&common) || common == "." {
        return var_export_str(&to);
    }
    let common = format!("{}/", common.trim_end_matches('/'));
    if let Some(sub) = to.strip_prefix(&format!("{from}/")) {
        return format!("__DIR__ . {}", var_export_str(&format!("/{sub}")));
    }
    let depth = from.get(common.len()..).unwrap_or("").matches('/').count();
    if common == "/" && depth > 1 {
        return var_export_str(&to);
    }
    let mut code = format!("__DIR__ . '{}'", "/..".repeat(depth));
    let rel = to.get(common.len()..).unwrap_or("");
    if !rel.is_empty() {
        code.push('.');
        code.push_str(&var_export_str(&format!("/{rel}")));
    }
    code
}

/// What `{^(#!.*\r?\n)?[\r\n\t ]*<\?php}` makes of the start of a bin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BinKind<'a> {
    Shell,
    Php { shebang: Option<&'a [u8]> },
}

fn bin_kind(head: &[u8]) -> BinKind<'_> {
    if head.starts_with(b"#!")
        && let Some(nl) = head.iter().position(|&b| b == b'\n')
        && starts_php(&head[nl + 1..])
    {
        return BinKind::Php {
            shebang: Some(&head[..=nl]),
        };
    }
    if starts_php(head) {
        BinKind::Php { shebang: None }
    } else {
        BinKind::Shell
    }
}

fn starts_php(bytes: &[u8]) -> bool {
    let start = bytes
        .iter()
        .position(|b| !matches!(b, b'\r' | b'\n' | b'\t' | b' '))
        .unwrap_or(bytes.len());
    bytes[start..].starts_with(b"<?php")
}

// PHP trim's default character list
fn php_trim(bytes: &[u8]) -> &[u8] {
    let ws = |b: &u8| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0 | 0x0b);
    let start = bytes.iter().position(|b| !ws(b)).unwrap_or(bytes.len());
    let end = bytes.iter().rposition(|b| !ws(b)).map_or(start, |i| i + 1);
    &bytes[start..end.max(start)]
}

const STREAM_HINT: &str =
    " using a stream wrapper to prevent the shebang from being output on PHP<8\n *";

const PHPUNIT_HACK_2: &str = "
                $data = str_replace('__DIR__', var_export(dirname($this->realpath), true), $data);
                $data = str_replace('__FILE__', var_export($this->realpath, true), $data);";

fn stream_proxy(hack1: &str, hack2: &str, bin_path_exported: &str) -> String {
    format!(
        r#"if (PHP_VERSION_ID < 80000) {{
    if (!class_exists('Composer\BinProxyWrapper')) {{
        /**
         * @internal
         */
        final class BinProxyWrapper
        {{
            private $handle;
            private $position;
            private $realpath;

            public function stream_open($path, $mode, $options, &$opened_path)
            {{
                // get rid of phpvfscomposer:// prefix for __FILE__ & __DIR__ resolution
                $opened_path = substr($path, 17);
                $this->realpath = realpath($opened_path) ?: $opened_path;
                $opened_path = {hack1}$this->realpath;
                $this->handle = fopen($this->realpath, $mode);
                $this->position = 0;

                return (bool) $this->handle;
            }}

            public function stream_read($count)
            {{
                $data = fread($this->handle, $count);

                if ($this->position === 0) {{
                    $data = preg_replace('{{^#!.*\r?\n}}', '', $data);
                }}{hack2}

                $this->position += strlen($data);

                return $data;
            }}

            public function stream_cast($castAs)
            {{
                return $this->handle;
            }}

            public function stream_close()
            {{
                fclose($this->handle);
            }}

            public function stream_lock($operation)
            {{
                return $operation ? flock($this->handle, $operation) : true;
            }}

            public function stream_seek($offset, $whence)
            {{
                if (0 === fseek($this->handle, $offset, $whence)) {{
                    $this->position = ftell($this->handle);
                    return true;
                }}

                return false;
            }}

            public function stream_tell()
            {{
                return $this->position;
            }}

            public function stream_eof()
            {{
                return feof($this->handle);
            }}

            public function stream_stat()
            {{
                return array();
            }}

            public function stream_set_option($option, $arg1, $arg2)
            {{
                return true;
            }}

            public function url_stat($path, $flags)
            {{
                $path = substr($path, 17);
                if (file_exists($path)) {{
                    return stat($path);
                }}

                return false;
            }}
        }}
    }}

    if (
        (function_exists('stream_get_wrappers') && in_array('phpvfscomposer', stream_get_wrappers(), true))
        || (function_exists('stream_wrapper_register') && stream_wrapper_register('phpvfscomposer', 'Composer\BinProxyWrapper'))
    ) {{
        return include("phpvfscomposer://" . {bin_path_exported});
    }}
}}
"#
    )
}

fn sh_proxy(bin_dir: &str, bin_file: &str) -> String {
    format!(
        r#"#!/usr/bin/env sh

# Support bash to support `source` with fallback on $0 if this does not run with bash
# https://stackoverflow.com/a/35006505/6512
selfArg="$BASH_SOURCE"
if [ -z "$selfArg" ]; then
    selfArg="$0"
fi

self=$(realpath "$selfArg" 2> /dev/null)
if [ -z "$self" ]; then
    self="$selfArg"
fi

dir=$(cd "${{self%[/\\]*}}" > /dev/null; cd {bin_dir} && pwd)

if [ -d /proc/cygdrive ]; then
    case $(which php) in
        $(readlink -n /proc/cygdrive)/*)
            # We are in Cygwin using Windows php, so the path must be translated
            dir=$(cygpath -m "$dir");
            ;;
    esac
fi

export COMPOSER_RUNTIME_BIN_DIR="$(cd "${{self%[/\\]*}}" > /dev/null; pwd)"

# If bash is sourcing this file, we have to source the target as well
bashSource="$BASH_SOURCE"
if [ -n "$bashSource" ]; then
    if [ "$bashSource" != "$0" ]; then
        source "${{dir}}/{bin_file}" "$@"
        return
    fi
fi

exec "${{dir}}/{bin_file}" "$@"
"#
    )
}

/// The `vendor/bin/<name>` proxy for `bin`, both absolute paths; `head` is
/// the first 500 bytes of the bin. `vendor_dir` is the configured vendor
/// directory and `vendor_dir_real` its realpath.
// Composer: Installer/BinaryInstaller.php generateUnixyProxyCode
pub(crate) fn unixy_proxy(
    bin: &str,
    link: &str,
    vendor_dir: &str,
    vendor_dir_real: &str,
    head: &[u8],
) -> String {
    let bin_path = find_shortest_path(link, bin, false).unwrap_or_else(|| bin.to_owned());
    let BinKind::Php { shebang } = bin_kind(head) else {
        return sh_proxy(
            &host_shell_escape(&php_dirname(&bin_path)),
            php_basename(&bin_path),
        );
    };
    let proxy_line = shebang.map_or_else(
        || "#!/usr/bin/env php".to_owned(),
        |line| String::from_utf8_lossy(php_trim(line)).into_owned(),
    );
    let exported = shortest_path_code(link, bin);
    let mut globals = "$GLOBALS['_composer_bin_dir'] = __DIR__;\n".to_owned();
    let _ = writeln!(
        globals,
        "$GLOBALS['_composer_autoload_path'] = {};",
        shortest_path_code(link, &format!("{vendor_dir_real}/autoload.php"))
    );
    let (mut hack1, mut hack2) = ("", "");
    if normalize_path(bin) == normalize_path(&format!("{vendor_dir}/phpunit/phpunit/phpunit")) {
        let _ = writeln!(
            globals,
            "$GLOBALS['__PHPUNIT_ISOLATION_EXCLUDE_LIST'] = $GLOBALS['__PHPUNIT_ISOLATION_BLACKLIST'] = array(realpath({exported}));"
        );
        hack1 = "'phpvfscomposer://'.";
        hack2 = PHPUNIT_HACK_2;
    }
    let (hint, stream) = if shebang.is_some() {
        (STREAM_HINT, stream_proxy(hack1, hack2, &exported))
    } else {
        ("", String::new())
    };
    format!(
        "{proxy_line}\n<?php\n\n/**\n * Proxy PHP file generated by Composer\n *\n * This file includes the referenced bin path ({bin_path})\n *{hint}\n * @generated\n */\n\nnamespace Composer;\n\n{globals}\n{stream}\nreturn include {exported};\n"
    )
}

// Composer: Installer/BinaryInstaller.php determineBinaryCaller
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "Composer's substr() check is case-sensitive"
)]
fn binary_caller(bin: &str, head: &[u8]) -> String {
    if bin.ends_with(".bat") || bin.ends_with(".exe") {
        return "call".to_owned();
    }
    let line = head.split(|&b| b == b'\n').next().unwrap_or_default();
    let Some(rest) = line.strip_prefix(b"#!/") else {
        return "php".to_owned();
    };
    let target = rest
        .strip_prefix(b"usr/bin/env ")
        .and_then(last_segment)
        .or_else(|| last_segment(rest));
    match target {
        Some(t) => String::from_utf8_lossy(php_trim(t)).into_owned(),
        None => "php".to_owned(),
    }
}

/// `(?:[^/]+/)*(.+)$` on one line: what is left after the most leading
/// `segment/` parts that still leave something behind.
fn last_segment(rest: &[u8]) -> Option<&[u8]> {
    let mut cut = 0;
    let mut best = 0;
    while let Some(i) = rest[cut..].iter().position(|&b| b == b'/') {
        if i == 0 {
            break;
        }
        cut += i + 1;
        if cut < rest.len() {
            best = cut;
        }
    }
    (best < rest.len()).then(|| &rest[best..])
}

/// The `.bat` proxy written next to the unixy one when `bin-compat` is `full`.
// Composer: Installer/BinaryInstaller.php generateWindowsProxyCode
pub(crate) fn windows_proxy(bin: &str, link: &str, head: &[u8]) -> String {
    let caller = binary_caller(bin, head);
    let target = if caller == "php" {
        let name = php_basename(link);
        name.strip_suffix(".bat").unwrap_or(name).to_owned()
    } else {
        find_shortest_path(link, bin, false).unwrap_or_else(|| bin.to_owned())
    };
    let target = shell_escape(&target);
    let target = target.trim_matches(['"', '\'']);
    format!(
        "@ECHO OFF\r\nsetlocal DISABLEDELAYEDEXPANSION\r\nSET BIN_TARGET=%~dp0/{target}\r\nSET COMPOSER_RUNTIME_BIN_DIR=%~dp0\r\n{caller} \"%BIN_TARGET%\" %*\r\n"
    )
}

/// Where proxies go and how they are written.
#[derive(Debug)]
pub(crate) struct BinInstaller {
    pub(crate) bin_dir: PathBuf,
    pub(crate) vendor_dir: String,
    pub(crate) vendor_dir_real: String,
    pub(crate) full_compat: bool,
    pub(crate) modes: Modes,
    bin_dir_real: Option<String>,
    written: BTreeSet<String>,
    pub(crate) warnings: Vec<String>,
}

impl BinInstaller {
    pub(crate) fn new(
        bin_dir: PathBuf,
        vendor_dir: String,
        vendor_dir_real: String,
        full_compat: bool,
        modes: Modes,
    ) -> Self {
        Self {
            bin_dir,
            vendor_dir,
            vendor_dir_real,
            full_compat,
            modes,
            bin_dir_real: None,
            written: BTreeSet::new(),
            warnings: Vec::new(),
        }
    }

    /// Proxy names written so far, `name` and `name.bat`.
    pub(crate) fn written(&self) -> &BTreeSet<String> {
        &self.written
    }

    fn bin_dir_real(&mut self) -> io::Result<String> {
        if let Some(dir) = &self.bin_dir_real {
            return Ok(dir.clone());
        }
        fs::create_dir_all(&self.bin_dir)?;
        let real = fsutil::path_string(&fsutil::canonical(&self.bin_dir)?);
        self.bin_dir_real = Some(real.clone());
        Ok(real)
    }

    /// Proxies for one package's `bin` entries.
    // Composer: Installer/BinaryInstaller.php installBinaries
    #[expect(
        clippy::case_sensitive_file_extension_comparisons,
        reason = "Composer's substr() check is case-sensitive"
    )]
    pub(crate) fn install(
        &mut self,
        package: &str,
        install_path: &str,
        bins: &[String],
        unchanged: bool,
    ) -> io::Result<()> {
        for bin in bins {
            let bin_path = format!("{install_path}/{bin}");
            let skip = |reason: &str| {
                format!("Skipped installation of bin {bin} for package {package}: {reason}")
            };
            let Ok(meta) = fs::metadata(&bin_path) else {
                self.warnings.push(skip("file not found in package"));
                continue;
            };
            if meta.is_dir() {
                self.warnings.push(skip("found a directory at that path"));
                continue;
            }
            let Some(real_bin) = inside_package(Path::new(install_path), Path::new(&bin_path))
            else {
                self.warnings.push(skip(
                    "the bin resolves to a path outside of the package directory",
                ));
                continue;
            };
            let name = php_basename(bin).to_owned();
            if self.written.contains(&name) {
                self.warnings
                    .push(skip("name conflicts with an existing file"));
                continue;
            }
            let link = format!("{}/{name}", self.bin_dir_real()?);
            // Composer only touches a proxy for a package an operation
            // actually (re)installs; one it leaves alone keeps whatever is
            // already at `link`, non-symlink or not, byte for byte.
            // Installer/BinaryInstaller.php installBinaries, ensureBinariesPresence
            if unchanged && fs::symlink_metadata(&link).is_ok() {
                self.written.insert(name.clone());
                let bat = format!("{link}.bat");
                if fs::symlink_metadata(&bat).is_ok() {
                    self.written.insert(format!("{name}.bat"));
                }
                continue;
            }
            let head = fsutil::read_head(Path::new(&bin_path), 500)?;
            let proxy = unixy_proxy(
                &bin_path,
                &link,
                &self.vendor_dir,
                &self.vendor_dir_real,
                &head,
            );
            fsutil::write_if_changed(Path::new(&link), proxy.as_bytes())?;
            self.modes.set_exec(Path::new(&link))?;
            self.written.insert(name.clone());
            if self.full_compat && !bin_path.ends_with(".bat") {
                let bat = format!("{link}.bat");
                fsutil::write_if_changed(
                    Path::new(&bat),
                    windows_proxy(&bin_path, &bat, &head).as_bytes(),
                )?;
                self.modes.set_exec(Path::new(&bat))?;
                self.written.insert(format!("{name}.bat"));
            }
            self.modes.set_exec_unshared(&real_bin)?;
        }
        Ok(())
    }

    /// Remove proxies named in `names` that this run did not write, then
    /// the bin directory if that left it empty.
    // Composer: Installer/BinaryInstaller.php removeBinaries
    pub(crate) fn remove_stale<'a>(
        &mut self,
        names: impl IntoIterator<Item = &'a str>,
    ) -> io::Result<()> {
        let mut removed = false;
        for name in names {
            for file in [name.to_owned(), format!("{name}.bat")] {
                if self.written.contains(&file) {
                    continue;
                }
                let path = self.bin_dir.join(&file);
                if fs::symlink_metadata(&path).is_ok() {
                    fs::remove_file(&path)?;
                    removed = true;
                }
            }
        }
        if removed && fs::read_dir(&self.bin_dir).is_ok_and(|mut entries| entries.next().is_none())
        {
            fs::remove_dir(&self.bin_dir)?;
        }
        Ok(())
    }
}

/// The bin's real path, if it resolves inside the package's real path.
// Composer: Installer/BinaryInstaller.php isBinPathInsidePackage
fn inside_package(install_path: &Path, bin_path: &Path) -> Option<PathBuf> {
    let bin = fsutil::canonical(bin_path).ok()?;
    let package = fsutil::canonical(install_path).ok()?;
    (bin != package && bin.starts_with(&package)).then_some(bin)
}

#[cfg(test)]
mod tests {
    use super::{
        BinKind, bin_kind, binary_caller, php_basename, php_dirname, shortest_path_code,
        unixy_proxy, windows_proxy, windows_shell_escape,
    };

    #[test]
    fn escapes_arguments_like_composer_does_on_windows() {
        // Composer: Util/ProcessExecutor.php escapeArgument, the Windows
        // branch; cases traced by hand against that algorithm.
        assert_eq!(windows_shell_escape(""), "\"\"");
        // No space, tab, comma or meta char: left bare, unlike the POSIX
        // branch which always wraps in single quotes (issue #67).
        assert_eq!(
            windows_shell_escape("C:/project/vendor/bin"),
            "C:/project/vendor/bin"
        );
        assert_eq!(
            windows_shell_escape("C:/path with space"),
            "\"C:/path with space\""
        );
        // A trailing backslash run is doubled so it cannot escape the
        // closing quote that quoting (triggered here by the space) adds.
        assert_eq!(windows_shell_escape("C:\\a b\\"), "\"C:\\a b\\\\\"");
        // An embedded '"' is escaped and counts as a meta character; with
        // no space/tab/comma present this stays unquoted.
        assert_eq!(windows_shell_escape("C:/a\"b"), "C:/a\\^\"b");
        // A %...% block is a meta character too, caret-escaped, unquoted.
        assert_eq!(windows_shell_escape("C:/%temp%/x"), "C:/^%temp^%/x");
        // '!' gets a double caret, independently of the single-caret set.
        assert_eq!(windows_shell_escape("C:/%a%!b"), "C:/^%a^%^^!b");
        // ^&|<>() alone (no meta, no space) still forces quoting.
        assert_eq!(windows_shell_escape("a&b"), "\"a&b\"");
    }

    #[test]
    fn detects_php_bins_like_composer() {
        let plain = BinKind::Php { shebang: None };
        assert_eq!(bin_kind(b"<?php\n"), plain);
        assert_eq!(bin_kind(b"\r\n\t <?php"), plain);
        assert_eq!(
            bin_kind(b"#!/usr/bin/env php\n<?php\n"),
            BinKind::Php {
                shebang: Some(b"#!/usr/bin/env php\n")
            }
        );
        assert_eq!(
            bin_kind(b"#!/usr/bin/php\r\n\n<?php"),
            BinKind::Php {
                shebang: Some(b"#!/usr/bin/php\r\n")
            }
        );
        assert_eq!(bin_kind(b"#!/bin/sh\necho <?php"), BinKind::Shell);
        assert_eq!(bin_kind(b"#!/usr/bin/env php"), BinKind::Shell);
        assert_eq!(bin_kind(b"<?ph"), BinKind::Shell);
        assert_eq!(bin_kind(b""), BinKind::Shell);
    }

    #[test]
    fn php_path_helpers() {
        assert_eq!(php_dirname("../a/b"), "../a");
        assert_eq!(php_dirname("b"), ".");
        assert_eq!(php_dirname("/b"), "/");
        assert_eq!(php_dirname("/"), "/");
        assert_eq!(php_dirname(""), ".");
        assert_eq!(php_basename("bin/tool/"), "tool");
        assert_eq!(php_basename("tool"), "tool");
    }

    #[test]
    fn exports_paths_relative_to_the_proxy() {
        assert_eq!(
            shortest_path_code("/p/vendor/bin/carbon", "/p/vendor/nesbot/carbon/bin/carbon"),
            "__DIR__ . '/..'.'/nesbot/carbon/bin/carbon'"
        );
        assert_eq!(
            shortest_path_code("/p/vendor/bin/x", "/p/vendor/bin/x"),
            "__FILE__"
        );
        assert_eq!(
            shortest_path_code("/p/bin/x", "/p/bin/x/y"),
            "__DIR__ . '/y'"
        );
        assert_eq!(shortest_path_code("/a/b/c", "/d/e"), "'/d/e'");
        assert_eq!(shortest_path_code("/a/c", "/d/e"), "__DIR__ . '/..'.'/d/e'");
    }

    #[test]
    fn plain_php_bins_get_no_stream_wrapper() {
        let code = unixy_proxy(
            "/p/vendor/a/b/bin/tool",
            "/p/vendor/bin/tool",
            "/p/vendor",
            "/p/vendor",
            b"<?php\n",
        );
        assert!(code.starts_with("#!/usr/bin/env php\n<?php\n"));
        assert!(code.contains(
            " * This file includes the referenced bin path (../a/b/bin/tool)\n *\n * @generated"
        ));
        assert!(!code.contains("BinProxyWrapper"));
        assert!(code.ends_with(
            "$GLOBALS['_composer_autoload_path'] = __DIR__ . '/..'.'/autoload.php';\n\n\nreturn include __DIR__ . '/..'.'/a/b/bin/tool';\n"
        ));
    }

    #[test]
    fn shell_bins_get_a_sh_proxy() {
        let code = unixy_proxy(
            "/p/vendor/it's/x/bin/run",
            "/p/vendor/bin/run",
            "/p/vendor",
            "/p/vendor",
            b"#!/bin/bash\necho hi\n",
        );
        assert!(code.starts_with("#!/usr/bin/env sh\n"));
        // Composer picks the quoting style by its own host OS: POSIX
        // single-quotes everywhere but Windows, where a plain path like
        // this one (no space, tab, comma or meta character) needs none.
        #[cfg(not(windows))]
        assert!(code.contains("cd '../it'\\''s/x/bin' && pwd)"));
        #[cfg(windows)]
        assert!(code.contains("cd ../it's/x/bin && pwd)"));
        assert!(code.ends_with("exec \"${dir}/run\" \"$@\"\n"));
    }

    #[test]
    fn determines_the_caller() {
        assert_eq!(binary_caller("a.bat", b""), "call");
        assert_eq!(binary_caller("a.exe", b""), "call");
        assert_eq!(binary_caller("a", b"#!/usr/bin/env php\n<?php"), "php");
        assert_eq!(binary_caller("a", b"#!/bin/bash\r\n"), "bash");
        assert_eq!(binary_caller("a", b"#!/usr/bin/env bash -e\n"), "bash -e");
        assert_eq!(binary_caller("a", b"echo"), "php");
        assert_eq!(binary_caller("a", b"#!/usr/bin/env "), "env");
        assert_eq!(binary_caller("a", b"#!/a/b/"), "b/");
        assert_eq!(binary_caller("a", b"#!/a//b"), "/b");
        assert_eq!(binary_caller("a", b"#!/"), "php");
    }

    #[test]
    fn windows_proxies_run_php_through_the_unixy_proxy() {
        assert_eq!(
            windows_proxy("/p/vendor/a/b/tool", "/p/vendor/bin/tool.bat", b"<?php"),
            "@ECHO OFF\r\nsetlocal DISABLEDELAYEDEXPANSION\r\nSET BIN_TARGET=%~dp0/tool\r\nSET COMPOSER_RUNTIME_BIN_DIR=%~dp0\r\nphp \"%BIN_TARGET%\" %*\r\n"
        );
        assert!(
            windows_proxy("/p/vendor/a/b/run", "/p/vendor/bin/run.bat", b"#!/bin/sh\n")
                .contains("SET BIN_TARGET=%~dp0/../a/b/run\r\nSET COMPOSER_RUNTIME_BIN_DIR=%~dp0\r\nsh \"%BIN_TARGET%\"")
        );
    }

    use std::fs;
    use std::path::Path;

    /// The packages `tests/golden/bins/generate.php` installs with Composer.
    const PACKAGES: [(&str, &[(&str, &str)]); 6] = [
        ("a/plain", &[("bin/plain", "<?php\necho 1;\n")]),
        ("a/spaced", &[("bin/spaced", "\n\t <?php\n")]),
        (
            "a/shebang",
            &[("bin/tool", "#!/usr/bin/env php\n<?php\necho 1;\n")],
        ),
        (
            "a/crlf",
            &[("bin/crlf", "#!/usr/bin/php -d x=1\r\n<?php\n")],
        ),
        (
            "phpunit/phpunit",
            &[("phpunit", "#!/usr/bin/env php\n<?php\n")],
        ),
        (
            "a/sh",
            &[
                ("bin/run.sh", "#!/bin/bash\necho hi\n"),
                ("bin/it's", "echo quoted\n"),
            ],
        ),
    ];

    fn golden(name: &str) -> Vec<u8> {
        let file = name.replace("it's", "its");
        fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/golden/bins")
                .join(file),
        )
        .unwrap()
    }

    #[test]
    fn proxies_match_composer_byte_for_byte() {
        for (name, files) in PACKAGES {
            for (rel, content) in files {
                let bin = format!("/p/vendor/{name}/{rel}");
                let base = php_basename(rel);
                let link = format!("/p/vendor/bin/{base}");
                let proxy = unixy_proxy(&bin, &link, "/p/vendor", "/p/vendor", content.as_bytes());
                // The golden files were captured from a real `composer`
                // running on a POSIX host. A shell (non-PHP) bin's proxy
                // embeds its directory escaped by Composer's own host OS
                // (issue #67), so on Windows it diverges from that POSIX
                // golden by design; sanity-check the unquoted path instead.
                if cfg!(windows) && matches!(bin_kind(content.as_bytes()), BinKind::Shell) {
                    assert!(proxy.contains("cd ../a/sh/bin && pwd)"), "{base}: {proxy}");
                } else {
                    assert_eq!(proxy.as_bytes(), golden(base), "{base}");
                }
                let bat = format!("{link}.bat");
                let proxy = windows_proxy(&bin, &bat, content.as_bytes());
                assert_eq!(
                    proxy.as_bytes(),
                    golden(&format!("{base}.bat")),
                    "{base}.bat"
                );
            }
        }
    }

    #[cfg(unix)]
    mod on_disk {
        use super::super::BinInstaller;
        use super::{PACKAGES, golden};
        use crate::fsutil::Modes;
        use std::collections::BTreeSet;
        use std::fs;
        use std::path::Path;

        fn installer(root: &Path, modes: Modes) -> BinInstaller {
            let vendor = root.join("vendor").to_string_lossy().into_owned();
            BinInstaller::new(root.join("vendor/bin"), vendor.clone(), vendor, true, modes)
        }

        fn lay_out(root: &Path) {
            for (name, files) in PACKAGES {
                for (rel, content) in files {
                    let path = root.join("vendor").join(name).join(rel);
                    fs::create_dir_all(path.parent().unwrap()).unwrap();
                    fs::write(path, content).unwrap();
                }
            }
        }

        #[test]
        fn installs_every_proxy_and_chmods_the_targets() {
            let tmp = tempfile::tempdir().unwrap();
            let root = fs::canonicalize(tmp.path()).unwrap();
            lay_out(&root);
            let modes = Modes::probe(&root).unwrap();
            let mut bins = installer(&root, modes);
            for (name, files) in PACKAGES {
                let list: Vec<String> = files.iter().map(|(r, _)| (*r).to_owned()).collect();
                let path = root.join("vendor").join(name);
                bins.install(name, &path.to_string_lossy(), &list, false)
                    .unwrap();
            }
            assert!(bins.warnings.is_empty(), "{:?}", bins.warnings);
            let mut names: Vec<&String> = bins.written().iter().collect();
            names.sort();
            assert_eq!(names.len(), 14);
            for name in names {
                let path = root.join("vendor/bin").join(name);
                assert_eq!(fs::read(&path).unwrap(), golden(name), "{name}");
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
                    assert_eq!(Modes::with_exec(mode), modes, "{name}");
                }
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let target = root.join("vendor/a/sh/bin/run.sh");
                let mode = fs::metadata(target).unwrap().permissions().mode() & 0o777;
                assert_eq!(Modes::with_exec(mode), modes);
            }
        }

        #[test]
        fn leaves_an_unchanged_packages_proxy_exactly_as_found() {
            // Composer's ensureBinariesPresence skips an existing, non-symlink
            // proxy without even a warning (BinaryInstaller::installBinaries,
            // warnOnOverwrite: false) when the package had no install/update
            // operation, so whatever is there (stale content, a committed
            // .bat, an odd mode) stays untouched.
            let tmp = tempfile::tempdir().unwrap();
            let root = fs::canonicalize(tmp.path()).unwrap();
            let pkg = root.join("vendor/a/b");
            fs::create_dir_all(&pkg).unwrap();
            fs::write(pkg.join("tool"), "<?php\n").unwrap();
            let bin_dir = root.join("vendor/bin");
            fs::create_dir_all(&bin_dir).unwrap();
            fs::write(bin_dir.join("tool"), "stale proxy\n").unwrap();
            fs::write(bin_dir.join("tool.bat"), "stale bat\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(bin_dir.join("tool"), fs::Permissions::from_mode(0o600))
                    .unwrap();
            }
            let mut bins = installer(&root, Modes::with_exec(0o755));
            bins.install("a/b", &pkg.to_string_lossy(), &["tool".to_owned()], true)
                .unwrap();
            assert!(bins.warnings.is_empty(), "{:?}", bins.warnings);
            assert_eq!(fs::read(bin_dir.join("tool")).unwrap(), b"stale proxy\n");
            assert_eq!(fs::read(bin_dir.join("tool.bat")).unwrap(), b"stale bat\n");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = fs::metadata(bin_dir.join("tool"))
                    .unwrap()
                    .permissions()
                    .mode();
                assert_eq!(mode & 0o777, 0o600, "mode was not touched");
            }
            // Kept, not rewritten: remove_stale must not delete either file.
            let both: BTreeSet<String> = ["tool".to_owned(), "tool.bat".to_owned()]
                .into_iter()
                .collect();
            assert_eq!(bins.written, both);
            bins.remove_stale(["tool"]).unwrap();
            assert!(bin_dir.join("tool").is_file());
            assert!(bin_dir.join("tool.bat").is_file());
        }

        #[test]
        fn skips_bins_it_cannot_proxy_and_cleans_up_old_ones() {
            let tmp = tempfile::tempdir().unwrap();
            let root = fs::canonicalize(tmp.path()).unwrap();
            let pkg = root.join("vendor/a/b");
            fs::create_dir_all(pkg.join("dir")).unwrap();
            fs::write(pkg.join("tool"), "<?php\n").unwrap();
            fs::write(root.join("outside"), "<?php\n").unwrap();
            #[cfg(unix)]
            std::os::unix::fs::symlink(root.join("outside"), pkg.join("escape")).unwrap();
            let mut bins = installer(&root, Modes::with_exec(0o755));
            let list: Vec<String> = ["missing", "dir", "../../../outside", "tool", "tool"]
                .iter()
                .map(|s| (*s).to_owned())
                .collect();
            bins.install("a/b", &pkg.to_string_lossy(), &list, false)
                .unwrap();
            #[cfg(unix)]
            bins.install("a/b", &pkg.to_string_lossy(), &["escape".to_owned()], false)
                .unwrap();
            let reasons: Vec<&str> = bins
                .warnings
                .iter()
                .map(|w| w.rsplit(": ").next().unwrap())
                .collect();
            assert_eq!(
                reasons[..4],
                [
                    "file not found in package",
                    "found a directory at that path",
                    "the bin resolves to a path outside of the package directory",
                    "name conflicts with an existing file",
                ]
            );
            let bin_dir = root.join("vendor/bin");
            assert!(bin_dir.join("tool").is_file());
            assert!(bin_dir.join("tool.bat").is_file());

            fs::write(bin_dir.join("old"), "x").unwrap();
            fs::write(bin_dir.join("old.bat"), "x").unwrap();
            bins.remove_stale(["old", "tool", "never"]).unwrap();
            assert!(!bin_dir.join("old").exists() && !bin_dir.join("old.bat").exists());
            assert!(bin_dir.join("tool").exists());

            fs::write(bin_dir.join("keep"), "x").unwrap();
            let mut fresh = installer(&root, Modes::with_exec(0o755));
            fresh.remove_stale(["tool"]).unwrap();
            assert!(!bin_dir.join("tool.bat").exists());
            assert!(bin_dir.join("keep").exists());
            fs::remove_file(bin_dir.join("keep")).unwrap();
            fs::write(bin_dir.join("last"), "x").unwrap();
            fresh.remove_stale(["last"]).unwrap();
            assert!(!bin_dir.exists());
        }
    }
}
