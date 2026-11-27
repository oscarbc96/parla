//! The theme model: the colour tokens Parla draws with, the registry and the active theme.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use gpui::{App, Global, Hsla, SharedString};

use crate::parla_tokens::ParlaTokens;

/// Whether a theme is light or dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

/// Interface colours. Only the tokens Parla uses; the JSON keys use dots
/// (`element.selected`), the fields underscores.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeColors {
    pub background: Hsla,
    pub surface_background: Hsla,
    pub elevated_surface_background: Hsla,
    pub panel_background: Hsla,
    pub title_bar_background: Hsla,
    pub editor_background: Hsla,
    pub border: Hsla,
    /// Keyboard focus ring (`border.focused`).
    pub border_focused: Hsla,
    /// Stronger outline of unticked check boxes and field frames (`border.strong`).
    pub border_strong: Hsla,
    /// Text on an `element_background` fill (`text.on_accent`); when a theme omits
    /// it, the better-contrasting of text and background (WCAG ratio).
    pub text_on_accent: Hsla,
    /// Soft accent wash behind a selected row (`accent.soft`).
    pub accent_soft: Hsla,
    /// Hover wash behind a row or control (`element.hover`).
    pub element_hover: Hsla,
    pub element_background: Hsla,
    pub element_selected: Hsla,
    /// Pressed or raised-menu hover fill (`element.active`); differs from the
    /// raised ground and from `accent_soft` in both Parla themes.
    pub element_active: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_accent: Hsla,
    pub editor_document_highlight_read_background: Hsla,
}

/// Status colours.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatusColors {
    pub success: Hsla,
    pub warning: Hsla,
    pub error: Hsla,
}

/// Chat colours from a theme's optional `parla` object; `None` means "derive from
/// the interface colours" (see [`crate::parla_tokens`]).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ParlaOverrides {
    pub sidebar_background: Option<Hsla>,
    pub sidebar_text: Option<Hsla>,
    pub sidebar_active: Option<Hsla>,
    pub message_mention_background: Option<Hsla>,
    pub presence_active: Option<Hsla>,
    pub presence_away: Option<Hsla>,
    pub avatar_text: Option<Hsla>,
    pub danger_text: Option<Hsla>,
    pub warning_text: Option<Hsla>,
    pub sidebar_rail: Option<Hsla>,
    pub sidebar_border: Option<Hsla>,
    pub sidebar_hover: Option<Hsla>,
    pub sidebar_muted: Option<Hsla>,
    pub sidebar_text_strong: Option<Hsla>,
    pub signal: Option<Hsla>,
    pub signal_text: Option<Hsla>,
    pub signal_soft: Option<Hsla>,
    pub text_subtle: Option<Hsla>,
    pub mention_text: Option<Hsla>,
    pub accent_soft_text: Option<Hsla>,
    pub border_subtle: Option<Hsla>,
    pub success_soft: Option<Hsla>,
    pub warning_soft: Option<Hsla>,
    pub danger_soft: Option<Hsla>,
    pub border_control: Option<Hsla>,
    pub bg_sunken: Option<Hsla>,
    pub bg_overlay: Option<Hsla>,
    pub code_border: Option<Hsla>,
    pub viewer_bg: Option<Hsla>,
    pub viewer_stage: Option<Hsla>,
    pub viewer_text: Option<Hsla>,
    pub viewer_muted: Option<Hsla>,
    pub viewer_border: Option<Hsla>,
    pub viewer_control_hover: Option<Hsla>,
    pub viewer_nav_bg: Option<Hsla>,
    pub viewer_focus: Option<Hsla>,
    pub code_keyword: Option<Hsla>,
    pub code_string: Option<Hsla>,
    pub code_function: Option<Hsla>,
    pub code_comment: Option<Hsla>,
    pub code_number: Option<Hsla>,
    pub code_type: Option<Hsla>,
    pub logo_background: Option<Hsla>,
    pub logo_bubble: Option<Hsla>,
    pub logo_rim: Option<Hsla>,
    pub logo_accent_1: Option<Hsla>,
    pub logo_accent_2: Option<Hsla>,
    pub logo_accent_3: Option<Hsla>,
}

/// Where a theme comes from: the built-in brand pair, the built-in high
/// contrast pair, or a user (or plugin) theme file, whatever its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeKind {
    Brand,
    HighContrast,
    User,
}

