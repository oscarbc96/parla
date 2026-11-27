//! Precompile cache: `$CACHE/parla/plugins/<id>/<version>-<engine>.cwasm`.
//!
//! - `<engine>` is [`engine_tag`](crate::engine::engine_tag): a new wasmtime
//!   version or engine config gets new files, and the old engine's files are
//!   removed the next time that plugin is compiled.
//! - Bounded: at most [`MAX_VERSIONS_PER_PLUGIN`] versions per plugin and
//!   [`MAX_FILES`] files in total, oldest (by modification time) removed. A
//!   `.cwasm` is about 2.8 times its `.wasm` (M8 A1 spike), so a 1 MB plugin
//!   costs about 3 MB here.
//! - The SHA-256 of every `.cwasm` is recorded through [`CwasmHashes`]
//!   (`app.db` from plan M8 B1). [`PrecompileCache::load`] reads the file
//!   once, checks that buffer against the hash and hands the same buffer to
//!   the engine, so a file swapped after the check is never loaded. A
//!   mismatch or a missing hash deletes the file and compiles it again.
//! - A component that imports anything the host does not provide is
//!   refused before it is compiled or cached.
//! - Compiling runs in a short-lived child process ([`Compiler`]) with a
//!   timeout, an output size cap and a capped stderr: an in-process compile
//!   of a 400 KB plugin leaves about 25 MB resident in the app (A1 spike).
//!   The app binary serves it through the hidden [`PRECOMPILE_SUBCOMMAND`],
//!   which calls [`precompile_child`]; `main.rs` wires that dispatch,
//!   together with the first caller of this cache.
//! - Compiles of one key are serialised by a process-wide lock, and each
//!   compile writes to its own temporary file, so concurrent callers get one
//!   compile and one hash.

use std::collections::HashMap;
use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

/// Versions of one plugin kept compiled (the current one and the previous
/// one, for rollback).
pub const MAX_VERSIONS_PER_PLUGIN: usize = 2;
/// `.cwasm` files kept in total.
pub const MAX_FILES: usize = 64;
/// Largest `.wasm` that is compiled.
pub const MAX_WASM_BYTES: u64 = 64 << 20;
/// Largest `.cwasm` the child may write (about 2.8 times the `.wasm`).
pub const MAX_CWASM_BYTES: u64 = 4 * MAX_WASM_BYTES;
/// How long the child may run. A 400 KB plugin compiles in about 0.2 s.
pub const COMPILE_TIMEOUT: Duration = Duration::from_secs(60);
/// Bytes of the child's stderr kept for the error message.
pub const STDERR_CAP: usize = 4096;
/// Hidden subcommand of the app binary that runs [`precompile_child`]:
/// `parla __plugin-precompile <wasm> <out>`. Dispatched by `main.rs`.
pub const PRECOMPILE_SUBCOMMAND: &str = "__plugin-precompile";
/// Set in the child's environment, so a binary can tell it was started as
/// the precompile child even if it does not read its arguments.
pub const PRECOMPILE_ENV: &str = "PARLA_PLUGIN_PRECOMPILE";

const EXT: &str = "cwasm";
/// Stripes of the per-key compile lock (fixed, so the lock table is bounded).
const LOCK_STRIPES: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("invalid plugin {what} {value:?}")]
    InvalidName { what: &'static str, value: String },
    #[error("plugin cache i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("plugin file is too large: {0} bytes")]
    TooLarge(u64),
    #[error("plugin is not a valid component: {0}")]
    Invalid(String),
    #[error("plugin imports {0}, which Parla does not provide")]
    ForbiddenImport(String),
    #[error("plugin compile failed: {0}")]
    Compile(String),
    #[error("plugin hash store: {0}")]
    Hashes(String),
}

/// Identifies one `.cwasm`: plugin id, plugin version and engine tag.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CwasmKey {
    pub id: String,
    pub version: String,
    pub engine: String,
}

impl CwasmKey {
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        engine: impl Into<String>,
    ) -> Self {
        CwasmKey {
            id: id.into(),
            version: version.into(),
            engine: engine.into(),
        }
    }
}

