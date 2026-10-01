//! Runs script events without Composer, the way Composer's `EventDispatcher`
//! runs shell, `@php`, `@composer`, `@putenv` and `@script` listeners.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use crate::error::Error;
use crate::exec::{self, PATH_SEP, escape, php_command, which};
use crate::fsutil::path_string;
use crate::out::Out;
use crate::project::Env;
use crate::scripts::{Listener, Scripts, listener, strip_no_args};

/// What a native script run needs to know about the project.
#[derive(Debug, Clone)]
pub(crate) struct Context {
    pub(crate) root: PathBuf,
    pub(crate) bin_dir: String,
    pub(crate) dev: bool,
    pub(crate) composer: Option<PathBuf>,
    pub(crate) php: Option<PathBuf>,
    pub(crate) timeout: Option<Duration>,
}

// Composer: Config::get('process-timeout')
pub(crate) fn timeout(config: Option<&Value>, env: Env<'_>) -> Option<Duration> {
    let seconds = match env("COMPOSER_PROCESS_TIMEOUT") {
        Some(v) => v.trim().parse::<i64>().unwrap_or(0),
        None => match config {
            Some(Value::Number(n)) => n.as_i64().unwrap_or(300),
            Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
            _ => 300,
        },
    };
    u64::try_from(seconds)
        .ok()
        .filter(|s| *s > 0)
        .map(Duration::from_secs)
}

pub(crate) struct Runner<'a> {
    scripts: &'a Scripts,
    ctx: Context,
    env: Env<'a>,
    vars: BTreeMap<String, Option<String>>,
    php_command: Option<String>,
}

impl std::fmt::Debug for Runner<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runner")
            .field("vars", &self.vars)
            .finish_non_exhaustive()
    }
}

impl<'a> Runner<'a> {
    pub(crate) fn new(scripts: &'a Scripts, ctx: Context, env: Env<'a>) -> Self {
        let mut vars = BTreeMap::new();
        vars.insert(
            "COMPOSER_DEV_MODE".to_owned(),
            Some(if ctx.dev { "1" } else { "0" }.to_owned()),
        );
        if let Some(composer) = &ctx.composer {
            vars.insert("COMPOSER_BINARY".to_owned(), Some(path_string(composer)));
        }
        Self {
            scripts,
            ctx,
            env,
            vars,
            php_command: None,
        }
    }

    fn var(&self, key: &str) -> Option<String> {
        match self.vars.get(key) {
            Some(v) => v.clone(),
            None => (self.env)(key),
        }
    }

    // Composer: EventDispatcher::ensureBinDirIsInPath
    fn ensure_bin_dir_in_path(&mut self) {
        let Ok(bin) = std::fs::canonicalize(&self.ctx.bin_dir) else {
            return;
        };
        let bin = bin.to_string_lossy().into_owned();
        let current = self.var("PATH").unwrap_or_default();
        if !current.split(PATH_SEP).any(|p| p == bin) {
            self.vars
                .insert("PATH".to_owned(), Some(format!("{bin}{PATH_SEP}{current}")));
        }
    }

    fn php(&mut self) -> Result<String, Error> {
        if let Some(cmd) = &self.php_command {
            return Ok(cmd.clone());
        }
        let php = self.ctx.php.clone().ok_or_else(|| {
            Error::install("Failed to locate PHP binary to execute a script; put php on PATH or set PHP_BINARY")
        })?;
        let cmd = php_command(&php, self.env)?;
        self.php_command = Some(cmd.clone());
        Ok(cmd)
    }

    fn composer_binary(&self, script: &str) -> Result<String, Error> {
        self.ctx
            .composer
            .as_deref()
            .map(path_string)
            .ok_or_else(|| {
                Error::install(format!(
                    "the script {script} runs Composer, and composer is not on PATH (or set PHPM_COMPOSER)"
                ))
            })
    }

