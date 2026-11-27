//! Settings: typed model, layered JSONC loading and schema.
//! File store, first run and watching.

mod edit;
mod error;
mod keymap;
mod load;
mod model;
pub mod paths;
mod schema;
mod store;
mod watch;
#[cfg(windows)]
pub mod win_sid;

pub use edit::{
    BindingFilter, edit_file, edit_keymap_file, remove_key, remove_key_in_file,
    remove_user_bindings, reset_file, set_user_binding, update_plugin_config, update_value,
    write_atomic,
};
pub use error::SettingsError;
pub use keymap::{
    Binding, KeymapBlock, KeymapOS, KeymapWarning, canonical_keys, parse_keymap, resolve_bindings,
    resolve_bindings_with_os,
};
pub use load::{Layer, default_layer, effective_for, layer_value, parse_layers};
pub use model::*;
pub use paths::ConfigPaths;
pub use schema::{settings_schema, unknown_keys};
pub use store::{ReloadOutcome, SettingsStore};
pub use watch::{ChangedFile, WatchHandle, watch};