/// One loaded theme.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub name: SharedString,
    pub appearance: Appearance,
    kind: ThemeKind,
    colors: ThemeColors,
    status: StatusColors,
    /// Chat tokens resolved once when the theme is built.
    tokens: ParlaTokens,
    parla: ParlaOverrides,
}

impl Theme {
    pub(crate) fn new(
        name: SharedString,
        appearance: Appearance,
        colors: ThemeColors,
        status: StatusColors,
        parla: ParlaOverrides,
    ) -> Self {
        let tokens = crate::parla_tokens::resolve(appearance, &colors, &status, &parla);
        Self {
            name,
            appearance,
            kind: ThemeKind::User,
            colors,
            status,
            tokens,
            parla,
        }
    }

    /// Built-in brand, built-in high contrast, or user.
    pub fn kind(&self) -> ThemeKind {
        self.kind
    }

    pub(crate) fn with_kind(mut self, kind: ThemeKind) -> Self {
        self.kind = kind;
        self
    }

    /// Interface colours.
    pub fn colors(&self) -> &ThemeColors {
        &self.colors
    }

    /// Status colours.
    pub fn status(&self) -> &StatusColors {
        &self.status
    }

    pub(crate) fn parla_overrides(&self) -> ParlaOverrides {
        self.parla
    }

    pub(crate) fn tokens(&self) -> ParlaTokens {
        self.tokens
    }
}

/// No theme with this name is registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeNotFound(pub String);

impl std::fmt::Display for ThemeNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "theme not found: {}", self.0)
    }
}

impl std::error::Error for ThemeNotFound {}

/// All known themes by name. A theme inserted under an existing name replaces it.
#[derive(Default)]
pub struct ThemeRegistry {
    themes: RwLock<HashMap<String, Arc<Theme>>>,
}

struct GlobalRegistry(Arc<ThemeRegistry>);

impl Global for GlobalRegistry {}

impl ThemeRegistry {
    /// The app-wide registry. Panics before [`crate::init`].
    pub fn global(cx: &App) -> Arc<Self> {
        cx.global::<GlobalRegistry>().0.clone()
    }

    pub(crate) fn set_global(cx: &mut App) {
        cx.set_global(GlobalRegistry(Arc::new(Self::default())));
    }

    /// Looks a theme up by name.
    pub fn get(&self, name: &str) -> Result<Arc<Theme>, ThemeNotFound> {
        self.themes
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(name)
            .cloned()
            .ok_or_else(|| ThemeNotFound(name.to_string()))
    }

    /// Every registered theme, light ones first, each group sorted by name
    /// (the Settings window lists them as cards).
    pub fn list(&self) -> Vec<Arc<Theme>> {
        let mut all: Vec<Arc<Theme>> = self
            .themes
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        // Parla Light, Parla Dark, the high contrast pair, then user themes,
        // light before dark, by name.
        let rank = |t: &Theme| match t.kind {
            ThemeKind::Brand => 0,
            ThemeKind::HighContrast => 1,
            ThemeKind::User => 2,
        };
        all.sort_by(|a, b| {
            (rank(a), a.appearance == Appearance::Dark, a.name.as_ref()).cmp(&(
                rank(b),
                b.appearance == Appearance::Dark,
                b.name.as_ref(),
            ))
        });
        all
    }

    /// Adds `theme`, replacing a same-named one.
    pub fn insert(&self, theme: Theme) {
        self.themes
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(theme.name.to_string(), Arc::new(theme));
    }
}

struct GlobalTheme(Arc<Theme>);

impl Global for GlobalTheme {}

/// Makes `theme` the active one.
pub(crate) fn set_active(cx: &mut App, theme: Arc<Theme>) {
    cx.set_global(GlobalTheme(theme));
}

/// Whether [`crate::init`] has activated a theme yet.
pub(crate) fn has_active(cx: &App) -> bool {
    cx.try_global::<GlobalTheme>().is_some()
}

/// Access to the active theme from an [`App`] (and anything that derefs to it).
pub trait ActiveTheme {
    /// The active theme. Panics before [`crate::init`].
    fn theme(&self) -> &Arc<Theme>;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Arc<Theme> {
        &self.global::<GlobalTheme>().0
    }
}
