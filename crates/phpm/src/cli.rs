use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "phpm",
    version,
    about = "An extremely fast, Composer-compatible PHP installer",
    disable_help_subcommand = true,
    arg_required_else_help = true
)]
#[expect(clippy::struct_excessive_bools, reason = "one per Composer flag")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,

    /// Use DIR as the working directory
    #[arg(short = 'd', long, global = true, value_name = "DIR")]
    pub(crate) working_dir: Option<PathBuf>,

    /// Only print errors
    #[arg(short, long, global = true)]
    pub(crate) quiet: bool,

    /// Print more detail
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub(crate) verbose: u8,

    #[arg(short = 'n', long, global = true, hide = true)]
    pub(crate) no_interaction: bool,

    #[arg(long, global = true, hide = true)]
    pub(crate) ansi: bool,

    #[arg(long, global = true, hide = true)]
    pub(crate) no_ansi: bool,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Install the packages in composer.lock into vendor/
    Install(InstallArgs),
    /// Send download notifications read from stdin, for a finished install
    #[command(name = "__notify", hide = true)]
    Notify,
}

#[derive(Debug, Args)]
#[expect(clippy::struct_excessive_bools, reason = "one per Composer flag")]
pub(crate) struct InstallArgs {
    /// Skip packages-dev
    #[arg(long)]
    pub(crate) no_dev: bool,

    /// How packages get from the store into vendor/
    #[arg(long, value_enum, value_name = "MODE")]
    pub(crate) link_mode: Option<LinkModeArg>,

    /// Scan PSR-0/4 directories into the class map
    #[arg(short = 'o', long)]
    pub(crate) optimize_autoloader: bool,

    /// Load classes from the class map only (implies -o)
    #[arg(short = 'a', long)]
    pub(crate) classmap_authoritative: bool,

    /// Do not write the autoloader
    #[arg(long)]
    pub(crate) no_autoloader: bool,

    /// Do not run scripts
    #[arg(long)]
    pub(crate) no_scripts: bool,

    /// Do not load plugins
    #[arg(long)]
    pub(crate) no_plugins: bool,

    /// Print which path each package and step takes (native or Composer) and why
    #[arg(long)]
    pub(crate) explain: bool,

    /// Leave `platform_check.php` out
    #[arg(long)]
    pub(crate) ignore_platform_reqs: bool,

    /// Leave one requirement out of `platform_check.php`
    #[arg(long, value_name = "REQ")]
    pub(crate) ignore_platform_req: Vec<String>,

    /// Run an audit after the install; exit 5 if it finds problems
    #[arg(long)]
    pub(crate) audit: bool,

    /// Audit output format: table, plain, json or summary
    #[arg(long, value_name = "FORMAT", default_value = "summary", value_parser = ["table", "plain", "json", "summary"])]
    pub(crate) audit_format: String,

    /// Disable all policy blocking (malware filter) for this run
    #[arg(long)]
    pub(crate) no_blocking: bool,

    #[arg(long, hide = true)]
    pub(crate) no_security_blocking: bool,

    #[arg(long, hide = true)]
    pub(crate) dev: bool,

    #[arg(long, hide = true)]
    pub(crate) no_progress: bool,

    #[arg(long, hide = true)]
    pub(crate) prefer_dist: bool,

    #[arg(long, hide = true)]
    pub(crate) no_audit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum LinkModeArg {
    Clone,
    Hardlink,
    Copy,
}

impl From<LinkModeArg> for phpm_store::LinkMode {
    fn from(mode: LinkModeArg) -> Self {
        match mode {
            LinkModeArg::Clone => Self::Clone,
            LinkModeArg::Hardlink => Self::Hardlink,
            LinkModeArg::Copy => Self::Copy,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command, LinkModeArg};
    use clap::{CommandFactory, Parser, error::ErrorKind};
    use phpm_store::LinkMode;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).unwrap()
    }

    fn args(cli: &Cli) -> &super::InstallArgs {
        let Command::Install(a) = &cli.command else {
            panic!("expected install");
        };
        a
    }

    #[test]
    fn definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_composer_style_flags() {
        let cli = parse(&[
            "phpm",
            "install",
            "--no-dev",
            "-o",
            "-a",
            "--no-scripts",
            "--no-plugins",
            "--explain",
            "-d",
            "/tmp/x",
            "-vv",
            "--link-mode",
            "hardlink",
            "--ignore-platform-req",
            "ext-*",
            "--audit",
            "--audit-format",
            "json",
            "--no-blocking",
        ]);
        let a = args(&cli);
        assert!(a.no_dev && a.optimize_autoloader && a.classmap_authoritative);
        assert!(a.no_scripts && a.no_plugins && a.explain);
        assert_eq!(
            cli.working_dir.as_deref(),
            Some(std::path::Path::new("/tmp/x"))
        );
        assert_eq!(cli.verbose, 2);
        assert_eq!(a.link_mode, Some(LinkModeArg::Hardlink));
        assert_eq!(a.ignore_platform_req, ["ext-*"]);
        assert!(a.audit && a.no_blocking && !a.no_security_blocking);
        assert_eq!(a.audit_format, "json");
        let err = Cli::try_parse_from(["phpm", "install", "--audit-format", "xml"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidValue);
    }

    #[test]
    fn accepts_flags_that_change_nothing() {
        let cli = parse(&[
            "phpm",
            "-n",
            "install",
            "--no-interaction",
            "--no-progress",
            "--prefer-dist",
            "--no-audit",
            "--dev",
            "--no-ansi",
            "--quiet",
        ]);
        let a = args(&cli);
        assert!(cli.no_interaction && cli.quiet && cli.no_ansi && !cli.ansi);
        assert!(a.no_progress && a.prefer_dist && a.no_audit && a.dev);
        assert!(!a.no_dev && a.link_mode.is_none());
    }

    #[test]
    fn rejects_unknown_flags_and_values() {
        let err = Cli::try_parse_from(["phpm", "install", "--frobnicate"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnknownArgument);
        assert_eq!(err.exit_code(), 2);
        let err = Cli::try_parse_from(["phpm", "install", "--link-mode", "symlink"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidValue);
        let err = Cli::try_parse_from(["phpm"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        let err = Cli::try_parse_from(["phpm", "--version"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayVersion);
        assert_eq!(err.exit_code(), 0);
    }

    #[test]
    fn maps_link_modes() {
        assert_eq!(LinkMode::from(LinkModeArg::Clone), LinkMode::Clone);
        assert_eq!(LinkMode::from(LinkModeArg::Hardlink), LinkMode::Hardlink);
        assert_eq!(LinkMode::from(LinkModeArg::Copy), LinkMode::Copy);
    }
}
