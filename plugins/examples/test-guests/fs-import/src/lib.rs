//! Engine test guest that imports `wasi:filesystem` (through `std::fs`), which
//! the host never links: loading it must fail.

wit_bindgen::generate!({ world: "parla:plugin/plugin", path: "../../../../crates/parla_plugin_api/wit" });

use exports::parla::plugin::lifecycle::{ActivateReason, Guest};
use parla::plugin::types::Error;

struct FsImport;

impl Guest for FsImport {
    fn activate(_reason: ActivateReason) -> Result<(), Error> {
        std::fs::read("/etc/hosts").map_err(|e| Error::Failed(e.to_string()))?;
        Ok(())
    }
}

export!(FsImport);
