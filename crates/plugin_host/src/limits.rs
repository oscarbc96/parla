//! Per-plugin memory and table caps.

use wasmtime::ResourceLimiter;

/// The manifest's `memory_mb` bounds (one definition, in `manifest`).
pub use crate::manifest::limits::{DEFAULT_MEMORY_MB, MAX_MEMORY_MB};

/// Table elements per plugin, summed over all its tables.
pub const MAX_TABLE_ELEMENTS: usize = 10_000;
/// Linear memories per store. A Rust guest has exactly one.
pub const MAX_MEMORIES: usize = 1;
/// Core tables per store (a wit-bindgen guest uses two or three).
pub const MAX_TABLES: usize = 4;
/// Core instances per store (a wit-bindgen guest uses three or four).
pub const MAX_INSTANCES: usize = 8;

const MIB: usize = 1 << 20;

/// Caps one store: the linear memory of all its memories together at
/// `memory_mb` MiB, the elements of all its tables at
/// [`MAX_TABLE_ELEMENTS`], and the number of memories, tables and core
/// instances. A refused grow makes the guest trap (Rust's allocator
/// aborts); [`refused`](Self::refused) tells that trap apart from others.
/// The cap is per store, so a trap ends only that plugin's instance.
#[derive(Debug)]
pub struct MemoryLimiter {
    cap_bytes: usize,
    memory_bytes: usize,
    table_elements: usize,
    refused: bool,
}

impl MemoryLimiter {
    /// `memory_mb` from the manifest: `None` is the default 16, values are
    /// clamped to 1..=64.
    pub fn new(memory_mb: Option<u32>) -> Self {
        let mb = memory_mb
            .unwrap_or(DEFAULT_MEMORY_MB)
            .clamp(1, MAX_MEMORY_MB);
        MemoryLimiter {
            cap_bytes: mb as usize * MIB,
            memory_bytes: 0,
            table_elements: 0,
            refused: false,
        }
    }

    pub fn cap_bytes(&self) -> usize {
        self.cap_bytes
    }

    /// Linear memory of every memory in the store, in bytes.
    pub fn memory_bytes(&self) -> usize {
        self.memory_bytes
    }

    /// True once a memory or table grow was refused.
    pub fn refused(&self) -> bool {
        self.refused
    }
}

impl ResourceLimiter for MemoryLimiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let total = self
            .memory_bytes
            .saturating_add(desired.saturating_sub(current));
        let ok = total <= self.cap_bytes;
        if ok {
            self.memory_bytes = total;
        }
        self.refused |= !ok;
        Ok(ok)
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let total = self
            .table_elements
            .saturating_add(desired.saturating_sub(current));
        let ok = total <= MAX_TABLE_ELEMENTS;
        if ok {
            self.table_elements = total;
        }
        self.refused |= !ok;
        Ok(ok)
    }

    fn instances(&self) -> usize {
        MAX_INSTANCES
    }

    fn tables(&self) -> usize {
        MAX_TABLES
    }

    fn memories(&self) -> usize {
        MAX_MEMORIES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_up_to_the_cap_only() {
        let mut l = MemoryLimiter::new(Some(2));
        assert!(l.memory_growing(0, 2 * MIB, None).unwrap());
        assert!(!l.refused());
        assert!(!l.memory_growing(2 * MIB, 2 * MIB + 1, None).unwrap());
        assert!(l.refused());
    }

    #[test]
    fn memory_cap_counts_every_memory() {
        let mut l = MemoryLimiter::new(Some(16));
        assert!(l.memory_growing(0, 10 * MIB, None).unwrap());
        // A second memory (or a second store user) does not get its own 16.
        assert!(!l.memory_growing(0, 10 * MIB, None).unwrap());
        assert!(l.refused());
        assert!(l.memory_growing(10 * MIB, 16 * MIB, None).unwrap());
        assert_eq!(l.memory_bytes(), 16 * MIB);
    }

    #[test]
    fn table_cap_counts_every_table() {
        let mut l = MemoryLimiter::new(None);
        assert!(l.table_growing(0, MAX_TABLE_ELEMENTS / 2, None).unwrap());
        assert!(l.table_growing(0, MAX_TABLE_ELEMENTS / 2, None).unwrap());
        assert!(!l.table_growing(0, 1, None).unwrap());
    }

    #[test]
    fn tables_are_capped() {
        let mut l = MemoryLimiter::new(None);
        assert!(l.table_growing(0, MAX_TABLE_ELEMENTS, None).unwrap());
        assert!(!l.table_growing(0, MAX_TABLE_ELEMENTS + 1, None).unwrap());
        assert!(l.refused());
    }
}
