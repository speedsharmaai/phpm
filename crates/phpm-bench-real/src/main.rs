use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = io::stdout().lock();
    let mut err = io::stderr().lock();
    let code = phpm_bench_real::cli::main(&args, &mut out, &mut err);
    let _ = out.flush();
    ExitCode::from(code)
}
