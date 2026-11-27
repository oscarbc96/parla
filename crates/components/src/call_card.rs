//! Huddle and third-party call cards inside messages. A live huddle card joins
//! through the session; a third-party call opens its own link outside the app
//! (only safe schemes, see [`open_link`]); Parla never carries call media.

use domain::{CallCard, CallKind, ConversationId};
use gpui::{AnyElement, Hsla, InteractiveElement as _, IntoElement as _, div, prelude::*};
use themes::metrics;

use crate::links::{LinkAction, open_link};
use crate::rich_text::{RichColors, RichTextCx, mention_name};
use crate::sidebar::best_name;

/// What a card's button does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallAction {
    /// Join the conversation's live huddle (the session opens it per
    /// `calls.join_with`).
    JoinHuddle(ConversationId),
    /// Open a third-party call's link in the default browser.
    OpenUrl(String),
}

/// A call card resolved against the session.
#[derive(Clone, Debug, PartialEq)]
pub struct CallNode {
    pub heading: String,
    /// "Live", "Ended after 15 min", or the provider's name for a third-party call.
    pub status: String,
    /// "Started by ..." when the room's creator is known.
    pub starter: Option<String>,
    pub people: Vec<String>,
    pub live: bool,
    /// Button label and action; `None` when there is nothing to join.
    pub join: Option<(String, CallAction)>,
}

/// Resolves `card` (shown in the message `cx.message`).
pub fn call_node(card: &CallCard, cx: &RichTextCx) -> CallNode {
    let l = &cx.localizer;
    let people = card
        .participants
        .iter()
        .map(|u| mention_name(cx, u, None))
        .collect();
    let join_label = t!(l, "call-card-join").into_owned();
    match &card.kind {
        CallKind::Huddle => {
            let conversation = cx.message.as_ref().map(|m| m.conversation.clone());
            let live = card.ended.is_none()
                && conversation
                    .as_ref()
                    .is_some_and(|c| cx.source.huddle_live(c));
            let status = match (live, card.started, card.ended) {
                (true, ..) => t!(l, "huddle-card-live").into_owned(),
                (false, Some(s), Some(e)) if e - s >= 60 => {
                    t!(l, "huddle-card-ended-after", minutes = (e - s) / 60).into_owned()
                }
                _ => t!(l, "huddle-card-ended").into_owned(),
            };
            // The room's creator, never the message's sender (often a bot).
            let starter = card
                .created_by
                .as_ref()
                .and_then(|u| cx.source.member(u).and_then(best_name))
                .map(|n| t!(l, "huddle-card-started-by", name = n.as_ref()).into_owned());
            CallNode {
                heading: t!(l, "huddle-card-heading").into_owned(),
                status,
                starter,
                people,
                live,
                join: conversation
                    .filter(|_| live)
                    .map(|c| (join_label, CallAction::JoinHuddle(c))),
            }
        }
        CallKind::External { provider } => {
            let heading = card
                .title
                .as_deref()
                .filter(|t| !t.is_empty())
                .unwrap_or(provider)
                .to_string();
            let join = card
                .join_url
                .as_deref()
                .and_then(|u| match open_link(u) {
                    LinkAction::OpenExternal(u) => Some(u),
                    LinkAction::Refuse => None,
                })
                .map(|u| (join_label, CallAction::OpenUrl(u)));
            CallNode {
                heading,
                status: provider.to_string(),
                starter: None,
                people,
                live: card.ended.is_none(),
                join,
            }
        }
    }
}

