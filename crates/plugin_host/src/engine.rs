//! The plugin engine.
//!
//! - Lazy: no wasmtime engine, linker or thread exists until the first
//!   plugin is loaded. On macOS the engine then keeps one blocked Mach
//!   exception-port thread for the life of the process (M8 A1 spike).
//! - Config: component model, cranelift, epoch interruption, copy-on-write
//!   memory images, no fuel, serial compilation (no `parallel-compilation`).
//! - Every store has a [`MemoryLimiter`] and an epoch deadline that traps; a
//!   trap ends that instance only, never the app.
//! - The [`EpochTicker`] thread runs only while a guest call is in flight.

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use sha2::{Digest, Sha256};
use wasmtime::component::{Component, Func, Instance, Linker, Val};
use wasmtime::{Config, Engine, Store, Strategy};

use crate::cache::{CacheError, PrecompileCache};
use crate::epoch::{EpochTicker, InFlight, TICK};
use crate::limits::MemoryLimiter;
use crate::linker::{self, PluginState};

/// The pinned wasmtime version (checked against `Cargo.toml` by a test).
pub const WASMTIME_VERSION: &str = "48.0.10";

/// Describes [`config`]; part of [`engine_tag`], so change it with the config.
const CONFIG_FINGERPRINT: &str =
    "component-model;cranelift;epoch-interruption;memory-init-cow;no-fuel;serial-compilation";

/// Deadline of instantiation (start functions), the same as `activate`.
pub const INSTANTIATE_DEADLINE: Duration = Duration::from_secs(1);

/// Epoch deadline while no call runs. The ticker is stopped then, so it is
/// never reached; it only keeps a stale deadline from firing.
const IDLE_DEADLINE: u64 = u64::MAX / 2;

/// The engine config shared by the app and the precompile child.
pub fn config() -> Config {
    let mut c = Config::new();
    c.wasm_component_model(true);
    c.strategy(Strategy::Cranelift);
    c.epoch_interruption(true);
    c.memory_init_cow(true);
    c.consume_fuel(false);
    c
}

