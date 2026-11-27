//! Scenario reports (`target/perf/<scenario>.json`) and the process
//! self-sampler behind them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What one scenario run measured. Metrics are lower-is-better numbers such
/// as `mem_mb`, `cpu_pct`, `wakeups_per_s`, `frame_p95_ms`, `latency_ms`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub scenario: String,
    pub commit: String,
    pub os: String,
    pub machine: String,
    pub seconds: f64,
    pub metrics: BTreeMap<String, f64>,
}

impl Report {
    /// Writes `<dir>/<scenario>.json` and returns its path.
    pub fn write(&self, dir: &Path) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("{}.json", self.scenario));
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(&path, json + "\n")?;
        Ok(path)
    }

    pub fn read(path: &Path) -> std::io::Result<Report> {
        serde_json::from_slice(&std::fs::read(path)?).map_err(std::io::Error::other)
    }

    /// Every `*.json` report directly in `dir`, ordered by scenario name.
    pub fn read_dir(dir: &Path) -> Vec<Report> {
        let mut out: Vec<Report> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .filter_map(|p| Report::read(&p).ok())
            .collect();
        out.sort_by(|a, b| a.scenario.cmp(&b.scenario));
        out
    }
}

pub use crate::process_stats::{Sample, sample};

/// `p` in 0..=100 of `values` (nearest rank); 0 when empty.
pub fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    let rank = ((p / 100.0) * v.len() as f64).ceil().max(1.0) as usize;
    v[rank.min(v.len()) - 1]
}

/// Share (0..=100) of frame intervals longer than 1.5 display refresh periods
/// (`refresh_ms`): such a frame missed at least one vsync. Where the refresh
/// period is unknown, the median interval stands in for it, which is the
/// vsync period only of a run that draws every vsync. Only meaningful for
/// frames requested back to back (a fling); an interval that includes an idle
/// gap is not a drop.
pub fn dropped_pct(intervals_ms: &[f64], refresh_ms: Option<f64>) -> f64 {
    if intervals_ms.is_empty() {
        return 0.0;
    }
    let period = refresh_ms.unwrap_or_else(|| percentile(intervals_ms, 50.0));
    share_over(intervals_ms, period * 1.5)
}

/// Share (0..=100) of input events whose frame started more than 1.5 display
/// refresh periods after the event (`latencies_ms`): the event missed the
/// vsync after it. Frame intervals cannot judge input-driven frames, because
/// input slower than the display spaces frames out without any frame being
/// late. Assumes 60 Hz where the refresh period is unknown.
pub fn late_pct(latencies_ms: &[f64], refresh_ms: Option<f64>) -> f64 {
    share_over(latencies_ms, refresh_ms.unwrap_or(1000.0 / 60.0) * 1.5)
}

fn share_over(values: &[f64], limit: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let over = values.iter().filter(|v| **v > limit).count();
    over as f64 / values.len() as f64 * 100.0
}

#[cfg(test)]
mod dropped_tests {
    use super::{dropped_pct, late_pct};

    #[test]
    fn counts_intervals_over_one_and_a_half_medians() {
        let mut v = vec![8.3; 96];
        v.extend([16.7, 16.7, 25.0, 12.4]);
        assert!((dropped_pct(&v, None) - 3.0).abs() < 1e-9);
        assert_eq!(dropped_pct(&[], None), 0.0);
        assert_eq!(dropped_pct(&[8.3; 50], None), 0.0);
    }

    #[test]
    fn judges_intervals_against_the_display_refresh_when_known() {
        // A run that draws every other vsync of a 120 Hz display: the median
        // is 16.7 ms, so a median rule sees no drops, but every frame missed
        // one vsync.
        let v = vec![16.7; 100];
        assert_eq!(dropped_pct(&v, None), 0.0);
        assert_eq!(dropped_pct(&v, Some(1000.0 / 120.0)), 100.0);
        assert_eq!(dropped_pct(&[8.3; 50], Some(1000.0 / 120.0)), 0.0);
    }

    #[test]
    fn late_events_missed_the_vsync_after_them() {
        let refresh = 1000.0 / 120.0;
        // Input slower than the display (one event every 9.4 ms or 16.6 ms)
        // spaces frames out, but each event is on the next vsync: none late.
        assert_eq!(late_pct(&[0.1, 4.0, 8.0, 3.3, 7.9], Some(refresh)), 0.0);
        // An event that waits past the next vsync (a link restart) is late.
        assert!((late_pct(&[1.0, 13.0, 2.0, 13.4], Some(refresh)) - 50.0).abs() < 1e-9);
        // Without a known refresh rate assume 60 Hz.
        assert_eq!(late_pct(&[13.0], None), 0.0);
        assert_eq!(late_pct(&[], Some(refresh)), 0.0);
    }
}
