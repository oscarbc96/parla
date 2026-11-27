//! Persisted pop-out windows: saved on open/close and restored after the main
//! window at launch. Window bounds are restored when known.

use std::path::{Path, PathBuf};

use domain::{ConversationId, MessageTs};
use gpui::{Bounds, Pixels};
use serde::{Deserialize, Serialize};
use settings::WorkspaceKey;

/// What a pop-out window shows. Only conversations and threads for now; more
/// targets can be added without breaking stored files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PopOutTarget {
    Conversation {
        #[serde(with = "ws_key")]
        ws: WorkspaceKey,
        #[serde(with = "conv_id")]
        id: ConversationId,
    },
    Thread {
        #[serde(with = "ws_key")]
        ws: WorkspaceKey,
        #[serde(with = "conv_id")]
        conversation: ConversationId,
        #[serde(with = "msg_ts")]
        root: MessageTs,
    },
}

impl PopOutTarget {
    pub fn workspace(&self) -> &WorkspaceKey {
        match self {
            Self::Conversation { ws, .. } | Self::Thread { ws, .. } => ws,
        }
    }

    pub fn conversation(ws: WorkspaceKey, id: ConversationId) -> Self {
        Self::Conversation { ws, id }
    }

    pub fn thread(ws: WorkspaceKey, conversation: ConversationId, root: MessageTs) -> Self {
        Self::Thread {
            ws,
            conversation,
            root,
        }
    }
}

mod ws_key {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use settings::WorkspaceKey;

