# parla_plugin_api

The WIT package `parla:plugin@0.1.0`: the interface between Parla and its
WebAssembly component plugins.

- `wit/world.wit`: the `plugin` world. It imports every host interface and
  exports only `lifecycle` (`activate`). Each optional extension point
  (`commands`, `renderer`, `deactivation`, ...) is its own interface; a
  guest adds the ones its manifest declares with `include` plus `export`,
  and the host looks each up by name.
- `wit/types.wit`: shared ids, records and the `error` variant.
- `wit/host.wit`, `workspace.wit`, `http.wit`, `storage.wit`,
  `secrets.wit`, `settings.wit`, `ui.wit`, `composer.wit`, `exec.wit`: host
  interfaces. Each function's doc comment names the capability or workspace
  area its grant must hold.
- `wit/view.wit`: the view tree, as a flat node list with parent indices.
- `wit/media.wit`: the media provider export used by the GIF picker.
- `src/lib.rs`: `API_VERSION`, `supports`, and `READ_AREAS` and
  `WRITE_AREAS`, the one list of workspace area names.

From 1.0, minor versions only add, and a host loads plugins built for its
major version and any minor up to its own. While the major is 0 a minor may
break, so a host loads only its exact minor.

## Versions

The crate version stays `0.0.0`, the workspace version every member shares
(`release:version-check`). The API version is the WIT package version,
`parla:plugin@0.1.0`, and `API_VERSION`.

Rust plugins use the guest SDK `parla_plugin_sdk` instead of these files
directly.

## Licence

Licensed under either of Apache License, Version 2.0 (`LICENSE-APACHE`) or
MIT licence (`LICENSE-MIT`), at your option, so plugin authors pick their own
licence. The rest of Parla is MPL-2.0.
