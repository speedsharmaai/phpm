//! The `sweep` command line.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::corpus::{Corpus, Project, Selection};
use crate::record::{Mode, Record};
use crate::report::{Results, add_history, badge, parse_lines};
use crate::runner::{Config, run_project};

pub const USAGE: &str = "usage:
  sweep run <owner/name|-> <commit|-> <pure|default> [--fixture NAME] [--stars N]
  sweep batch <corpus.json> <pure|default> [--shard I/N | --slice A..B] [--out FILE]
  sweep publish <site-dir> <date> [--meta KEY=VALUE]... <records.jsonl>...

env: PHPM_BIN (phpm), SWEEP_COMPOSER (composer), SWEEP_TIMEOUT (900 s),
     SWEEP_WORK (kept when set), SWEEP_CACHE, SWEEP_FIXTURES (fixtures),
     SWEEP_GIT_BASE (https://github.com/), SWEEP_FETCH_TRIES (3)";

pub type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Run the command line; the exit code.
pub fn main(args: &[String], env: Env<'_>, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let result = match args.first().map(String::as_str) {
        Some("run") => run(&args[1..], env, out),
        Some("batch") => batch(&args[1..], env, err),
        Some("publish") => publish(&args[1..], out),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(err, "sweep: {e}");
            2
        }
    }
}

type Options<'a> = BTreeMap<&'a str, Vec<&'a str>>;

/// Positional arguments and `--name value` options.
fn split_args(args: &[String]) -> Result<(Vec<&str>, Options<'_>), String> {
    let mut positional = Vec::new();
    let mut options = Options::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if let Some(name) = a.strip_prefix("--") {
            let value = it.next().ok_or_else(|| format!("--{name} needs a value"))?;
            options.entry(name).or_default().push(value);
        } else {
            positional.push(a.as_str());
        }
    }
    Ok((positional, options))
}

fn one<'a>(options: &Options<'a>, name: &str) -> Option<&'a str> {
    options.get(name).and_then(|v| v.last().copied())
}

fn mode(s: &str) -> Result<Mode, String> {
    Mode::parse(s).ok_or_else(|| format!("mode must be pure or default, not {s}"))
}

/// The runner's settings from the environment; the temp dir lives as long
/// as the returned guard.
fn config(env: Env<'_>) -> Result<(Config, Option<tempfile::TempDir>), String> {
    let words = |k: &str, default: &str| -> Vec<String> {
        env(k)
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| default.to_owned())
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    };
    let timeout = match env("SWEEP_TIMEOUT") {
        Some(t) => t
            .parse()
            .map_err(|_| format!("SWEEP_TIMEOUT must be seconds, not {t}"))?,
        None => 900,
    };
    let fetch_attempts = match env("SWEEP_FETCH_TRIES") {
        Some(t) => t
            .parse()
            .map_err(|_| format!("SWEEP_FETCH_TRIES must be a number, not {t}"))?,
        None => 3,
    };
    let (work, guard, keep) = if let Some(w) = env("SWEEP_WORK").filter(|w| !w.is_empty()) {
        (PathBuf::from(w), None, true)
    } else {
        let t = tempfile::tempdir().map_err(|e| format!("temp dir: {e}"))?;
        (t.path().to_path_buf(), Some(t), false)
    };
    fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let work = fs::canonicalize(&work)
        .map(without_verbatim_prefix)
        .map_err(|e| format!("{}: {e}", work.display()))?;
    let caches = env("SWEEP_CACHE").map_or_else(|| work.join("cache"), PathBuf::from);
    Ok((
        Config {
            composer: words("SWEEP_COMPOSER", "composer"),
            phpm: words("PHPM_BIN", "phpm"),
            git_base: env("SWEEP_GIT_BASE").unwrap_or_else(|| "https://github.com/".to_owned()),
            fixtures: PathBuf::from(env("SWEEP_FIXTURES").unwrap_or_else(|| "fixtures".to_owned())),
            timeout: Duration::from_secs(timeout),
            work,
            caches,
            keep,
            fetch_attempts,
            retry_pause: Duration::from_secs(10),
        },
        guard,
    ))
}

/// `C:\x` for `\\?\C:\x`: `canonicalize` gives verbatim paths on Windows,
/// and a project at one is not what users run Composer in.
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => path,
    }
}