/// Where the SHA-256 of each `.cwasm` is kept, outside the cache directory
/// (`app.db` in the app). The cache removes a key whenever it removes the
/// file, so a store holds at most [`MAX_FILES`] entries.
pub trait CwasmHashes: Send + Sync {
    fn get(&self, key: &CwasmKey) -> Option<[u8; 32]>;
    fn put(&self, key: &CwasmKey, sha256: [u8; 32]) -> Result<(), CacheError>;
    fn remove(&self, key: &CwasmKey) -> Result<(), CacheError>;
}

/// In-memory [`CwasmHashes`] for tests and for running without `app.db`.
/// Bounded by the cache: entries are removed with their files.
#[derive(Default)]
pub struct MemoryCwasmHashes {
    map: Mutex<HashMap<CwasmKey, [u8; 32]>>,
}

impl MemoryCwasmHashes {
    fn lock(&self) -> MutexGuard<'_, HashMap<CwasmKey, [u8; 32]>> {
        self.map.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl CwasmHashes for MemoryCwasmHashes {
    fn get(&self, key: &CwasmKey) -> Option<[u8; 32]> {
        self.lock().get(key).copied()
    }

    fn put(&self, key: &CwasmKey, sha256: [u8; 32]) -> Result<(), CacheError> {
        self.lock().insert(key.clone(), sha256);
        Ok(())
    }

    fn remove(&self, key: &CwasmKey) -> Result<(), CacheError> {
        self.lock().remove(key);
        Ok(())
    }
}

/// The program that compiles a `.wasm`: run as `<program> <args..> <wasm>
/// <out>`, it writes the `.cwasm` to `<out>` and exits 0. It is killed after
/// its timeout; its output may not exceed its size cap; only the first
/// [`STDERR_CAP`] bytes of its stderr are kept. A program that exits 0
/// without writing `<out>` (an app binary that ignores the subcommand) is an
/// error, never a cached file.
#[derive(Clone, Debug)]
pub struct Compiler {
    program: PathBuf,
    args: Vec<OsString>,
    envs: Vec<(OsString, OsString)>,
    timeout: Duration,
    max_output: u64,
}

impl Compiler {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Compiler {
            program: program.into(),
            args: Vec::new(),
            envs: vec![(PRECOMPILE_ENV.into(), "1".into())],
            timeout: COMPILE_TIMEOUT,
            max_output: MAX_CWASM_BYTES,
        }
    }

    /// The running app binary with [`PRECOMPILE_SUBCOMMAND`]. Until
    /// `main.rs` dispatches it the app would ignore the
    /// argument: that run is killed at the timeout and, having written no
    /// `.cwasm`, fails the compile.
    pub fn current_exe() -> std::io::Result<Self> {
        Ok(Compiler::new(std::env::current_exe()?).args([PRECOMPILE_SUBCOMMAND]))
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    /// How long the child may run before it is killed.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Largest `.cwasm` accepted, in bytes.
    pub fn max_output(mut self, bytes: u64) -> Self {
        self.max_output = bytes;
        self
    }

    fn run(&self, wasm: &Path, out: &Path, stderr_path: &Path) -> Result<(), CacheError> {
        use wait_timeout::ChildExt;

        let stderr_file = std::fs::File::create(stderr_path)?;
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .arg(wasm)
            .arg(out)
            .envs(self.envs.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr_file);
        limit_file_size(&mut command, self.max_output.max(STDERR_CAP as u64));
        let mut child = command
            .spawn()
            .map_err(|e| CacheError::Compile(format!("{}: {e}", self.program.display())))?;
        let status = match child.wait_timeout(self.timeout)? {
            Some(status) => status,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CacheError::Compile(format!(
                    "timed out after {} ms",
                    self.timeout.as_millis()
                )));
            }
        };
        let len = std::fs::metadata(out).map_or(0, |m| m.len());
        // Past the cap the child is stopped by `SIGXFSZ` with the file cut
        // at the cap, or (elsewhere) finishes with a larger file.
        if len > self.max_output || (!status.success() && len >= self.max_output) {
            return Err(CacheError::TooLarge(len));
        }
        if !status.success() {
            let mut tail = Vec::new();
            std::fs::File::open(stderr_path)?
                .take(STDERR_CAP as u64)
                .read_to_end(&mut tail)?;
            let text = String::from_utf8_lossy(&tail);
            return Err(CacheError::Compile(format!("{status}: {}", text.trim())));
        }
        if len == 0 {
            return Err(CacheError::Compile("the compiler wrote no output".into()));
        }
        Ok(())
    }
}

