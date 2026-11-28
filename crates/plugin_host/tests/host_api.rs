//! Host API surface: a plugin built with the SDK now links against every
//! `parla:plugin` import it declares, so it instantiates and runs.

mod common;

use plugin_host::engine::PluginEngine;

#[test]
fn hello_command_example_instantiates() {
    common::runtime().block_on(async {
        let engine = PluginEngine::new();
        let wasm = common::example("hello_command");
        let mut plugin = common::instantiate(&engine, &wasm, Default::default()).await;
        common::activate(&engine, &mut plugin, "invoked")
            .await
            .expect("activate");
    });
}