    /// Run every listener of `event` in order; the first failure stops the install.
    pub(crate) fn dispatch(
        &mut self,
        event: &str,
        args: &[String],
        out: &mut Out<'_>,
    ) -> Result<(), Error> {
        let listeners: Vec<Value> = self.scripts.listeners(event).to_vec();
        for value in listeners {
            let Value::String(raw) = value else {
                return Err(Error::install(format!(
                    "{event} has a listener phpm cannot run"
                )));
            };
            self.ensure_bin_dir_in_path();
            let (callable, no_args) = strip_no_args(&raw);
            let extra: &[String] = if no_args { &[] } else { args };
            match listener(&callable) {
                Listener::ComposerCall(text) => {
                    let mut parts: Vec<String> =
                        text.split(' ').skip(1).map(str::to_owned).collect();
                    parts.extend(extra.iter().cloned());
                    let line = format!(
                        "{} {} {}",
                        self.php()?,
                        escape(&self.composer_binary(text)?),
                        parts.join(" ")
                    );
                    self.execute(&line, text, event)?;
                }
                Listener::Reference { name, args: own } => {
                    if !self.scripts.exists(name) {
                        out.warn(&format!(
                            "You made a reference to a non-existent script {callable}"
                        ));
                        continue;
                    }
                    let mut nested: Vec<String> = own.iter().map(|s| (*s).to_owned()).collect();
                    nested.extend(extra.iter().cloned());
                    let name = name.to_owned();
                    if let Err(e) = self.dispatch(&name, &nested, out) {
                        out.error(&format!("Script {callable} was called via {event}"));
                        return Err(e);
                    }
                }
                Listener::Callable(c) | Listener::CommandClass(c) => {
                    return Err(Error::install(format!("{c} needs Composer to run")));
                }
                Listener::Exec(text) => self.exec(text, extra, event, out)?,
            }
        }
        Ok(())
    }

    fn exec(
        &mut self,
        callable: &str,
        args: &[String],
        event: &str,
        out: &mut Out<'_>,
    ) -> Result<(), Error> {
        let escaped: Vec<String> = args.iter().map(|a| escape(a)).collect();
        let joined = escaped.join(" ");
        let line = if callable.starts_with("@putenv ") {
            callable.to_owned()
        } else if callable.contains("@additional_args") {
            callable.replace("@additional_args", &joined)
        } else if joined.is_empty() {
            callable.to_owned()
        } else {
            format!("{callable} {joined}")
        };
        out.info(&format!("> {line}"));

        if let Some(assignment) = line.strip_prefix("@putenv ") {
            match assignment.split_once('=') {
                Some((k, v)) => self.vars.insert(k.to_owned(), Some(v.to_owned())),
                None => self.vars.insert(assignment.to_owned(), None),
            };
            return Ok(());
        }
        let mut line = if let Some(path_and_args) = line.strip_prefix("@php ") {
            format!("{} {}", self.php()?, self.resolve_php_target(path_and_args))
        } else {
            if let Some(php) = &self.ctx.php {
                self.vars
                    .insert("PHP_BINARY".to_owned(), Some(path_string(php)));
            }
            line
        };
        if let Some(rest) = line.strip_prefix("composer ") {
            line = format!(
                "{} {} {rest}",
                self.php()?,
                escape(&self.composer_binary(callable)?)
            );
        }
        self.execute(&line, callable, event)
    }

    // `@php foo` runs `foo` from PATH when it is not a path from the project root
    fn resolve_php_target(&self, path_and_args: &str) -> String {
        let end = path_and_args
            .find(|c: char| c.is_whitespace() || matches!(c, '\'' | '"' | '/' | '\\'))
            .unwrap_or(path_and_args.len());
        let first = &path_and_args[..end];
        let rest = &path_and_args[end..];
        if first.is_empty() || self.ctx.root.join(first).exists() {
            return path_and_args.to_owned();
        }
        match which(first, self.var("PATH").as_deref()) {
            Some(found) => format!("{}{rest}", path_string(&found)),
            None => path_and_args.to_owned(),
        }
    }

