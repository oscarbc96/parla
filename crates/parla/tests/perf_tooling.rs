//! The perf reports and `perf_check`: absolute limits,
//! regression against the median of the last five runs, and the CI job's
//! shape. Plain tests on temp files.

use std::collections::BTreeMap;

use parla::perf::check::{check_absolute, check_regression, markdown};
use parla::perf::report::Report;

fn report(scenario: &str, metrics: &[(&str, f64)]) -> Report {
    Report {
        scenario: scenario.into(),
        commit: "abc1234".into(),
        os: "macos".into(),
        machine: "test machine".into(),
        seconds: 10.0,
        metrics: metrics
            .iter()
            .map(|(k, v)| ((*k).to_string(), *v))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn limits(src: &str) -> toml::Table {
    src.parse().unwrap()
}

fn runs(scenario: &str, metric: &str, values: &[f64]) -> Vec<Vec<Report>> {
    values
        .iter()
        .map(|v| vec![report(scenario, &[(metric, *v)])])
        .collect()
}

#[test]
fn check_flags_absolute_breach() {
    let l = limits("[limits]\n\"idle.mem_mb\" = 60.0\n\"idle.cpu_pct\" = 0.1\n");
    let f = check_absolute(
        &[report("idle", &[("mem_mb", 70.0), ("cpu_pct", 0.05)])],
        &l,
    );
    assert_eq!(f.len(), 2);
    let mem = f.iter().find(|f| f.metric == "mem_mb").unwrap();
    assert!(!mem.ok, "70 MB is over the 60 MB budget");
    assert!(f.iter().find(|f| f.metric == "cpu_pct").unwrap().ok);
    assert!(markdown(&f).contains("| idle | mem_mb |"));
}

#[test]
fn check_flags_regression_over_threshold_against_median_of_five() {
    // The median of [1, 1, 1, 5, 9] is 1, so 1.2 is 20 % worse: over 15 %.
    let base = runs("scroll", "cpu_pct", &[1.0, 9.0, 1.0, 5.0, 1.0]);
    let f = check_regression(&[report("scroll", &[("cpu_pct", 1.2)])], &base, 15.0, 5.0);
    assert!(!f[0].ok, "{f:?}");
    assert_eq!(f[0].limit, 1.0 * 1.15);
}

#[test]
fn check_passes_within_threshold() {
    let base = runs("scroll", "cpu_pct", &[1.0, 1.0, 1.0, 1.0, 1.0]);
    let f = check_regression(&[report("scroll", &[("cpu_pct", 1.1)])], &base, 15.0, 5.0);
    assert!(f.iter().all(|f| f.ok), "{f:?}");
    // No baseline yet: nothing to compare, nothing flagged.
    assert!(check_regression(&[report("scroll", &[("cpu_pct", 99.0)])], &[], 15.0, 5.0).is_empty());
}

#[test]
fn memory_threshold_is_absolute_mb() {
    let base = runs("idle", "mem_mb", &[20.0; 5]);
    // 24 MB is 20 % worse but only 4 MB: fine. 26 MB is 6 MB worse: flagged.
    let ok = check_regression(&[report("idle", &[("mem_mb", 24.0)])], &base, 15.0, 5.0);
    assert!(ok.iter().all(|f| f.ok), "{ok:?}");
    let bad = check_regression(&[report("idle", &[("mem_mb", 26.0)])], &base, 15.0, 5.0);
    assert!(!bad[0].ok, "{bad:?}");
    assert_eq!(bad[0].limit, 25.0);
}

#[test]
fn report_schema_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let r = report("switch", &[("latency_ms", 12.5), ("mem_mb", 41.0)]);
    let path = r.write(dir.path()).unwrap();
    assert_eq!(path.file_name().unwrap(), "switch.json");
    let back = Report::read(&path).unwrap();
    assert_eq!(back, r);
    assert_eq!(Report::read_dir(dir.path()), vec![r]);
}
