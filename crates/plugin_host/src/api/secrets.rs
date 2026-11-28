//! `parla:plugin/secrets`: the plugin's own keychain entries, namespaced by
//! its id. No capability.

use super::parla::plugin::secrets::Host;
use super::parla::plugin::types::{self as error, Error};
use crate::backend::ApiError;
use crate::linker::PluginState;

/// Longest secret key.
pub const MAX_KEY_BYTES: usize = 128;
/// Longest secret value.
pub const MAX_VALUE_BYTES: usize = 4 * 1024;
/// Most keys one plugin may hold.
pub const MAX_KEYS: usize = 32;

impl Host for PluginState {
    async fn secret_get(&mut self, key: String) -> Result<Option<String>, Error> {
        check_key(&key)?;
        let backend = self.context().backend.clone();
        let plugin = self.plugin_id().clone();
        backend
            .secret_get(&plugin, &key)
            .await
            .map_err(error::Error::from)
    }

    async fn secret_set(&mut self, key: String, value: String) -> Result<(), Error> {
        check_key(&key)?;
        if value.len() > MAX_VALUE_BYTES {
            return Err(ApiError::Limit("secret value".to_owned()).into());
        }
        let backend = self.context().backend.clone();
        let plugin = self.plugin_id().clone();
        let keys = backend.secret_keys(&plugin).await.map_err(Error::from)?;
        if keys.len() >= MAX_KEYS && !keys.iter().any(|k| k == &key) {
            return Err(ApiError::Limit("secret keys".to_owned()).into());
        }
        backend
            .secret_set(&plugin, &key, &value)
            .await
            .map_err(Error::from)
    }

    async fn secret_delete(&mut self, key: String) -> Result<(), Error> {
        check_key(&key)?;
        let backend = self.context().backend.clone();
        let plugin = self.plugin_id().clone();
        backend
            .secret_delete(&plugin, &key)
            .await
            .map_err(Error::from)
    }
}

fn check_key(key: &str) -> Result<(), Error> {
    if key.len() > MAX_KEY_BYTES {
        return Err(ApiError::Invalid("secret key is too long".to_owned()).into());
    }
    Ok(())
}
