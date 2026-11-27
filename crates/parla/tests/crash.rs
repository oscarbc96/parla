use std::sync::Arc;

use gpui::TestAppContext;
use redact::{HostSuffix, RuleSet};
use regex::Regex;
use url::Url;

struct TestRules {
    tokens: Vec<Regex>,
    ids: Regex,
    suffixes: Vec<HostSuffix>,
    files: Regex,
}

impl Default for TestRules {
    fn default() -> Self {
        Self {
            tokens: vec![
                Regex::new(r"xox[a-z]-[A-Za-z0-9-]+").unwrap(),
                Regex::new(r"xoxd-[A-Za-z0-9%._=-]+").unwrap(),
            ],
            ids: Regex::new(r"\b$^").unwrap(),
            suffixes: Vec::new(),
            files: Regex::new(r"^$").unwrap(),
        }
    }
}

impl RuleSet for TestRules {
    fn id(&self) -> &str {
        "test"
    }
    fn tokens(&self) -> &[Regex] {
        &self.tokens
    }
    fn ids(&self) -> &Regex {
        &self.ids
    }
    fn host_suffixes(&self) -> &[HostSuffix] {
        &self.suffixes
    }
    fn file_hosts(&self) -> &Regex {
        &self.files
    }
    fn mention(&self) -> Option<&Regex> {
        None
    }
    fn keep_header(&self, _name: &str) -> bool {
        false
    }
    fn method_name(&self, _url: &Url) -> Option<String> {
        None
    }
}

fn test_rules() -> Arc<dyn RuleSet> {
    Arc::new(TestRules::default())
}

#[test]
fn panic_writes_report_without_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let crashes = dir.path().join("crashes");
    parla::crash::set_mode(settings::CrashReports::Ask);
    parla::crash::install_panic_hook(crashes.clone(), test_rules());
    let _ = std::panic::catch_unwind(|| {
        panic!(
            "boom xoxc-SECRET123 and cookie d=xoxd-COOKIE456 cookie=plainvalue token=plainvalue"
        );
    });
    let entries: Vec<_> = std::fs::read_dir(&crashes)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .collect();
    assert_eq!(entries.len(), 1, "one report written: {entries:?}");
    let text = std::fs::read_to_string(&entries[0]).unwrap();
    assert!(!text.contains("SECRET123"), "{text}");
    assert!(!text.contains("COOKIE456"), "{text}");
    assert!(!text.contains("xoxc-SECRET123"), "{text}");
    assert!(!text.contains("xoxd-COOKIE456"), "{text}");
    assert!(!text.contains("cookie=plainvalue"), "{text}");
    assert!(!text.contains("token=plainvalue"), "{text}");
    assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
}

#[test]
fn reports_pruned_to_20() {
    let dir = tempfile::tempdir().unwrap();
    let crashes = dir.path().join("crashes");
    std::fs::create_dir_all(&crashes).unwrap();
    for i in 0..25 {
        let name = format!("2026-01-01T00-00-{i:02}Z.txt");
        std::fs::write(crashes.join(name), "old").unwrap();
    }
    parla::crash::prune(&crashes, 20);
    let count = std::fs::read_dir(&crashes).unwrap().count();
    assert_eq!(count, 20);
}

#[cfg(unix)]
#[test]
fn report_file_is_0600() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let crashes = dir.path().join("crashes");
    parla::crash::set_mode(settings::CrashReports::Ask);
    parla::crash::install_panic_hook(crashes.clone(), test_rules());
    let _ = std::panic::catch_unwind(|| panic!("boom"));
    let dir_mode = std::fs::metadata(&crashes).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o700);
    let path = std::fs::read_dir(&crashes)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().is_some_and(|e| e == "txt"))
        .unwrap();
    let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[gpui::test]
async fn next_launch_offers_to_show(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let crashes = dir.path().join("crashes");
    std::fs::create_dir_all(&crashes).unwrap();
    // simulate marker written by a previous crash
    std::fs::write(crashes.join(".pending"), "").unwrap();
    parla::crash::set_mode(settings::CrashReports::Ask);
    cx.update(|cx| {
        i18n::gpui::init(
            cx,
            &[parla::catalog()],
            i18n::LocalePref::Code("en".parse().unwrap()),
        );
        cx.set_global(parla::ConfigBanner::default());
        parla::crash::show_pending_notice(&crashes, cx);
    });
    // marker removed once shown
    assert!(!crashes.join(".pending").exists());
    cx.update(|cx| {
        let b = cx.global::<parla::ConfigBanner>();
        assert!(b.line().is_some() || !b.notices.is_empty());
    });
}

