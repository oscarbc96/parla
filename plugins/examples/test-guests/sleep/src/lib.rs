//! Engine test guest: `activate(install)` sleeps for 30 s through
//! `wasi:clocks` and `wasi:io/poll`, so it is not running guest code and the
//! epoch deadline alone would not stop it; any other reason returns at once.

wit_bindgen::generate!({ world: "parla:plugin/plugin", path: "../../../../crates/parla_plugin_api/wit" });

use exports::parla::plugin::lifecycle::{ActivateReason, Guest};
use parla::plugin::types::Error;

struct Sleep;

impl Guest for Sleep {
    fn activate(reason: ActivateReason) -> Result<(), Error> {
        if reason == ActivateReason::Install {
            std::thread::sleep(std::time::Duration::from_secs(30));
        }
        Ok(())
    }
}

export!(Sleep);
