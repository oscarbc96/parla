//! The redacted diagnostics export (Settings, Advanced): one text file with
//! the Parla version, the operating system, the settings with anything
//! personal removed, the last log lines, the performance counters and the
//! newest local crash report, for a bug report. Nothing is sent anywhere; the file is written next to the log
//! and shown.
//!
//! What it never holds: message text (the log does not carry it unless
//! `log.include_content` is on, and the export drops every log line then),
//! names, tokens and ids. Every string goes through the `redact` engine with
//! the registered providers' rules, and settings values that can name
//! something (lists, device names, workspace keys) are reduced to counts.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use redact::{IdMap, Redactor, RuleSet};
use serde_json::Value;

/// Log lines the export keeps.
pub const LOG_LINES: usize = 500;

/// Exports kept in the folder; older ones are deleted when a new one is written.
const KEEP: usize = 5;

/// Folder inside the data dir.
pub const DIR: &str = "diagnostics";

/// Settings keys whose value names a device, a folder, a zone or a release:
/// shown as `[set]` or `null`.
const NAMING_KEYS: &[&str] = &[
    "input_device",
    "output_device",
    "timezone",
    "folder",
    "skip",
    "$schema",
];

/// What the export is made of, before redaction.
pub struct Inputs<'a> {
    pub version: &'a str,
    pub os: &'a str,
    pub settings: &'a settings::Settings,
    pub log_lines: Vec<String>,
    /// Name and value of each performance counter.
    pub counters: Vec<(String, String)>,
    /// The home folder and the data folder: both become `~` in the report.
    pub home: Option<&'a Path>,
    pub data_dir: &'a Path,
    /// How many crash reports are saved, and the newest one's text.
    pub crashes: (usize, Option<String>),
}

/// The settings as JSON with secrets and names removed: arrays and the
/// workspace policy map and the plugin entries become counts, naming keys
/// become `[set]`.
pub fn settings_value(s: &settings::Settings) -> Value {
    let mut v = serde_json::to_value(s).unwrap_or(Value::Null);
    scrub_value(None, &mut v);
    if let Some(p) = v.get_mut("plugins") {
        *p = Value::String(format!("[{} plugins]", s.plugins.entries.len()));
    }
    v
}

fn scrub_value(key: Option<&str>, v: &mut Value) {
    match v {
        Value::Array(items) => *v = Value::String(format!("[{} items]", items.len())),
        Value::Object(map) if key == Some("workspaces") => {
            *v = Value::String(format!("[{} workspaces]", map.len()));
        }
        Value::Object(map) => {
            for (k, child) in map.iter_mut() {
                scrub_value(Some(k), child);
            }
        }
        Value::String(_) if key.is_some_and(|k| NAMING_KEYS.contains(&k)) => {
            *v = Value::String("[set]".into());
        }
        _ => {}
    }
}

/// The report text. Every line of free text runs through the redactor.
pub fn render(inputs: &Inputs<'_>, rules: Arc<dyn RuleSet>) -> String {
    let mut redactor = Redactor::new(IdMap::new(), rules);
    let mut out = String::new();
    out.push_str("Parla diagnostics\n");
    out.push_str("Tokens, cookies, ids, email addresses and workspace hosts are removed; message text is never included.\n\n");
    out.push_str(&format!(
        "version: {}\n",
        redactor.redact_str(inputs.version)
    ));
    out.push_str(&format!("os: {}\n\n", redactor.redact_str(inputs.os)));
    out.push_str("settings:\n");
    let settings = serde_json::to_string_pretty(&settings_value(inputs.settings))
        .unwrap_or_else(|_| "{}".into());
    out.push_str(&redactor.redact_str(&settings));
    out.push_str("\n\ncounters:\n");
    for (name, value) in &inputs.counters {
        out.push_str(&format!("  {name}: {}\n", redactor.redact_str(value)));
    }
    out.push_str(&format!("\ncrash reports: {}\n", inputs.crashes.0));
    if let Some(newest) = &inputs.crashes.1 {
        out.push_str("newest crash report:\n");
        out.push_str(&redactor.redact_str(&crate::log::scrub(newest)));
        out.push('\n');
    }
    out.push_str(&format!("\nlog (last {} lines):\n", inputs.log_lines.len()));
    for line in &inputs.log_lines {
        let line = crate::log::scrub(line);
        out.push_str(&redactor.redact_str(&line));
        out.push('\n');
    }
    strip_paths(out, inputs.data_dir, inputs.home)
}

