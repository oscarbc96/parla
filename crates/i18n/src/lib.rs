//! Locale list and Fluent catalogs for Parla. Provider-neutral.

// The crate embeds its own catalog through `namespace!`.
extern crate self as i18n;

mod catalog;
#[cfg(feature = "check")]
pub mod check;
mod format;
#[cfg(feature = "gpui")]
pub mod gpui;
mod locales;
mod localizer;
mod macros;
mod negotiate;
mod text;

namespace!("i18n");

pub use catalog::{Catalog, CatalogError};
pub use fluent_bundle::FluentArgs;
pub use format::HourCycle;
pub use locales::{LocaleInfo, LocaleSet, LocalesError, PSEUDO};
pub use localizer::{Localizer, Translate};
pub use negotiate::{LocalePref, chain};
pub use rust_embed;
pub use text::{Arg, LocalizedText};
pub use unic_langid::LanguageIdentifier;
