use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

fn tree(name: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("phpm-diffvendor-cli-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    for (rel, bytes) in files {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().expect("file has a parent")).expect("create fixture dir");
        fs::write(path, bytes).expect("write fixture file");
    }
    fs::create_dir_all(&dir).expect("create tree root");
    dir
}

fn run(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_diffvendor"))
        .args(args)
        .output()
        .expect("diffvendor runs")
}

#[test]
fn identical_trees_exit_zero() {
    let l = tree("same-l", &[("a.php", b"<?php")]);
    let r = tree("same-r", &[("a.php", b"<?php")]);
    let out = run(&[l.as_os_str(), r.as_os_str()]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "identical\n");
}

#[test]
fn differences_exit_one_and_are_listed() {
    let l = tree("diff-l", &[("a.php", b"<?php")]);
    let r = tree("diff-r", &[("a.php", b"<?php ")]);
    let out = run(&[l.as_os_str(), r.as_os_str()]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("bytes differ:  a.php (first at offset 5)"));
    assert!(stdout.ends_with("1 differences\n"));
}

#[test]
fn ignore_flag_skips_a_name() {
    let l = tree("ign-l", &[("a.php", b"x")]);
    let r = tree("ign-r", &[("a.php", b"x"), ("state", b"y")]);
    let out = run(&[
        l.as_os_str(),
        r.as_os_str(),
        "--ignore".as_ref(),
        "state".as_ref(),
    ]);
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn wrong_arguments_exit_two() {
    assert_eq!(run(&[]).status.code(), Some(2));
    assert_eq!(run(&["only-one".as_ref()]).status.code(), Some(2));
    assert_eq!(
        run(&["a".as_ref(), "b".as_ref(), "--ignore".as_ref()])
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn missing_directory_exits_two() {
    let out = run(&[
        "/nonexistent/phpm-left".as_ref(),
        "/nonexistent/phpm-right".as_ref(),
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("diffvendor: "));
}
