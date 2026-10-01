#![no_main]

use libfuzzer_sys::fuzz_target;
use phpm_store::Store;

fuzz_target!(|data: &[u8]| {
    let Ok(jail) = tempfile::tempdir() else {
        return;
    };
    let root = jail.path().join("cache");
    let store = Store::new(&root);
    if let Ok(dir) = store.insert_zip("fuzz/zip", "1", data) {
        assert!(dir.starts_with(&root), "extracted outside the store");
    }
    if let Ok(dir) = store.insert_tar("fuzz/tar", "1", data) {
        assert!(dir.starts_with(&root), "extracted outside the store");
    }
    let names: Vec<_> = std::fs::read_dir(jail.path())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert!(
        names.iter().all(|n| n == "cache"),
        "an archive wrote next to the store: {names:?}"
    );
});