    fn execute(&self, line: &str, callable: &str, event: &str) -> Result<(), Error> {
        let mut cmd = exec::shell(line);
        cmd.current_dir(&self.ctx.root);
        for (k, v) in &self.vars {
            match v {
                Some(v) => cmd.env(k, v),
                None => cmd.env_remove(k),
            };
        }
        let status = exec::wait(&mut cmd, self.ctx.timeout, line)?;
        if status.success() {
            return Ok(());
        }
        let code = exec::exit_code(status);
        Err(Error {
            message: format!(
                "Script {callable} handling the {event} event returned with error code {code}"
            ),
            code,
        })
    }

    #[cfg(all(test, unix))]
    pub(crate) fn vars(&self) -> &BTreeMap<String, Option<String>> {
        &self.vars
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::{Context, Runner, timeout};
    use crate::out::tests::capture;
    use crate::scripts::Scripts;
    use phpm_lock::ComposerJson;
    use serde_json::{Value, json};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    #[test]
    fn reads_the_process_timeout() {
        let none = |_: &str| None;
        assert_eq!(timeout(None, &none), Some(Duration::from_secs(300)));
        assert_eq!(timeout(Some(&json!(0)), &none), None);
        assert_eq!(
            timeout(Some(&json!("12")), &none),
            Some(Duration::from_secs(12))
        );
        assert_eq!(
            timeout(Some(&json!(true)), &none),
            Some(Duration::from_secs(300))
        );
        let env = |k: &str| (k == "COMPOSER_PROCESS_TIMEOUT").then(|| "7".to_owned());
        assert_eq!(timeout(Some(&json!(1)), &env), Some(Duration::from_secs(7)));
        let env = |k: &str| (k == "COMPOSER_PROCESS_TIMEOUT").then(|| "-3".to_owned());
        assert_eq!(timeout(None, &env), None);
    }

    fn fake(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    struct Setup {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        log: PathBuf,
        ctx: Context,
        path: String,
    }

    fn setup() -> Setup {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let tools = root.join("tools");
        std::fs::create_dir_all(&tools).unwrap();
        std::fs::create_dir_all(root.join("vendor/bin")).unwrap();
        let log = root.join("log");
        let php = fake(
            &tools,
            "php",
            &format!(
                "#!/bin/sh\nif [ \"$1\" = -r ]; then printf '1\\n\\n128M\\n'; exit 0; fi\necho \"php $*\" >> {}\n",
                log.display()
            ),
        );
        let composer = fake(&tools, "composer", "#!/bin/sh\n");
        fake(&tools, "helper", "#!/bin/sh\n");
        let path = format!("{}:/usr/bin:/bin", tools.display());
        let ctx = Context {
            root: root.clone(),
            bin_dir: format!("{}/vendor/bin", root.display()),
            dev: true,
            composer: Some(composer),
            php: Some(php),
            timeout: Some(Duration::from_secs(30)),
        };
        Setup {
            _tmp: tmp,
            root,
            log,
            ctx,
            path,
        }
    }

    fn scripts(v: &Value) -> Scripts {
        Scripts::new(
            &ComposerJson::from_value(json!({"scripts": v})).unwrap(),
            &|_| None,
        )
    }

    #[test]
    fn runs_listeners_with_composer_environment() {
        let s = setup();
        let log = s.log.display().to_string();
        let list = scripts(&json!({
            "post-install-cmd": [
                "@putenv GREETING=hi",
                format!("echo \"$GREETING $COMPOSER_DEV_MODE $(basename $COMPOSER_BINARY) $(basename $PHP_BINARY)\" >> {log}"),
                format!("echo \"${{PATH%%:*}}\" >> {log}"),
                "@php artisan package:discover",
                "@php helper --x",
                "@composer --version",
                "composer about",
                "@more extra",
                "@putenv GREETING",
                format!("echo \"[$GREETING]\" >> {log}"),
                "@missing",
            ],
            "more": [format!("echo more >> {log}"), format!("echo args @additional_args >> {log} @no_additional_args")],
        }));
        let path = s.path.clone();
        let env = move |k: &str| (k == "PATH").then(|| path.clone());
        let (mut out, _, stderr) = capture();
        let mut runner = Runner::new(&list, s.ctx.clone(), &env);
        runner.dispatch("post-install-cmd", &[], &mut out).unwrap();
        let logged = std::fs::read_to_string(&s.log).unwrap();
        let lines: Vec<&str> = logged.lines().collect();
        let php = "-d allow_url_fopen=1 -d disable_functions= -d memory_limit=1536M";
        let composer = s.ctx.composer.as_ref().unwrap().display().to_string();
        let helper = s.root.join("tools/helper").display().to_string();
        assert_eq!(
            lines,
            [
                "hi 1 composer php".to_owned(),
                format!("{}/vendor/bin", s.root.display()),
                format!("php {php} artisan package:discover"),
                format!("php {php} {helper} --x"),
                format!("php {php} {composer} --version"),
                format!("php {php} {composer} about"),
                "more extra".to_owned(),
                "args".to_owned(),
                "[]".to_owned(),
            ]
        );
        let err = stderr.text();
        assert!(err.contains("> @php artisan package:discover\n"), "{err}");
        assert!(err.contains("non-existent script @missing"), "{err}");
        assert_eq!(runner.vars().get("GREETING"), Some(&None));
        assert!(format!("{runner:?}").contains("Runner"));
    }

    #[test]
    fn passes_arguments_to_referenced_scripts() {
        let s = setup();
        let log = s.log.display().to_string();
        let list = scripts(&json!({
            "post-install-cmd": ["@say one 'two'"],
            "say": [format!("echo said >> {log}; echo")],
        }));
        let (mut out, _, stderr) = capture();
        let env = |_: &str| None;
        Runner::new(&list, s.ctx.clone(), &env)
            .dispatch("post-install-cmd", &[], &mut out)
            .unwrap();
        assert!(
            stderr.text().contains("echo 'one' ''\\''two'\\'''"),
            "{}",
            stderr.text()
        );
        assert_eq!(std::fs::read_to_string(&s.log).unwrap(), "said\n");
    }

    #[test]
    fn stops_on_the_first_failure_with_its_exit_code() {
        let s = setup();
        let list = scripts(&json!({
            "post-install-cmd": ["@fail"],
            "fail": ["exit 7", "echo never"],
            "callable": ["A\\B::c"],
            "odd": [1],
        }));
        let env = |_: &str| None;
        let (mut out, _, stderr) = capture();
        let mut runner = Runner::new(&list, s.ctx, &env);
        let err = runner
            .dispatch("post-install-cmd", &[], &mut out)
            .unwrap_err();
        assert_eq!(err.code, 7);
        assert_eq!(
            err.message,
            "Script exit 7 handling the fail event returned with error code 7"
        );
        assert!(
            stderr
                .text()
                .contains("Script @fail was called via post-install-cmd")
        );
        assert!(runner.dispatch("callable", &[], &mut out).is_err());
        assert!(runner.dispatch("odd", &[], &mut out).is_err());
    }

    #[test]
    fn needs_php_and_composer_only_when_a_script_uses_them() {
        let s = setup();
        let list = scripts(
            &json!({"a": ["@php -v"], "b": ["@composer x"], "c": ["composer x"], "d": ["true"]}),
        );
        let ctx = Context {
            php: None,
            composer: None,
            ..s.ctx
        };
        let env = |_: &str| None;
        let (mut out, _, _) = capture();
        let mut runner = Runner::new(&list, ctx, &env);
        assert!(
            runner
                .dispatch("a", &[], &mut out)
                .unwrap_err()
                .message
                .contains("PHP binary")
        );
        let mut runner = Runner {
            php_command: Some("php".into()),
            ..runner
        };
        assert!(
            runner
                .dispatch("b", &[], &mut out)
                .unwrap_err()
                .message
                .contains("composer is not on PATH")
        );
        assert!(
            runner
                .dispatch("c", &[], &mut out)
                .unwrap_err()
                .message
                .contains("composer is not on PATH")
        );
        runner.dispatch("d", &[], &mut out).unwrap();
        assert!(!runner.vars().contains_key("COMPOSER_BINARY"));
    }
}
