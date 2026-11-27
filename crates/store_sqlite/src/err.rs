//! The one mapping from backend errors to `StoreError`.

use store::StoreError;

/// Any displayable backend error (rusqlite, serde, io) becomes `StoreError::Other`.
pub(crate) fn other(e: impl std::fmt::Display) -> StoreError {
    StoreError::Other(e.to_string())
}