/// Caps every file the child writes (unix: `RLIMIT_FSIZE`, the child gets
/// `SIGXFSZ` past it). The parent also checks the output's size.
#[cfg(unix)]
fn limit_file_size(command: &mut Command, bytes: u64) {
    use std::os::unix::process::CommandExt;
    let limit = libc::rlimit {
        rlim_cur: bytes as libc::rlim_t,
        rlim_max: bytes as libc::rlim_t,
    };
    // SAFETY: `setrlimit` is async-signal-safe and the closure touches only
    // the copied `limit`.
    unsafe {
        command.pre_exec(move || {
            if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn limit_file_size(_command: &mut Command, _bytes: u64) {}

/// Body of the hidden precompile subcommand: compiles `wasm` with the
/// plugin engine's config and writes the `.cwasm` to `out`. Runs in its own
/// short-lived process, so the compiler's memory goes when it exits.
pub fn precompile_child(wasm: &Path, out: &Path) -> Result<(), CacheError> {
    let bytes = read_wasm(wasm)?;
    let engine = wasmtime::Engine::new(&crate::engine::config())
        .map_err(|e| CacheError::Compile(format!("{e:#}")))?;
    let cwasm = engine
        .precompile_component(&bytes)
        .map_err(|e| CacheError::Compile(format!("{e:#}")))?;
    std::fs::write(out, cwasm)?;
    Ok(())
}

fn read_wasm(wasm: &Path) -> Result<Vec<u8>, CacheError> {
    let len = std::fs::metadata(wasm)?.len();
    if len > MAX_WASM_BYTES {
        return Err(CacheError::TooLarge(len));
    }
    Ok(std::fs::read(wasm)?)
}

/// The first top-level import of a component that the host does not
/// provide ([`crate::linker::import_allowed`]), read without an engine.
pub fn forbidden_import_in(wasm: &[u8]) -> Result<Option<String>, CacheError> {
    use wasmparser::{Parser, Payload};
    let mut depth = 0usize;
    let mut component = false;
    for payload in Parser::new(0).parse_all(wasm) {
        let payload = payload.map_err(|e| CacheError::Invalid(e.to_string()))?;
        match payload {
            Payload::Version { encoding, .. } => {
                depth += 1;
                if depth == 1 {
                    component = encoding == wasmparser::Encoding::Component;
                }
            }
            Payload::End(_) => depth = depth.saturating_sub(1),
            Payload::ComponentImportSection(reader) if depth == 1 => {
                for import in reader {
                    let import = import.map_err(|e| CacheError::Invalid(e.to_string()))?;
                    let name = import.name.name;
                    if !crate::linker::import_allowed(name) {
                        return Ok(Some(name.to_owned()));
                    }
                }
            }
            Payload::ImportSection(_) if depth == 1 => {
                return Err(CacheError::Invalid("a core module, not a component".into()));
            }
            _ => {}
        }
    }
    if !component {
        return Err(CacheError::Invalid("not a component".into()));
    }
    Ok(None)
}

/// Process-wide compile locks, striped by key.
fn key_lock(key: &CwasmKey) -> MutexGuard<'static, ()> {
    static LOCKS: [Mutex<()>; LOCK_STRIPES] = [const { Mutex::new(()) }; LOCK_STRIPES];
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    LOCKS[(h.finish() as usize) % LOCK_STRIPES]
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

type VerifyHook = Arc<dyn Fn(&Path) + Send + Sync>;

/// The bounded `.cwasm` cache.
pub struct PrecompileCache {
    root: PathBuf,
    compiler: Compiler,
    hashes: Arc<dyn CwasmHashes>,
    engine_tag: String,
    after_verify: Option<VerifyHook>,
}

impl PrecompileCache {
    /// `root` is `$CACHE/parla/plugins`.
    pub fn new(root: impl Into<PathBuf>, compiler: Compiler, hashes: Arc<dyn CwasmHashes>) -> Self {
        PrecompileCache {
            root: root.into(),
            compiler,
            hashes,
            engine_tag: crate::engine::engine_tag(),
            after_verify: None,
        }
    }

    /// Replaces the engine tag (tests; a tag has no `-`).
    pub fn with_engine_tag(mut self, tag: impl Into<String>) -> Self {
        self.engine_tag = tag.into();
        self
    }

    /// Test hook run after a file's hash is checked, with its path.
    #[doc(hidden)]
    pub fn after_verify(mut self, hook: impl Fn(&Path) + Send + Sync + 'static) -> Self {
        self.after_verify = Some(Arc::new(hook));
        self
    }

    pub fn engine_tag(&self) -> &str {
        &self.engine_tag
    }

    /// Path of a verified `.cwasm` for this plugin version, compiling it in
    /// the child process when it is missing, has no recorded hash or does
    /// not match it. Blocking: call it off the UI thread.
    pub fn ensure(&self, id: &str, version: &str, wasm: &Path) -> Result<PathBuf, CacheError> {
        let (key, path) = self.locate(id, version)?;
        self.load_locked(&key, &path, wasm)?;
        Ok(path)
    }

    /// The verified `.cwasm` bytes for this plugin version, compiling it
    /// first when needed. The returned buffer is the one whose hash was
    /// checked: hand it to the engine, never the path. Blocking.
    pub fn load(&self, id: &str, version: &str, wasm: &Path) -> Result<Vec<u8>, CacheError> {
        let (key, path) = self.locate(id, version)?;
        self.load_locked(&key, &path, wasm)
    }

    /// Deletes this version's `.cwasm` and its hash (for example after the
    /// engine refused to load it).
    pub fn invalidate(&self, id: &str, version: &str) -> Result<(), CacheError> {
        let (key, path) = self.locate(id, version)?;
        let _lock = key_lock(&key);
        self.remove(&key, &path)
    }

    fn load_locked(&self, key: &CwasmKey, path: &Path, wasm: &Path) -> Result<Vec<u8>, CacheError> {
        let _lock = key_lock(key);
        if let Some(bytes) = self.read_verified(key, path) {
            // Mark it recently used for the oldest-first eviction.
            let _ = std::fs::File::options()
                .write(true)
                .open(path)
                .and_then(|f| f.set_modified(SystemTime::now()));
            return Ok(bytes);
        }
        if path.exists() {
            self.remove(key, path)?;
        }
        let bytes = self.compile(key, wasm, path)?;
        self.evict(key, path);
        Ok(bytes)
    }

    fn locate(&self, id: &str, version: &str) -> Result<(CwasmKey, PathBuf), CacheError> {
        check_segment("id", id, true)?;
        check_segment("version", version, true)?;
        check_segment("engine tag", &self.engine_tag, false)?;
        let path = self
            .root
            .join(id)
            .join(format!("{version}-{}.{EXT}", self.engine_tag));
        Ok((CwasmKey::new(id, version, &self.engine_tag), path))
    }

    /// Reads the file once and returns it only if it matches its hash.
    fn read_verified(&self, key: &CwasmKey, path: &Path) -> Option<Vec<u8>> {
        let want = self.hashes.get(key)?;
        let bytes = std::fs::read(path).ok()?;
        if sha256(&bytes) != want {
            return None;
        }
        if let Some(hook) = &self.after_verify {
            hook(path);
        }
        Some(bytes)
    }

    fn remove(&self, key: &CwasmKey, path: &Path) -> Result<(), CacheError> {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        self.hashes.remove(key)
    }

    /// Compiles under the key lock. On failure nothing of this compile is
    /// left: no temporary file, no `.cwasm`, no hash.
    fn compile(&self, key: &CwasmKey, wasm: &Path, path: &Path) -> Result<Vec<u8>, CacheError> {
        let source = read_wasm(wasm)?;
        if let Some(name) = forbidden_import_in(&source)? {
            return Err(CacheError::ForbiddenImport(name));
        }
        drop(source);
        let dir = path.parent().unwrap_or(&self.root);
        std::fs::create_dir_all(dir)?;
        let tmp = unique_temp(dir, "tmp");
        let err = unique_temp(dir, "err");
        let result = (|| {
            self.compiler.run(wasm, &tmp, &err)?;
            let bytes = std::fs::read(&tmp)?;
            std::fs::rename(&tmp, path)?;
            if let Err(e) = self.hashes.put(key, sha256(&bytes)) {
                let _ = std::fs::remove_file(path);
                return Err(e);
            }
            Ok(bytes)
        })();
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::remove_file(&err);
        if result.is_err() {
            remove_dir_if_empty(dir);
        }
        result
    }

    /// Applies the bounds after `kept` was written. Best effort: a file that
    /// cannot be removed now is retried on the next compile.
    fn evict(&self, kept: &CwasmKey, kept_path: &Path) {
        // This plugin: other engines' files go; then the newest versions stay.
        let dir = self.root.join(&kept.id);
        let mut mine = Vec::new();
        for entry in cwasm_entries(&dir) {
            if entry.key.engine != self.engine_tag {
                let _ = self.remove(&entry.key, &entry.path);
            } else {
                mine.push(entry);
            }
        }
        newest_first(&mut mine);
        for entry in mine.into_iter().skip(MAX_VERSIONS_PER_PLUGIN) {
            if entry.path != kept_path {
                let _ = self.remove(&entry.key, &entry.path);
            }
        }

        // Every plugin: at most MAX_FILES, oldest first.
        let mut all: Vec<Entry> = std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .flat_map(|e| cwasm_entries(&e.path()))
            .collect();
        if all.len() <= MAX_FILES {
            return;
        }
        newest_first(&mut all);
        for entry in all.into_iter().skip(MAX_FILES) {
            if entry.path != kept_path
                && self.remove(&entry.key, &entry.path).is_ok()
                && let Some(dir) = entry.path.parent()
            {
                remove_dir_if_empty(dir);
            }
        }
    }
}

/// A temporary file name no other compile uses: process id, a process-wide
/// counter and the time.
fn unique_temp(dir: &Path, ext: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    dir.join(format!(".compile-{}-{n}-{nanos}.{ext}", std::process::id()))
}

struct Entry {
    key: CwasmKey,
    path: PathBuf,
    modified: SystemTime,
}

/// The `.cwasm` files in one plugin's directory.
fn cwasm_entries(dir: &Path) -> Vec<Entry> {
    let Some(id) = dir.file_name().and_then(|n| n.to_str()) else {
        return Vec::new();
    };
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    read.flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = path.file_name()?.to_str()?;
            let stem = name.strip_suffix(&format!(".{EXT}"))?;
            let (version, engine) = stem.rsplit_once('-')?;
            let modified = e.metadata().ok()?.modified().ok()?;
            Some(Entry {
                key: CwasmKey::new(id, version, engine),
                path,
                modified,
            })
        })
        .collect()
}

fn newest_first(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| b.path.cmp(&a.path))
    });
}

