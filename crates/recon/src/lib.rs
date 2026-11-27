//! Dev-only web-client recon tooling. Provider binaries are a
//! thin front end over `cli::run`; nothing here is linked into `parla`.

pub mod catalogue;
pub mod cli;
pub mod diff;
pub mod extract;
pub mod har;
pub mod output;
pub mod shape;
pub mod summary;
pub mod verify;
