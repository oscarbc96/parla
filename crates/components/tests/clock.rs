mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use components::thread_summary::relative_time;
use components::{Motion, NowGlobal, motion, now_secs};
use domain::FixedNow;
use gpui::TestAppContext;
use jiff::Timestamp;

#[gpui::test]
fn relative_time_uses_now_global(cx: &mut TestAppContext) {
    let fixed = Timestamp::from_second(1_700_000_000).unwrap();
    cx.update(|cx| {
        cx.set_global(NowGlobal(Arc::new(FixedNow(fixed))));
        let now = now_secs(cx);
        assert_eq!(now, 1_700_000_000);
        assert_eq!(
            relative_time(now - 300, now, &common::en()),
            "5 minutes ago"
        );
    });
}

#[gpui::test]
fn defaults_are_system_clock_and_live_motion(cx: &mut TestAppContext) {
    cx.update(|cx| {
        assert_eq!(motion(cx), Motion::Live);
        assert!(now_secs(cx) > 1_600_000_000);
    });
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn no_wall_clock_in_ui_code() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    rs_files(&root.join("components/src"), &mut files);
    rs_files(&root.join("parla/src"), &mut files);
    let mut hits = Vec::new();
    for f in files {
        if f.file_name().is_some_and(|n| n == "clock.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&f).unwrap();
        for (i, line) in text.lines().enumerate() {
            if line.contains("SystemTime::now") || line.contains("Timestamp::now") {
                hits.push(format!("{}:{}", f.display(), i + 1));
            }
        }
    }
    assert!(hits.is_empty(), "wall clock read in UI code: {hits:#?}");
}
