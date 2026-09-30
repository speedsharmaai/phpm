use std::io::{self, Write};
use std::process::ExitCode;

fn version_line() -> String {
    format!("phpm {}", env!("CARGO_PKG_VERSION"))
}

fn main() -> ExitCode {
    let mut out = io::stdout().lock();
    match writeln!(out, "{}", version_line()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::version_line;

    #[test]
    fn version_line_names_the_tool() {
        assert!(version_line().starts_with("phpm "));
    }
}
