//! Encrypted SQLite (SQLCipher) backend: per-workspace databases, keys from a
//! `KeySource`, forward-only migrations and one writer thread (`parla-store`).

mod app_db;
mod conn;
mod emoji;
mod err;
mod free_space;
mod key;
mod migrate;
mod notify_prefs;
mod reminders;
mod retention;
mod rows;
mod schedules;
mod search;
mod store_impl;
mod thread;

pub use free_space::{FREE_SPACE_CHECK_MS, FreeSpace, SystemFreeSpace};
pub use key::{CachingKeySource, DbId, Key, KeySource, MemoryKeySource};
#[cfg(feature = "test-support")]
pub use search::fts_query;
pub use thread::{SqliteConfig, SqliteStore};
