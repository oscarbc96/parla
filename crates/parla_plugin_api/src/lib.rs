//! The WIT package `parla:plugin`: the interface
//! between Parla and its WebAssembly component plugins.

/// Plugin API version as `(major, minor)`.
pub const API_VERSION: (u16, u16) = (0, 1);

/// Directory that holds the WIT package.
pub const WIT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/wit");

/// Read areas a manifest may list in `workspace_read`. The
/// one list of these names: the WIT doc comments and the design use them.
pub const READ_AREAS: &[&str] = &[
    "workspace:read",
    "conversations:read",
    "messages:read",
    "history:read",
    "dms:read",
    "people:read",
    "search:read",
    "files:read",
    "reminders:read",
    "scheduled:read",
    "saved:read",
    "pins:read",
    "status:read",
];

/// Write areas a manifest may list in `workspace_write`.
pub const WRITE_AREAS: &[&str] = &[
    "messages:write",
    "messages:edit",
    "reactions:write",
    "reminders:write",
    "scheduled:write",
    "files:write",
    "status:write",
    "saved:write",
    "pins:write",
    "read_state:write",
];

/// Whether this host can load a plugin built for `api`. Minor
/// versions only add, so from 1.0 on any minor up to this one loads; while
/// the major is 0, minors may break, so only the exact minor loads.
pub fn supports(api: (u16, u16)) -> bool {
    let (major, minor) = API_VERSION;
    if api.0 != major {
        return false;
    }
    if major == 0 {
        api.1 == minor
    } else {
        api.1 <= minor
    }
}
