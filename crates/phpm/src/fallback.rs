//! The parts of an install that go to real Composer (decision 0004), and the
//! command lines phpm runs for them.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::Error;
use crate::exec;
use crate::install::Request;
use crate::out::{Out, Verbosity};
use crate::plugins::Plugins;
use crate::scripts::{self, Route, Scripts};

/// How one step of the install runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    Skip(String),
    Native(String),
    Composer(String),
}

impl Step {
    fn line(&self, name: &str) -> String {
        match self {
            Self::Skip(why) => format!("decision {name}: skipped, {why}"),
            Self::Native(what) => format!("decision {name}: native, {what}"),
            Self::Composer(why) => format!("decision {name}: fallback to Composer, {why}"),
        }
    }

    pub(crate) fn is_composer(&self) -> bool {
        matches!(self, Self::Composer(_))
    }
}

/// Every decision an install makes before it touches `vendor/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) plugins: Plugins,
    pub(crate) full_install: Option<String>,
    pub(crate) pre_install: Step,
    /// Native: phpm writes the autoloader. Composer: `composer dump-autoload`,
    /// which also fires the autoload events.
    pub(crate) autoload: Step,
    pub(crate) pre_autoload: Step,
    pub(crate) post_autoload: Step,
    pub(crate) post_install: Step,
    /// `post-install-cmd` goes to Composer only for the plugins listening to it.
    pub(crate) post_install_for_plugins: bool,
    /// A script phpm runs itself calls `composer`.
    pub(crate) scripts_call_composer: bool,
}

/// Events only Composer's own install loop fires, so scripts on them need it.
// Composer: Installer/PackageEvents.php, Installer/InstallerEvents.php, Plugin/PluginEvents.php
const PLACING_EVENTS: [&str; 7] = [
    "pre-package-install",
    "post-package-install",
    "pre-package-update",
    "post-package-update",
    "pre-operations-exec",
    "pre-file-download",
    "post-file-download",
];
const REMOVING_EVENTS: [&str; 2] = ["pre-package-uninstall", "post-package-uninstall"];
const ALWAYS_EVENTS: [&str; 4] = ["init", "command", "pre-command-run", "pre-pool-create"];

fn names(plugins: &Plugins) -> String {
    let list: Vec<&str> = plugins.active.iter().map(|p| p.name.as_str()).collect();
    list.join(", ")
}

fn script_step(scripts: &Scripts, event: &str) -> Step {
    match scripts.route(event) {
        Route::Empty => Step::Skip("no scripts".to_owned()),
        Route::Native { listeners, .. } => Step::Native(format!(
            "{listeners} script{}",
            if listeners == 1 { "" } else { "s" }
        )),
        Route::Composer(why) => Step::Composer(why),
    }
}

fn calls_composer(scripts: &Scripts, event: &str) -> bool {
    matches!(scripts.route(event), Route::Native { composer: true, .. })
}

/// Decide each step. `placing` and `removing` say whether packages change.
pub(crate) fn plan(
    req: &Request,
    scripts: &Scripts,
    plugins: Plugins,
    placing: bool,
    removing: bool,
) -> Plan {
    let mut full_install = plugins.active.iter().find_map(|p| {
        p.needs_full_install
            .as_ref()
            .map(|why| format!("{}: {why}", p.name))
    });
    if full_install.is_none() && !req.no_scripts {
        let fired = ALWAYS_EVENTS
            .iter()
            .chain(PLACING_EVENTS.iter().filter(|_| placing))
            .chain(REMOVING_EVENTS.iter().filter(|_| removing))
            .find(|e| scripts.has(e));
        full_install = fired.map(|e| {
            format!("composer.json has scripts for {e}, which only Composer's own install fires")
        });
    }
    let with_plugins = !plugins.active.is_empty();
    let no_scripts = || Step::Skip("--no-scripts".to_owned());
    let pre_install = if req.no_scripts {
        no_scripts()
    } else if with_plugins && scripts.has(scripts::PRE_INSTALL) {
        Step::Composer(format!("plugins are loaded ({})", names(&plugins)))
    } else {
        script_step(scripts, scripts::PRE_INSTALL)
    };
    let by_dump = || Step::Skip("composer dump-autoload fires it".to_owned());
    let (autoload, pre_autoload, post_autoload) = if req.no_autoloader {
        let off = || Step::Skip("--no-autoloader".to_owned());
        (off(), off(), off())
    } else if with_plugins {
        (
            Step::Composer(format!("plugins are loaded ({})", names(&plugins))),
            by_dump(),
            by_dump(),
        )
    } else if req.no_scripts {
        (
            Step::Native("phpm writes the autoloader".to_owned()),
            no_scripts(),
            no_scripts(),
        )
    } else {
        (
            Step::Native("phpm writes the autoloader".to_owned()),
            script_step(scripts, scripts::PRE_AUTOLOAD),
            script_step(scripts, scripts::POST_AUTOLOAD),
        )
    };
    let post_install_for_plugins = with_plugins && !req.no_scripts;
    let post_install = if req.no_scripts {
        no_scripts()
    } else if with_plugins {
        Step::Composer(format!("plugins are loaded ({})", names(&plugins)))
    } else {
        script_step(scripts, scripts::POST_INSTALL)
    };
    let scripts_call_composer = !req.no_scripts
        && [
            (&pre_install, scripts::PRE_INSTALL),
            (&pre_autoload, scripts::PRE_AUTOLOAD),
            (&post_autoload, scripts::POST_AUTOLOAD),
            (&post_install, scripts::POST_INSTALL),
        ]
        .iter()
        .any(|(step, event)| matches!(step, Step::Native(_)) && calls_composer(scripts, event));
    Plan {
        plugins,
        full_install,
        pre_install,
        autoload,
        pre_autoload,
        post_autoload,
        post_install,
        post_install_for_plugins,
        scripts_call_composer,
    }
}

