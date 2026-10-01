//! Finding and running the programs phpm hands work to: Composer, PHP and
//! the shell.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};

use crate::error::Error;
use crate::project::Env;

#[cfg(unix)]
pub(crate) const PATH_SEP: char = ':';
#[cfg(not(unix))]
pub(crate) const PATH_SEP: char = ';';

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(unix)]
fn candidates(name: &str) -> Vec<String> {
    vec![name.to_owned()]
}

#[cfg(not(unix))]
fn candidates(name: &str) -> Vec<String> {
    let mut list = vec![name.to_owned()];
    for ext in [".exe", ".bat", ".cmd", ".com"] {
        list.push(format!("{name}{ext}"));
    }
    list
}

/// The first `name` on `path` that can be run.
pub(crate) fn which(name: &str, path: Option<&str>) -> Option<PathBuf> {
    let path = path?;
    path.split(PATH_SEP)
        .filter(|dir| !dir.is_empty())
        .flat_map(|dir| {
            candidates(name)
                .into_iter()
                .map(move |n| Path::new(dir).join(n))
        })
        .find(|p| is_executable(p))
}

/// The Composer phpm falls back to: `PHPM_COMPOSER`, else `composer` on PATH,
/// resolved like Composer's own `realpath($argv[0])`.
pub(crate) fn find_composer(env: Env<'_>) -> Option<PathBuf> {
    let found = match env("PHPM_COMPOSER").filter(|v| !v.is_empty()) {
        Some(explicit) => Some(PathBuf::from(explicit)).filter(|p| is_executable(p)),
        None => which("composer", env("PATH").as_deref()),
    }?;
    Some(std::fs::canonicalize(&found).unwrap_or(found))
}

/// The PHP Composer's `@php` would use: `PHP_BINARY`, else `php` on PATH.
pub(crate) fn find_php(env: Env<'_>) -> Option<PathBuf> {
    if let Some(explicit) = env("PHP_BINARY").filter(|v| !v.is_empty()) {
        let path = PathBuf::from(&explicit);
        return if is_executable(&path) {
            Some(path)
        } else {
            which(&explicit, env("PATH").as_deref())
        };
    }
    which("php", env("PATH").as_deref())
}

// Composer: ProcessExecutor::escapeArgument
#[cfg(unix)]
pub(crate) fn escape(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\\''"))
}

#[cfg(not(unix))]
pub(crate) fn escape(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".to_owned();
    }
    let needs_quotes = arg.contains([' ', '\t', ',', '^', '&', '|', '<', '>', '(', ')', '"']);
    let escaped = arg.replace('"', "\\\"");
    if needs_quotes {
        format!("\"{escaped}\"")
    } else {
        escaped
    }
}

// Composer: bin/composer, the memory_limit it runs with
pub(crate) fn composer_memory_limit(ini: &str, from_env: Option<&str>) -> String {
    if let Some(limit) = from_env.filter(|v| !v.is_empty() && *v != "0") {
        return limit.to_owned();
    }
    let ini = ini.trim();
    if ini != "-1" && php_bytes(ini) < 1024 * 1024 * 1536 {
        "1536M".to_owned()
    } else {
        ini.to_owned()
    }
}

fn php_bytes(value: &str) -> i64 {
    let digits: String = value
        .trim_start()
        .chars()
        .enumerate()
        .take_while(|(i, c)| c.is_ascii_digit() || (*i == 0 && (*c == '-' || *c == '+')))
        .map(|(_, c)| c)
        .collect();
    let number: i64 = digits.parse().unwrap_or(0);
    let factor = match value.chars().last().map(|c| c.to_ascii_lowercase()) {
        Some('g') => 1024 * 1024 * 1024,
        Some('m') => 1024 * 1024,
        Some('k') => 1024,
        _ => 1,
    };
    number.saturating_mul(factor)
}