fn remove_dir_if_empty(dir: &Path) {
    // Fails, and so keeps the directory, unless it is empty.
    let _ = std::fs::remove_dir(dir);
}

/// One path segment: ASCII letters, digits, `.`, `_`, `+` and (when
/// `dash`) `-`; not empty, not starting or ending with `.`, at most 128
/// bytes, and not a Windows reserved device name (`con`, `nul`, `com1`,
/// ... with or without an extension).
fn check_segment(what: &'static str, value: &str, dash: bool) -> Result<(), CacheError> {
    let ok = !value.is_empty()
        && value.len() <= 128
        && !value.starts_with('.')
        && !value.ends_with('.')
        && value.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+') || (dash && b == b'-')
        })
        && !windows_reserved(value);
    if ok {
        Ok(())
    } else {
        Err(CacheError::InvalidName {
            what,
            value: value.to_owned(),
        })
    }
}

fn windows_reserved(value: &str) -> bool {
    let stem = value
        .split('.')
        .next()
        .unwrap_or(value)
        .to_ascii_lowercase();
    match stem.as_str() {
        "con" | "prn" | "aux" | "nul" => true,
        s if s.len() == 4 && (s.starts_with("com") || s.starts_with("lpt")) => {
            s.as_bytes()[3].is_ascii_digit()
        }
        _ => false,
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_names() {
        for name in ["con", "CON", "nul.txt", "com1", "LPT9", "aux.plugin"] {
            assert!(windows_reserved(name), "{name}");
        }
        for name in ["console", "com", "comx", "lpt10", "nullable", "my-con"] {
            assert!(!windows_reserved(name), "{name}");
        }
    }

    #[test]
    fn temp_names_are_unique() {
        let dir = Path::new("/x");
        assert_ne!(unique_temp(dir, "tmp"), unique_temp(dir, "tmp"));
    }
}