impl Plan {
    /// Why this install needs a `composer` binary, if it does.
    pub(crate) fn needs_composer(&self) -> Option<String> {
        if let Some(why) = &self.full_install {
            return Some(format!("Composer must run this install ({why})"));
        }
        for (name, step) in [
            (scripts::PRE_INSTALL, &self.pre_install),
            ("autoload", &self.autoload),
            (scripts::PRE_AUTOLOAD, &self.pre_autoload),
            (scripts::POST_AUTOLOAD, &self.post_autoload),
            (scripts::POST_INSTALL, &self.post_install),
        ] {
            if let Step::Composer(why) = step {
                return Some(format!("{name} needs Composer ({why})"));
            }
        }
        self.scripts_call_composer
            .then(|| "a script calls composer".to_owned())
    }

    pub(crate) fn uses_composer(&self) -> bool {
        self.full_install.is_some()
            || self.pre_install.is_composer()
            || self.autoload.is_composer()
            || self.pre_autoload.is_composer()
            || self.post_autoload.is_composer()
            || self.post_install.is_composer()
    }

    pub(crate) fn decision_lines(&self, req: &Request) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(if req.no_plugins {
            "decision plugins: none loaded, --no-plugins".to_owned()
        } else if self.plugins.active.is_empty() {
            "decision plugins: none to load".to_owned()
        } else {
            format!(
                "decision plugins: {} allowed, so Composer runs the steps they hook",
                names(&self.plugins)
            )
        });
        if let Some(why) = &self.full_install {
            lines.push(format!(
                "decision install: fallback to composer install, {why}"
            ));
            return lines;
        }
        lines.push("decision install: native, phpm fetches and places every package".to_owned());
        lines.push(self.pre_install.line(scripts::PRE_INSTALL));
        lines.push(self.autoload.line("autoload"));
        lines.push(self.pre_autoload.line(scripts::PRE_AUTOLOAD));
        lines.push(self.post_autoload.line(scripts::POST_AUTOLOAD));
        lines.push(self.post_install.line(scripts::POST_INSTALL));
        lines
    }
}

/// A real `composer` and the flags every call passes on.
#[derive(Debug, Clone)]
pub(crate) struct Composer {
    bin: PathBuf,
    root: PathBuf,
    verbosity: Verbosity,
}

fn platform_flags(req: &Request, args: &mut Vec<String>) {
    if req.ignore_platform_reqs {
        args.push("--ignore-platform-reqs".to_owned());
    }
    for r in &req.ignore_platform_req {
        args.push(format!("--ignore-platform-req={r}"));
    }
}

fn flag(args: &mut Vec<String>, on: bool, name: &str) {
    if on {
        args.push(name.to_owned());
    }
}

