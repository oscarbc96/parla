//! Shared helpers for the plugin host API and media tests.
//!
//! The guests under `plugins/examples/test-guests/` are built into
//! `target/plugins/` by `mise run plugins:build` (a dependency of
//! `mise run test`).

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use plugin_host::backend::{HostBackend, MemoryBackend};
use plugin_host::caps::Grant;
use plugin_host::context::PluginContext;
use plugin_host::engine::{PluginEngine, PluginInstance};
use plugin_host::manifest::{Manifest, ManifestSource, parse};
use wasmtime::component::{Component, Val};

/// The built guest `test_guest_<name>.wasm`.
pub fn guest(name: &str) -> PathBuf {
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

/// The built example plugin `plugins/examples/<name>`.
pub fn example(name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/plugins")
        .join(format!("{name}.wasm"));
    assert!(
        path.exists(),
        "{}: missing; run `mise run plugins:build`",
        path.display()
    );
    path
}

pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
}

/// Compiles `wasm` for `engine` (the precompile cache is not needed here).
pub fn component(engine: &PluginEngine, wasm: &std::path::Path) -> Component {
    Component::from_file(engine.engine().expect("wasmtime engine"), wasm)
        .unwrap_or_else(|e| panic!("{}: {e:#}", wasm.display()))
}

pub fn manifest(toml: &str) -> Arc<Manifest> {
    Arc::new(parse(toml, ManifestSource::Package).expect("valid manifest"))
}

pub fn context(
    manifest: Arc<Manifest>,
    grant: Grant,
    backend: Arc<dyn HostBackend>,
) -> PluginContext {
    PluginContext::new(
        manifest.id.clone(),
        manifest.clone(),
        Arc::new(grant),
        backend,
    )
}

pub fn memory_backend() -> Arc<MemoryBackend> {
    Arc::new(MemoryBackend::new())
}

/// Instantiates `wasm` with `ctx`.
pub async fn instantiate(
    engine: &PluginEngine,
    wasm: &std::path::Path,
    ctx: PluginContext,
) -> PluginInstance {
    let component = component(engine, wasm);
    engine
        .instantiate_with(&component, ctx)
        .await
        .expect("instantiate")
}

/// Calls the guest's `lifecycle.activate` with a 1 s deadline.
pub async fn activate(
    engine: &PluginEngine,
    plugin: &mut PluginInstance,
    why: &str,
) -> Result<(), plugin_host::engine::CallError> {
    let func = plugin
        .export_func("parla:plugin/lifecycle@0.1.0", "activate")
        .expect("activate export");
    let mut results = [Val::Bool(false)];
    engine
        .call(
            plugin,
            func,
            &[Val::Enum(why.into())],
            &mut results,
            Duration::from_secs(1),
        )
        .await?;
    match results {
        [Val::Result(Ok(_))] => Ok(()),
        [other] => panic!("activate returned {other:?}"),
    }
}
