//! The `bench-real` command line.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use crate::aggregate::{Results, add_history, badge};
use crate::readme::top_table;
use crate::record::{CiScenarioRecord, ProjectRecord, WorktreeRecord, parse_lines};

pub const USAGE: &str = "usage:
  bench-real aggregate <site-dir> <date> [--meta KEY=VALUE]... \\
    --projects FILE [FILE...] [--worktree FILE] [--ci FILE [FILE...]]
  bench-real readme-table <results.json> <os> <n> <page-url>";

/// Run the command line; the exit code.
pub fn main(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let result = match args.first().map(String::as_str) {
        Some("aggregate") => aggregate(&args[1..], out),
        Some("readme-table") => readme_table(&args[1..], out),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(err, "bench-real: {e}");
            2
        }
    }
}

type Options<'a> = BTreeMap<&'a str, Vec<&'a str>>;

/// The index just past the run of values following an `--name` at `start`:
/// every argument up to the next `--flag` or the end.
fn value_run_end(args: &[String], start: usize) -> usize {
    args[start..]
        .iter()
        .position(|a| a.starts_with("--"))
        .map_or(args.len(), |offset| start + offset)
}

/// Positional arguments and `--name value...` options; a name may repeat
/// and take more than one value before the next `--name`.
fn split_args(args: &[String]) -> Result<(Vec<&str>, Options<'_>), String> {
    let mut positional = Vec::new();
    let mut options = Options::new();
    let mut i = 0;
    while i < args.len() {
        let Some(name) = args[i].strip_prefix("--") else {
            positional.push(args[i].as_str());
            i += 1;
            continue;
        };
        let end = value_run_end(args, i + 1);
        if end == i + 1 {
            return Err(format!("--{name} needs a value"));
        }
        let values = args[i + 1..end].iter().map(String::as_str);
        options.entry(name).or_default().extend(values);
        i = end;
    }
    Ok((positional, options))
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    fs::write(path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

fn read_lines<T: for<'de> serde::Deserialize<'de>>(paths: &[&str]) -> Result<Vec<T>, String> {
    let mut all = Vec::new();
    for p in paths {
        let text = fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
        all.extend(parse_lines::<T>(&text).map_err(|e| format!("{p}: {e}"))?);
    }
    Ok(all)
}

fn aggregate(args: &[String], out: &mut dyn Write) -> Result<(), String> {
    let (pos, opts) = split_args(args)?;
    let [site, date] = pos.as_slice() else {
        return Err(USAGE.to_owned());
    };
    let mut meta = BTreeMap::new();
    for kv in opts.get("meta").into_iter().flatten() {
        let (k, v) = kv
            .split_once('=')
            .ok_or_else(|| format!("--meta {kv}: want KEY=VALUE"))?;
        meta.insert((*k).to_owned(), v.to_owned());
    }
    let projects = opts
        .get("projects")
        .ok_or("--projects needs at least one file")?;
    let records: Vec<ProjectRecord> = read_lines(projects)?;
    let worktree: Vec<WorktreeRecord> = match opts.get("worktree") {
        Some(files) => read_lines(files)?,
        None => Vec::new(),
    };
    let ci: Vec<CiScenarioRecord> = match opts.get("ci") {
        Some(files) => read_lines(files)?,
        None => Vec::new(),
    };

    let results = Results::new(date, meta, records, &worktree, &ci);
    let site = Path::new(site);
    let history = fs::read_to_string(site.join("history.json")).ok();
    write_json(&site.join("results.json"), &results)?;
    write_json(&site.join("results").join(format!("{date}.json")), &results)?;
    write_json(
        &site.join("history.json"),
        &add_history(history.as_deref(), &results),
    )?;
    write_json(&site.join("badge.json"), &badge(&results))?;

    for (os, s) in &results.summary {
        let warm = s.warm.as_ref().map_or("no data".to_owned(), |w| {
            format!("{:.1}x median warm speedup", w.median_speedup)
        });
        writeln!(
            out,
            "{os}: {} ({:.1}% identical), {warm}",
            s.headline(),
            s.identity_rate * 100.0
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn readme_table(args: &[String], out: &mut dyn Write) -> Result<(), String> {
    let (pos, _) = split_args(args)?;
    let [results_path, os, n, page_url] = pos.as_slice() else {
        return Err(USAGE.to_owned());
    };
    let n: usize = n
        .parse()
        .map_err(|_| format!("<n> must be a number, not {n}"))?;
    let text = fs::read_to_string(results_path).map_err(|e| format!("{results_path}: {e}"))?;
    let results: Results =
        serde_json::from_str(&text).map_err(|e| format!("{results_path}: {e}"))?;
    write!(out, "{}", top_table(&results, os, n, page_url)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{USAGE, main, split_args};
    use std::fs;

    fn call(args: &[&str]) -> (u8, String, String) {
        let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = main(&args, &mut out, &mut err);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn unknown_commands_print_usage() {
        let (code, _, err) = call(&["nope"]);
        assert_eq!(code, 2);
        assert!(err.contains(USAGE));
        assert_eq!(call(&[]).0, 2);
    }

    #[test]
    fn split_args_groups_repeated_options() {
        let args: Vec<String> = vec![
            "x".into(),
            "--projects".into(),
            "a.jsonl".into(),
            "b.jsonl".into(),
            "--meta".into(),
            "k=v".into(),
        ];
        let (pos, opts) = split_args(&args).unwrap();
        assert_eq!(pos, ["x"]);
        assert_eq!(opts["projects"], ["a.jsonl", "b.jsonl"]);
        assert_eq!(opts["meta"], ["k=v"]);
        let missing: Vec<String> = vec!["--meta".into()];
        assert_eq!(split_args(&missing).unwrap_err(), "--meta needs a value");
    }

    #[test]
    fn aggregate_needs_a_site_date_and_projects_option() {
        assert!(call(&["aggregate", "site"]).2.contains("usage"));
        assert!(
            call(&["aggregate", "site", "d"])
                .2
                .contains("--projects needs")
        );
        assert!(
            call(&[
                "aggregate",
                "site",
                "d",
                "--meta",
                "novalue",
                "--projects",
                "x"
            ])
            .2
            .contains("want KEY=VALUE")
        );
        assert!(
            call(&["aggregate", "site", "d", "--projects", "/nonexistent.jsonl"])
                .2
                .contains("/nonexistent.jsonl")
        );
    }

    #[test]
    fn aggregate_writes_results_history_and_badge() {
        let tmp = tempfile::tempdir().unwrap();
        let line = r#"{"group":"big-apps","repo":"a/a","os":"ubuntu-latest","identity":"identical","warm":{"composer_seconds":4.0,"phpm_seconds":1.0}}"#;
        let shard = tmp.path().join("shard.jsonl");
        fs::write(&shard, line).unwrap();
        let site = tmp.path().join("site");
        let (s, sh) = (site.to_string_lossy(), shard.to_string_lossy());
        let (code, out, err) = call(&[
            "aggregate",
            &s,
            "2026-10-01",
            "--meta",
            "phpm=abc",
            "--projects",
            &sh,
        ]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("ubuntu-latest: 1 of 1 identical"), "{out}");
        assert!(site.join("results.json").is_file());
        assert!(site.join("results/2026-10-01.json").is_file());
        assert!(site.join("history.json").is_file());
        assert!(site.join("badge.json").is_file());
    }

    #[test]
    fn readme_table_reads_a_results_file() {
        let tmp = tempfile::tempdir().unwrap();
        let results = tmp.path().join("results.json");
        fs::write(
            &results,
            r#"{"date":"d","summary":{},"records":[{"group":"big-apps","repo":"a/a","os":"ubuntu-latest","identity":"identical","warm":{"composer_seconds":4.0,"phpm_seconds":1.0}}]}"#,
        )
        .unwrap();
        let r = results.to_string_lossy();
        let (code, out, err) = call(&["readme-table", &r, "ubuntu-latest", "5", "https://x.test/"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("a/a"));
        assert!(
            call(&["readme-table", &r, "ubuntu-latest", "x", "https://x.test/"])
                .2
                .contains("must be a number")
        );
        assert!(
            call(&[
                "readme-table",
                "/nonexistent.json",
                "ubuntu-latest",
                "5",
                "https://x.test/"
            ])
            .2
            .contains("/nonexistent.json")
        );
    }

    proptest::proptest! {
        #[test]
        fn split_args_never_panics(words in proptest::collection::vec("[-a-z0-9=./]{0,8}", 0..12)) {
            let args: Vec<String> = words;
            let _ = split_args(&args);
        }
    }
}
