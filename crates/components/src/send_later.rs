//! Send later: the presets menu and its computed times.

use i18n::Localizer;
use jiff::ZonedDateTime;
use jiff::tz::TimeZone;
use settings::Clock;

/// What the menu needs from settings and the provider.
#[derive(Clone, Debug)]
pub struct SendLaterConfig {
    /// `send_later.presets` (see `domain::parse_preset`).
    pub presets: Vec<String>,
    /// The zone times are read and shown in.
    pub zone: TimeZone,
    pub clock: Clock,
    /// The provider can schedule a message itself.
    pub server: bool,
}

/// One preset with its time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendChoice {
    pub preset: String,
    /// Unix seconds.
    pub at: i64,
}

/// The presets that parse and lie ahead, in the given order, without repeats.
pub fn preset_choices(presets: &[String], now: &ZonedDateTime) -> Vec<SendChoice> {
    let mut out: Vec<SendChoice> = Vec::new();
    for p in presets {
        let Some(t) = domain::parse_preset(p, now) else {
            continue;
        };
        let at = t.as_second();
        // "tomorrow" and "monday" can name the same moment: list it once.
        if !out.iter().any(|c| c.at == at) {
            out.push(SendChoice {
                preset: p.clone(),
                at,
            });
        }
    }
    out
}

/// Files, or a provider that cannot schedule, keep the message in Parla.
pub fn needs_local(has_files: bool, server: bool) -> bool {
    has_files || !server
}

/// One row of the Send later menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendLaterRow {
    pub label: String,
    /// Unix seconds; `None` for "Custom...".
    pub at: Option<i64>,
    /// Why the row cannot be chosen (too far ahead).
    pub disabled: Option<String>,
}

/// The date and time of `at` (unix seconds) in the schedule zone.
pub fn time_label(at: i64, cfg: &SendLaterConfig, l: &Localizer) -> String {
    match jiff::Timestamp::from_second(at) {
        Ok(t) => l.date_time(t, &cfg.zone, crate::message_row::hour_cycle(cfg.clock)),
        Err(_) => String::new(),
    }
}
