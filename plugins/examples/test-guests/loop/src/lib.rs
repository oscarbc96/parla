//! Engine test guest: `activate(install)` spins forever, so the host's epoch
//! deadline must stop it; any other reason returns at once.

use std::hint::black_box;

wit_bindgen::generate!({ world: "parla:plugin/plugin", path: "../../../../crates/parla_plugin_api/wit" });

use exports::parla::plugin::lifecycle::{ActivateReason, Guest};
use parla::plugin::types::Error;

struct Loop;

impl Guest for Loop {
    fn activate(reason: ActivateReason) -> Result<(), Error> {
        if reason == ActivateReason::Install {
            let mut x: u64 = 1;
            loop {
                x = black_box(x.wrapping_mul(6364136223846793005).wrapping_add(1));
            }
        }
        Ok(())
    }
}

export!(Loop);
