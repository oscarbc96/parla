//! The `parla mcp` shim as its own binary, so `crates/mcp/tests/shim.rs` can run
//! it without the app (the `parla` binary is covered by `crates/parla/tests/cli.rs`).

fn main() -> std::process::ExitCode {
    mcp::shim::main()
}
