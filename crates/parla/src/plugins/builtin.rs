//! The first-party media providers this binary carries: their manifest, their
//! guest wasm and where the user gets an API key.
//!
//! The wasm is copied into `$OUT_DIR/builtin/` by `crates/parla/build.rs`;
//! an empty file means "not bundled" (a plain `cargo build` without
//! `mise run plugins:build`) and the provider is not offered. The manifests
//! are the plugin folders' own `plugin.toml`, embedded at compile time.

use std::sync::{Arc, OnceLock};

use plugin_host::manifest::{LocalisedText, Manifest, ManifestSource, parse};

/// One bundled plugin, in the order the settings page lists them.
pub struct Builtin {
    /// The plugin (and media provider) id.
    pub id: &'static str,
    /// Where the user creates an API key, shown on the set-up card.
    pub key_url: &'static str,
    /// `plugins/<id>/plugin.toml`.
    pub manifest_toml: &'static str,
    /// `$OUT_DIR/builtin/<id>.wasm`; empty when it was not bundled.
    pub wasm: &'static [u8],
    manifest: OnceLock<Arc<Manifest>>,
}

impl Builtin {
    /// The parsed manifest; a build error would have failed the tests.
    pub fn manifest(&self) -> &Arc<Manifest> {
        self.manifest.get_or_init(|| {
            Arc::new(
                parse(self.manifest_toml, ManifestSource::Package)
                    .unwrap_or_else(|e| panic!("built-in {} manifest: {e}", self.id)),
            )
        })
    }

    /// The name as the service wants it shown (`GIPHY`, `KLIPY`).
    pub fn title(&self) -> String {
        let m = self.manifest();
        let text = m
            .contributions
            .media_providers
            .first()
            .map(|p| &p.title)
            .unwrap_or(&m.name);
        match text {
            LocalisedText::Literal(s) => s.clone(),
            // A key needs the plugin's own Fluent files, which the picker does
            // not load; the manifest name is the fallback.
            LocalisedText::Key(_) => match &m.name {
                LocalisedText::Literal(s) => s.clone(),
                LocalisedText::Key(_) => self.id.to_ascii_uppercase(),
            },
        }
    }

    /// The `[[media_providers]]` id the guest's calls name.
    pub fn media_id(&self) -> &str {
        self.manifest()
            .contributions
            .media_providers
            .first()
            .map_or(self.id, |p| p.id.as_str())
    }

    pub fn version(&self) -> String {
        self.manifest().version.to_string()
    }

    pub fn bundled(&self) -> bool {
        !self.wasm.is_empty()
    }
}

static BUILTINS: [Builtin; 2] = [giphy(), klipy()];

const fn giphy() -> Builtin {
    Builtin {
        id: "giphy",
        key_url: "https://developers.giphy.com/dashboard/",
        manifest_toml: include_str!("../../../../plugins/giphy/plugin.toml"),
        wasm: include_bytes!(concat!(env!("OUT_DIR"), "/builtin/giphy.wasm")),
        manifest: OnceLock::new(),
    }
}

const fn klipy() -> Builtin {
    Builtin {
        id: "klipy",
        key_url: "https://partner.klipy.com/",
        manifest_toml: include_str!("../../../../plugins/klipy/plugin.toml"),
        wasm: include_bytes!(concat!(env!("OUT_DIR"), "/builtin/klipy.wasm")),
        manifest: OnceLock::new(),
    }
}

/// Every built-in, bundled or not.
pub fn all() -> &'static [Builtin] {
    &BUILTINS
}

/// The built-in named `id`, if it is one.
pub fn get(id: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.id == id)
}

/// True when at least one provider's wasm was embedded: only then does the
/// app offer the GIF picker.
pub fn any_bundled() -> bool {
    BUILTINS.iter().any(Builtin::bundled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_names_one_provider_each() {
        for b in all() {
            assert_eq!(get(b.id).map(|b| b.id), Some(b.id));
            assert!(b.key_url.starts_with("https://"));
            assert_eq!(b.version(), "0.1.0");
        }
        assert!(get("nope").is_none());
    }
}
