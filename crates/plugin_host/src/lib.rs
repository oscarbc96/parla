//! Parla's WebAssembly plugin host. Plugins are `wasm32` components built
//! against `parla_plugin_api`. The host loads them with wasmtime, gives each
//! a capability set from its manifest, and enforces epoch and memory limits.

mod api;
pub mod backend;
pub mod cache;
pub mod caps;
pub mod context;
pub mod engine;
pub mod epoch;
pub mod install;
pub mod lifecycle;
pub mod limits;
pub mod linker;
pub mod manifest;
pub mod media;
pub mod package;
pub mod registry;

pub use context::PluginContext;

/// The component model types [`engine::PluginEngine::call`] exchanges, so the
/// app can call a plugin export without a direct `wasmtime` dependency.
pub use wasmtime;