/// The framed card: raised ground, 1 px border, `RADIUS_LG`; a live card
/// takes the success border, a dot before "Live" and a success Join button.
/// `next_id` gives the button its element id.
pub(crate) fn render_call(
    n: &CallNode,
    c: RichColors,
    id: (&'static str, usize),
    on_join: impl Fn(CallAction, &mut gpui::Window, &mut gpui::App) + 'static,
) -> AnyElement {
    // Only a live huddle (joined through the session) gets the success look; a
    // third-party call keeps the link colour of its outlined button.
    let huddle_live = n.live && matches!(n.join, Some((_, CallAction::JoinHuddle(_))));
    let status = div()
        .flex()
        .items_center()
        .gap(metrics::SPACE_3)
        .text_size(metrics::FONT_SIZE_XS)
        .text_color(if huddle_live {
            c.success
        } else if n.live {
            c.link
        } else {
            c.subtle
        });
    let status = if huddle_live {
        status.child(
            div()
                .debug_selector(|| "call-card-live-dot".to_string())
                .flex_none()
                .size(metrics::SPACE_4)
                .rounded_full()
                .bg(c.success),
        )
    } else {
        status
    }
    .child(
        div()
            .when(huddle_live, |d| d.font_weight(metrics::FONT_WEIGHT_BOLD))
            .child(n.status.clone()),
    );
    let mut text = div()
        .flex()
        .flex_col()
        .gap_0p5()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                .child(n.heading.clone()),
        )
        .child(status);
    if let Some(s) = &n.starter {
        text = text.child(div().text_xs().text_color(c.muted).child(s.clone()));
    }
    if !n.people.is_empty() {
        text = text.child(
            div()
                .text_xs()
                .text_color(c.muted)
                .child(n.people.join(", ")),
        );
    }
    let mut row = div()
        .debug_selector(|| "call-card".to_string())
        .flex()
        .items_center()
        .gap(metrics::SPACE_5)
        .max_w(metrics::LAYOUT_CALL_CARD_MAX)
        .p(metrics::SPACE_5)
        .rounded(metrics::RADIUS_LG)
        .border_1()
        .border_color(if huddle_live { c.success } else { c.border })
        .bg(c.raised)
        .child(text);
    if let Some((label, action)) = n.join.clone() {
        row = row.child(
            div()
                .id(id)
                .debug_selector(|| "call-card-join".to_string())
                .flex_none()
                .px(metrics::SPACE_6)
                .py(metrics::SPACE_2)
                .rounded(metrics::RADIUS_MD)
                .border_1()
                .border_color(if huddle_live { c.success } else { c.link })
                .bg(if huddle_live { c.success } else { c.raised })
                .text_color(if huddle_live { c.on_success } else { c.link })
                .font_weight(metrics::FONT_WEIGHT_MEDIUM)
                .cursor_pointer()
                // A colour change, not element opacity: GPUI Fast multiplies
                // opacity into each paint, so a border the colour of the fill
                // drew as a darker outline over the faded fill on hover.
                .hover(|s| {
                    let (fill, border, text) = join_hover(huddle_live, &c);
                    s.bg(fill).border_color(border).text_color(text)
                })
                .child(label)
                .on_click(move |_, w, app| on_join(action.clone(), w, app)),
        );
    }
    row.into_any_element()
}

/// The Join button's hover look, as `(fill, border, text)`. Main faded the
/// whole button to 0.9 as one image; GPUI Fast multiplies opacity into each
/// paint, so the fill and the border that sits on it must be the same opaque
/// colour or the border draws as a darker ring.
fn join_hover(huddle_live: bool, c: &RichColors) -> (Hsla, Hsla, Hsla) {
    // Main drew the whole button at 0.9 as one image. Each colour is
    // composited once over the card's raised ground instead: GPUI Fast would
    // multiply the opacity into the border paint, which sits on the faded
    // fill, and draw a darker ring.
    let strength = 0.9;
    if huddle_live {
        let fill = themes::color::faded(c.success, c.raised, strength);
        let text = themes::color::faded(c.on_success, c.raised, strength);
        (fill, fill, text)
    } else {
        let fg = themes::color::faded(c.link, c.raised, strength);
        (c.raised, fg, fg)
    }
}

#[cfg(test)]
mod tests {
    use super::join_hover;
    use crate::rich_text::RichColors;
    use gpui::{Hsla, TestAppContext};

    fn opaque(c: Hsla) -> bool {
        (themes::color::hsla_to_rgba(c).a - 1.0).abs() < 0.001
    }

    #[gpui::test]
    fn a_live_join_button_hovers_as_one_opaque_faded_fill(cx: &mut TestAppContext) {
        cx.update(|cx| {
            themes::init_default(cx);
            let reg = themes::ThemeRegistry::global(cx);
            for name in [themes::DEFAULT_LIGHT, themes::DEFAULT_DARK] {
                let theme = reg.get(name).unwrap();
                let c = RichColors::from_theme(&theme);
                let (fill, border, text) = join_hover(true, &c);
                let want = themes::color::faded(c.success, c.raised, 0.9);
                assert_eq!(
                    fill, border,
                    "{name}: the border must not ring the fill darker"
                );
                assert_eq!(fill, want, "{name}: in main the whole button faded to 0.9");
                assert_ne!(
                    fill, c.success,
                    "{name}: the fill is faded, not full success"
                );
                assert_eq!(text, themes::color::faded(c.on_success, c.raised, 0.9));
                for (label, colour) in [("fill", fill), ("border", border), ("text", text)] {
                    assert!(opaque(colour), "{name}: {label} must be opaque: {colour:?}");
                }
            }
        });
    }

    #[gpui::test]
    fn an_external_join_button_hovers_without_an_off_token_tint(cx: &mut TestAppContext) {
        cx.update(|cx| {
            themes::init_default(cx);
            let reg = themes::ThemeRegistry::global(cx);
            for name in [themes::DEFAULT_LIGHT, themes::DEFAULT_DARK] {
                let theme = reg.get(name).unwrap();
                let c = RichColors::from_theme(&theme);
                let (fill, border, text) = join_hover(false, &c);
                let want = themes::color::faded(c.link, c.raised, 0.9);
                assert_eq!(fill, c.raised, "{name}: the card's ground stays");
                assert_eq!(border, want, "{name}: main faded the border to 0.9");
                assert_eq!(text, want, "{name}: main faded the label to 0.9");
                assert_ne!(border, c.link, "{name}: not the full-strength link colour");
                assert!(
                    opaque(border) && opaque(text),
                    "{name}: opaque hover colours"
                );
            }
        });
    }
}
