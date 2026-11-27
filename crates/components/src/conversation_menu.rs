//! Per-conversation notification level: the menu's choices and the mark a
//! sidebar row shows for a level other than the default.

use domain::{ConversationPrefs, NotifyLevel};
use i18n::Localizer;

/// The levels in menu order.
pub const LEVELS: [NotifyLevel; 4] = [
    NotifyLevel::Default,
    NotifyLevel::All,
    NotifyLevel::Mentions,
    NotifyLevel::Nothing,
];

/// The menu's name of a level.
pub fn level_label(level: NotifyLevel, l: &Localizer) -> String {
    match level {
        NotifyLevel::Default => t!(l, "notify-level-default"),
        NotifyLevel::All => t!(l, "notify-level-all"),
        NotifyLevel::Mentions => t!(l, "notify-level-mentions"),
        NotifyLevel::Nothing => t!(l, "notify-level-nothing"),
    }
    .into_owned()
}

/// The sidebar mark of a level; `None` for the default.
pub fn level_mark(level: NotifyLevel) -> Option<&'static str> {
    match level {
        NotifyLevel::Default => None,
        NotifyLevel::All => Some("\u{25CF}"),
        NotifyLevel::Mentions => Some("@"),
        NotifyLevel::Nothing => Some("\u{00D7}"),
    }
}

/// `prefs` with the level replaced; the mute flag is kept.
pub fn with_level(prefs: ConversationPrefs, level: NotifyLevel) -> ConversationPrefs {
    ConversationPrefs { level, ..prefs }
}

/// `prefs` with the mute flag flipped; the level is kept.
pub fn toggled_mute(prefs: ConversationPrefs) -> ConversationPrefs {
    ConversationPrefs {
        muted: !prefs.muted,
        ..prefs
    }
}
