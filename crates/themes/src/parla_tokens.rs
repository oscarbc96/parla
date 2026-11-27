//! Chat-specific colour tokens: the optional `parla` object in a theme's `style`,
//! falling back to the interface tokens.

use gpui::Hsla;

use crate::color::rgb_to_hsla;
use crate::model::{Appearance, ParlaOverrides, StatusColors, Theme, ThemeColors};

/// Colours of the Parla logo (`assets/brand/logo.svg`). `rim` is the outline of
/// the speech bubble; a fully transparent rim means no outline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogoTokens {
    pub background: Hsla,
    pub bubble: Hsla,
    pub rim: Hsla,
    pub accent_1: Hsla,
    pub accent_2: Hsla,
    pub accent_3: Hsla,
}

impl LogoTokens {
    /// The logo colours of the light or dark brand artwork.
    pub fn defaults(appearance: Appearance) -> Self {
        let rgb = |hex: u32| rgb_to_hsla(gpui::rgb(hex));
        match appearance {
            Appearance::Light => Self {
                background: rgb(0xf8f7f5),
                bubble: rgb(0x211e4c),
                rim: rgb_to_hsla(gpui::rgba(0x0000_0000)),
                accent_1: rgb(0xffa611),
                accent_2: rgb(0xfe6129),
                accent_3: rgb(0xfd3459),
            },
            Appearance::Dark => Self {
                background: rgb(0x121215),
                bubble: rgb(0x211f47),
                rim: rgb(0x3a3a78),
                accent_1: rgb(0xfea604),
                accent_2: rgb(0xf85e22),
                accent_3: rgb(0xfd2e57),
            },
        }
    }
}

/// Chat colours resolved for one theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParlaTokens {
    pub sidebar_background: Hsla,
    pub sidebar_text: Hsla,
    pub sidebar_active: Hsla,
    pub message_mention_background: Hsla,
    pub presence_active: Hsla,
    pub presence_away: Hsla,
    /// Text on a workspace avatar whose colour is computed from its name.
    pub avatar_text: Hsla,
    /// Text on a danger-coloured badge.
    pub danger_text: Hsla,
    /// Text on a warning-coloured badge.
    pub warning_text: Hsla,
    pub logo: LogoTokens,
    /// Workspace rail background.
    pub sidebar_rail: Hsla,
    /// Hairlines on the sidebar and rail.
    pub sidebar_border: Hsla,
    /// Hovered sidebar row.
    pub sidebar_hover: Hsla,
    /// Muted sidebar text and icons.
    pub sidebar_muted: Hsla,
    /// Active and unread sidebar text.
    pub sidebar_text_strong: Hsla,
    /// The signal colour: active marks, the new-messages line.
    pub signal: Hsla,
    /// Signal-coloured text on the surface.
    pub signal_text: Hsla,
    /// Soft signal ground: image file badges.
    pub signal_soft: Hsla,
    /// Subtle text: timestamps, hints.
    pub text_subtle: Hsla,
    /// Text of an @mention chip.
    pub mention_text: Hsla,
    /// Text on the soft accent ground (`accent.soft`).
    pub accent_soft_text: Hsla,
    /// Quiet separators inside a surface.
    pub border_subtle: Hsla,
    /// Soft success ground (an outcome pill); `status.success` text reads on it.
    pub success_soft: Hsla,
    /// Soft warning ground; `status.warning` text reads on it.
    pub warning_soft: Hsla,
    /// Soft danger ground; `status.error` text reads on it.
    pub danger_soft: Hsla,
    /// Border of a text field or an unticked check box (3:1 on the grounds).
    pub border_control: Hsla,
    /// Sunken ground (`bg.sunken`): the head of a code block.
    pub bg_sunken: Hsla,
    /// Dimmed ground behind a first-run tour spotlight (`bg.overlay`).
    pub bg_overlay: Hsla,
    /// Frame of a code block (`code.border`).
    pub code_border: Hsla,
    /// Viewer window ground (header and footer) (`viewer.bg`).
    pub viewer_bg: Hsla,
    /// Ground behind the image (`viewer.stage`).
    pub viewer_stage: Hsla,
    /// Viewer text and active controls (`viewer.text`).
    pub viewer_text: Hsla,
    /// Viewer secondary text and icons (`viewer.muted`).
    pub viewer_muted: Hsla,
    /// Viewer hairlines (`viewer.border`).
    pub viewer_border: Hsla,
    /// Hovered viewer control (`viewer.control-hover`).
    pub viewer_control_hover: Hsla,
    /// Previous and next buttons over the image (`viewer.nav-bg`).
    pub viewer_nav_bg: Hsla,
    /// Focus ring on the viewer's dark surface (`viewer.focus`).
    pub viewer_focus: Hsla,
    /// Syntax colours of code (`code.keyword` ... `code.type`).
    pub code_keyword: Hsla,
    pub code_string: Hsla,
    pub code_function: Hsla,
    pub code_comment: Hsla,
    pub code_number: Hsla,
    pub code_type: Hsla,
}

