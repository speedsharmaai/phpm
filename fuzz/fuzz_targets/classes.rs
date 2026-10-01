#![no_main]

use libfuzzer_sys::fuzz_target;
use phpm_autoload::scan::{find_classes, strip_whitespace};

fuzz_target!(|data: &[u8]| {
    let Some((flags, source)) = data.split_first() else {
        return;
    };
    let stripped = strip_whitespace(source, flags & 1 == 1);
    assert!(
        stripped.len() <= source.len() + 2,
        "stripping grew the source"
    );
    let _ = find_classes(source, flags & 2 == 2);
});