fn reports(crashes: &std::path::Path) -> usize {
    std::fs::read_dir(crashes)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|x| x == "txt"))
                .count()
        })
        .unwrap_or(0)
}

#[test]
fn off_mode_writes_no_report() {
    let dir = tempfile::tempdir().unwrap();
    let crashes = dir.path().join("crashes");
    parla::crash::set_mode(settings::CrashReports::Off);
    parla::crash::install_panic_hook(crashes.clone(), test_rules());
    let _ = std::panic::catch_unwind(|| panic!("boom"));
    assert_eq!(reports(&crashes), 0);
    assert!(!crashes.join(".pending").exists());
}

#[test]
fn local_mode_keeps_the_report_without_asking_next_launch() {
    let dir = tempfile::tempdir().unwrap();
    let crashes = dir.path().join("crashes");
    parla::crash::set_mode(settings::CrashReports::Local);
    parla::crash::install_panic_hook(crashes.clone(), test_rules());
    let _ = std::panic::catch_unwind(|| panic!("boom"));
    assert_eq!(reports(&crashes), 1);
    assert!(!crashes.join(".pending").exists(), "no notice next launch");
}

#[gpui::test]
async fn ask_mode_offers_show_and_delete(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let crashes = dir.path().join("crashes");
    parla::crash::set_mode(settings::CrashReports::Ask);
    parla::crash::install_panic_hook(crashes.clone(), test_rules());
    let _ = std::panic::catch_unwind(|| panic!("boom"));
    assert_eq!(reports(&crashes), 1);
    assert!(crashes.join(".pending").exists(), "asks next launch");
    cx.update(|cx| {
        i18n::gpui::init(
            cx,
            &[parla::catalog()],
            i18n::LocalePref::Code("en".parse().unwrap()),
        );
        cx.set_global(parla::ConfigBanner::default());
        parla::crash::show_pending_notice(&crashes, cx);
        let notice = cx.global::<parla::ConfigBanner>().crash.clone().unwrap();
        assert_eq!(notice.delete_label, "Delete");
    });
    parla::crash::delete_reports(&crashes);
    assert_eq!(reports(&crashes), 0, "Delete removes the saved reports");
}

#[test]
fn the_hook_writes_nothing_until_the_mode_is_set() {
    let dir = tempfile::tempdir().unwrap();
    let crashes = dir.path().join("crashes");
    parla::crash::install_panic_hook(crashes.clone(), test_rules());
    let _ = std::panic::catch_unwind(|| panic!("boom before settings"));
    assert_eq!(reports(&crashes), 0, "the setting is not read yet");
}

#[gpui::test]
async fn off_shows_no_notice_for_an_old_report(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let crashes = dir.path().join("crashes");
    std::fs::create_dir_all(&crashes).unwrap();
    std::fs::write(crashes.join(".pending"), "").unwrap();
    parla::crash::set_mode(settings::CrashReports::Off);
    cx.update(|cx| {
        i18n::gpui::init(
            cx,
            &[parla::catalog()],
            i18n::LocalePref::Code("en".parse().unwrap()),
        );
        cx.set_global(parla::ConfigBanner::default());
        parla::crash::show_pending_notice(&crashes, cx);
        assert!(cx.global::<parla::ConfigBanner>().crash.is_none());
    });
    assert!(!crashes.join(".pending").exists());
}

#[test]
fn the_report_keeps_no_string_contents_from_the_panic() {
    let dir = tempfile::tempdir().unwrap();
    let crashes = dir.path().join("crashes");
    parla::crash::set_mode(settings::CrashReports::Local);
    parla::crash::install_panic_hook(crashes.clone(), test_rules());
    let _ = std::panic::catch_unwind(|| {
        panic!(
            "byte index 9 is not a char boundary; it is inside 'é' (bytes 8..10) of `meet Jordan at the café tonight` and \"private note\" {}",
            "x".repeat(2000)
        );
    });
    let path = std::fs::read_dir(&crashes)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().is_some_and(|e| e == "txt"))
        .unwrap();
    let text = std::fs::read_to_string(path).unwrap();
    assert!(
        text.contains("is not a char boundary"),
        "the skeleton stays: {text}"
    );
    assert!(text.contains("location:"), "{text}");
    for leaked in ["Jordan", "café", "private note", "é'"] {
        assert!(!text.contains(leaked), "{leaked} leaked: {text}");
    }
    let panic_line = text.lines().find(|l| l.starts_with("panic:")).unwrap();
    assert!(
        panic_line.chars().count() <= 320,
        "truncated: {}",
        panic_line.len()
    );
}
