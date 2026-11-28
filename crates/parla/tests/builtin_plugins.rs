//! The built-in GIF plugins as the binary carries them: both manifests name
//! one media provider and an `api_key` secret, and the bundled wasm is a
//! component exporting the media-provider interface.
//!
//! The wasm is embedded by `crates/parla/build.rs` from
//! `$PARLA_BUILTIN_PLUGINS_DIR` or `<repo>/target/plugins`; a missing file is
//! embedded empty (not bundled) and the wasm checks skip.

use std::path::{Path, PathBuf};
use std::process::Command;

use parla::plugins::builtin;
use plugin_host::cache::PRECOMPILE_SUBCOMMAND;
use plugin_host::manifest::SettingKind;
use wasmparser::{ComponentExternalKind, Parser, Payload};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

#[test]
fn both_manifests_are_valid_and_name_one_media_provider() {
    for id in ["giphy", "klipy"] {
        let b = builtin::get(id).unwrap_or_else(|| panic!("{id} is built in"));
        assert_eq!(b.id, id);
        let m = b.manifest();
        assert_eq!(m.id.as_str(), id);
        assert_eq!(
            m.contributions.media_providers.len(),
            1,
            "{id}: one media provider"
        );
        assert_eq!(m.contributions.media_providers[0].id, id);
        let key = m
            .settings
            .iter()
            .find(|s| s.key == "api_key")
            .unwrap_or_else(|| panic!("{id} declares api_key"));
        assert_eq!(key.kind, SettingKind::Secret, "{id}");
        assert!(!m.capabilities.network.is_empty(), "{id}: network hosts");
        assert!(b.key_url.starts_with("https://"), "{id}");
        assert!(b.title() == "GIPHY" || b.title() == "KLIPY", "{id}");
    }
}

#[test]
fn the_manifests_match_the_plugin_folders() {
    for id in ["giphy", "klipy"] {
        let path = repo_root().join("plugins").join(id).join("plugin.toml");
        let on_disk = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            builtin::get(id).unwrap().manifest_toml,
            on_disk,
            "{id}: the embedded manifest is plugins/{id}/plugin.toml"
        );
    }
}

#[test]
fn bundled_wasm_is_a_media_provider_component() {
    for id in ["giphy", "klipy"] {
        let b = builtin::get(id).unwrap();
        if b.wasm.is_empty() {
            eprintln!("skipped: {id}.wasm is not bundled; run `mise run plugins:build`");
            continue;
        }
        assert_eq!(&b.wasm[0..4], b"\0asm", "{id}: a wasm binary");
        let mut exports = Vec::new();
        for payload in Parser::new(0).parse_all(b.wasm) {
            if let Payload::ComponentExportSection(reader) = payload.unwrap() {
                for export in reader {
                    let export = export.unwrap();
                    exports.push((export.name.name.to_owned(), export.kind));
                }
            }
        }
        assert!(
            exports.iter().any(|(name, kind)| {
                *name == "parla:plugin/media-provider@0.1.0"
                    && *kind == ComponentExternalKind::Instance
            }),
            "{id}: exports {exports:?}"
        );
    }
}

/// The app binary's hidden subcommand compiles a plugin for the plugin
/// engine and writes the `.cwasm`; it runs before GPUI and the crash hook.
#[test]
fn the_precompile_subcommand_compiles_a_component() {
    let Some(b) = builtin::get("giphy").filter(|b| !b.wasm.is_empty()) else {
        eprintln!("skipped: giphy.wasm is not bundled; run `mise run plugins:build`");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let wasm = dir.path().join("giphy.wasm");
    std::fs::write(&wasm, b.wasm).unwrap();
    let out = dir.path().join("giphy.cwasm");
    let output = Command::new(env!("CARGO_BIN_EXE_parla"))
        .arg(PRECOMPILE_SUBCOMMAND)
        .arg(&wasm)
        .arg(&out)
        .output()
        .expect("run the precompile subcommand");
    assert!(
        output.status.success(),
        "status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.is_file(), "the .cwasm was written");
    assert!(std::fs::metadata(&out).unwrap().len() > 0);
}

#[test]
fn the_precompile_subcommand_fails_on_a_bad_file() {
    let dir = tempfile::tempdir().unwrap();
    let wasm = dir.path().join("bad.wasm");
    std::fs::write(&wasm, b"not wasm").unwrap();
    let out = dir.path().join("bad.cwasm");
    let output = Command::new(env!("CARGO_BIN_EXE_parla"))
        .arg(PRECOMPILE_SUBCOMMAND)
        .arg(&wasm)
        .arg(&out)
        .output()
        .expect("run the precompile subcommand");
    assert_eq!(output.status.code(), Some(1));
    assert!(!output.stderr.is_empty(), "a message on stderr");
    assert!(!out.exists());
}

#[test]
fn a_builtin_with_an_empty_wasm_is_not_offered() {
    // The rule the app relies on: an empty embed means "not bundled".
    for id in ["giphy", "klipy"] {
        if builtin::get(id).unwrap().wasm.is_empty() {
            assert!(
                !Path::new(&format!("target/plugins/{id}.wasm")).exists(),
                "{id}: empty embed but a built guest"
            );
        }
    }
}
