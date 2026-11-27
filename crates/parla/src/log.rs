//! A small diagnostic log: one line per event in `<data_dir>/parla.log` (mode
//! 0600, rotated to `parla.log.1` past 1 MiB), also echoed to stderr when
//! stderr is still there (a failed echo is ignored, never a panic). Every
//! line is scrubbed of tokens and cookies before it is written. Until
//! [`init`] runs (tests) logging does nothing.

use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

use settings::LogLevel;

/// File name inside the data dir.
pub const FILE_NAME: &str = "parla.log";

const MAX_BYTES: u64 = 1 << 20;

/// The open log file and the bytes it holds, so a long session rotates too.
struct Sink {
    file: File,
    dir: PathBuf,
    len: u64,
    max: u64,
}

fn open_log(path: &Path) -> std::io::Result<File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    opts.open(path)
}

impl Sink {
    /// Opens `<dir>/parla.log`, rotating it first when it is already past `max`.
    fn open(dir: &Path, max: u64) -> Option<Sink> {
        let path = path_in(dir);
        let len = std::fs::metadata(&path).map_or(0, |m| m.len());
        let mut sink = Sink {
            file: open_log(&path).ok()?,
            dir: dir.to_path_buf(),
            len,
            max,
        };
        if len > max {
            sink.rotate();
        }
        Some(sink)
    }

    /// Moves the log to `parla.log.1` and starts an empty one. On failure the
    /// current file keeps being written.
    fn rotate(&mut self) {
        let path = path_in(&self.dir);
        let _ = std::fs::rename(&path, self.dir.join(format!("{FILE_NAME}.1")));
        if let Ok(file) = open_log(&path) {
            self.file = file;
            self.len = 0;
        }
    }

    fn write_line(&mut self, text: &str) {
        let line = format!("{text}\n");
        if self.len > 0 && self.len + line.len() as u64 > self.max {
            self.rotate();
        }
        if self.file.write_all(line.as_bytes()).is_ok() {
            self.len += line.len() as u64;
        }
    }
}

static SINK: OnceLock<Mutex<Sink>> = OnceLock::new();

/// The `log.level` setting in force, as a [`LogLevel`] index. Info until the
/// settings are read.
static LEVEL: AtomicU8 = AtomicU8::new(1);

/// Applies the `log.level` setting to the next line written.
pub fn set_level(level: LogLevel) {
    LEVEL.store(level as u8, Ordering::Relaxed);
}

fn current_level() -> LogLevel {
    match LEVEL.load(Ordering::Relaxed) {
        0 => LogLevel::Error,
        2 => LogLevel::Debug,
        _ => LogLevel::Info,
    }
}

/// Whether a line of `line_level` (`ERROR`, `WARN`, `INFO`, `DEBUG`) is
/// written at `level`. A level name this does not know is always written.
pub fn allows(level: LogLevel, line_level: &str) -> bool {
    match line_level {
        "DEBUG" => level == LogLevel::Debug,
        "INFO" | "WARN" => level != LogLevel::Error,
        _ => true,
    }
}

/// [`allows`] for the level in force.
pub fn enabled(line_level: &str) -> bool {
    allows(current_level(), line_level)
}

/// The last `n` lines of the log (the rotated file first, then the current
/// one), oldest first. Empty when there is no log.
pub fn tail(data_dir: &Path, n: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for path in [data_dir.join(format!("{FILE_NAME}.1")), path_in(data_dir)] {
        if let Ok(text) = std::fs::read_to_string(path) {
            lines.extend(text.lines().map(str::to_string));
        }
    }
    let skip = lines.len().saturating_sub(n);
    lines.split_off(skip)
}

