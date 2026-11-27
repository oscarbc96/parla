//! The workspace badge on the left rail: initials on a colour derived from the name.

use domain::ImageRef;
use gpui::{
    AnyElement, App, BorrowAppContext as _, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, Styled as _, Window, div, img, prelude::FluentBuilder as _,
    size,
};

use themes::{ActiveTheme as _, parla_tokens};

use crate::media::{ImageCache, ImageState};

/// Side of the square badge.
pub const SIZE: gpui::Pixels = themes::metrics::LAYOUT_WORKSPACE_BADGE;

/// Up to two initials: the first letters of the first two words, or the first
/// two letters of a single word. `?` for an empty name.
pub fn initials(name: &str) -> String {
    let mut words = name
        .split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric));
    let first = words.next();
    let second = words.next();
    let pick = |w: &str, n: usize| -> String {
        w.chars().filter(|c| c.is_alphanumeric()).take(n).collect()
    };
    let s = match (first, second) {
        (Some(a), Some(b)) => format!("{}{}", pick(a, 1), pick(b, 1)),
        (Some(a), None) => pick(a, 2),
        _ => String::new(),
    };
    if s.is_empty() {
        "?".into()
    } else {
        s.to_uppercase()
    }
}

/// A stable hue in `0.0..1.0` for the name (FNV-1a).
pub fn hue(name: &str) -> f32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in name.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    (h % 360) as f32 / 360.0
}

/// The badge element: the team icon once it is loaded (through the media
/// cache), initials on a colour until then and when it cannot be fetched.
pub fn workspace_badge(name: &str, icon: Option<ImageRef>) -> AnyElement {
    workspace_badge_styled(name, icon, SIZE, themes::metrics::RADIUS_MD, 1.0, None)
}

/// [`workspace_badge`] with its fill or picture at `strength` over `ground`:
/// main faded the whole badge as one image, so its initials stayed white on
/// the pale fill in light and grey in dark. Element opacity would fade the
/// initials over the already-faded fill; [`themes::color::faded`] keeps main's
/// look.
pub fn workspace_badge_dimmed(
    name: &str,
    icon: Option<ImageRef>,
    strength: f32,
    ground: gpui::Hsla,
) -> AnyElement {
    workspace_badge_styled(
        name,
        icon,
        SIZE,
        themes::metrics::RADIUS_MD,
        strength,
        Some(ground),
    )
}

/// [`workspace_badge`] at another side and corner radius (the rail's tile).
pub fn workspace_badge_sized(
    name: &str,
    icon: Option<ImageRef>,
    side: gpui::Pixels,
    radius: gpui::Pixels,
) -> AnyElement {
    workspace_badge_styled(name, icon, side, radius, 1.0, None)
}

fn workspace_badge_styled(
    name: &str,
    icon: Option<ImageRef>,
    side: gpui::Pixels,
    radius: gpui::Pixels,
    strength: f32,
    ground: Option<gpui::Hsla>,
) -> AnyElement {
    WorkspaceBadge {
        name: name.to_string(),
        icon,
        side,
        radius,
        strength,
        ground,
    }
    .into_any_element()
}

#[derive(IntoElement)]
struct WorkspaceBadge {
    name: String,
    icon: Option<ImageRef>,
    side: gpui::Pixels,
    radius: gpui::Pixels,
    /// Strength of the fill or picture; the initials are composited over
    /// `ground` at the same strength, as main's group opacity drew them.
    strength: f32,
    /// The ground a dimmed badge sits on; `None` at full strength.
    ground: Option<gpui::Hsla>,
}

impl WorkspaceBadge {
    fn initials(&self, cx: &App) -> AnyElement {
        let fill = themes::avatar_fill(hue(&self.name), cx.theme());
        let text = parla_tokens(cx.theme()).avatar_text;
        let fade = |colour: gpui::Hsla| match self.ground {
            Some(ground) => themes::color::faded(colour, ground, self.strength),
            None => colour,
        };
        div()
            .debug_selector(|| "workspace-badge-initials".into())
            .flex()
            .items_center()
            .justify_center()
            .size_full()
            .rounded(self.radius)
            .bg(fade(fill))
            .text_color(fade(text))
            .font_weight(FontWeight::BOLD)
            .child(initials(&self.name))
            .into_any_element()
    }
}

impl RenderOnce for WorkspaceBadge {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let image = match (&self.icon, cx.has_global::<ImageCache>()) {
            (Some(src), true) => {
                let scale = window.scale_factor();
                let boxed = size(self.side, self.side);
                match cx.update_global::<ImageCache, _>(|c, cx| c.get(src, boxed, scale, cx)) {
                    ImageState::Ready(image) => Some(image),
                    _ => None,
                }
            }
            _ => None,
        };
        let inner = match image {
            Some(image) => img(image)
                .debug_selector(|| "workspace-badge-icon".into())
                .size_full()
                .rounded(self.radius)
                .when(self.strength < 1.0, |img| img.opacity(self.strength))
                .into_any_element(),
            None => self.initials(cx),
        };
        div()
            .debug_selector(|| "workspace-badge".into())
            .size(self.side)
            .rounded(self.radius)
            .overflow_hidden()
            .child(inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_cases() {
        assert_eq!(initials("Acme Corp"), "AC");
        assert_eq!(initials("acme"), "AC");
        assert_eq!(initials("  "), "?");
        assert_eq!(initials("Acme - Corp Inc"), "AC");
        assert_eq!(initials("x"), "X");
    }

    #[test]
    fn hue_is_stable_and_in_range() {
        assert_eq!(hue("Acme"), hue("Acme"));
        assert!((0.0..1.0).contains(&hue("Acme")));
        assert_ne!(hue("Acme"), hue("Globex"));
    }
}