pub(crate) fn install_args(req: &Request) -> Vec<String> {
    let mut args = vec!["install".to_owned()];
    flag(&mut args, !req.dev, "--no-dev");
    flag(&mut args, req.no_scripts, "--no-scripts");
    flag(&mut args, req.no_plugins, "--no-plugins");
    flag(&mut args, req.no_autoloader, "--no-autoloader");
    flag(&mut args, req.optimize, "--optimize-autoloader");
    flag(
        &mut args,
        req.classmap_authoritative,
        "--classmap-authoritative",
    );
    flag(&mut args, req.no_audit, "--no-audit");
    platform_flags(req, &mut args);
    args
}

pub(crate) fn dump_args(req: &Request) -> Vec<String> {
    let mut args = vec!["dump-autoload".to_owned()];
    args.push(if req.dev { "--dev" } else { "--no-dev" }.to_owned());
    flag(&mut args, req.no_scripts, "--no-scripts");
    flag(&mut args, req.optimize, "--optimize");
    flag(
        &mut args,
        req.classmap_authoritative,
        "--classmap-authoritative",
    );
    platform_flags(req, &mut args);
    args
}

pub(crate) fn run_script_args(req: &Request, event: &str) -> Vec<String> {
    let mut args = vec!["run-script".to_owned(), event.to_owned()];
    args.push(if req.dev { "--dev" } else { "--no-dev" }.to_owned());
    args
}

impl Composer {
    pub(crate) fn new(bin: PathBuf, root: &Path, verbosity: Verbosity) -> Self {
        Self {
            bin,
            root: root.to_path_buf(),
            verbosity,
        }
    }

