#![no_main]

use libfuzzer_sys::fuzz_target;
use phpm_lock::{ComposerJson, Lock};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(composer) = ComposerJson::parse(text) {
        let _ = composer.vendor_dir();
        let _ = composer.install_preferences();
    }
    let Ok(lock) = Lock::parse(text) else {
        return;
    };
    let _ = lock.content_hash();
    let _ = lock.plugin_api_version();
    let _ = lock.aliases();
    for entries in [lock.packages(), lock.packages_dev()].into_iter().flatten() {
        for entry in entries {
            for key in ["require", "require-dev", "conflict"] {
                let links = entry.get(key).and_then(|v| v.as_object());
                for constraint in links.into_iter().flatten().filter_map(|(_, c)| c.as_str()) {
                    let _ = phpm_lock::constraint::parse(constraint);
                }
            }
        }
    }
});
