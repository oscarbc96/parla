//! The UI icon set: Parla's own 24 px line icons (`assets/icons/ui/*.svg`,
//! stroke `currentColor`), compiled into the binary and drawn from memory, so
//! they load the same in the app and in the headless visual harness and need
//! no asset source. [`Icon`] takes its size from `themes::metrics::ICON_SIZE_*`
//! and its colour from the active theme.

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, Hsla, InspectorElementId, IntoElement,
    LayoutId, Pixels, RenderOnce, SharedString, Style, TransformationMatrix, Window, size,
};
use themes::{ActiveTheme, metrics};

macro_rules! icons {
    ($($variant:ident => $file:literal,)*) => {
        // One icon of the set; the variant name is the file name in PascalCase.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum IconName {
            $($variant,)*
        }

        impl IconName {
            // Every icon, in file-name order.
            pub const ALL: &'static [IconName] = &[$(IconName::$variant,)*];

            // The file stem under `assets/icons/ui/`.
            pub fn file(self) -> &'static str {
                match self { $(IconName::$variant => $file,)* }
            }

            // The sprite-atlas key of the rasterised icon, a static string so
            // drawing an icon allocates nothing.
            fn atlas_key(self) -> &'static str {
                match self { $(IconName::$variant => concat!("parla-icon:", $file),)* }
            }

            // The SVG source, compiled in.
            pub fn svg(self) -> &'static [u8] {
                match self {
                    $(IconName::$variant => {
                        include_bytes!(concat!("../../../assets/icons/ui/", $file, ".svg"))
                    })*
                }
            }
        }
    };
}

icons! {
    Alert => "alert",
    Archive => "archive",
    ArrowDown => "arrow-down",
    ArrowLeft => "arrow-left",
    ArrowRight => "arrow-right",
    ArrowUp => "arrow-up",
    At => "at",
    Backspace => "backspace",
    Bell => "bell",
    BellOff => "bell-off",
    Bold => "bold",
    Bookmark => "bookmark",
    Calendar => "calendar",
    Captions => "captions",
    Check => "check",
    CheckCircle => "check-circle",
    ChevDown => "chev-down",
    ChevLeft => "chev-left",
    ChevRight => "chev-right",
    ChevUp => "chev-up",
    Clock => "clock",
    Code => "code",
    Command => "command",
    Compose => "compose",
    // The control key (a caret), as drawn in key caps; not a general chevron.
    Control => "control",
    Copy => "copy",
    Download => "download",
    External => "external",
    Eye => "eye",
    EyeOff => "eye-off",
    File => "file",
    Filter => "filter",
    Folder => "folder",
    Forward => "forward",
    Gif => "gif",
    Globe => "globe",
    Grid => "grid",
    Hand => "hand",
    Hash => "hash",
    Headphones => "headphones",
    Home => "home",
    Image => "image",
    Inbox => "inbox",
    Info => "info",
    Italic => "italic",
    Key => "key",
    KeyTab => "key-tab",
    Keyboard => "keyboard",
    Layers => "layers",
    Link => "link",
    List => "list",
    ListOrdered => "list-ordered",
    Lock => "lock",
    Logout => "logout",
    Maximize => "maximize",
    Message => "message",
    Mic => "mic",
    MicOff => "mic-off",
    Monitor => "monitor",
    Moon => "moon",
    More => "more",
    MoreV => "more-v",
    Palette => "palette",
    PanelRight => "panel-right",
    Paperclip => "paperclip",
    Pause => "pause",
    Pencil => "pencil",
    PhoneOff => "phone-off",
    Pin => "pin",
    Play => "play",
    OptionKey => "option",
    Plug => "plug",
    Plus => "plus",
    Quote => "quote",
    Refresh => "refresh",
    Reply => "reply",
    Return => "return",
    Screen => "screen",
    Search => "search",
    Send => "send",
    Server => "server",
    Shield => "shield",
    Shift => "shift",
    Sidebar => "sidebar",
    Sliders => "sliders",
    Smile => "smile",
    Star => "star",
    Strike => "strike",
    Sun => "sun",
    Threads => "threads",
    Trash => "trash",
    Type => "type",
    Upload => "upload",
    User => "user",
    Users => "users",
    Video => "video",
    VideoOff => "video-off",
    Volume => "volume",
    VolumeOff => "volume-off",
    WifiOff => "wifi-off",
    X => "x",
    Zap => "zap",
}

/// An icon drawn at a size from the icon scale, in a theme colour (muted text
/// by default).
#[derive(IntoElement)]
pub struct Icon {
    name: IconName,
    size: Pixels,
    color: Option<Hsla>,
}

impl Icon {
    /// An icon at the default size ([`metrics::ICON_SIZE_MD`]).
    pub fn new(name: IconName) -> Self {
        Self {
            name,
            size: metrics::ICON_SIZE_MD,
            color: None,
        }
    }

    /// Draws the icon at `size`; pass one of `themes::metrics::ICON_SIZE_*`.
    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    /// Draws the icon in `color`; pass a colour read from the theme.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl RenderOnce for Icon {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        IconElement {
            name: self.name,
            size: self.size,
            color: self.color.unwrap_or(cx.theme().colors().text_muted),
        }
    }
}

/// Paints the compiled-in SVG straight from its static bytes under a static
/// atlas key. `gpui::svg().data(..)` copies the bytes and formats a key on
/// every frame; an icon is redrawn every frame, so this one allocates nothing.
struct IconElement {
    name: IconName,
    size: Pixels,
    color: Hsla,
}

impl Element for IconElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let style = Style {
            size: size(self.size.into(), self.size.into()),
            flex_grow: 0.,
            flex_shrink: 0.,
            ..Style::default()
        };
        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window
            .paint_svg(
                bounds,
                SharedString::new_static(self.name.atlas_key()),
                Some(self.name.svg()),
                TransformationMatrix::unit(),
                self.color,
                cx,
            )
            .ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons/ui")
    }

    #[test]
    fn every_variant_has_a_file_and_every_file_a_variant() {
        let on_disk: BTreeSet<String> = std::fs::read_dir(dir())
            .expect("icons dir")
            .filter_map(|e| {
                let name = e.ok()?.file_name().into_string().ok()?;
                name.strip_suffix(".svg").map(str::to_string)
            })
            .collect();
        let variants: BTreeSet<String> =
            IconName::ALL.iter().map(|i| i.file().to_string()).collect();
        assert_eq!(variants.len(), IconName::ALL.len(), "duplicate file name");
        assert_eq!(on_disk, variants, "assets/icons/ui and IconName differ");
        for i in IconName::ALL {
            let bytes = std::fs::read(dir().join(format!("{}.svg", i.file()))).unwrap();
            assert_eq!(bytes, i.svg(), "{i:?} embeds its own file");
        }
    }

    #[test]
    fn every_icon_is_a_24_px_currentcolor_svg() {
        for i in IconName::ALL {
            let s = std::str::from_utf8(i.svg()).unwrap();
            assert!(s.contains("viewBox=\"0 0 24 24\""), "{i:?} viewBox");
            assert!(s.contains("currentColor"), "{i:?} stroke");
        }
    }
}
