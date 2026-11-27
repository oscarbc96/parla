//! Row density (`settings.density`): the sidebar row height and the vertical
//! padding of message rows. Read once per list or per rebuild, never per row.

use gpui::{App, Pixels};
use settings::{Density, SettingsStore};

/// The user's density; compact when no settings store is installed.
pub fn current(cx: &App) -> Density {
    cx.try_global::<SettingsStore>()
        .map_or(Density::Compact, |s| s.current().density)
}

/// Height of a sidebar row: 28 compact, 34 cozy.
pub fn row_height(d: Density) -> Pixels {
    match d {
        Density::Compact => themes::metrics::LAYOUT_ROW_COMPACT,
        Density::Cozy => themes::metrics::LAYOUT_ROW_COZY,
    }
}

/// Vertical padding of a message row: 6 compact, 10 cozy.
pub fn message_pad(d: Density) -> Pixels {
    match d {
        Density::Compact => themes::metrics::SPACE_3,
        Density::Cozy => themes::metrics::LAYOUT_MSG_PAD_COZY,
    }
}

/// Vertical padding of a message that continues the previous author's: 2 compact, 6 cozy.
pub fn message_gap(d: Density) -> Pixels {
    match d {
        Density::Compact => themes::metrics::SPACE_1,
        Density::Cozy => themes::metrics::SPACE_3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heights_follow_density() {
        assert_eq!(
            row_height(Density::Compact),
            themes::metrics::LAYOUT_ROW_COMPACT
        );
        assert_eq!(row_height(Density::Cozy), themes::metrics::LAYOUT_ROW_COZY);
        assert!(message_pad(Density::Cozy) > message_pad(Density::Compact));
        assert!(message_gap(Density::Cozy) > message_gap(Density::Compact));
    }
}