    fn command(&self, args: &[String]) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.args(args)
            .arg("--no-interaction")
            .current_dir(&self.root);
        match self.verbosity {
            Verbosity::Quiet => {
                cmd.arg("--quiet");
            }
            Verbosity::Verbose => {
                cmd.arg("-v");
            }
            Verbosity::Normal => {}
        }
        cmd
    }

    fn failed(&self, args: &[String], status: std::process::ExitStatus) -> Error {
        let code = exec::exit_code(status);
        Error {
            message: format!(
                "{} {} failed with exit code {code}",
                self.bin.display(),
                args.first().map_or("", String::as_str)
            ),
            code,
        }
    }

    pub(crate) fn run(&self, args: &[String], out: &mut Out<'_>) -> Result<(), Error> {
        out.detail(&format!(
            "running {} {}",
            self.bin.display(),
            args.join(" ")
        ));
        let status = exec::wait(&mut self.command(args), None, &args.join(" "))?;
        if status.success() {
            Ok(())
        } else {
            Err(self.failed(args, status))
        }
    }

    /// `run-script` for an event no root script listens to fails unless a
    /// plugin does; that failure means there was nothing to run.
    pub(crate) fn run_script_for_plugins(
        &self,
        args: &[String],
        out: &mut Out<'_>,
    ) -> Result<(), Error> {
        out.detail(&format!(
            "running {} {}",
            self.bin.display(),
            args.join(" ")
        ));
        let output = self
            .command(args)
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| Error::install(format!("{}: {e}", self.bin.display())))?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() && stderr.contains("is not defined in this package") {
            out.detail("no plugin listens to it");
            return Ok(());
        }
        out.raw_error(&stderr);
        if output.status.success() {
            Ok(())
        } else {
            Err(self.failed(args, output.status))
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::Composer;
    use super::{Plan, Step, dump_args, install_args, plan, run_script_args};
    use crate::install::Request;
    #[cfg(unix)]
    use crate::out::{Verbosity, tests::capture};
    use crate::plugins::{Plugin, Plugins};
    use crate::scripts::Scripts;
    use phpm_lock::ComposerJson;
    use serde_json::{Value, json};

    fn scripts(v: &Value) -> Scripts {
        Scripts::new(
            &ComposerJson::from_value(json!({"scripts": v})).unwrap(),
            &|_| None,
        )
    }

    fn plugins(list: &[(&str, Option<&'static str>)]) -> Plugins {
        Plugins {
            active: list
                .iter()
                .map(|(n, f)| Plugin {
                    name: (*n).to_owned(),
                    global: false,
                    needs_full_install: f.map(str::to_owned),
                })
                .collect(),
            skipped: Vec::new(),
        }
    }

    fn req() -> Request {
        Request {
            dev: true,
            ..Request::default()
        }
    }

    #[test]
    fn plugin_free_projects_without_scripts_stay_native() {
        let p = plan(&req(), &scripts(&json!({})), Plugins::default(), true, true);
        assert_eq!(p.full_install, None);
        assert_eq!(p.pre_install, Step::Skip("no scripts".into()));
        assert!(
            p.autoload == Step::Native("phpm writes the autoloader".into())
                && p.post_autoload == Step::Skip("no scripts".into())
        );
        assert_eq!(p.needs_composer(), None);
        assert!(!p.uses_composer());
        let lines = p.decision_lines(&req());
        assert_eq!(lines[0], "decision plugins: none to load");
        assert_eq!(
            lines[1],
            "decision install: native, phpm fetches and places every package"
        );
        assert_eq!(lines[2], "decision pre-install-cmd: skipped, no scripts");
    }

    #[test]
    fn php_callables_send_only_their_event_to_composer() {
        let s = scripts(&json!({
            "post-autoload-dump": ["Illuminate\\Foundation\\ComposerScripts::postAutoloadDump", "@php artisan package:discover --ansi"],
            "pre-package-uninstall": ["Illuminate\\Foundation\\ComposerScripts::prePackageUninstall"],
            "post-install-cmd": ["@php -v", "@composer --version"],
        }));
        let p = plan(&req(), &s, Plugins::default(), true, false);
        assert_eq!(p.full_install, None);
        assert_eq!(
            p.autoload,
            Step::Native("phpm writes the autoloader".into())
        );
        assert_eq!(p.pre_autoload, Step::Skip("no scripts".into()));
        assert!(
            matches!(&p.post_autoload, Step::Composer(why) if why.contains("postAutoloadDump, a PHP callable"))
        );
        assert_eq!(p.post_install, Step::Native("2 scripts".into()));
        assert!(p.scripts_call_composer);
        assert!(
            p.needs_composer()
                .unwrap()
                .starts_with("post-autoload-dump needs Composer")
        );
        assert!(p.uses_composer());

        let removing = plan(&req(), &s, Plugins::default(), false, true);
        assert!(
            removing
                .full_install
                .unwrap()
                .contains("pre-package-uninstall")
        );
        let no_scripts = Request {
            no_scripts: true,
            ..req()
        };
        let p = plan(&no_scripts, &s, Plugins::default(), true, true);
        assert_eq!(p.full_install, None);
        assert_eq!(
            p.autoload,
            Step::Native("phpm writes the autoloader".into())
        );
        assert_eq!(p.post_install, Step::Skip("--no-scripts".into()));
        assert_eq!(p.needs_composer(), None);
        let no_autoloader = Request {
            no_autoloader: true,
            ..req()
        };
        let p = plan(&no_autoloader, &s, Plugins::default(), false, false);
        assert_eq!(p.autoload, Step::Skip("--no-autoloader".into()));
        assert_eq!(
            p.needs_composer().as_deref(),
            Some("a script calls composer")
        );
    }

    #[test]
    fn plugins_send_autoload_and_install_events_to_composer() {
        let s = scripts(&json!({"pre-install-cmd": "echo a", "init": "echo b"}));
        let p = plan(
            &req(),
            &scripts(&json!({})),
            plugins(&[("symfony/flex", None), ("symfony/runtime", None)]),
            true,
            false,
        );
        assert_eq!(p.full_install, None);
        assert_eq!(p.pre_install, Step::Skip("no scripts".into()));
        assert_eq!(
            p.autoload,
            Step::Composer("plugins are loaded (symfony/flex, symfony/runtime)".into())
        );
        assert_eq!(
            p.post_autoload,
            Step::Skip("composer dump-autoload fires it".into())
        );
        assert!(p.post_install.is_composer() && p.post_install_for_plugins);
        let lines = p.decision_lines(&req());
        assert!(lines[0].contains("symfony/flex, symfony/runtime allowed"));
        assert_eq!(
            lines[4],
            "decision pre-autoload-dump: skipped, composer dump-autoload fires it"
        );

        let with_init = plan(&req(), &s, Plugins::default(), false, false);
        assert!(with_init.full_install.unwrap().contains("init"));
        let pre = plan(
            &Request {
                no_scripts: false,
                ..req()
            },
            &scripts(&json!({"pre-install-cmd": "echo a"})),
            plugins(&[("a/b", None)]),
            false,
            false,
        );
        assert!(pre.pre_install.is_composer());
        assert!(
            pre.needs_composer()
                .unwrap()
                .starts_with("pre-install-cmd needs Composer")
        );
        let quiet = Request {
            no_scripts: true,
            ..req()
        };
        let p = plan(&quiet, &s, plugins(&[("a/b", None)]), false, false);
        assert!(p.autoload.is_composer() && !p.post_install_for_plugins);
    }

    #[test]
    fn path_changing_plugins_hand_over_the_whole_install() {
        let p = plan(
            &req(),
            &scripts(&json!({})),
            plugins(&[
                ("symfony/runtime", None),
                (
                    "composer/installers",
                    Some("it changes where packages are installed"),
                ),
            ]),
            true,
            false,
        );
        let why = p.full_install.clone().unwrap();
        assert_eq!(
            why,
            "composer/installers: it changes where packages are installed"
        );
        assert!(
            p.needs_composer()
                .unwrap()
                .contains("Composer must run this install")
        );
        let lines: Vec<String> = p.decision_lines(&Request {
            no_plugins: false,
            ..req()
        });
        assert_eq!(lines.len(), 2);
        assert!(lines[1].starts_with("decision install: fallback to composer install"));
        assert_eq!(
            Plan {
                full_install: None,
                ..p
            }
            .decision_lines(&Request {
                no_plugins: true,
                ..req()
            })[0],
            "decision plugins: none loaded, --no-plugins"
        );
    }

    #[test]
    fn maps_flags_onto_composer_commands() {
        let r = Request {
            dev: false,
            no_scripts: true,
            no_plugins: true,
            no_autoloader: true,
            optimize: true,
            classmap_authoritative: true,
            no_audit: true,
            ignore_platform_reqs: true,
            ignore_platform_req: vec!["ext-intl".into()],
            ..Request::default()
        };
        assert_eq!(
            install_args(&r),
            [
                "install",
                "--no-dev",
                "--no-scripts",
                "--no-plugins",
                "--no-autoloader",
                "--optimize-autoloader",
                "--classmap-authoritative",
                "--no-audit",
                "--ignore-platform-reqs",
                "--ignore-platform-req=ext-intl"
            ]
        );
        assert_eq!(
            dump_args(&r),
            [
                "dump-autoload",
                "--no-dev",
                "--no-scripts",
                "--optimize",
                "--classmap-authoritative",
                "--ignore-platform-reqs",
                "--ignore-platform-req=ext-intl"
            ]
        );
        assert_eq!(install_args(&req()), ["install"]);
        assert_eq!(dump_args(&req()), ["dump-autoload", "--dev"]);
        assert_eq!(
            run_script_args(&req(), "post-install-cmd"),
            ["run-script", "post-install-cmd", "--dev"]
        );
        assert_eq!(run_script_args(&r, "x"), ["run-script", "x", "--no-dev"]);
    }

    #[cfg(unix)]
    #[test]
    fn runs_composer_and_passes_failures_on() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("composer");
        let log = tmp.path().join("log");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\necho \"$*\" >> {}\ncase \"$2\" in\n  missing) echo 'Script \"missing\" is not defined in this package' >&2; exit 1;;\n  bad) echo boom >&2; exit 4;;\nesac\n[ \"$1\" = fail ] && exit 3\nexit 0\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (mut out, _, stderr) = capture();
        let quiet = Composer::new(bin.clone(), tmp.path(), Verbosity::Quiet);
        quiet.run(&["dump-autoload".into()], &mut out).unwrap();
        let err = quiet.run(&["fail".into()], &mut out).unwrap_err();
        assert_eq!(err.code, 3);
        assert!(
            err.message.ends_with("fail failed with exit code 3"),
            "{err}"
        );
        let verbose = Composer::new(bin, tmp.path(), Verbosity::Verbose);
        verbose
            .run_script_for_plugins(&["run-script".into(), "missing".into()], &mut out)
            .unwrap();
        verbose
            .run_script_for_plugins(&["run-script".into(), "fine".into()], &mut out)
            .unwrap();
        let err = verbose
            .run_script_for_plugins(&["run-script".into(), "bad".into()], &mut out)
            .unwrap_err();
        assert_eq!(err.code, 4);
        assert!(stderr.text().contains("boom"));
        let logged = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            logged,
            "dump-autoload --no-interaction --quiet\nfail --no-interaction --quiet\nrun-script missing --no-interaction -v\nrun-script fine --no-interaction -v\nrun-script bad --no-interaction -v\n"
        );
        let normal = Composer::new(tmp.path().join("nope"), tmp.path(), Verbosity::Normal);
        assert!(normal.run(&["x".into()], &mut out).is_err());
        assert!(
            normal
                .run_script_for_plugins(&["x".into()], &mut out)
                .is_err()
        );
    }
}
