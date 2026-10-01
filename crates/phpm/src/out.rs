//! The one place phpm writes to the terminal.

use std::io::{self, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Verbosity {
    Quiet,
    Normal,
    Verbose,
}

/// Status lines go to stderr, like Composer's; `--version` and help go to stdout.
pub(crate) struct Out<'a> {
    stdout: Box<dyn Write + 'a>,
    stderr: Box<dyn Write + 'a>,
    verbosity: Verbosity,
}

impl std::fmt::Debug for Out<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Out")
            .field("verbosity", &self.verbosity)
            .finish_non_exhaustive()
    }
}

impl<'a> Out<'a> {
    pub(crate) fn new(stdout: impl Write + 'a, stderr: impl Write + 'a) -> Self {
        Self {
            stdout: Box::new(stdout),
            stderr: Box::new(stderr),
            verbosity: Verbosity::Normal,
        }
    }

    pub(crate) fn terminal() -> Out<'static> {
        Out::new(io::stdout(), io::stderr())
    }

    pub(crate) fn set_verbosity(&mut self, verbosity: Verbosity) {
        self.verbosity = verbosity;
    }

    pub(crate) fn stdout(&mut self, text: &str) {
        let _ = self.stdout.write_all(text.as_bytes());
        let _ = self.stdout.flush();
    }

    fn line(&mut self, min: Verbosity, text: &str) {
        if self.verbosity >= min {
            let _ = writeln!(self.stderr, "{text}");
        }
    }

    pub(crate) fn info(&mut self, text: &str) {
        self.line(Verbosity::Normal, text);
    }

    pub(crate) fn detail(&mut self, text: &str) {
        self.line(Verbosity::Verbose, text);
    }

    pub(crate) fn warn(&mut self, text: &str) {
        self.line(Verbosity::Normal, &format!("warning: {text}"));
    }

    /// Text that is already formatted, such as clap's usage errors.
    pub(crate) fn raw_error(&mut self, text: &str) {
        let _ = self.stderr.write_all(text.as_bytes());
    }

    /// Errors are printed even with `--quiet`.
    pub(crate) fn error(&mut self, text: &str) {
        let _ = writeln!(self.stderr, "error: {text}");
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{Out, Verbosity};
    use std::cell::RefCell;
    use std::io::{self, Write};
    use std::rc::Rc;

    #[derive(Debug, Clone, Default)]
    pub(crate) struct Buf(pub(crate) Rc<RefCell<Vec<u8>>>);

    impl Buf {
        pub(crate) fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.borrow()).into_owned()
        }
    }

    impl Write for Buf {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(data);
            Ok(data.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    pub(crate) fn capture() -> (Out<'static>, Buf, Buf) {
        let (o, e) = (Buf::default(), Buf::default());
        (Out::new(o.clone(), e.clone()), o, e)
    }

    #[test]
    fn quiet_keeps_only_errors() {
        let (mut out, stdout, stderr) = capture();
        out.set_verbosity(Verbosity::Quiet);
        out.info("a");
        out.warn("b");
        out.detail("c");
        out.error("d");
        out.raw_error("e\n");
        out.stdout("v\n");
        assert_eq!(stderr.text(), "error: d\ne\n");
        assert_eq!(stdout.text(), "v\n");
    }

    #[test]
    fn verbose_adds_details() {
        let (mut out, _, stderr) = capture();
        out.info("a");
        out.detail("hidden");
        out.set_verbosity(Verbosity::Verbose);
        out.detail("shown");
        out.warn("w");
        assert_eq!(stderr.text(), "a\nshown\nwarning: w\n");
        assert!(format!("{out:?}").contains("Verbose"));
    }
}
