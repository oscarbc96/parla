//! `parla:plugin/settings`: the plugin's own settings. Only keys its manifest
//! declares; a `secret` setting is never returned here.

use super::parla::plugin::settings::{Host, SettingValue as WitValue};
use super::parla::plugin::types::{self as error, Error};
use crate::backend::SettingValue;
use crate::linker::PluginState;
use crate::manifest::SettingKind;

impl Host for PluginState {
    async fn setting(&mut self, key: String) -> Result<Option<WitValue>, Error> {
        let declared = self.manifest().settings.iter().find(|spec| spec.key == key);
        match declared {
            // Undeclared keys, and secret ones, are not settings the plugin
            // may read.
            None => Err(Error::NotFound),
            Some(spec) if spec.kind == SettingKind::Secret => Err(Error::NotFound),
            Some(_) => {
                let backend = self.context().backend.clone();
                let plugin = self.plugin_id().clone();
                backend
                    .setting(&plugin, &key)
                    .await
                    .map(|value| value.map(into_wit))
                    .map_err(error::Error::from)
            }
        }
    }
}

fn into_wit(value: SettingValue) -> WitValue {
    match value {
        SettingValue::Text(text) => WitValue::Text(text),
        SettingValue::Number(number) => WitValue::Number(number),
        SettingValue::Toggle(on) => WitValue::Toggle(on),
        SettingValue::Choice(choice) => WitValue::Choice(choice),
    }
}
