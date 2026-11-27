//! The example plugins in `plugins/examples/` (each with a `plugin.toml`):
//! the manifest parses, and the built component (`target/plugins/`, from
//! `mise run plugins:build`) exports exactly the interfaces its
//! contributions need, as the install check will require.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use plugin_host::manifest::{Contributions, ManifestSource, parse};
use wasmtime::component::Component;
use wasmtime::{Config, Engine};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The extension interfaces the contributions call.
fn required(c: &Contributions) -> BTreeSet<&'static str> {
    let mut r = BTreeSet::new();
    let mut need = |cond: bool, name| {
        if cond {
            r.insert(name);
        }
    };
    need(!c.commands.is_empty(), "commands");
    need(!c.slash.is_empty(), "slash-commands");
    need(!c.message_actions.is_empty(), "message-actions");
    need(!c.composer_transforms.is_empty(), "composer-transforms");
    need(!c.renderers.is_empty(), "renderer");
    need(!c.media_providers.is_empty(), "media-provider");
    need(!c.panels.is_empty(), "views");
    need(!c.mcp_tools.is_empty(), "mcp-tools");
    let hooks = c.hooks.as_ref();
    need(hooks.is_some_and(|h| !h.events.is_empty()), "event-hooks");
    need(
        hooks.is_some_and(|h| h.notification_filter),
        "notification-filter",
    );
    r
}

/// Exports every plugin may add without a contribution.
const OPTIONAL: &[&str] = &["deactivation", "migration", "settings-listener", "views"];

fn exports(engine: &Engine, wasm: &Path) -> BTreeSet<String> {
    let component = Component::from_file(engine, wasm)
        .unwrap_or_else(|e| panic!("{}: {e:#}; run `mise run plugins:build`", wasm.display()));
    component
        .component_type()
        .exports(engine)
        .map(|(name, _)| {
            name.strip_prefix("parla:plugin/")
                .and_then(|n| n.strip_suffix("@0.1.0"))
                .unwrap_or_else(|| panic!("{}: unexpected export {name}", wasm.display()))
                .to_string()
        })
        .collect()
}

fn crate_name(cargo_toml: &str) -> String {
    let line = cargo_toml
        .lines()
        .find(|l| l.trim_start().starts_with("name"))
        .expect("Cargo.toml name");
    line.split('"').nth(1).unwrap().replace('-', "_")
}

#[test]
fn example_exports_match_their_manifests() {
    let engine = Engine::new(&Config::new()).unwrap();
    let mut checked = 0;
    let mut dirs: Vec<_> = std::fs::read_dir(repo().join("plugins/examples"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("plugin.toml").is_file())
        .collect();
    dirs.sort();
    for dir in dirs {
        let src = std::fs::read_to_string(dir.join("plugin.toml")).unwrap();
        let manifest = parse(&src, ManifestSource::Package)
            .unwrap_or_else(|e| panic!("{}: {e:?}", dir.display()));
        let name = crate_name(&std::fs::read_to_string(dir.join("Cargo.toml")).unwrap());
        let got = exports(&engine, &repo().join(format!("target/plugins/{name}.wasm")));
        let need = required(&manifest.contributions);
        assert!(got.contains("lifecycle"), "{name}: no lifecycle export");
        for n in &need {
            assert!(
                got.contains(*n),
                "{name}: manifest needs `{n}`, not exported: {got:?}"
            );
        }
        for g in &got {
            assert!(
                g == "lifecycle" || need.contains(g.as_str()) || OPTIONAL.contains(&g.as_str()),
                "{name}: exports `{g}`, which its manifest does not declare"
            );
        }
        checked += 1;
    }
    assert!(checked >= 1, "no example plugins found");
}
