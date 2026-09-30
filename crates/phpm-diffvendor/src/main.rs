use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use phpm_diffvendor::{Ignore, compare};

const USAGE: &str = "usage: diffvendor <left> <right> [--ignore NAME]...";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut ignored: Vec<String> = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--ignore" {
            match args.next() {
                Some(name) => ignored.push(name),
                None => return usage(),
            }
        } else {
            roots.push(arg.into());
        }
    }
    let [left, right] = roots.as_slice() else {
        return usage();
    };

    let names: Vec<&str> = ignored.iter().map(String::as_str).collect();
    let mut out = io::stdout().lock();
    match compare(left, right, &Ignore::names(&names)) {
        Ok(diffs) if diffs.is_empty() => {
            let _ = writeln!(out, "identical");
            ExitCode::SUCCESS
        }
        Ok(diffs) => {
            for d in &diffs {
                let _ = writeln!(out, "{d}");
            }
            let _ = writeln!(out, "{} differences", diffs.len());
            ExitCode::from(1)
        }
        Err(e) => {
            let _ = writeln!(io::stderr(), "diffvendor: {e}");
            ExitCode::from(2)
        }
    }
}

fn usage() -> ExitCode {
    let _ = writeln!(io::stderr(), "{USAGE}");
    ExitCode::from(2)
}
