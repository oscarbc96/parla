# parla_plugin_sdk

Rust guest SDK for Parla plugins. A plugin is a
WebAssembly component built for `wasm32-wasip2` against the WIT package
`parla:plugin@0.1.0` (`crates/parla_plugin_api/wit`).

```rust
use parla_plugin_sdk::host::ui::{self, ToastKind};
use parla_plugin_sdk::types::Context;
use parla_plugin_sdk::{Error, Plugin, export};

pub struct Hello;

impl Plugin for Hello {
    fn run_command(_id: String, _context: Context) -> Result<(), Error> {
        ui::toast("Hello from a plugin", ToastKind::Success)
    }
}

export!(Hello with [commands]);
```

Build with `cargo build --release --target wasm32-wasip2`; the `.wasm` in
`target/wasm32-wasip2/release/` is the component. See
`plugins/examples/hello-command` for the whole plugin with its manifest.

## What is in it

- `Plugin`: one static method per extension point.
  `activate` succeeds by default; every other method returns
  `Error::Unsupported` until you implement it.
- `export!(Type with [..])`: exports `lifecycle` plus the listed extension
  points, which must match the contributions in `plugin.toml`. Names:
  `deactivation`, `migration`, `commands`, `slash_commands`,
  `message_actions`, `event_hooks`, `notification_filter`,
  `composer_transforms`, `renderer`, `views`, `settings_listener`,
  `mcp_tools`, `media_provider`. The component exports exactly those
  interfaces: each comes from its own one-export world, and wit-component
  merges the worlds the macro names. The type may be a path
  (`export!(crate::plugin::Mine with [renderer])`). The host's install
  check refuses a package whose exports do not match its manifest
.
- `plugin_state!(static NAME: Type = init)`: state for the static
  `Plugin` methods, one value per thread, so parallel tests never share it.
- `host`: the host calls, each returning `Result<T, Error>`.
  The `host` interface's functions (`log`, `now`, `t`, ...) sit at
  `host::`; the others are modules (`workspace`, `http`, `storage`,
  `secrets`, `settings`, `ui`, `composer`, `exec`). Cursors are
  `Option<&str>`.
- `view`: the view tree types and builders (`column()`, `text(..)`,
  `button(id, label).look(..)`, `input(id, label).placeholder(..)`, ...)
  that flatten to the WIT flat node list; `validate` and `size`, the rules
  and byte measure in `wit/view.wit`, tested against the shared vectors in
  `crates/parla_plugin_api/tests/fixtures/view-trees/`.
- `testing::FakeHost` (feature `testing`, native targets): an in-memory
  host with the plugin's grants and the documented limits, so plugin logic
  runs under plain `cargo test`:

  ```rust
  let fake = FakeHost::from_manifest(include_str!("../plugin.toml"));
  Hello::run_command("greet".into(), context).unwrap();
  assert_eq!(fake.toasts()[0].0, "Hello from a plugin");
  ```

  Calls without their grant return `denied`; reads are stubbed per method
  (`fake.on_history(..)`); `fake.inject("http.fetch", Error::Offline)`
  makes a call fail. The limits it does not enforce are listed at the top
  of `src/testing.rs`. Add the SDK as a dev-dependency with
  `features = ["testing"]`. `export!` does nothing on native targets, and
  a native build without `testing` stops with a `compile_error!`.

## In this repo

The SDK lives in the guest workspace `plugins/`, which the host workspace
never builds. `mise run plugins:build` builds every member for
`wasm32-wasip2` into `target/plugins/`; `mise run plugins:test` runs the
guest tests natively with the fake host (part of `mise run test`), and
`mise run clippy` checks both workspaces.

## Licence

`Apache-2.0 OR MIT`, like `parla_plugin_api`, so plugin authors pick their
own licence.
