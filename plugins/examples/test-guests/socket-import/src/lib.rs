//! Engine test guest that imports `wasi:sockets` (through `std::net`), which
//! the host never links: loading it must fail.

wit_bindgen::generate!({ world: "parla:plugin/plugin", path: "../../../../crates/parla_plugin_api/wit" });

use exports::parla::plugin::lifecycle::{ActivateReason, Guest};
use parla::plugin::types::Error;

struct SocketImport;

impl Guest for SocketImport {
    fn activate(_reason: ActivateReason) -> Result<(), Error> {
        std::net::TcpStream::connect("127.0.0.1:9").map_err(|e| Error::Failed(e.to_string()))?;
        Ok(())
    }
}

export!(SocketImport);
