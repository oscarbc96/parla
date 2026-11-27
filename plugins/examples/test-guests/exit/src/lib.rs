//! Engine test guest: `activate(install)` calls `exit(3)` through
//! `wasi:cli/exit`; any other reason returns at once.

wit_bindgen::generate!({ world: "parla:plugin/plugin", path: "../../../../crates/parla_plugin_api/wit" });

use exports::parla::plugin::lifecycle::{ActivateReason, Guest};
use parla::plugin::types::Error;

struct Exit;

impl Guest for Exit {
    fn activate(reason: ActivateReason) -> Result<(), Error> {
        if reason == ActivateReason::Install {
            std::process::exit(3);
        }
        Ok(())
    }
}

export!(Exit);
