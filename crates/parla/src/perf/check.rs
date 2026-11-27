//! `perf_check`: compares scenario reports with the `perf_budget.toml` limits
//! (absolute mode) and with the median of earlier `main` runs (regression
//! mode). Every metric is lower-is-better.

use std::fmt::Write as _;

use crate::perf::report::Report;

/// One compared metric.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub scenario: String,
    pub metric: String,
    pub value: f64,
    /// The most the metric may be: the budget, or the baseline median plus
    /// the allowed regression.
    pub limit: f64,
    pub ok: bool,
    /// `"budget"` or `"median of N"`.
    pub basis: String,
}

/// Memory metrics regress by an absolute number of MB, not a percentage.
fn is_memory(metric: &str) -> bool {
    metric.ends_with("_mb") && metric.starts_with("mem")
}

/// Every reported metric that has a `[limits]` entry `<scenario>.<metric>`.
pub fn check_absolute(reports: &[Report], budget: &toml::Table) -> Vec<Finding> {
    let limits = budget.get("limits").and_then(|l| l.as_table());
    let mut out = Vec::new();
    for r in reports {
        for (metric, value) in &r.metrics {
            let key = format!("{}.{metric}", r.scenario);
            let Some(limit) = limits
                .and_then(|l| l.get(&key))
                .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
            else {
                continue;
            };
            // A limit of 0 means "none allowed" (frames while idle); a limit
            // not yet measured (the binary size ceiling) is skipped.
            if key == "size.bin_mb" && limit == 0.0 {
                continue;
            }
            out.push(Finding {
                scenario: r.scenario.clone(),
                metric: metric.clone(),
                value: *value,
                limit,
                ok: *value <= limit,
                basis: "budget".into(),
            });
        }
    }
    out
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// Compares `current` with the median of each metric over `baseline` runs
/// (the last five `main` runs). `pct` is the allowed worsening in percent,
/// `mem_mb` the allowed worsening of memory metrics in MB. Metrics with no
/// baseline are not reported.
pub fn check_regression(
    current: &[Report],
    baseline: &[Vec<Report>],
    pct: f64,
    mem_mb: f64,
) -> Vec<Finding> {
    let mut out = Vec::new();
    for r in current {
        for (metric, value) in &r.metrics {
            let past: Vec<f64> = baseline
                .iter()
                .filter_map(|run| run.iter().find(|b| b.scenario == r.scenario))
                .filter_map(|b| b.metrics.get(metric).copied())
                .collect();
            if past.is_empty() {
                continue;
            }
            let n = past.len();
            let med = median(past);
            let limit = if is_memory(metric) {
                med + mem_mb
            } else {
                med * (1.0 + pct / 100.0)
            };
            out.push(Finding {
                scenario: r.scenario.clone(),
                metric: metric.clone(),
                value: *value,
                limit,
                ok: *value <= limit + 1e-9,
                basis: format!("median of {n}"),
            });
        }
    }
    out
}

/// A Markdown table for the job summary.
pub fn markdown(findings: &[Finding]) -> String {
    let mut s = String::from(
        "| scenario | metric | value | limit | basis | result |\n|---|---|---|---|---|---|\n",
    );
    for f in findings {
        let _ = writeln!(
            s,
            "| {} | {} | {:.3} | {:.3} | {} | {} |",
            f.scenario,
            f.metric,
            f.value,
            f.limit,
            f.basis,
            if f.ok { "ok" } else { "OVER" }
        );
    }
    s
}
