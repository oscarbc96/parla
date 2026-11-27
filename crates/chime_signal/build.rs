//! Compiles the vendored, unmodified `proto/SignalingProtocol.proto` into Rust
//! messages. `protox` is a pure Rust protobuf compiler, so no `protoc` binary
//! is needed on any machine or CI runner.

use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=proto/SignalingProtocol.proto");
    let descriptors = protox::compile(["proto/SignalingProtocol.proto"], ["proto"])?;
    prost_build::Config::new().compile_fds(descriptors)?;
    Ok(())
}
