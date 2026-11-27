//! Plugin engine:
//! lazy engine, memory cap, epoch deadline, an epoch ticker that exists only
//! while a call is in flight, the WASI allow list and the bounded precompile
//! cache compiled in a child process.
//!
//! The guests under `plugins/examples/test-guests/` are built into
//! `target/plugins/` by `mise run plugins:build` (a dependency of
//! `mise run test`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use plugin_host::cache::{
    Compiler, CwasmHashes, CwasmKey, MAX_FILES, MemoryCwasmHashes, PrecompileCache,
};
use plugin_host::engine::{CallError, EngineError, PluginEngine};
use wasmtime::component::Val;

/// Env var that turns this test binary into the precompile child.
const CHILD_ENV: &str = "PARLA_TEST_PRECOMPILE_CHILD";
/// File the child appends its process id to.
const CHILD_LOG_ENV: &str = "PARLA_TEST_PRECOMPILE_LOG";
/// Makes the child hang instead of compiling.
const CHILD_HANG_ENV: &str = "PARLA_TEST_PRECOMPILE_HANG";
/// Makes the child write 1 MiB to stderr and fail.
const CHILD_SPAM_ENV: &str = "PARLA_TEST_PRECOMPILE_SPAM";

/// Run by the test binary re-executed as the precompile child (the app uses
/// a hidden subcommand of its own binary). Passes trivially otherwise.
#[test]
fn precompile_child_entry() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    let args: Vec<OsString> = std::env::args_os().collect();
    let [.., wasm, out] = args.as_slice() else {
        panic!("child needs <wasm> <out>");
    };
    if let Some(log) = std::env::var_os(CHILD_LOG_ENV) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .unwrap();
        writeln!(f, "{}", std::process::id()).unwrap();
    }
    if std::env::var_os(CHILD_HANG_ENV).is_some() {
        std::thread::sleep(Duration::from_secs(60));
    }
    if std::env::var_os(CHILD_SPAM_ENV).is_some() {
        use std::io::Write;
        std::io::stderr().write_all(&vec![b'e'; 1 << 20]).unwrap();
        std::process::exit(2);
    }
    plugin_host::cache::precompile_child(Path::new(wasm), Path::new(out)).unwrap();
}

fn guest(name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/plugins")
        .join(format!("test_guest_{name}.wasm"));
    assert!(
        path.exists(),
        "{}: missing; run `mise run plugins:build`",
        path.display()
    );
    path
}

struct Fixture {
    dir: tempfile::TempDir,
    hashes: Arc<MemoryCwasmHashes>,
    log: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("children.log");
        Fixture {
            dir,
            hashes: Arc::new(MemoryCwasmHashes::default()),
            log,
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("cache/parla/plugins")
    }

    fn compiler(&self) -> Compiler {
        // libtest: `--exact` runs only the named test; the trailing paths
        // match no test name, and the child reads them from its argv.
        Compiler::new(std::env::current_exe().unwrap())
            .args(["precompile_child_entry", "--exact", "--nocapture"])
            .env(CHILD_ENV, "1")
            .env(CHILD_LOG_ENV, &self.log)
    }

    fn cache(&self) -> PrecompileCache {
        PrecompileCache::new(self.root(), self.compiler(), self.hashes.clone())
    }

    fn cache_with_tag(&self, tag: &str) -> PrecompileCache {
        self.cache().with_engine_tag(tag)
    }

    fn children(&self) -> Vec<u32> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|l| l.parse().unwrap())
            .collect()
    }
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
}

fn reason(name: &str) -> Val {
    Val::Enum(name.into())
}

/// Calls `parla:plugin/lifecycle.activate(reason)` with a deadline.
async fn activate(
    engine: &PluginEngine,
    plugin: &mut plugin_host::engine::PluginInstance,
    why: &str,
    deadline: Duration,
) -> Result<Val, CallError> {
    let func = plugin
        .export_func("parla:plugin/lifecycle@0.1.0", "activate")
        .expect("activate export");
    let mut results = [Val::Bool(false)];
    engine
        .call(plugin, func, &[reason(why)], &mut results, deadline)
        .await?;
    let [result] = results;
    Ok(result)
}

