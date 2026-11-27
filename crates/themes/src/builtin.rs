//! The built-in `Parla` theme family (`Parla Light` and `Parla Dark`) and its high
//! contrast pair (`Parla High Contrast Light` and `Parla High Contrast Dark`).

use crate::model::ThemeRegistry;

const PARLA_THEMES: &str = include_str!("../../../assets/themes/parla.json");
const HIGH_CONTRAST_THEMES: &str = include_str!("../../../assets/themes/parla-high-contrast.json");

/// Loads the built-in themes into `registry`.
pub(crate) fn load(registry: &ThemeRegistry) {
    use crate::model::ThemeKind;
    crate::load::load_family_as(registry, PARLA_THEMES.as_bytes(), ThemeKind::Brand)
        .expect("built-in theme assets/themes/parla.json is valid");
    crate::load::load_family_as(
        registry,
        HIGH_CONTRAST_THEMES.as_bytes(),
        ThemeKind::HighContrast,
    )
    .expect("built-in theme assets/themes/parla-high-contrast.json is valid");
}
