//! Day dividers and the "New" divider of the timeline, computed once per
//! window change (never per frame): pure functions over the loaded messages.

use domain::{DeliveryState, Message, MessageTs, UserId};
use gpui::{AnyElement, div, prelude::*};
use i18n::Localizer;
use jiff::{Timestamp, civil::Date, tz::TimeZone};
use themes::{Theme, metrics, parla_tokens};

/// Which day a day divider names, relative to the clock at computation time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DayLabel {
    Today,
    Yesterday,
    /// Any other day, shown as a localised medium date.
    On(i64),
}

/// What is drawn above one message row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Lead {
    pub day: Option<DayLabel>,
    pub new: bool,
}

fn secs(ts: &MessageTs) -> Option<i64> {
    ts.0.split('.').next()?.parse().ok()
}

/// The local civil date of `secs` in `tz` (changes at local midnight).
pub fn local_day(secs: i64, tz: &TimeZone) -> Option<Date> {
    local_date(secs, tz)
}

fn local_date(secs: i64, tz: &TimeZone) -> Option<Date> {
    Some(
        Timestamp::from_second(secs)
            .ok()?
            .to_zoned_datetime(tz.clone())
            .date(),
    )
}

/// The dividers of `msgs`, one entry per message.
///
/// A day divider sits above the first message of each local day in `tz`
/// (civil dates, so DST shifts and zone offsets cannot split or merge a day).
/// The "New" divider sits above the first message after `read` that is
/// delivered and not the user's own; there is none without a marker.
pub fn compute(
    msgs: &[Message],
    tz: &TimeZone,
    now_secs: i64,
    read: Option<&MessageTs>,
    me: Option<&UserId>,
) -> Vec<Lead> {
    let today = local_date(now_secs, tz);
    let yesterday = today.and_then(|d| d.yesterday().ok());
    let mut out = Vec::with_capacity(msgs.len());
    let mut prev: Option<Date> = None;
    let mut new_done = read.is_none();
    for m in msgs {
        let mut lead = Lead::default();
        if let Some(s) = secs(&m.id.ts)
            && let Some(d) = local_date(s, tz)
        {
            // An unconfirmed row at the bottom may carry an older local day:
            // it never draws a divider out of order.
            let behind = m.state != DeliveryState::Delivered && prev.is_some_and(|p| d < p);
            if behind {
                out.push(lead_new(&mut new_done, read, m, me));
                continue;
            }
            if prev != Some(d) {
                lead.day = Some(if Some(d) == today {
                    DayLabel::Today
                } else if Some(d) == yesterday {
                    DayLabel::Yesterday
                } else {
                    DayLabel::On(s)
                });
            }
            prev = Some(d);
        }
        lead.new = lead_new(&mut new_done, read, m, me).new;
        out.push(lead);
    }
    out
}

/// The "New" part of a row's lead: the first delivered message after `read`
/// that is not the user's own.
fn lead_new(done: &mut bool, read: Option<&MessageTs>, m: &Message, me: Option<&UserId>) -> Lead {
    let mut lead = Lead::default();
    if !*done
        && let Some(r) = read
        && m.id.ts > *r
        && m.state == DeliveryState::Delivered
        && !(m.author.is_some() && m.author.as_ref() == me)
    {
        lead.new = true;
        *done = true;
    }
    lead
}

/// The text of a day divider.
pub fn day_text(day: DayLabel, tz: &TimeZone, l: &Localizer) -> String {
    match day {
        DayLabel::Today => t!(l, "timeline-day-today").into_owned(),
        DayLabel::Yesterday => t!(l, "timeline-day-yesterday").into_owned(),
        DayLabel::On(s) => Timestamp::from_second(s)
            .map(|t| l.date(t, tz))
            .unwrap_or_default(),
    }
}

/// The day divider element: a pill centred on a hairline.
pub fn render_day(text: String, theme: &Theme) -> AnyElement {
    let c = theme.colors();
    let rule = || div().h(gpui::px(1.0)).flex_1().bg(c.border);
    div()
        .debug_selector(|| "timeline-day-divider".into())
        .w_full()
        .flex()
        .items_center()
        .gap(metrics::SPACE_5)
        .px(metrics::SPACE_7)
        .pt(metrics::SPACE_5)
        .pb(metrics::SPACE_3)
        .child(rule())
        .child(
            div()
                .h(metrics::SPACE_8)
                .flex()
                .items_center()
                .px(metrics::SPACE_5)
                .rounded(metrics::RADIUS_FULL)
                .border_1()
                .border_color(c.border)
                .bg(c.surface_background)
                .text_size(metrics::FONT_SIZE_XS)
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(c.text_muted)
                .child(text),
        )
        .child(rule())
        .into_any_element()
}

/// The "New" divider element: a signal-coloured label and line.
pub fn render_new(l: &Localizer, theme: &Theme) -> AnyElement {
    let tk = parla_tokens(theme);
    div()
        .debug_selector(|| "timeline-new-divider".into())
        .w_full()
        .flex()
        .items_center()
        .gap(metrics::SPACE_4)
        .px(metrics::SPACE_7)
        .py(metrics::SPACE_2)
        .text_size(metrics::FONT_SIZE_2XS)
        // Caps tracking (tokens: tracking.caps 0.06em) waits for letter spacing in GPUI.
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(tk.signal_text)
        .child(div().h(gpui::px(1.0)).flex_1().bg(tk.signal))
        .child(t!(l, "timeline-new-divider").to_uppercase())
        .into_any_element()
}