/// Where the log lives for a data dir.
pub fn path_in(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

/// Opens the log (once). Failure to open leaves logging off, never fatal.
pub fn init(data_dir: &Path) {
    let _ = std::fs::create_dir_all(data_dir);
    if let Some(sink) = Sink::open(data_dir, MAX_BYTES) {
        let _ = SINK.set(Mutex::new(sink));
    }
    // Lets the provider crates log timing lines through the `log` facade.
    if log::set_logger(&FACADE).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

/// Target prefixes (`log` record targets, i.e. crate paths) whose INFO and
/// above records reach the log file. Empty until a provider registers its own.
static FORWARDED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Lets records of crates whose target starts with one of `prefixes` through
/// the `log` facade into the log file; every other target is dropped.
pub fn add_forwarded_targets(prefixes: &[&str]) {
    if let Ok(mut f) = FORWARDED.lock() {
        f.extend(prefixes.iter().map(|p| (*p).to_string()));
    }
}

fn forwards(target: &str) -> bool {
    FORWARDED
        .lock()
        .is_ok_and(|f| f.iter().any(|p| target.starts_with(p.as_str())))
}

/// The line a `log` record becomes: its level name and scrubbed text, or
/// `None` when its target is not forwarded or it is more detailed than INFO.
fn record_line(
    target: &str,
    level: log::Level,
    args: &std::fmt::Arguments<'_>,
) -> Option<(&'static str, String)> {
    if level > log::Level::Info || !forwards(target) {
        return None;
    }
    let name = match level {
        log::Level::Error => "ERROR",
        log::Level::Warn => "WARN",
        _ => "INFO",
    };
    Some((name, scrub(&args.to_string())))
}

struct Facade;

static FACADE: Facade = Facade;

impl log::Log for Facade {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info && forwards(m.target())
    }

    fn log(&self, r: &log::Record) {
        if let Some((level, text)) = record_line(r.target(), r.level(), r.args()) {
            line(level, &text);
        }
    }

    fn flush(&self) {}
}

/// Writes one line at `level` (`INFO`, `ERROR`) after scrubbing secrets.
pub fn line(level: &str, msg: &str) {
    if !enabled(level) {
        return;
    }
    let text = scrub(msg);
    if let Some(sink) = SINK.get()
        && let Ok(mut s) = sink.lock()
    {
        let now = domain::Now::now(&domain::SystemNow).as_second();
        s.write_line(&format!("{now} {level} {text}"));
    }
    stderr_line(&format!("parla {level}: {text}"));
}

/// Writes `text` to stderr, ignoring failure. `eprintln!` panics when stderr
/// is gone (a closed pipe once the launching script or terminal exits), which
/// aborts the app; a diagnostic line must never do that.
pub fn stderr_line(text: &str) {
    let _ = writeln!(std::io::stderr(), "{text}");
}

pub fn debug(msg: &str) {
    line("DEBUG", msg);
}

pub fn info(msg: &str) {
    line("INFO", msg);
}

pub fn warn(msg: &str) {
    line("WARN", msg);
}

pub fn error(msg: &str) {
    line("ERROR", msg);
}

const MASK: &str = "[redacted]";

fn is_secret_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '%' | '.' | '/' | '+' | '=')
}

