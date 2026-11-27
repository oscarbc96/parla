//! The example plugins build as a wasm32-wasip2 component that exports only
//! what it declares. Reads `target/plugins/`, so run
//! after `mise run plugins:build` (`mise run plugins:test` does).

use std::path::PathBuf;

fn hello_wasm() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/plugins/hello_command.wasm")
}

/// The component exists and exports `lifecycle` plus `commands`, the one
/// extension point its manifest declares, and nothing else.
#[test]
fn hello_command_builds_for_wasip2() {
    let path = hello_wasm();
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|_| panic!("{} missing: run `mise run plugins:build`", path.display()));
    assert_eq!(
        exports_of(&bytes),
        [
            "parla:plugin/commands@0.1.0",
            "parla:plugin/lifecycle@0.1.0"
        ]
    );
}

/// The component's exported interface names, sorted.
fn exports_of(bytes: &[u8]) -> Vec<String> {
    let wit_component::DecodedWasm::Component(resolve, world) =
        wit_component::decode(bytes).expect("decodes")
    else {
        panic!("not a component");
    };
    let world = &resolve.worlds[world];
    let mut exports: Vec<String> = world
        .exports
        .keys()
        .map(|k| resolve.name_world_key(k))
        .collect();
    exports.sort();
    exports
}

#[test]
fn every_extension_point_exports_together() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/plugins/test_guest_sdk_all_exports.wasm");
    let bytes = std::fs::read(&path).expect("run `mise run plugins:build` first");
    let exports = exports_of(&bytes);
    let want: Vec<String> = [
        "commands",
        "composer-transforms",
        "deactivation",
        "event-hooks",
        "lifecycle",
        "mcp-tools",
        "media-provider",
        "message-actions",
        "migration",
        "notification-filter",
        "renderer",
        "settings-listener",
        "slash-commands",
        "views",
    ]
    .iter()
    .map(|i| format!("parla:plugin/{i}@0.1.0"))
    .collect();
    assert_eq!(exports, want);
}