fn is_ok(v: &Val) -> bool {
    matches!(v, Val::Result(Ok(_)))
}

/// Threads of this process.
fn threads() -> usize {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_dir("/proc/self/task").unwrap().count()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let out = std::process::Command::new("ps")
            .args(["-M", "-p", &std::process::id().to_string()])
            .output()
            .expect("ps");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .count()
            .saturating_sub(1)
    }
}

#[test]
fn no_engine_until_first_activation() {
    let fx = Fixture::new();
    let engine = PluginEngine::new();
    assert!(!engine.is_created());
    // Precompiling (install) happens in a child; it does not need the engine.
    let cache = fx.cache();
    cache.ensure("loop", "1.0.0", &guest("loop")).unwrap();
    assert!(!engine.is_created(), "precompile created the engine");
    rt().block_on(async {
        let component = engine
            .load(&cache, "loop", "1.0.0", &guest("loop"))
            .unwrap();
        assert!(engine.is_created());
        let mut plugin = engine.instantiate(&component, None).await.unwrap();
        let r = activate(&engine, &mut plugin, "resident", Duration::from_secs(1))
            .await
            .unwrap();
        assert!(is_ok(&r), "{r:?}");
    });
}

#[test]
fn memory_cap_traps_store_not_app() {
    let fx = Fixture::new();
    let cache = fx.cache();
    let engine = PluginEngine::new();
    rt().block_on(async {
        let alloc = engine
            .load(&cache, "alloc", "1.0.0", &guest("alloc"))
            .unwrap();
        let mut big = engine.instantiate(&alloc, Some(16)).await.unwrap();
        let err = activate(&engine, &mut big, "install", Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(matches!(err, CallError::MemoryLimit), "{err:?}");
        drop(big);

        // The same plugin in a new store, and another plugin, still run.
        let mut small = engine.instantiate(&alloc, Some(16)).await.unwrap();
        let r = activate(&engine, &mut small, "invoked", Duration::from_secs(5))
            .await
            .unwrap();
        assert!(is_ok(&r), "{r:?}");
        assert!(
            small.log().contains("alloc guest: 1048576 bytes"),
            "{}",
            small.log()
        );
        let other = engine
            .load(&cache, "loop", "1.0.0", &guest("loop"))
            .unwrap();
        let mut other = engine.instantiate(&other, None).await.unwrap();
        let r = activate(&engine, &mut other, "resident", Duration::from_secs(1))
            .await
            .unwrap();
        assert!(is_ok(&r), "{r:?}");
    });
}

#[test]
fn memory_cap_is_clamped() {
    use plugin_host::limits::MemoryLimiter;
    assert_eq!(MemoryLimiter::new(None).cap_bytes(), 16 << 20);
    assert_eq!(MemoryLimiter::new(Some(8)).cap_bytes(), 8 << 20);
    assert_eq!(MemoryLimiter::new(Some(500)).cap_bytes(), 64 << 20);
    assert_eq!(MemoryLimiter::new(Some(0)).cap_bytes(), 1 << 20);
}

#[test]
fn busy_loop_hits_deadline() {
    let fx = Fixture::new();
    let cache = fx.cache();
    let engine = PluginEngine::new();
    rt().block_on(async {
        let c = engine
            .load(&cache, "loop", "1.0.0", &guest("loop"))
            .unwrap();
        let mut plugin = engine.instantiate(&c, None).await.unwrap();
        let started = Instant::now();
        let err = activate(&engine, &mut plugin, "install", Duration::from_millis(50))
            .await
            .unwrap_err();
        let took = started.elapsed();
        assert!(matches!(err, CallError::Deadline), "{err:?}");
        assert!(took >= Duration::from_millis(40), "{took:?}");
        assert!(took < Duration::from_secs(2), "{took:?}");
        assert_eq!(engine.calls_in_flight(), 0);
    });
}

#[test]
fn epoch_thread_only_while_calls_in_flight() {
    let fx = Fixture::new();
    let cache = fx.cache();
    let engine = PluginEngine::new();
    rt().block_on(async {
        let c = engine
            .load(&cache, "loop", "1.0.0", &guest("loop"))
            .unwrap();
        let mut plugin = engine.instantiate(&c, None).await.unwrap();
        // One call first, so every lazy thread the engine owns exists.
        activate(&engine, &mut plugin, "resident", Duration::from_secs(1))
            .await
            .unwrap();
    });
    // Counted after the engine exists: on macOS it keeps one Mach port thread.
    let idle = threads();
    assert!(!engine.ticker_running());
    let first = engine.begin_call().unwrap();
    assert!(engine.ticker_running());
    assert_eq!(threads(), idle + 1, "one ticker thread during a call");
    let second = engine.begin_call().unwrap();
    assert_eq!(threads(), idle + 1, "calls share the ticker");
    drop(first);
    assert!(engine.ticker_running());
    assert_eq!(threads(), idle + 1);
    drop(second);
    assert!(!engine.ticker_running());
    assert_eq!(threads(), idle, "no ticker when nothing runs");
    // And again: the ticker restarts on the next call.
    let again = engine.begin_call().unwrap();
    assert_eq!(threads(), idle + 1);
    drop(again);
    assert_eq!(threads(), idle);
}

#[test]
fn component_importing_filesystem_is_rejected() {
    let fx = Fixture::new();
    let engine = PluginEngine::new();
    let err = engine
        .load(&fx.cache(), "fs", "1.0.0", &guest("fs_import"))
        .unwrap_err();
    match err {
        EngineError::ForbiddenImport(name) => {
            assert!(name.starts_with("wasi:filesystem/"), "{name}")
        }
        other => panic!("{other:?}"),
    }
    assert!(fx.children().is_empty(), "checked before compiling");
    assert!(!fx.root().join("fs").exists(), "nothing cached");
}

#[test]
fn component_importing_sockets_is_rejected() {
    let fx = Fixture::new();
    let engine = PluginEngine::new();
    let err = engine
        .load(&fx.cache(), "net", "1.0.0", &guest("socket_import"))
        .unwrap_err();
    match err {
        EngineError::ForbiddenImport(name) => assert!(name.starts_with("wasi:sockets/"), "{name}"),
        other => panic!("{other:?}"),
    }
    assert!(fx.children().is_empty(), "checked before compiling");
    assert!(!fx.root().join("net").exists(), "nothing cached");
}

fn cwasm_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in std::fs::read_dir(root).unwrap() {
        let dir = dir.unwrap().path();
        for f in std::fs::read_dir(&dir).unwrap() {
            let f = f.unwrap().path();
            if f.extension().is_some_and(|e| e == "cwasm") {
                out.push(f);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn precompile_cache_is_bounded() {
    let fx = Fixture::new();
    let cache = fx.cache_with_tag("e1");
    let wasm = guest("loop");
    // At most two versions per plugin: the oldest goes.
    for v in ["1.0.0", "1.1.0", "1.2.0"] {
        cache.ensure("loop", v, &wasm).unwrap();
    }
    let names: Vec<String> = cwasm_files(&fx.root())
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["1.1.0-e1.cwasm", "1.2.0-e1.cwasm"]);
    assert!(
        fx.hashes
            .get(&CwasmKey::new("loop", "1.0.0", "e1"))
            .is_none()
    );
    assert!(
        fx.hashes
            .get(&CwasmKey::new("loop", "1.2.0", "e1"))
            .is_some()
    );

    // At most MAX_FILES in total: fill with other plugins' files, then add one.
    for i in 0..MAX_FILES {
        let dir = fx.root().join(format!("p{i:03}"));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("1.0.0-e1.cwasm");
        std::fs::write(&f, b"x").unwrap();
        // Older than anything the cache writes from now on.
        let t = std::time::SystemTime::now() - Duration::from_secs(3600 - i as u64);
        std::fs::File::options()
            .write(true)
            .open(&f)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }
    cache.ensure("loop", "1.3.0", &wasm).unwrap();
    let files = cwasm_files(&fx.root());
    assert_eq!(files.len(), MAX_FILES);
    assert!(fx.root().join("loop/1.3.0-e1.cwasm").exists());
    assert!(
        !fx.root().join("p000/1.0.0-e1.cwasm").exists(),
        "oldest removed"
    );
    assert!(!fx.root().join("p000").exists(), "empty plugin dir removed");
}

#[test]
fn tampered_cwasm_is_recompiled() {
    let fx = Fixture::new();
    let cache = fx.cache();
    let engine = PluginEngine::new();
    let path = cache.ensure("loop", "1.0.0", &guest("loop")).unwrap();
    assert_eq!(fx.children().len(), 1);
    let mut bytes = std::fs::read(&path).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xff;
    std::fs::write(&path, &bytes).unwrap();
    rt().block_on(async {
        let c = engine
            .load(&cache, "loop", "1.0.0", &guest("loop"))
            .unwrap();
        assert_eq!(fx.children().len(), 2, "recompiled after the hash mismatch");
        let mut plugin = engine.instantiate(&c, None).await.unwrap();
        let r = activate(&engine, &mut plugin, "resident", Duration::from_secs(1))
            .await
            .unwrap();
        assert!(is_ok(&r));
    });
    // A file with no stored hash is not trusted either.
    let key = CwasmKey::new("loop", "1.0.0", cache.engine_tag());
    fx.hashes.remove(&key).unwrap();
    cache.ensure("loop", "1.0.0", &guest("loop")).unwrap();
    assert_eq!(fx.children().len(), 3);
    // An intact file with its hash is reused.
    cache.ensure("loop", "1.0.0", &guest("loop")).unwrap();
    assert_eq!(fx.children().len(), 3);
}

#[test]
fn precompile_runs_in_child_process() {
    let fx = Fixture::new();
    let cache = fx.cache();
    let path = cache.ensure("loop", "1.0.0", &guest("loop")).unwrap();
    assert!(path.exists());
    let children = fx.children();
    assert_eq!(children.len(), 1);
    assert_ne!(children[0], std::process::id(), "compiled in this process");
    // A compiler that fails leaves no file behind and reports the error.
    let broken = PrecompileCache::new(
        fx.root(),
        Compiler::new(fx.dir.path().join("no-such-binary")),
        fx.hashes.clone(),
    );
    let err = broken
        .ensure("alloc", "1.0.0", &guest("alloc"))
        .unwrap_err();
    assert!(!err.to_string().is_empty());
    assert!(!fx.root().join("alloc").exists() || cwasm_files(&fx.root()).len() == 1);
}

#[test]
fn engine_hash_change_invalidates_cache() {
    let fx = Fixture::new();
    let wasm = guest("loop");
    let a = fx.cache_with_tag("aaaa");
    a.ensure("loop", "1.0.0", &wasm).unwrap();
    assert!(fx.root().join("loop/1.0.0-aaaa.cwasm").exists());
    let b = fx.cache_with_tag("bbbb");
    b.ensure("loop", "1.0.0", &wasm).unwrap();
    assert_eq!(fx.children().len(), 2, "a new engine recompiles");
    assert!(fx.root().join("loop/1.0.0-bbbb.cwasm").exists());
    assert!(
        !fx.root().join("loop/1.0.0-aaaa.cwasm").exists(),
        "old engine's file removed"
    );
    assert!(
        fx.hashes
            .get(&CwasmKey::new("loop", "1.0.0", "aaaa"))
            .is_none()
    );
    // The default tag names the wasmtime version and config, and is stable.
    let tag = plugin_host::engine::engine_tag();
    assert_eq!(tag, plugin_host::engine::engine_tag());
    assert!(
        tag.len() >= 16 && tag.chars().all(|c| c.is_ascii_hexdigit()),
        "{tag}"
    );
}

#[test]
fn ids_cannot_escape_the_cache() {
    let fx = Fixture::new();
    let cache = fx.cache();
    for (id, v) in [
        ("../x", "1.0.0"),
        ("a/b", "1.0.0"),
        ("ok", "../1"),
        ("", "1"),
        (".hidden", "1"),
        ("con", "1.0.0"),
        ("NUL", "1.0.0"),
        ("com1", "1.0.0"),
        ("Lpt9.x", "1.0.0"),
        ("aux.plugin", "1.0.0"),
        ("trailing.", "1.0.0"),
    ] {
        assert!(cache.ensure(id, v, &guest("loop")).is_err(), "{id} {v}");
    }
    assert!(fx.children().is_empty());
}

#[test]
fn cwasm_swapped_after_hash_check_is_not_loaded() {
    let fx = Fixture::new();
    let engine = PluginEngine::new();
    // Between the hash check and the load, something replaces the file.
    let cache = fx.cache().after_verify(|path: &Path| {
        std::fs::write(path, b"not the bytes that were checked").unwrap();
    });
    rt().block_on(async {
        let c = engine
            .load(&cache, "loop", "1.0.0", &guest("loop"))
            .unwrap();
        assert_eq!(fx.children().len(), 1, "loaded the checked bytes, no retry");
        let mut plugin = engine.instantiate(&c, None).await.unwrap();
        let r = activate(&engine, &mut plugin, "resident", Duration::from_secs(1))
            .await
            .unwrap();
        assert!(is_ok(&r));
    });
}

/// A component with two core modules, one memory each.
fn two_memory_component() -> Vec<u8> {
    use wasm_encoder::{
        Component, InstanceSection, MemorySection, MemoryType, Module, ModuleArg, ModuleSection,
    };
    let mut module = Module::new();
    let mut memories = MemorySection::new();
    memories.memory(MemoryType {
        minimum: 1,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    module.section(&memories);
    let mut component = Component::new();
    component.section(&ModuleSection(&module));
    component.section(&ModuleSection(&module));
    let mut instances = InstanceSection::new();
    instances.instantiate(0, std::iter::empty::<(&str, ModuleArg)>());
    instances.instantiate(1, std::iter::empty::<(&str, ModuleArg)>());
    component.section(&instances);
    component.finish()
}

#[test]
fn second_memory_is_refused() {
    let fx = Fixture::new();
    let wasm = fx.dir.path().join("two_memories.wasm");
    std::fs::write(&wasm, two_memory_component()).unwrap();
    let engine = PluginEngine::new();
    rt().block_on(async {
        let c = engine.load(&fx.cache(), "mem", "1.0.0", &wasm).unwrap();
        assert!(engine.instantiate(&c, Some(64)).await.is_err());
    });
}

#[test]
fn concurrent_compiles_share_one_result() {
    let fx = Arc::new(Fixture::new());
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let fx = fx.clone();
            std::thread::spawn(move || fx.cache().ensure("loop", "1.0.0", &guest("loop")))
        })
        .collect();
    let paths: Vec<PathBuf> = threads
        .into_iter()
        .map(|t| t.join().unwrap().unwrap())
        .collect();
    assert!(paths.windows(2).all(|w| w[0] == w[1]));
    assert_eq!(fx.children().len(), 1, "one compile for one key");
    let key = CwasmKey::new("loop", "1.0.0", fx.cache().engine_tag());
    assert!(fx.hashes.get(&key).is_some(), "the winner's hash is kept");
    // The file matches its hash: loading it needs no recompile.
    fx.cache().ensure("loop", "1.0.0", &guest("loop")).unwrap();
    assert_eq!(fx.children().len(), 1);
    let leftovers: Vec<_> = std::fs::read_dir(fx.root().join("loop"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .filter(|n| !n.to_string_lossy().ends_with(".cwasm"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn sleeping_guest_hits_deadline() {
    let fx = Fixture::new();
    let engine = PluginEngine::new();
    rt().block_on(async {
        let c = engine
            .load(&fx.cache(), "sleep", "1.0.0", &guest("sleep"))
            .unwrap();
        let mut plugin = engine.instantiate(&c, None).await.unwrap();
        let started = Instant::now();
        let err = activate(&engine, &mut plugin, "install", Duration::from_millis(100))
            .await
            .unwrap_err();
        let took = started.elapsed();
        assert!(matches!(err, CallError::Deadline), "{err:?}");
        assert!(took < Duration::from_secs(2), "{took:?}");
        assert!(plugin.trapped());
        assert_eq!(engine.calls_in_flight(), 0);
        assert!(!engine.ticker_running());
    });
}

#[test]
fn exit_marks_instance_trapped() {
    let fx = Fixture::new();
    let engine = PluginEngine::new();
    rt().block_on(async {
        let c = engine
            .load(&fx.cache(), "exit", "1.0.0", &guest("exit"))
            .unwrap();
        let mut plugin = engine.instantiate(&c, None).await.unwrap();
        let err = activate(&engine, &mut plugin, "install", Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(matches!(err, CallError::Exit(3)), "{err:?}");
        assert!(plugin.trapped());
        // Not entered again.
        let err = activate(&engine, &mut plugin, "resident", Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(matches!(err, CallError::Trap(_)), "{err:?}");
    });
}

fn no_temp_files(root: &Path) -> bool {
    !std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .any(|d| {
            std::fs::read_dir(d.path())
                .into_iter()
                .flatten()
                .flatten()
                .any(|f| !f.file_name().to_string_lossy().ends_with(".cwasm"))
        })
}

#[test]
fn precompile_child_times_out() {
    let fx = Fixture::new();
    let cache = PrecompileCache::new(
        fx.root(),
        fx.compiler()
            .env(CHILD_HANG_ENV, "1")
            .timeout(Duration::from_millis(500)),
        fx.hashes.clone(),
    );
    let started = Instant::now();
    let err = cache.ensure("loop", "1.0.0", &guest("loop")).unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(err.to_string().contains("timed out"), "{err}");
    assert!(cwasm_files_or_none(&fx.root()).is_empty());
    assert!(no_temp_files(&fx.root()));
}

#[test]
fn precompile_child_stderr_is_capped() {
    let fx = Fixture::new();
    let cache = PrecompileCache::new(
        fx.root(),
        fx.compiler().env(CHILD_SPAM_ENV, "1"),
        fx.hashes.clone(),
    );
    let err = cache.ensure("loop", "1.0.0", &guest("loop")).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.len() <= plugin_host::cache::STDERR_CAP + 200,
        "{}",
        msg.len()
    );
    assert!(no_temp_files(&fx.root()));
}

#[test]
fn precompile_output_is_capped() {
    let fx = Fixture::new();
    let cache = PrecompileCache::new(fx.root(), fx.compiler().max_output(1024), fx.hashes.clone());
    let err = cache.ensure("loop", "1.0.0", &guest("loop")).unwrap_err();
    assert!(err.to_string().contains("too large"), "{err}");
    assert!(cwasm_files_or_none(&fx.root()).is_empty());
    assert!(no_temp_files(&fx.root()));
}

#[test]
fn app_ignoring_the_subcommand_is_an_error() {
    let fx = Fixture::new();
    // Runs no test and exits 0 without writing anything, as an app binary
    // that does not know the subcommand would (after the timeout at worst).
    let cache = PrecompileCache::new(
        fx.root(),
        Compiler::new(std::env::current_exe().unwrap()).args(["no_such_test_name", "--exact"]),
        fx.hashes.clone(),
    );
    let err = cache.ensure("loop", "1.0.0", &guest("loop")).unwrap_err();
    assert!(err.to_string().contains("no output"), "{err}");
    assert!(cwasm_files_or_none(&fx.root()).is_empty());
    let key = CwasmKey::new("loop", "1.0.0", cache.engine_tag());
    assert!(fx.hashes.get(&key).is_none());
}

fn cwasm_files_or_none(root: &Path) -> Vec<PathBuf> {
    if root.exists() {
        cwasm_files(root)
    } else {
        Vec::new()
    }
}
