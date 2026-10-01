#![no_main]

use libfuzzer_sys::fuzz_target;
use phpm_autoload::scan::{find_classes, strip_whitespace};

fuzz_target!(|data: &[u8]| {
    let Some((flags, source)) = data.split_first() else {
        return;
    };
    let _ = strip_whitespace(source, flags & 1 == 1);
    let _ = find_classes(source, flags & 2 == 2);
});