/// Removes workspace tokens (`xox*-...`) and the values of `token=`,
/// `cookie=`, `d=` and `Bearer` pairs. Never panics on odd input.
pub fn scrub(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut prev_word = false;
    while !rest.is_empty() {
        if let Some(n) = (!prev_word).then(|| secret_len(rest)).flatten() {
            out.push_str(MASK);
            rest = &rest[n..];
        } else {
            let c = rest.chars().next().expect("non-empty");
            out.push(c);
            prev_word = c.is_alphanumeric() || c == '_';
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// Length of a secret starting at the beginning of `s`, if one does.
fn secret_len(s: &str) -> Option<usize> {
    if s.starts_with("xox") {
        let n = s.find(|c: char| !is_secret_char(c)).unwrap_or(s.len());
        return (n > 4).then_some(n);
    }
    for key in ["token=", "cookie=", "d=", "Bearer "] {
        if let Some(after) = s.strip_prefix(key) {
            let n = after
                .find(|c: char| !is_secret_char(c))
                .unwrap_or(after.len());
            return (n > 0).then_some(key.len() + n);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_level_decides_which_lines_are_written() {
        use settings::LogLevel::{Debug, Error, Info};
        assert!(allows(Error, "ERROR") && !allows(Error, "WARN") && !allows(Error, "INFO"));
        assert!(allows(Info, "ERROR") && allows(Info, "WARN") && allows(Info, "INFO"));
        assert!(!allows(Info, "DEBUG"));
        assert!(allows(Debug, "DEBUG") && allows(Debug, "INFO"));
        assert!(
            allows(Error, "UNKNOWN"),
            "an unknown level is never dropped"
        );
    }

    #[test]
    fn set_level_applies_to_the_next_line() {
        set_level(settings::LogLevel::Error);
        assert!(!enabled("INFO"));
        set_level(settings::LogLevel::Debug);
        assert!(enabled("DEBUG"));
        set_level(settings::LogLevel::Info);
    }

    #[test]
    fn the_tail_returns_the_last_lines_of_both_files_in_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("parla.log.1"), "1 INFO a\n2 INFO b\n").unwrap();
        std::fs::write(path_in(dir.path()), "3 INFO c\n4 INFO d\n").unwrap();
        assert_eq!(tail(dir.path(), 3), ["2 INFO b", "3 INFO c", "4 INFO d"]);
        assert_eq!(tail(dir.path(), 10).len(), 4);
        assert!(tail(&dir.path().join("none"), 5).is_empty());
    }

    #[test]
    fn tokens_and_cookies_are_scrubbed() {
        let s = scrub("call failed: xoxc-1-2-abc with cookie=xoxd-sec%2Fret; token=abc123 ok");
        assert!(!s.contains("xox") && !s.contains("abc123"), "{s}");
        assert!(s.contains("call failed") && s.contains("ok"), "{s}");
        assert_eq!(scrub("Bearer abc.def x"), "[redacted] x");
        assert_eq!(scrub("plain text, no secrets"), "plain text, no secrets");
    }

    #[test]
    fn the_log_rotates_while_running() {
        let dir = tempfile::tempdir().unwrap();
        let mut sink = Sink::open(dir.path(), 100).unwrap();
        for i in 0..20 {
            sink.write_line(&format!("line number {i}"));
        }
        let current = std::fs::metadata(path_in(dir.path())).unwrap().len();
        assert!(current <= 100, "current log is {current} bytes");
        let old = dir.path().join(format!("{FILE_NAME}.1"));
        assert!(old.exists(), "rotated file exists");
        let text = std::fs::read_to_string(path_in(dir.path())).unwrap();
        assert!(text.ends_with("line number 19\n"), "{text}");
    }

    #[test]
    fn only_forwarded_targets_reach_the_log() {
        let args = format_args!("hello token=abc123 x");
        assert!(record_line("fwd_a::m", log::Level::Info, &args).is_none());
        add_forwarded_targets(&["fwd_a"]);
        let (level, text) = record_line("fwd_a::m", log::Level::Info, &args).unwrap();
        assert_eq!(level, "INFO");
        assert!(!text.contains("abc123"), "{text}");
        assert!(text.starts_with("hello"), "{text}");
        assert!(record_line("other::m", log::Level::Info, &args).is_none());
        assert!(record_line("fwd_a::m", log::Level::Debug, &args).is_none());
        assert_eq!(
            record_line("fwd_a", log::Level::Warn, &args).unwrap().0,
            "WARN"
        );
    }

    #[test]
    fn short_lookalikes_are_kept() {
        assert_eq!(scrub("xox"), "xox");
        assert_eq!(scrub("sad=1"), "sad=1");
        assert_eq!(scrub("(d=abc)"), "([redacted])");
    }
}