fn dash(s: &str) -> Option<String> {
    (s != "-").then(|| s.to_owned())
}

fn run(args: &[String], env: Env<'_>, out: &mut dyn Write) -> Result<(), String> {
    let (pos, opts) = split_args(args)?;
    let [repo, commit, m] = pos.as_slice() else {
        return Err(USAGE.to_owned());
    };
    let stars = one(&opts, "stars")
        .map(|s| {
            s.parse()
                .map_err(|_| format!("--stars must be a number, not {s}"))
        })
        .transpose()?;
    let project = Project {
        repo: dash(repo),
        commit: dash(commit),
        stars,
        fixture: one(&opts, "fixture").map(str::to_owned),
    };
    let mode = mode(m)?;
    let (cfg, _guard) = config(env)?;
    let record = run_project(&project, mode, &cfg);
    let line = serde_json::to_string(&record).map_err(|e| e.to_string())?;
    writeln!(out, "{line}").map_err(|e| e.to_string())
}

fn batch(args: &[String], env: Env<'_>, err: &mut dyn Write) -> Result<(), String> {
    let (pos, opts) = split_args(args)?;
    let [corpus, m] = pos.as_slice() else {
        return Err(USAGE.to_owned());
    };
    let mode = mode(m)?;
    let selection = match (one(&opts, "shard"), one(&opts, "slice")) {
        (Some(_), Some(_)) => return Err("--shard or --slice, not both".to_owned()),
        (Some(s), None) => Selection::parse_shard(s).ok_or_else(|| format!("bad --shard {s}"))?,
        (None, Some(s)) => Selection::parse_slice(s).ok_or_else(|| format!("bad --slice {s}"))?,
        (None, None) => Selection::All,
    };
    let text = fs::read_to_string(corpus).map_err(|e| format!("{corpus}: {e}"))?;
    let corpus = Corpus::parse(&text)?;
    let projects = selection.apply(&corpus.projects);
    let out_path = one(&opts, "out").map(PathBuf::from);
    let mut sink: Box<dyn Write> = match &out_path {
        Some(p) => Box::new(
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .map_err(|e| format!("{}: {e}", p.display()))?,
        ),
        None => Box::new(std::io::stdout()),
    };
    let (cfg, _guard) = config(env)?;
    for (i, p) in projects.iter().enumerate() {
        let record = run_project(p, mode, &cfg);
        let _ = writeln!(err, "[{}/{}] {}", i + 1, projects.len(), progress(&record));
        let line = serde_json::to_string(&record).map_err(|e| e.to_string())?;
        writeln!(sink, "{line}").map_err(|e| e.to_string())?;
        sink.flush().map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn progress(r: &Record) -> String {
    let secs = |run: &Option<crate::record::Run>| run.as_ref().map_or(0.0, |r| r.seconds);
    let outcome = serde_json::to_value(r.outcome)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    format!(
        "{} {}: {outcome} (composer {:.1} s, phpm {:.1} s{})",
        r.name(),
        r.mode,
        secs(&r.composer),
        secs(&r.phpm),
        r.first_differences
            .first()
            .or(r.error.as_ref())
            .map(|d| format!("; {d}"))
            .unwrap_or_default()
    )
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    fs::write(path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

fn publish(args: &[String], out: &mut dyn Write) -> Result<(), String> {
    let (pos, opts) = split_args(args)?;
    let [site, date, files @ ..] = pos.as_slice() else {
        return Err(USAGE.to_owned());
    };
    let mut meta = BTreeMap::new();
    for kv in opts.get("meta").into_iter().flatten() {
        let (k, v) = kv
            .split_once('=')
            .ok_or_else(|| format!("--meta {kv}: want KEY=VALUE"))?;
        meta.insert(k.to_owned(), v.to_owned());
    }
    let mut records = Vec::new();
    for f in files {
        let text = fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?;
        records.extend(parse_lines(&text).map_err(|e| format!("{f}: {e}"))?);
    }
    let results = Results::new(date, meta, records);
    let site = Path::new(site);
    let history = fs::read_to_string(site.join("history.json")).ok();
    write_json(&site.join("results.json"), &results)?;
    write_json(&site.join("results").join(format!("{date}.json")), &results)?;
    write_json(
        &site.join("history.json"),
        &add_history(history.as_deref(), &results),
    )?;
    write_json(&site.join("badge.json"), &badge(&results))?;
    for (group, s) in &results.summary {
        writeln!(
            out,
            "{group}: {}, native {:.1}%, composer-failed {}",
            s.headline(),
            s.native_rate * 100.0,
            s.composer_failed + s.fetch_failed
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{USAGE, main, progress, split_args, without_verbatim_prefix};
    use crate::record::{Mode, Outcome, PhpmPath, Record};
    use serde_json::Value;
    use std::fs;
    use std::path::PathBuf;

    fn call(args: &[&str], env: &[(&str, String)]) -> (u8, String, String) {
        let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        let lookup = |k: &str| env.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone());
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = main(&args, &lookup, &mut out, &mut err);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    fn record(repo: &str, outcome: Outcome) -> Record {
        Record {
            repo: Some(repo.to_owned()),
            commit: Some("c".to_owned()),
            fixture: None,
            stars: Some(1),
            mode: Mode::Pure,
            os: "linux".to_owned(),
            outcome,
            packages: Some(1),
            compared: vec![],
            differences: 0,
            first_differences: vec![],
            normalized: vec![],
            phpm_path: PhpmPath::Native,
            phpm_reasons: vec![],
            composer: None,
            phpm: None,
            error: None,
        }
    }

    #[test]
    fn verbatim_windows_paths_lose_their_prefix() {
        let plain = |s: &str| without_verbatim_prefix(PathBuf::from(s));
        assert_eq!(plain(r"\\?\C:\work"), PathBuf::from(r"C:\work"));
        assert_eq!(
            plain(r"\\?\UNC\server\share"),
            PathBuf::from(r"\\?\UNC\server\share")
        );
        assert_eq!(plain("/tmp/work"), PathBuf::from("/tmp/work"));
    }

    #[test]
    fn unknown_commands_print_usage() {
        let (code, _, err) = call(&["nope"], &[]);
        assert_eq!(code, 2);
        assert!(err.contains(USAGE));
        assert_eq!(call(&[], &[]).0, 2);
    }

    #[test]
    fn options_need_values() {
        let args: Vec<String> = vec!["a".into(), "--out".into()];
        assert_eq!(split_args(&args).unwrap_err(), "--out needs a value");
        let args: Vec<String> = vec![
            "--meta".into(),
            "a=1".into(),
            "x".into(),
            "--meta".into(),
            "b=2".into(),
        ];
        let (pos, opts) = split_args(&args).unwrap();
        assert_eq!(pos, ["x"]);
        assert_eq!(opts["meta"], ["a=1", "b=2"]);
    }

    #[test]
    fn bad_arguments_are_errors() {
        assert!(
            call(&["run", "a/b", "c", "fast"], &[])
                .2
                .contains("mode must be pure or default")
        );
        assert!(call(&["run", "a/b"], &[]).2.contains("usage"));
        assert!(
            call(&["run", "a/b", "c", "pure", "--stars", "x"], &[])
                .2
                .contains("--stars must be a number")
        );
        assert!(
            call(
                &["run", "a/b", "c", "pure"],
                &[("SWEEP_TIMEOUT", "soon".to_owned())]
            )
            .2
            .contains("SWEEP_TIMEOUT must be seconds")
        );
        assert!(
            call(
                &["run", "a/b", "c", "pure"],
                &[("SWEEP_FETCH_TRIES", "x".to_owned())]
            )
            .2
            .contains("SWEEP_FETCH_TRIES must be a number")
        );
        assert!(call(&["batch", "c.json"], &[]).2.contains("usage"));
        assert!(
            call(
                &[
                    "batch", "c.json", "pure", "--shard", "1/2", "--slice", "0..1"
                ],
                &[]
            )
            .2
            .contains("not both")
        );
        assert!(
            call(&["batch", "c.json", "pure", "--shard", "9/2"], &[])
                .2
                .contains("bad --shard")
        );
        assert!(
            call(&["batch", "c.json", "pure", "--slice", "x"], &[])
                .2
                .contains("bad --slice")
        );
        assert!(
            call(&["batch", "/nonexistent/c.json", "pure"], &[])
                .2
                .contains("/nonexistent/c.json")
        );
        assert!(call(&["publish", "site"], &[]).2.contains("usage"));
        assert!(
            call(&["publish", "site", "d", "--meta", "novalue"], &[])
                .2
                .contains("want KEY=VALUE")
        );
    }

    #[test]
    fn run_records_a_fetch_failure_as_json() {
        let tmp = tempfile::tempdir().unwrap();
        let env = [
            (
                "SWEEP_WORK",
                tmp.path().join("w").to_string_lossy().into_owned(),
            ),
            (
                "SWEEP_GIT_BASE",
                format!("file://{}/none/", tmp.path().display()),
            ),
            ("SWEEP_COMPOSER", "  ".to_owned()),
            ("SWEEP_FETCH_TRIES", "1".to_owned()),
        ];
        let (code, out, _) = call(&["run", "o/r", "abc", "pure", "--stars", "4"], &env);
        assert_eq!(code, 0);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["outcome"], "fetch-failed");
        assert_eq!(v["stars"], 4);
        assert!(tmp.path().join("w").is_dir(), "SWEEP_WORK is kept");
    }

    #[test]
    fn batch_appends_one_line_per_project() {
        let tmp = tempfile::tempdir().unwrap();
        let corpus = tmp.path().join("corpus.json");
        fs::write(
            &corpus,
            r#"{"projects": [{"repo": null, "commit": null, "fixture": "missing"}, {"repo": null, "commit": null}]}"#,
        )
        .unwrap();
        let out = tmp.path().join("out.jsonl");
        let env = [("SWEEP_FIXTURES", tmp.path().to_string_lossy().into_owned())];
        let c = corpus.to_string_lossy();
        let o = out.to_string_lossy();
        let (code, _, err) = call(
            &["batch", &c, "default", "--slice", "0..1", "--out", &o],
            &env,
        );
        assert_eq!(code, 0, "{err}");
        assert!(
            err.contains("[1/1] fixtures/missing default: fetch-failed"),
            "{err}"
        );
        let (code, _, _) = call(
            &["batch", &c, "default", "--shard", "2/2", "--out", &o],
            &env,
        );
        assert_eq!(code, 0);
        let lines = fs::read_to_string(&out).unwrap();
        assert_eq!(lines.lines().count(), 2);
    }

    #[test]
    fn publish_writes_results_history_and_badge() {
        let tmp = tempfile::tempdir().unwrap();
        let lines = [
            record("a/a", Outcome::Identical),
            record("a/b", Outcome::Different),
        ]
        .iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
        let shard = tmp.path().join("shard.jsonl");
        fs::write(&shard, lines).unwrap();
        let site = tmp.path().join("site");
        let (s, sh) = (site.to_string_lossy(), shard.to_string_lossy());
        let (code, out, err) = call(
            &["publish", &s, "2026-10-01", "--meta", "phpm=abc", &sh],
            &[],
        );
        assert_eq!(code, 0, "{err}");
        assert!(
            out.contains("pure: identical vendor/ on 1 of 2 projects (50.0%), native 100.0%"),
            "{out}"
        );
        let results: Value =
            serde_json::from_str(&fs::read_to_string(site.join("results.json")).unwrap()).unwrap();
        assert_eq!(results["meta"]["phpm"], "abc");
        assert!(site.join("results/2026-10-01.json").is_file());
        let badge: Value =
            serde_json::from_str(&fs::read_to_string(site.join("badge.json")).unwrap()).unwrap();
        assert_eq!(badge["color"], "red");
        call(&["publish", &s, "2026-10-02", &sh], &[]);
        let history: Value =
            serde_json::from_str(&fs::read_to_string(site.join("history.json")).unwrap()).unwrap();
        assert_eq!(history.as_array().unwrap().len(), 2);
        fs::write(&shard, "{").unwrap();
        assert!(call(&["publish", &s, "d", &sh], &[]).2.contains("line 1"));
        assert!(
            call(&["publish", &s, "d", "/nonexistent.jsonl"], &[])
                .2
                .contains("/nonexistent.jsonl")
        );
    }

    #[test]
    fn progress_names_the_first_difference() {
        let mut r = record("a/a", Outcome::Different);
        r.first_differences = vec!["vendor/x: only from phpm".to_owned()];
        assert_eq!(
            progress(&r),
            "a/a pure: different (composer 0.0 s, phpm 0.0 s; vendor/x: only from phpm)"
        );
    }
}