/// Replaces the data folder, then the home folder, with `~`, so a user name in
/// a path never leaves the machine.
fn strip_paths(mut text: String, data_dir: &Path, home: Option<&Path>) -> String {
    for dir in [Some(data_dir), home].into_iter().flatten() {
        let dir = dir.to_string_lossy();
        if dir.len() > 1 {
            text = text.replace(dir.as_ref(), "~");
        }
    }
    text
}

/// The counters of the running process.
pub fn counters() -> Vec<(String, String)> {
    use crate::process_stats::sample;
    let mut out = Vec::new();
    if let Some(s) = sample() {
        out.push(("memory_mb".into(), format!("{:.1}", s.memory_mb)));
        out.push(("cpu_seconds".into(), format!("{:.1}", s.cpu_s)));
        out.push(("idle_wakeups".into(), format!("{:.0}", s.wakeups)));
    }
    // The frame and allocation counters belong to the perf harness build.
    #[cfg(feature = "fixture")]
    {
        out.push((
            "frames_drawn".into(),
            crate::perf::frames_drawn().to_string(),
        ));
        out.push((
            "input_frames".into(),
            crate::perf::input_frames().to_string(),
        ));
        out.push(("allocations".into(), crate::perf::allocations().to_string()));
        out.push((
            "live_heap_bytes".into(),
            crate::perf::live_heap().to_string(),
        ));
    }
    out
}

/// Writes the report to `<data_dir>/diagnostics/parla-diagnostics-<unix>.txt`
/// (mode 0600), keeping the newest few, and returns its path.
pub fn export(
    data_dir: &Path,
    settings: &settings::Settings,
    now_unix: i64,
    rules: Arc<dyn RuleSet>,
) -> std::io::Result<PathBuf> {
    // `log.include_content` lets message text into some lines: drop the log.
    let log_lines = if settings.log.include_content {
        vec!["(omitted: log.include_content is on)".to_string()]
    } else {
        crate::log::tail(data_dir, LOG_LINES)
    };
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let text = render(
        &Inputs {
            version: env!("CARGO_PKG_VERSION"),
            os: &format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            settings,
            log_lines,
            counters: counters(),
            home: home.as_deref(),
            data_dir,
            crashes: crate::crash::newest_report(&crate::crash::dir(data_dir)),
        },
        rules,
    );
    let dir = data_dir.join(DIR);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("parla-diagnostics-{now_unix}.txt"));
    redact::write_private(&path, text.as_bytes())?;
    prune(&dir);
    Ok(path)
}

