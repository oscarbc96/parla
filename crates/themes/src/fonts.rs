//! The bundled Inter faces (SIL OFL 1.1): `assets/fonts`, see its README.
//!
//! The default UI font is "Inter". The app registers these at start-up and
//! the visual harness shapes from the same bytes, so every design weight
//! renders the same on a machine without Inter installed.

/// Inter Regular (400), upright.
pub const INTER_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
/// Inter Italic (400).
pub const INTER_ITALIC: &[u8] = include_bytes!("../../../assets/fonts/Inter-Italic.ttf");
/// Inter Medium (500).
pub const INTER_MEDIUM: &[u8] = include_bytes!("../../../assets/fonts/Inter-Medium.ttf");
/// Inter SemiBold (600).
pub const INTER_SEMIBOLD: &[u8] = include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf");
/// Inter Bold (700).
pub const INTER_BOLD: &[u8] = include_bytes!("../../../assets/fonts/Inter-Bold.ttf");

/// Every bundled Inter face, in the order the text system registers them.
pub const INTER_FACES: [&[u8]; 5] = [
    INTER_REGULAR,
    INTER_ITALIC,
    INTER_MEDIUM,
    INTER_SEMIBOLD,
    INTER_BOLD,
];
