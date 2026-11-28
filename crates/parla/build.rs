//! Embeds `macos/Info.plist` (microphone and camera usage strings) into the
//! `parla` binary on macOS, so a plain `cargo run` build carries the text the
//! system shows in its media prompts and the call helper (this same binary)
//! is attributed to Parla. The bundle build ships the same file.
//!
//! Also embeds the built-in GIF plugins (`giphy`, `klipy`): each guest wasm
//! is copied into `$OUT_DIR/builtin/<id>.wasm`, from
//! `$PARLA_BUILTIN_PLUGINS_DIR/<id>.wasm` when set, else
//! `<repo>/target/plugins/<id>.wasm` (`mise run plugins:build`). A missing
//! guest is written as an empty file: the provider is then simply not
//! offered, and a plain `cargo build` still works.

use std::path::{Path, PathBuf};

/// The first-party media providers, built by `mise run plugins:build`.
const BUILTINS: &[&str] = &["giphy", "klipy"];

fn main() {
    println!("cargo:rerun-if-changed=macos/Info.plist");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir");
        println!(
            "cargo:rustc-link-arg-bin=parla=-Wl,-sectcreate,__TEXT,__info_plist,{dir}/macos/Info.plist"
        );
    }
    embed_builtins();
}

fn embed_builtins() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("builtin");
    std::fs::create_dir_all(&out_dir).expect("create OUT_DIR/builtin");
    let crate_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let repo = crate_dir.join("../..");
    let override_dir = std::env::var_os("PARLA_BUILTIN_PLUGINS_DIR").map(PathBuf::from);
    println!("cargo:rerun-if-env-changed=PARLA_BUILTIN_PLUGINS_DIR");

    for id in BUILTINS {
        let mut sources = Vec::new();
        if let Some(dir) = &override_dir {
            sources.push(dir.join(format!("{id}.wasm")));
        }
        sources.push(repo.join("target/plugins").join(format!("{id}.wasm")));
        let manifest = repo.join("plugins").join(id).join("plugin.toml");
        let dest = out_dir.join(format!("{id}.wasm"));
        let found = sources.iter().find(|p| p.is_file());
        for source in &sources {
            println!("cargo:rerun-if-changed={}", source.display());
        }
        match found {
            Some(source) => {
                std::fs::copy(source, &dest).unwrap_or_else(|e| {
                    panic!("copy {} to {}: {e}", source.display(), dest.display())
                });
            }
            None => {
                std::fs::write(&dest, []).expect("write the empty placeholder");
                println!(
                    "cargo:warning=built-in plugin `{id}` is not bundled: run `mise run plugins:build`"
                );
            }
        }
        // The manifests are embedded with `include_str!`, so a change rebuilds
        // the crate even when the wasm did not move.
        emit_changed(&manifest);
    }
}

fn emit_changed(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
}
