//! The GIF picker: a composer popover that searches the app's GIF providers.

#[cfg(feature = "test-support")]
pub mod fake;
mod picker;
mod source;

pub use picker::{
    COLUMNS, Confirm, Dismiss, GifPickMode, GifPicked, GifPicker, GifPickerDismissed, Insert,
    KEY_CONTEXT, MAX_ITEMS, NextTab, PrevTab, SEARCH_DEBOUNCE, SelectLeft, SelectNext, SelectPrev,
    SelectRight, enabled, init, notify_closed,
};
pub use source::{GifError, GifItem, GifPage, GifSource, GifSources, GifTab, GifTabState};
