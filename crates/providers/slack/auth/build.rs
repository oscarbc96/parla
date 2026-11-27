//! Embeds an Info.plist (camera and microphone usage strings) into the example
//! binaries on macOS, so a `cargo run --example call_probe` binary can ask for
//! media access with its own prompt text.

fn main() {
    println!("cargo:rerun-if-changed=examples/call_probe.Info.plist");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir");
        println!(
            "cargo:rustc-link-arg-examples=-Wl,-sectcreate,__TEXT,__info_plist,{dir}/examples/call_probe.Info.plist"
        );
    }
}
