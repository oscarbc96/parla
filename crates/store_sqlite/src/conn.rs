//! Opening one encrypted database connection.

use std::path::Path;

use rusqlite::{Connection, ErrorCode, OpenFlags};
use store::StoreError;
use zeroize::Zeroizing;

use crate::err::other;
use crate::key::Key;

/// Create the file with owner-only permissions before SQLite does.
pub(crate) fn create_private_file(path: &Path) -> std::io::Result<()> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(path).map(|_| ())
}

pub(crate) fn create_private_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(path)
}

/// `PRAGMA key` first, then prove the key by reading the schema.
/// A wrong key or corrupt file is `Unreadable`; the file is not touched.
pub(crate) fn open_keyed(
    path: &Path,
    key: &Key,
    flags: OpenFlags,
) -> Result<Connection, StoreError> {
    let conn = Connection::open_with_flags(path, flags).map_err(other)?;
    let pragma = Zeroizing::new(format!("PRAGMA key = \"x'{}'\"", key.to_hex().as_str()));
    conn.execute_batch(&pragma).map_err(other)?;
    match conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    }) {
        Ok(_) => Ok(conn),
        Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == ErrorCode::NotADatabase => {
            Err(StoreError::Unreadable)
        }
        Err(e) => Err(other(e)),
    }
}

/// Writer connection settings. `auto_vacuum` comes before `journal_mode`:
/// switching to WAL writes the header, which fixes it for a new file.
pub(crate) fn apply_writer_pragmas(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        "PRAGMA auto_vacuum = INCREMENTAL;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA journal_size_limit = 67108864;
         PRAGMA foreign_keys = ON;
         PRAGMA mmap_size = 0;",
    )
    .map_err(other)
}

pub(crate) fn apply_reader_pragmas(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA mmap_size = 0; PRAGMA query_only = ON;")
        .map_err(other)
}

pub(crate) fn set_cache_kib(conn: &Connection, kib: i64) {
    let _ = conn.execute_batch(&format!("PRAGMA cache_size = -{kib}"));
}
