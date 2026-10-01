use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env = |k: &str| std::env::var(k).ok();
    let code = phpm_sweep::cli::main(&args, &env, &mut io::stdout().lock(), &mut io::stderr());
    ExitCode::from(code)
}