/// Names the engine a `.cwasm` was compiled for: a hash of the wasmtime
/// version, [`config`] and the target. 16 hex digits.
pub fn engine_tag() -> String {
    let digest = Sha256::digest(format!(
        "wasmtime={WASMTIME_VERSION};config={CONFIG_FINGERPRINT};target={}-{}",
        std::env::consts::ARCH,
        std::env::consts::OS
    ));
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Cache(#[from] CacheError),
    #[error("plugin engine: {0}")]
    Wasmtime(String),
    #[error("plugin imports {0}, which Parla does not provide")]
    ForbiddenImport(String),
}

impl EngineError {
    fn from_cache(e: CacheError) -> Self {
        match e {
            CacheError::ForbiddenImport(name) => EngineError::ForbiddenImport(name),
            other => EngineError::Cache(other),
        }
    }
}

/// Why a guest call failed.
#[derive(Debug, thiserror::Error)]
pub enum CallError {
    #[error("the plugin ran past its deadline")]
    Deadline,
    #[error("the plugin ran out of memory")]
    MemoryLimit,
    #[error("the plugin crashed: {0}")]
    Trap(String),
    #[error("the plugin exited with status {0}")]
    Exit(i32),
    #[error("plugin call failed: {0}")]
    Host(String),
}

impl CallError {
    /// A trap leaves the instance unusable; its store must be dropped.
    pub fn is_trap(&self) -> bool {
        matches!(
            self,
            CallError::Deadline | CallError::MemoryLimit | CallError::Trap(_)
        )
    }
}

struct Parts {
    engine: Engine,
    linker: Linker<PluginState>,
    ticker: EpochTicker,
}

/// Owns the lazily created wasmtime engine, the linker and the epoch ticker.
#[derive(Default)]
pub struct PluginEngine {
    parts: OnceLock<Result<Parts, String>>,
}

impl PluginEngine {
    /// Creates nothing: the engine comes with the first [`load`](Self::load).
    pub fn new() -> Self {
        Self::default()
    }

    /// True once the wasmtime engine exists.
    pub fn is_created(&self) -> bool {
        self.parts.get().is_some()
    }

    fn parts(&self) -> Result<&Parts, EngineError> {
        self.parts
            .get_or_init(|| {
                let engine = Engine::new(&config()).map_err(|e| format!("{e:#}"))?;
                let linker = linker::linker(&engine).map_err(|e| format!("{e:#}"))?;
                let ticker = EpochTicker::new(engine.clone());
                Ok(Parts {
                    engine,
                    linker,
                    ticker,
                })
            })
            .as_ref()
            .map_err(|e| EngineError::Wasmtime(e.clone()))
    }

    /// The wasmtime engine, created on first use.
    pub fn engine(&self) -> Result<&Engine, EngineError> {
        Ok(&self.parts()?.engine)
    }

    /// Loads a plugin version from the precompile cache (compiling it in the
    /// child process if needed) and checks its imports. Blocking: file reads
    /// and possibly a compile.
    pub fn load(
        &self,
        cache: &PrecompileCache,
        id: &str,
        version: &str,
        wasm: &Path,
    ) -> Result<Component, EngineError> {
        let bytes = cache
            .load(id, version, wasm)
            .map_err(EngineError::from_cache)?;
        let engine = self.engine()?;
        let component = match deserialize(engine, &bytes) {
            Ok(c) => c,
            Err(_) => {
                // Built by another engine despite its hash: compile once
                // more and give up if that fails too.
                drop(bytes);
                cache.invalidate(id, version)?;
                let bytes = cache
                    .load(id, version, wasm)
                    .map_err(EngineError::from_cache)?;
                deserialize(engine, &bytes)?
            }
        };
        if let Some(name) = linker::forbidden_import(engine, &component) {
            return Err(EngineError::ForbiddenImport(name));
        }
        Ok(component)
    }

    /// A new instance in its own store, capped at `memory_mb` (manifest
    /// value; `None` is the default 16 MiB).
    pub async fn instantiate(
        &self,
        component: &Component,
        memory_mb: Option<u32>,
    ) -> Result<PluginInstance, EngineError> {
        let parts = self.parts()?;
        let mut store = Store::new(
            &parts.engine,
            PluginState::new(MemoryLimiter::new(memory_mb)),
        );
        store.limiter(|s| &mut s.limiter);
        store.epoch_deadline_trap();
        // Start functions run during instantiation: bound them too.
        let guard = parts.ticker.enter();
        store.set_epoch_deadline(ticks(INSTANTIATE_DEADLINE));
        // Bounded in wall time too: a start function that sleeps in a host
        // call is not running guest code, so the epoch alone cannot stop it.
        let instance = match tokio::time::timeout(
            wall_deadline(INSTANTIATE_DEADLINE),
            parts.linker.instantiate_async(&mut store, component),
        )
        .await
        {
            Ok(r) => r.map_err(|e| EngineError::Wasmtime(format!("{e:#}"))),
            Err(_) => Err(EngineError::Wasmtime(
                "instantiation ran past its deadline".into(),
            )),
        };
        store.set_epoch_deadline(IDLE_DEADLINE);
        drop(guard);
        Ok(PluginInstance {
            store,
            instance: instance?,
            trapped: false,
        })
    }

    /// Marks a guest call in flight (the ticker runs until the guard drops).
    pub fn begin_call(&self) -> Result<InFlight, EngineError> {
        Ok(self.parts()?.ticker.enter())
    }

    /// Calls `func` with a deadline: the epoch deadline stops a guest that
    /// runs, and a wall-clock timeout one tick later stops a guest that waits
    /// in a host call (a sleep through `wasi:io/poll`). Any failure (a trap,
    /// the deadline, `exit`, a host error) marks the instance unusable; later
    /// calls fail without entering the guest. Needs a Tokio runtime with time
    /// enabled.
    pub async fn call(
        &self,
        plugin: &mut PluginInstance,
        func: Func,
        params: &[Val],
        results: &mut [Val],
        deadline: Duration,
    ) -> Result<(), CallError> {
        if plugin.trapped {
            return Err(CallError::Trap("the instance trapped earlier".into()));
        }
        let guard = self
            .begin_call()
            .map_err(|e| CallError::Host(e.to_string()))?;
        plugin.store.set_epoch_deadline(ticks(deadline));
        let result = tokio::time::timeout(
            wall_deadline(deadline),
            func.call_async(&mut plugin.store, params, results),
        )
        .await;
        plugin.store.set_epoch_deadline(IDLE_DEADLINE);
        drop(guard);
        let result = match result {
            Ok(r) => r.map_err(|e| classify(plugin.store.data().limiter(), &e)),
            // The call future was dropped mid-call: the store is unusable.
            Err(_) => Err(CallError::Deadline),
        };
        plugin.trapped |= result.is_err();
        result
    }

    /// Guest calls in flight now.
    pub fn calls_in_flight(&self) -> usize {
        match self.parts.get() {
            Some(Ok(p)) => p.ticker.in_flight(),
            _ => 0,
        }
    }

    /// True while the epoch ticker thread exists.
    pub fn ticker_running(&self) -> bool {
        matches!(self.parts.get(), Some(Ok(p)) if p.ticker.is_running())
    }
}

/// One plugin instance and its store.
pub struct PluginInstance {
    store: Store<PluginState>,
    instance: Instance,
    trapped: bool,
}

impl PluginInstance {
    /// The function `name` of the exported interface `interface`
    /// (`parla:plugin/lifecycle@0.1.0`, `activate`).
    pub fn export_func(&mut self, interface: &str, name: &str) -> Option<Func> {
        let iface = self
            .instance
            .get_export_index(&mut self.store, None, interface)?;
        let func = self
            .instance
            .get_export_index(&mut self.store, Some(&iface), name)?;
        self.instance.get_func(&mut self.store, func)
    }

    /// True after a trap: drop this instance and make a new one.
    pub fn trapped(&self) -> bool {
        self.trapped
    }

    /// The instance's stdout and stderr (bounded).
    pub fn log(&self) -> String {
        self.store.data().log()
    }

    pub fn store(&self) -> &Store<PluginState> {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut Store<PluginState> {
        &mut self.store
    }

    pub fn instance(&self) -> Instance {
        self.instance
    }
}

fn deserialize(engine: &Engine, bytes: &[u8]) -> Result<Component, EngineError> {
    // SAFETY: `bytes` is the buffer `PrecompileCache::load` read once and
    // checked against the SHA-256 recorded when this app's precompile child
    // wrote it; no later change to the file reaches it. wasmtime also checks
    // the engine compatibility header.
    unsafe { Component::deserialize(engine, bytes) }
        .map_err(|e| EngineError::Wasmtime(format!("{e:#}")))
}

/// Whole ticks covering `d`, at least one.
fn ticks(d: Duration) -> u64 {
    let tick = TICK.as_nanos();
    (d.as_nanos().div_ceil(tick)).max(1) as u64
}

/// The wall-clock bound of a call: the deadline plus one tick, so a running
/// guest is normally stopped by the epoch first.
fn wall_deadline(d: Duration) -> Duration {
    d + TICK
}

fn classify(limiter: &MemoryLimiter, e: &wasmtime::Error) -> CallError {
    if let Some(exit) = e.downcast_ref::<wasmtime_wasi::I32Exit>() {
        return CallError::Exit(exit.0);
    }
    match e.downcast_ref::<wasmtime::Trap>() {
        Some(wasmtime::Trap::Interrupt) => CallError::Deadline,
        Some(_) if limiter.refused() => CallError::MemoryLimit,
        Some(trap) => CallError::Trap(trap.to_string()),
        None => CallError::Host(format!("{e:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wasmtime_version_matches_the_pin() {
        let manifest = include_str!("../Cargo.toml");
        let pin = format!("wasmtime = {{ version = \"={WASMTIME_VERSION}\"");
        assert!(
            manifest.contains(&pin),
            "update WASMTIME_VERSION with the pin"
        );
    }

    #[test]
    fn deadlines_round_up_to_ticks() {
        assert_eq!(ticks(Duration::from_millis(50)), 5);
        assert_eq!(ticks(Duration::from_millis(51)), 6);
        assert_eq!(ticks(Duration::ZERO), 1);
    }

    #[test]
    fn engine_tag_is_16_hex() {
        let tag = engine_tag();
        assert_eq!(tag.len(), 16);
        assert!(tag.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}
