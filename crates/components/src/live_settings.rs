//! The settings views show live: code font, avatars, clock and inline images.
//! Views copy them from the settings global when it changes (never per row).

use gpui::{App, Global, Pixels, SharedString};
use settings::{Clock, InlineImages, SettingsStore};

/// The display settings a message list follows.
#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    pub code_font: SharedString,
    pub show_avatars: bool,
    pub clock: Clock,
    pub inline_images: InlineImages,
}

/// The current display settings; `None` when no settings store is installed.
pub fn display(cx: &App) -> Option<Display> {
    let s = cx.try_global::<SettingsStore>()?.current();
    Some(Display {
        code_font: s.buffer_font_family.clone().into(),
        show_avatars: s.messages.show_avatars,
        clock: s.messages.clock,
        inline_images: s.messages.inline_images,
    })
}

/// The code font family; the default when no settings store is installed.
pub fn code_font(cx: &App) -> SharedString {
    cx.try_global::<SettingsStore>().map_or_else(
        || crate::rich_text::DEFAULT_CODE_FONT.into(),
        |s| s.current().buffer_font_family.clone().into(),
    )
}

/// The composer send key; Enter when no settings store is installed.
pub fn send_key(cx: &App) -> settings::SendKey {
    cx.try_global::<SettingsStore>()
        .map_or(settings::SendKey::Enter, |s| s.current().send_key)
}

/// The interface font, cached when settings change so a window root's render
/// clones a `SharedString` (a refcount) and never a `String`.
#[derive(Clone)]
struct UiFont {
    family: SharedString,
    size: Pixels,
}

impl Global for UiFont {}

fn read_ui_font(cx: &App) -> Option<UiFont> {
    let s = cx.try_global::<SettingsStore>()?.current();
    Some(UiFont {
        family: s.ui_font_family.clone().into(),
        size: gpui::px(s.ui_font_size),
    })
}

/// Caches `ui_font_family` and `ui_font_size` and refreshes them on every
/// settings reload. Call once after the settings global is installed.
pub fn init_ui_font(cx: &mut App) {
    if let Some(f) = read_ui_font(cx) {
        cx.set_global(f);
    }
    cx.observe_global::<SettingsStore>(|cx| {
        if let Some(f) = read_ui_font(cx) {
            cx.set_global(f);
            cx.refresh_windows();
        }
    })
    .detach();
}

/// The interface font family and size for a window root; Inter at 14 until
/// [`init_ui_font`] ran.
pub fn ui_font(cx: &App) -> (SharedString, Pixels) {
    cx.try_global::<UiFont>().map_or_else(
        || {
            let d = settings::Settings::default();
            (d.ui_font_family.into(), gpui::px(d.ui_font_size))
        },
        |f| (f.family.clone(), f.size),
    )
}
