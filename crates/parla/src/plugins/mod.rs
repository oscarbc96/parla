//! The built-in GIF provider plugins, wired to the app: their embedded wasm
//! and manifests, the plugin secrets store, the host backend and the manager
//! that runs them.
//!
//! Only `parla` wires `plugin_host` to the app; the core crates stay
//! provider-neutral and the picker only sees [`components::gif_picker`].

pub mod backend;
pub mod builtin;
pub mod manager;
pub mod secrets;

pub use manager::{GifSourceHandle, ManagerConfig, PluginManager, ProviderInfo, ProviderState};

/// Installs the GIF providers at startup. Does no work until the first GIF
/// call; a binary built without the bundled wasm offers no provider at all.
pub use manager::install;
