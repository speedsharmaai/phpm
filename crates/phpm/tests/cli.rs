use std::process::Command;

#[test]
fn prints_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_phpm"))
        .output()
        .expect("phpm binary runs");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).expect("utf-8 output");
    assert_eq!(stdout, format!("phpm {}\n", env!("CARGO_PKG_VERSION")));
}
