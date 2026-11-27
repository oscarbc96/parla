//! Local crash reports: a panic hook writes a redacted report on disk and the
//! next launch offers to show or delete it. Reports never leave the machine.
//! The `crash_reports` setting picks the mode: `off` writes nothing, `ask`
//! (the default) writes the report and asks at the next launch, `local`
//! writes it without asking. Export diagnostics carries the newest one.

use std::backtrace::Backtrace;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use gpui::App;
use redact::{Redactor, RuleSet};

static INSTALLED: OnceLock<()> = OnceLock::new();
static SEQ: AtomicU64 = AtomicU64::new(0);
/// The `crash_reports` mode the hook reads when a panic happens. Until the
/// settings are read it is unset and the hook writes nothing, so `off` is
/// honoured from the first instruction.
static MODE: AtomicU8 = AtomicU8::new(MODE_UNSET);
const MODE_UNSET: u8 = u8::MAX;
const MODE_OFF: u8 = 0;
const MODE_ASK: u8 = 1;
const MODE_LOCAL: u8 = 2;

/// Sets the mode the panic hook follows (at startup and on every settings
/// change).
pub fn set_mode(mode: settings::CrashReports) {
    let v = match mode {
        settings::CrashReports::Off => MODE_OFF,
        settings::CrashReports::Ask => MODE_ASK,
        settings::CrashReports::Local => MODE_LOCAL,
    };
    MODE.store(v, Ordering::Relaxed);
}

const MARKER: &str = ".pending";

/// The `<data_dir>/crashes` folder.
pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("crashes")
}

fn marker_path(dir: &Path) -> PathBuf {
    dir.join(MARKER)
}

fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir)
    }
}

/// Installs the panic hook that writes crash reports under `dir`.
/// Prunes old reports at startup and chains the previous hook.
pub fn install_panic_hook(dir: PathBuf, rules: Arc<dyn RuleSet>) {
    let _ = INSTALLED.get_or_init(|| {
        let _ = ensure_dir(&dir);
        prune(&dir, 20);
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let mode = MODE.load(Ordering::Relaxed);
            if mode == MODE_ASK || mode == MODE_LOCAL {
                let _ = write_report(&dir, &rules, info, mode == MODE_ASK);
            }
            prev(info);
        }));
    });
}

/// Keeps at most `keep` `.txt` report files in `dir`, removing the oldest first.
pub fn prune(dir: &Path, keep: usize) {
    let mut reports: Vec<(String, PathBuf)> = match fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let path = e.path();
                let name = path.file_name()?.to_str()?.to_string();
                (path.extension().is_some_and(|x| x == "txt")).then_some((name, path))
            })
            .collect(),
        Err(_) => return,
    };
    if reports.len() <= keep {
        return;
    }
    reports.sort_by(|a, b| a.0.cmp(&b.0));
    let remove = reports.len().saturating_sub(keep);
    for (_, path) in reports.into_iter().take(remove) {
        let _ = fs::remove_file(path);
    }
}

fn write_report(
    dir: &Path,
    rules: &Arc<dyn RuleSet>,
    info: &std::panic::PanicHookInfo<'_>,
    ask: bool,
) -> std::io::Result<()> {
    let _ = ensure_dir(dir);
    let thread = std::thread::current()
        .name()
        .unwrap_or("<unnamed>")
        .to_string();
    let payload = skeleton(&panic_payload(info));
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "<unknown>".into());
    let backtrace = Backtrace::force_capture();
    let body = format!(
        "version: {}\nos: {} {}\nthread: {thread}\nlocation: {location}\npanic: {payload}\n\nbacktrace:\n{backtrace}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    let mut redactor = Redactor::new(redact::IdMap::new(), Arc::clone(rules));
    let redacted = redactor.redact_str(&body);
    let scrubbed = crate::log::scrub(&redacted);

    let stamp = timestamp();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let name = format!("{stamp}-{seq:04}.txt");
    let path = dir.join(name);
    write_private(&path, scrubbed.as_bytes())?;
    // marker: written after the report so a shown notice means a report exists
    if ask {
        let _ = fs::write(marker_path(dir), b"");
    }
    Ok(())
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut f = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::write(path, bytes)
    }
}

fn panic_payload(info: &std::panic::PanicHookInfo<'_>) -> String {
    if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic>".to_string()
    }
}

fn timestamp() -> String {
    // UTC, filesystem-safe, sortable.
    let now = domain::Now::now(&domain::SystemNow);
    now.strftime("%Y-%m-%dT%H-%M-%SZ").to_string()
}

/// Longest panic message a report keeps, in characters.
const MESSAGE_CHARS: usize = 300;

/// The panic message without any string it quotes: spans in double quotes,
/// single quotes or backticks become `…`, so names, message text or paths in
/// a formatted value never reach the report; the rest (the message skeleton)
/// stays, cut to [`MESSAGE_CHARS`] characters.
pub fn skeleton(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len().min(MESSAGE_CHARS * 4));
    let mut open: Option<char> = None;
    let mut count = 0;
    for c in msg.chars() {
        if count >= MESSAGE_CHARS {
            out.push('…');
            break;
        }
        match open {
            Some(q) if c == q => {
                out.push('…');
                out.push(c);
                count += 2;
                open = None;
            }
            Some(_) => {}
            None if matches!(c, '"' | '\'' | '`') => {
                out.push(c);
                count += 1;
                open = Some(c);
            }
            None => {
                out.push(c);
                count += 1;
            }
        }
    }
    if open.is_some() && count < MESSAGE_CHARS {
        out.push('…');
    }
    out
}

/// Shows the "Parla quit unexpectedly" notice when a previous crash left a marker,
/// and removes the marker once shown. With `crash_reports` off it only
/// removes the marker.
pub fn show_pending_notice(dir: &Path, cx: &mut App) {
    let marker = marker_path(dir);
    if fs::remove_file(&marker).is_err() {
        return;
    }
    if MODE.load(Ordering::Relaxed) == MODE_OFF {
        return;
    }
    let l = i18n::gpui::localizer(cx);
    let text = t!(l, "crash-notice").into_owned();
    let show = t!(l, "crash-show").into_owned();
    let delete = t!(l, "crash-delete").into_owned();
    let banner = cx.global_mut::<crate::toast::ConfigBanner>();
    banner.set_crash_notice(text, show, delete, dir.to_path_buf());
}

/// The `.txt` reports in `dir`, oldest first (names sort by time).
fn report_paths(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "txt"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// How many reports `dir` holds and the text of the newest (already
/// redacted when it was written).
pub fn newest_report(dir: &Path) -> (usize, Option<String>) {
    let paths = report_paths(dir);
    let newest = paths.last().and_then(|p| fs::read_to_string(p).ok());
    (paths.len(), newest)
}

/// Deletes every saved report and the pending marker.
pub fn delete_reports(dir: &Path) {
    for p in report_paths(dir) {
        let _ = fs::remove_file(p);
    }
    let _ = fs::remove_file(marker_path(dir));
}

/// Opens `path` in the system file manager.
pub fn open_dir(path: &Path) {
    let _ = fs::create_dir_all(path);
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

/// Registers `app::ShowCrashReports`.
pub fn install_actions(cx: &mut App) {
    cx.on_action(|_: &crate::actions::ShowCrashReports, _cx| {
        open_dir(&dir(&settings::paths::data_dir()));
    });
}