/// Reads the `parla` extension object of `theme`, falling back to interface tokens:
/// `panel.background`, `text`, `element.selected`,
/// `editor.document_highlight.read_background`, `status.success`, `status.warning`;
/// `avatar_text` and `danger_text` to white, `warning_text` to black.
/// The new chrome keys (`sidebar_rail`, `signal`, `text_subtle`, ...) fall back to the sidebar or interface colours.
/// The `logo.*` keys fall back to the defaults of the theme's appearance.
pub fn parla_tokens(theme: &Theme) -> ParlaTokens {
    theme.tokens()
}

/// Resolves the tokens once, when a theme is built.
pub(crate) fn resolve(
    appearance: Appearance,
    colors: &ThemeColors,
    status: &StatusColors,
    o: &ParlaOverrides,
) -> ParlaTokens {
    let d = LogoTokens::defaults(appearance);
    let sidebar_background = o.sidebar_background.unwrap_or(colors.panel_background);
    let sidebar_text = o.sidebar_text.unwrap_or(colors.text);
    ParlaTokens {
        sidebar_background,
        sidebar_text,
        sidebar_active: o.sidebar_active.unwrap_or(colors.element_selected),
        message_mention_background: o
            .message_mention_background
            .unwrap_or(colors.editor_document_highlight_read_background),
        presence_active: o.presence_active.unwrap_or(status.success),
        presence_away: o.presence_away.unwrap_or(status.warning),
        avatar_text: o.avatar_text.unwrap_or_else(gpui::white),
        danger_text: o.danger_text.unwrap_or_else(gpui::white),
        warning_text: o.warning_text.unwrap_or_else(gpui::black),
        logo: LogoTokens {
            background: o.logo_background.unwrap_or(d.background),
            bubble: o.logo_bubble.unwrap_or(d.bubble),
            rim: o.logo_rim.unwrap_or(d.rim),
            accent_1: o.logo_accent_1.unwrap_or(d.accent_1),
            accent_2: o.logo_accent_2.unwrap_or(d.accent_2),
            accent_3: o.logo_accent_3.unwrap_or(d.accent_3),
        },
        sidebar_rail: o.sidebar_rail.unwrap_or(sidebar_background),
        sidebar_border: o.sidebar_border.unwrap_or(sidebar_text.opacity(0.1)),
        sidebar_hover: o.sidebar_hover.unwrap_or(sidebar_text.opacity(0.07)),
        sidebar_muted: o.sidebar_muted.unwrap_or(sidebar_text.opacity(0.65)),
        sidebar_text_strong: o.sidebar_text_strong.unwrap_or(sidebar_text),
        signal: o.signal.unwrap_or(colors.text_accent),
        signal_text: o.signal_text.unwrap_or(colors.text_accent),
        signal_soft: o.signal_soft.unwrap_or(colors.text_accent.opacity(0.14)),
        text_subtle: o.text_subtle.unwrap_or(colors.text_muted),
        mention_text: o.mention_text.unwrap_or(colors.text),
        accent_soft_text: o.accent_soft_text.unwrap_or(colors.text),
        border_subtle: o.border_subtle.unwrap_or(colors.border),
        success_soft: o.success_soft.unwrap_or(status.success.opacity(0.14)),
        warning_soft: o.warning_soft.unwrap_or(status.warning.opacity(0.14)),
        danger_soft: o.danger_soft.unwrap_or(status.error.opacity(0.14)),
        border_control: o.border_control.unwrap_or(colors.border_strong),
        bg_sunken: o.bg_sunken.unwrap_or(colors.background),
        bg_overlay: o.bg_overlay.unwrap_or(gpui::black().opacity(0.45)),
        code_border: o.code_border.unwrap_or(colors.border),
        viewer_bg: o.viewer_bg.unwrap_or(colors.background),
        viewer_stage: o.viewer_stage.unwrap_or(colors.background),
        viewer_text: o.viewer_text.unwrap_or(colors.text),
        viewer_muted: o.viewer_muted.unwrap_or(colors.text_muted),
        viewer_border: o.viewer_border.unwrap_or(colors.border),
        viewer_control_hover: o.viewer_control_hover.unwrap_or(colors.element_hover),
        viewer_nav_bg: o
            .viewer_nav_bg
            .unwrap_or(colors.elevated_surface_background),
        viewer_focus: o.viewer_focus.unwrap_or(colors.border_focused),
        code_keyword: o.code_keyword.unwrap_or(colors.text_accent),
        code_string: o.code_string.unwrap_or(status.success),
        code_function: o.code_function.unwrap_or(colors.text),
        code_comment: o.code_comment.unwrap_or(colors.text_muted),
        code_number: o.code_number.unwrap_or(status.warning),
        code_type: o.code_type.unwrap_or(status.error),
    }
}