    pub fn serialize<S: Serializer>(k: &WorkspaceKey, s: S) -> Result<S::Ok, S::Error> {
        k.to_string().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<WorkspaceKey, D::Error> {
        let text = String::deserialize(d)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

mod conv_id {
    use std::sync::Arc;

    use domain::ConversationId;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(id: &ConversationId, s: S) -> Result<S::Ok, S::Error> {
        id.0.as_ref().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<ConversationId, D::Error> {
        let text = String::deserialize(d)?;
        Ok(ConversationId(Arc::from(text)))
    }
}

mod msg_ts {
    use std::sync::Arc;

    use domain::MessageTs;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(ts: &MessageTs, s: S) -> Result<S::Ok, S::Error> {
        ts.0.as_ref().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<MessageTs, D::Error> {
        let text = String::deserialize(d)?;
        Ok(MessageTs(Arc::from(text)))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedWindow {
    pub target: PopOutTarget,
    #[serde(default, with = "bounds_serde")]
    pub bounds: Option<Bounds<Pixels>>,
}

mod bounds_serde {
    use gpui::{Bounds, Pixels, point, px, size};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct B {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    }

    pub fn serialize<S: Serializer>(v: &Option<Bounds<Pixels>>, s: S) -> Result<S::Ok, S::Error> {
        let b = v.as_ref().map(|b| B {
            x: f32::from(b.origin.x),
            y: f32::from(b.origin.y),
            width: f32::from(b.size.width),
            height: f32::from(b.size.height),
        });
        b.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<Bounds<Pixels>>, D::Error> {
        let b = Option::<B>::deserialize(d)?;
        Ok(b.map(|b| Bounds {
            origin: point(px(b.x), px(b.y)),
            size: size(px(b.width), px(b.height)),
        }))
    }
}

/// Reads `<path>`. A missing or corrupt file yields an empty list, never a panic.
pub fn load(path: &Path) -> Vec<SavedWindow> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Writes the list atomically (temp file then rename).
pub fn save(path: &Path, windows: &[SavedWindow]) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(windows).map_err(std::io::Error::other)?;
    settings::write_atomic(path, &bytes)
}

/// `<data_dir>/windows.json`.
pub fn path_in(data_dir: &Path) -> PathBuf {
    data_dir.join("windows.json")
}

/// `<data_dir>/main_window.json`: the main window's last size and position.
pub fn main_path_in(data_dir: &Path) -> PathBuf {
    data_dir.join("main_window.json")
}

/// The main window as last left: its bounds, relative to the display it was
/// on (the platform places a window relative to its display), and that
/// display's stable id.
#[derive(Clone, Debug, PartialEq)]
pub struct MainWindow {
    pub bounds: Bounds<Pixels>,
    pub display: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct MainFile {
    #[serde(default, with = "bounds_serde")]
    bounds: Option<Bounds<Pixels>>,
    #[serde(default)]
    display: Option<String>,
}

/// The saved main window; `None` when missing or unreadable.
pub fn load_main(path: &Path) -> Option<MainWindow> {
    let bytes = std::fs::read(path).ok()?;
    let f = serde_json::from_slice::<MainFile>(&bytes).ok()?;
    Some(MainWindow {
        bounds: f.bounds?,
        display: f.display,
    })
}

/// Writes the main window atomically (temp file then rename).
pub fn save_main(path: &Path, window: &MainWindow) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(&MainFile {
        bounds: Some(window.bounds),
        display: window.display.clone(),
    })
    .map_err(std::io::Error::other)?;
    settings::write_atomic(path, &bytes)
}

/// A display as the restore sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayInfo {
    /// Stable id (survives a restart); `None` when the platform has none.
    pub uuid: Option<String>,
    /// The usable area, in the display's own coordinates.
    pub visible: Bounds<Pixels>,
}

/// The display to reopen on: the one the window was saved on, else the
/// primary. `None` without any display.
pub fn restore_display<'a>(
    saved: Option<&str>,
    displays: &'a [DisplayInfo],
    primary: usize,
) -> Option<(usize, &'a DisplayInfo)> {
    let found = saved.and_then(|u| displays.iter().position(|d| d.uuid.as_deref() == Some(u)));
    let i = found.unwrap_or(primary);
    displays.get(i).map(|d| (i, d))
}

/// Least a saved window may overlap its display (each way) to count as
/// reachable: enough of the title bar to grab.
const MIN_VISIBLE: f32 = 120.0;

/// `saved` (relative to the display) moved and shrunk to sit fully inside
/// that display's usable area `visible`. `None` when too little of it would
/// show to be reachable: the caller then uses the default placement.
pub fn clamp_to_display(
    saved: Bounds<Pixels>,
    visible: Bounds<Pixels>,
    min: gpui::Size<Pixels>,
) -> Option<Bounds<Pixels>> {
    let overlap = saved.intersect(&visible);
    if f32::from(overlap.size.width) < MIN_VISIBLE.min(f32::from(saved.size.width))
        || f32::from(overlap.size.height) < MIN_VISIBLE.min(f32::from(saved.size.height))
    {
        return None;
    }
    let width = saved
        .size
        .width
        .min(visible.size.width)
        .max(min.width.min(visible.size.width));
    let height = saved
        .size
        .height
        .min(visible.size.height)
        .max(min.height.min(visible.size.height));
    let x = saved
        .origin
        .x
        .max(visible.origin.x)
        .min(visible.origin.x + visible.size.width - width);
    let y = saved
        .origin
        .y
        .max(visible.origin.y)
        .min(visible.origin.y + visible.size.height - height);
    Some(Bounds {
        origin: gpui::point(x, y),
        size: gpui::size(width, height),
    })
}

/// How long the window must stay still before its bounds are written: a drag
/// or resize is one write at its end, not one per frame. The timer is armed
/// only by a change, so an idle window has none.
const SAVE_AFTER: std::time::Duration = std::time::Duration::from_millis(500);

/// Saves the window's bounds and display to `path` once it has stopped moving
/// or resizing, and at once when the window is released or the app quits. It
/// lasts as long as the view `cx` belongs to.
pub fn track_main<T: 'static>(path: PathBuf, window: &mut gpui::Window, cx: &mut gpui::Context<T>) {
    use std::cell::RefCell;
    use std::rc::Rc;
    // The latest unsaved state; `None` once written.
    let dirty = Rc::new(RefCell::new(None::<MainWindow>));
    let pending = Rc::new(RefCell::new(None::<gpui::Task<()>>));
    let flush = {
        let (dirty, path) = (dirty.clone(), path.clone());
        move || {
            if let Some(w) = dirty.borrow_mut().take()
                && let Err(e) = save_main(&path, &w)
            {
                crate::log::error(&format!("saving the window position failed: {e}"));
            }
        }
    };
    {
        let flush = flush.clone();
        cx.on_app_quit(move |_, _| {
            flush();
            std::future::ready(())
        })
        .detach();
    }
    {
        let flush = flush.clone();
        cx.on_release(move |_, _| flush()).detach();
    }
    cx.observe_window_bounds(window, move |_, window, cx| {
        let bounds = match window.window_bounds() {
            gpui::WindowBounds::Windowed(b) => b,
            // Maximised or full screen: keep the last windowed bounds.
            _ => return,
        };
        let display = window
            .display(cx)
            .and_then(|d| d.uuid().ok())
            .map(|u| u.to_string());
        *dirty.borrow_mut() = Some(MainWindow { bounds, display });
        let flush = flush.clone();
        let task = cx.spawn(async move |_, cx| {
            cx.background_executor().timer(SAVE_AFTER).await;
            flush();
        });
        // Replacing the task drops (cancels) the previous one.
        *pending.borrow_mut() = Some(task);
    })
    .detach();
}