/// `php -d ...` the way Composer's `getPhpExecCommand` builds it, from one
/// probe of the PHP binary's ini settings.
pub(crate) fn php_command(php: &Path, env: Env<'_>) -> Result<String, Error> {
    let output = Command::new(php)
        .args([
            "-r",
            "echo ini_get('allow_url_fopen'), \"\\n\", ini_get('disable_functions'), \"\\n\", ini_get('memory_limit'), \"\\n\";",
        ])
        .output()
        .map_err(|e| Error::install(format!("{}: {e}", php.display())))?;
    if !output.status.success() {
        return Err(Error::install(format!(
            "{} could not report its settings: {}",
            php.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines();
    let fopen = lines.next().unwrap_or_default();
    let disabled = lines.next().unwrap_or_default();
    let memory = composer_memory_limit(
        lines.next().unwrap_or_default(),
        env("COMPOSER_MEMORY_LIMIT").as_deref(),
    );
    Ok(format!(
        "{} -d allow_url_fopen={} -d disable_functions={} -d memory_limit={}",
        escape(&crate::fsutil::path_string(php)),
        escape(fopen),
        escape(disabled),
        escape(&memory),
    ))
}

/// A command line run by the system shell, as Symfony Process runs it.
pub(crate) fn shell(line: &str) -> Command {
    #[cfg(unix)]
    {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg(line);
        cmd
    }
    #[cfg(not(unix))]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new("cmd");
        cmd.args(["/d", "/s", "/c"]).raw_arg(format!("\"{line}\""));
        cmd
    }
}

/// Run to the end, or kill it after `timeout`.
pub(crate) fn wait(
    cmd: &mut Command,
    timeout: Option<Duration>,
    what: &str,
) -> Result<ExitStatus, Error> {
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::install(format!("{what}: {e}")))?;
    let Some(limit) = timeout else {
        return child
            .wait()
            .map_err(|e| Error::install(format!("{what}: {e}")));
    };
    let started = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| Error::install(format!("{what}: {e}")))?
        {
            return Ok(status);
        }
        if started.elapsed() >= limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::install(format!(
                "The process \"{what}\" exceeded the timeout of {} seconds.",
                limit.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The exit code to pass on: the child's, or 1 when it has none.
pub(crate) fn exit_code(status: ExitStatus) -> u8 {
    status
        .code()
        .and_then(|c| u8::try_from(c).ok())
        .filter(|c| *c != 0)
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::{composer_memory_limit, php_bytes};
    #[cfg(unix)]
    use super::{escape, exit_code, find_composer, find_php, shell, wait, which};
    #[cfg(unix)]
    use std::time::Duration;

    #[test]
    fn computes_composer_memory_limit() {
        assert_eq!(composer_memory_limit("128M", None), "1536M");
        assert_eq!(composer_memory_limit("-1", None), "-1");
        assert_eq!(composer_memory_limit("2G", None), "2G");
        assert_eq!(composer_memory_limit("2048M", Some("")), "2048M");
        assert_eq!(composer_memory_limit("128M", Some("512M")), "512M");
        assert_eq!(composer_memory_limit("128M", Some("0")), "1536M");
        assert_eq!(php_bytes("1k"), 1024);
        assert_eq!(php_bytes("12"), 12);
        assert_eq!(php_bytes("x"), 0);
    }

    #[cfg(unix)]
    #[test]
    fn escapes_like_composer_on_unix() {
        assert_eq!(escape("a b"), "'a b'");
        assert_eq!(escape("it's"), "'it'\\''s'");
        assert_eq!(escape(""), "''");
    }

    #[cfg(unix)]
    #[test]
    fn finds_programs_on_path() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().canonicalize().unwrap();
        let tool = dir.join("composer");
        std::fs::write(&tool, "#!/bin/sh\n").unwrap();
        assert_eq!(which("composer", Some(&dir.to_string_lossy())), None);
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!(":/nonexistent:{}", dir.display());
        assert_eq!(which("composer", Some(&path)), Some(tool.clone()));
        assert_eq!(which("composer", None), None);

        let env = |k: &str| (k == "PATH").then(|| path.clone());
        assert_eq!(find_composer(&env), Some(tool.clone()));
        let explicit = tool.to_string_lossy().into_owned();
        let env = |k: &str| (k == "PHPM_COMPOSER").then(|| explicit.clone());
        assert_eq!(find_composer(&env), Some(tool.clone()));
        let env = |k: &str| (k == "PHPM_COMPOSER").then(|| "/nonexistent/composer".to_owned());
        assert_eq!(find_composer(&env), None);

        let php = dir.join("php");
        std::fs::copy(&tool, &php).unwrap();
        std::fs::set_permissions(&php, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = |k: &str| (k == "PATH").then(|| path.clone());
        assert_eq!(find_php(&env), Some(php.clone()));
        let explicit = php.to_string_lossy().into_owned();
        let env = |k: &str| (k == "PHP_BINARY").then(|| explicit.clone());
        assert_eq!(find_php(&env), Some(php.clone()));
        let env = |k: &str| match k {
            "PHP_BINARY" => Some("php".to_owned()),
            "PATH" => Some(path.clone()),
            _ => None,
        };
        assert_eq!(find_php(&env), Some(php));
    }

    #[cfg(unix)]
    #[test]
    fn runs_shell_lines_with_a_timeout() {
        let status = wait(&mut shell("exit 3"), None, "exit 3").unwrap();
        assert_eq!(exit_code(status), 3);
        let status = wait(&mut shell("true"), Some(Duration::from_secs(5)), "true").unwrap();
        assert!(status.success());
        assert_eq!(exit_code(status), 1);
        let err = wait(
            &mut shell("sleep 5"),
            Some(Duration::from_millis(50)),
            "sleep 5",
        )
        .unwrap_err();
        assert!(
            err.message.contains("exceeded the timeout of 0 seconds"),
            "{err}"
        );
        let mut missing = std::process::Command::new("/nonexistent/phpm-binary");
        assert!(wait(&mut missing, None, "x").is_err());
    }
}
