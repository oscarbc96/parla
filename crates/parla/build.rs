//! Embeds `macos/Info.plist` (microphone and camera usage strings) into the
//! `parla` binary on macOS, so a plain `cargo run` build carries the text the
//! system shows in its media prompts and the call helper (this same binary)
//! is attributed to Parla. The bundle build ships the same file.

fn main() {
    println!("cargo:rerun-if-changed=macos/Info.plist");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir");
        println!(
            "cargo:rustc-link-arg-bin=parla=-Wl,-sectcreate,__TEXT,__info_plist,{dir}/macos/Info.plist"
        );
    }
}
