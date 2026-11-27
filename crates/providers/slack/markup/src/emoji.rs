//! Emoji shortcodes: standard (unicode from `emojis`) or custom.

use domain::EmojiRef;
use emojis::SkinTone;

/// Parse a shortcode as used in text (`thumbsup`) or in reactions
/// (`+1::skin-tone-3`). Surrounding colons are ignored.
pub fn emoji_ref(name: &str) -> EmojiRef {
    let name = name.trim_matches(':');
    let (base, tone) = match name.split_once("::skin-tone-") {
        Some((b, t)) => (b, parse_tone(t.trim_end_matches(':'))),
        None => (name, None),
    };
    build(base, tone)
}

/// Slack tones run 2 (light) to 6 (dark).
pub(crate) fn parse_tone(s: &str) -> Option<u8> {
    s.parse::<u8>().ok().filter(|n| (2..=6).contains(n))
}

pub(crate) fn build(base: &str, tone: Option<u8>) -> EmojiRef {
    let found = emojis::get_by_shortcode(base);
    let unicode = found.map(|e| {
        let toned = tone.and_then(|n| e.with_skin_tone(tone_of(n)));
        toned.unwrap_or(e).as_str().into()
    });
    EmojiRef {
        shortcode: base.into(),
        unicode,
        skin_tone: tone,
    }
}

fn tone_of(n: u8) -> SkinTone {
    match n {
        2 => SkinTone::Light,
        3 => SkinTone::MediumLight,
        4 => SkinTone::Medium,
        5 => SkinTone::MediumDark,
        _ => SkinTone::Dark,
    }
}

pub(crate) fn is_name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'+' | b'-' | b'\'')
}
