//! Engine test guest: `activate(install)` allocates and touches 32 MiB, over
//! the 16 MB default cap; any other reason allocates 1 MiB and returns.

use std::hint::black_box;

wit_bindgen::generate!({ world: "parla:plugin/plugin", path: "../../../../crates/parla_plugin_api/wit" });

use exports::parla::plugin::lifecycle::{ActivateReason, Guest};
use parla::plugin::types::Error;

struct Alloc;

impl Guest for Alloc {
    fn activate(reason: ActivateReason) -> Result<(), Error> {
        let n = if reason == ActivateReason::Install {
            32 << 20
        } else {
            1 << 20
        };
        let buf = black_box(vec![1u8; n]);
        println!("alloc guest: {} bytes", buf.len());
        Ok(())
    }
}

export!(Alloc);
