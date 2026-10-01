mod bins;
mod classes;
mod cli;
mod error;
mod exec;
mod fallback;
mod fsutil;
mod install;
mod notify;
mod out;
mod pathrepo;
mod platform;
mod plugins;
mod policy;
mod prefetch;
mod project;
mod runner;
mod scripts;
mod state;

use std::ffi::OsString;
use std::process::ExitCode;

use clap::Parser;
use clap::error::ErrorKind;

use crate::cli::{Cli, Command};
use crate::install::Request;
use crate::out::{Out, Verbosity};
use crate::project::Env;

fn run(args: impl IntoIterator<Item = OsString>, env: Env<'_>, out: &mut Out<'_>) -> ExitCode {
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(e) => {
            let text = e.render().to_string();
            if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
                out.stdout(&text);
            } else {
                out.raw_error(&text);
            }
            return ExitCode::from(u8::try_from(e.exit_code()).unwrap_or(2));
        }
    };
    out.set_verbosity(if cli.quiet {
        Verbosity::Quiet
    } else if cli.verbose > 0 {
        Verbosity::Verbose
    } else {
        Verbosity::Normal
    });
    let Command::Install(args) = cli.command;
    let request = Request {
        working_dir: cli.working_dir,
        dev: !args.no_dev,
        link_mode: args.link_mode.map(Into::into),
        optimize: args.optimize_autoloader,
        classmap_authoritative: args.classmap_authoritative,
        no_autoloader: args.no_autoloader,
        no_scripts: args.no_scripts,
        no_plugins: args.no_plugins,
        explain: args.explain,
        ignore_platform_reqs: args.ignore_platform_reqs,
        ignore_platform_req: args.ignore_platform_req,
        audit: args
            .audit
            .then(|| policy::AuditFormat::parse(&args.audit_format))
            .flatten(),
        no_blocking: args.no_blocking || args.no_security_blocking,
    };
    match install::run(&request, env, out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if !e.message.is_empty() {
                out.error(&e.message);
            }
            e.exit_code()
        }
    }
}

fn main() -> ExitCode {
    let env = |key: &str| std::env::var(key).ok();
    run(std::env::args_os(), &env, &mut Out::terminal())
}

#[cfg(test)]
mod tests {
    use super::run;
    use crate::out::tests::capture;
    use std::ffi::OsString;
    use std::process::ExitCode;

    fn call(args: &[&str]) -> (ExitCode, String, String) {
        let (mut out, stdout, stderr) = capture();
        let env = |_: &str| None;
        let code = run(args.iter().map(OsString::from), &env, &mut out);
        (code, stdout.text(), stderr.text())
    }

    #[test]
    fn prints_the_version_on_stdout() {
        let (code, stdout, stderr) = call(&["phpm", "--version"]);
        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(stdout, format!("phpm {}\n", env!("CARGO_PKG_VERSION")));
        assert!(stderr.is_empty());
    }

    #[test]
    fn usage_errors_exit_2() {
        let (code, stdout, stderr) = call(&["phpm", "install", "--bogus"]);
        assert_eq!(code, ExitCode::from(2));
        assert!(stdout.is_empty());
        assert!(stderr.contains("--bogus"), "{stderr}");
    }

    #[test]
    fn a_missing_project_is_a_usage_error() {
        let (code, _, stderr) = call(&["phpm", "install", "-d", "/nonexistent/phpm/project"]);
        assert_eq!(code, ExitCode::from(2));
        assert!(stderr.starts_with("error: working directory"), "{stderr}");
    }
}
