//! Bindings for the `plugin` world: the host side of every `parla:plugin`
//! interface, so any plugin built with the SDK links and instantiates.

wasmtime::component::bindgen!({
    // The shared WIT package, next to this crate.
    path: "../parla_plugin_api/wit",
    world: "plugin",
    imports: { default: async },
});

mod host;
mod http;
mod secrets;
mod settings;
mod unsupported;

use wasmtime::component::{HasSelf, Linker};

use crate::backend::ApiError;
use crate::linker::PluginState;

use self::parla::plugin::types;

/// Links every import of the `plugin` world for [`PluginState`].
pub(crate) fn add_to_linker(l: &mut Linker<PluginState>) -> wasmtime::Result<()> {
    Plugin::add_to_linker::<PluginState, HasSelf<PluginState>>(l, |t| t)?;
    Ok(())
}

impl From<ApiError> for types::Error {
    fn from(e: ApiError) -> Self {
        match e {
            ApiError::Denied(why) => Self::Denied(why),
            ApiError::Unsupported => Self::Unsupported,
            ApiError::NotFound => Self::NotFound,
            ApiError::Limit(what) => Self::Limit(what),
            ApiError::Invalid(why) => Self::Invalid(why),
            ApiError::Offline => Self::Offline,
            ApiError::Cancelled => Self::Cancelled,
            ApiError::Failed(why) => Self::Failed(why),
        }
    }
}
