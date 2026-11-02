mod css;
mod engine;
mod reftest;
mod report;
mod source;
mod tokenizer;
mod tree;

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const SUITES: [&str; 4] = [
    "html5lib-tokenizer",
    "html5lib-tree",
    "css-parsing",
    "wpt-reftests",
];

const USAGE: &str = "usage: conformance [--failures] [--summary FILE] [--baseline FILE] [SUITE...]
suites: html5lib-tokenizer html5lib-tree css-parsing wpt-reftests (default: all)";

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("conformance: {error}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<bool, String> {
    let mut failures = false;
    let mut summary = None;
    let mut baseline = None;
    let mut suites = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--failures" => failures = true,
            "--summary" => summary = Some(PathBuf::from(args.next().ok_or(USAGE)?)),
            "--baseline" => baseline = Some(PathBuf::from(args.next().ok_or(USAGE)?)),
            suite if SUITES.contains(&suite) => suites.push(arg),
            _ => return Err(USAGE.to_owned()),
        }
    }
    if suites.is_empty() {
        suites = SUITES.map(String::from).into();
    }
    std::panic::set_hook(Box::new(|_| {}));
    let mut tallies = Vec::new();
    for suite in &suites {
        let results = match suite.as_str() {
            "html5lib-tokenizer" => vec![tokenizer::run(&source::HTML5LIB.checkout()?)?],
            "html5lib-tree" => vec![tree::run(&source::WPT.checkout()?)?],
            "css-parsing" => css::run(&source::CSS_PARSING.checkout()?)?,
            _ => reftest::run(&source::WPT.checkout()?)?,
        };
        for tally in &results {
            tally.print(failures);
        }
        tallies.extend(results);
    }
    if let Some(path) = summary {
        report::write_summary(&path, &tallies)?;
    }
    let regressions = match baseline {
        Some(path) => report::regressions(&path, &tallies)?,
        None => Vec::new(),
    };
    for regression in &regressions {
        eprintln!("regression: {regression}");
    }
    Ok(regressions.is_empty())
}

fn files(dir: &Path, extension: &str) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries =
            std::fs::read_dir(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                pending.push(path);
            } else if extension.is_empty()
                || path.extension().is_some_and(|found| found == extension)
            {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

fn read_json(path: &Path) -> Result<serde_json::Value, String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn guarded(check: impl FnOnce() -> bool) -> bool {
    std::panic::catch_unwind(AssertUnwindSafe(check)).unwrap_or(false)
}
