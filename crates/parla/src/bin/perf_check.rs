//! `perf_check`: compares `target/perf/*.json` reports with the limits in
//! `perf_budget.toml` and, with `--baseline <dir>`, with the median
//! of the runs in its subdirectories. Prints a Markdown table; exits 1 on a
//! breach.
//!
//! `perf_check [--reports <dir>] [--baseline <dir>] [--regression <pct>] [--mem-mb <mb>]`

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use parla::perf::check::{Finding, check_absolute, check_regression, markdown};
use parla::perf::report::Report;

const BUDGET: &str = include_str!("../../tests/perf_budget.toml");

fn arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Baseline runs: each subdirectory of `dir` holding reports is one run, the
/// newest five by name; reports directly in `dir` count as one run.
fn baseline_runs(dir: &Path) -> Vec<Vec<Report>> {
    let mut subs: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    subs.sort();
    let mut runs: Vec<Vec<Report>> = subs
        .iter()
        .rev()
        .take(5)
        .map(|d| Report::read_dir(d))
        .collect();
    let flat = Report::read_dir(dir);
    if !flat.is_empty() {
        runs.push(flat);
    }
    runs.retain(|r| !r.is_empty());
    runs
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let reports_dir =
        PathBuf::from(arg(&args, "--reports").unwrap_or_else(|| "target/perf".into()));
    let reports = Report::read_dir(&reports_dir);
    if reports.is_empty() {
        eprintln!(
            "no reports in {}; run `mise run perf:measure -- <scenario>` first",
            reports_dir.display()
        );
        return ExitCode::FAILURE;
    }
    let budget: toml::Table = BUDGET.parse().expect("perf_budget.toml parses");
    let mut findings: Vec<Finding> = Vec::new();
    match arg(&args, "--baseline") {
        Some(dir) => {
            let pct = arg(&args, "--regression")
                .and_then(|p| p.parse().ok())
                .unwrap_or(15.0);
            let mem = arg(&args, "--mem-mb")
                .and_then(|p| p.parse().ok())
                .unwrap_or(5.0);
            let runs = baseline_runs(Path::new(&dir));
            if runs.is_empty() {
                println!("No baseline runs in {dir}; nothing to compare yet.\n");
            }
            findings = check_regression(&reports, &runs, pct, mem);
        }
        None => findings.extend(check_absolute(&reports, &budget)),
    }
    print!("{}", markdown(&findings));
    if findings.iter().any(|f| !f.ok) {
        eprintln!("perf_check: over the limit");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