/// Deletes all but the newest [`KEEP`] exports (names sort by time).
fn prune(dir: &Path) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("parla-diagnostics-"))
        })
        .collect();
    files.sort();
    let extra = files.len().saturating_sub(KEEP);
    for old in files.into_iter().take(extra) {
        let _ = std::fs::remove_file(old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redact::FakeRules;

    fn rules() -> Arc<dyn RuleSet> {
        Arc::new(FakeRules::default())
    }

    fn sample_settings() -> settings::Settings {
        let mut s = settings::Settings::default();
        s.notifications.keywords = vec!["hushhush-keyword".into()];
        s.plugins.dev_paths = vec!["/Users/x/code/my-secret-plugin".into()];
        s.plugins.entries.insert(
            "my-secret-plugin".into(),
            settings::PluginEntry {
                config: serde_json::json!({"k": 1}).as_object().unwrap().clone(),
                ..Default::default()
            },
        );
        s.calls.input_device = Some("Jo's AirPods Pro".into());
        s.mcp.workspaces.insert(
            "fake/A1234567890".into(),
            settings::McpWorkspaceSettings {
                allow_channels: vec!["C0123456789".into()],
                ..Default::default()
            },
        );
        s
    }

    #[test]
    fn the_report_holds_no_token_id_email_host_or_naming_setting() {
        let s = sample_settings();
        let text = render(
            &Inputs {
                version: "1.2.3",
                os: "macos aarch64",
                settings: &s,
                log_lines: vec![
                    "1700000000 INFO opened C0123456789 with fake-abc-123 for jo@corp.test".into(),
                    "1700000001 ERROR https://acme.chat.example.test/x failed".into(),
                    "1700000002 INFO token=plainsecret99 sent".into(),
                    "1700000003 INFO store at /Users/x/Library/Application Support/parla/store/a.db and /Users/x/Documents".into(),
                ],
                counters: vec![("memory_mb".into(), "42.0".into())],
                home: Some(Path::new("/Users/x")),
                data_dir: Path::new("/Users/x/Library/Application Support/parla"),
                crashes: (0, None),
            },
            rules(),
        );
        for leaked in [
            "fake-abc-123",
            "C0123456789",
            "A1234567890",
            "jo@corp.test",
            "acme",
            "plainsecret99",
            "hushhush-keyword",
            "AirPods",
            "/Users/x",
            "my-secret-plugin",
        ] {
            assert!(!text.contains(leaked), "{leaked} leaked:\n{text}");
        }
        assert!(text.contains("version: 1.2.3"), "{text}");
        assert!(text.contains("os: macos aarch64"), "{text}");
        assert!(text.contains("memory_mb: 42.0"), "{text}");
        assert!(text.contains("\"level\": \"info\""), "{text}");
        assert!(text.contains("[1 items]"), "{text}");
        assert!(text.contains("[1 workspaces]"), "{text}");
        assert!(text.contains("[1 plugins]"), "{text}");
        assert!(
            text.contains("store at ~/store/a.db and ~/Documents"),
            "{text}"
        );
        assert!(text.contains("opened") && text.contains("failed"), "{text}");
    }

    #[test]
    fn export_writes_a_private_file_keeps_the_newest_and_omits_content_logs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            crate::log::path_in(dir.path()),
            "1 INFO message text: the secret plan\n",
        )
        .unwrap();
        let mut s = settings::Settings::default();
        let first = export(dir.path(), &s, 100, rules()).unwrap();
        let text = std::fs::read_to_string(&first).unwrap();
        assert!(text.contains("message text"), "{text}");
        for t in 101..110 {
            export(dir.path(), &s, t, rules()).unwrap();
        }
        assert!(!first.exists(), "the oldest export is pruned");
        assert_eq!(
            std::fs::read_dir(dir.path().join(DIR)).unwrap().count(),
            KEEP
        );
        s.log.include_content = true;
        let p = export(dir.path(), &s, 200, rules()).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("secret plan"), "{text}");
        assert!(text.contains("log.include_content is on"), "{text}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn export_carries_the_newest_crash_report_redacted() {
        let dir = tempfile::tempdir().unwrap();
        let crashes = crate::crash::dir(dir.path());
        std::fs::create_dir_all(&crashes).unwrap();
        std::fs::write(
            crashes.join("2026-01-01T00-00-00Z-0000.txt"),
            "panic: older one\n",
        )
        .unwrap();
        std::fs::write(
            crashes.join("2026-01-02T00-00-00Z-0000.txt"),
            "panic: newest boom token=plainsecret99\n",
        )
        .unwrap();
        let p = export(dir.path(), &settings::Settings::default(), 100, rules()).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("crash reports: 2"), "{text}");
        assert!(text.contains("newest boom"), "{text}");
        assert!(!text.contains("older one"), "{text}");
        assert!(!text.contains("plainsecret99"), "{text}");
    }
}
