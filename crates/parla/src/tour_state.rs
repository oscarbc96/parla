//! Whether the first-run tour was dismissed: a key in the non-secret app state
//! file `<data_dir>/onboarding.json`, so the tour shows once per install. Without
//! the app's data dir (tests) it is kept in memory only.

use std::path::PathBuf;

use gpui::{App, Global};

const FILE: &str = "onboarding.json";
const KEY: &str = "tour_dismissed";

struct TourState {
    dismissed: bool,
}

impl Global for TourState {}

fn path(cx: &App) -> Option<PathBuf> {
    cx.try_global::<crate::removal::AppDirs>()
        .map(|d| d.data.join(FILE))
}

fn read(path: Option<PathBuf>) -> bool {
    let Some(text) = path.and_then(|p| std::fs::read_to_string(p).ok()) else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v.get(KEY)?.as_bool())
        .unwrap_or(false)
}

/// True once the tour was skipped or finished. Read from disk the first time.
pub fn dismissed(cx: &mut App) -> bool {
    if let Some(s) = cx.try_global::<TourState>() {
        return s.dismissed;
    }
    let dismissed = read(path(cx));
    cx.set_global(TourState { dismissed });
    dismissed
}

/// Remembers that the tour was dismissed (written in the background).
pub fn dismiss(cx: &mut App) {
    cx.set_global(TourState { dismissed: true });
    let Some(path) = path(cx) else {
        return;
    };
    cx.background_executor()
        .spawn(async move {
            // Keep any other keys of the app state; a corrupt file starts over.
            let mut state = std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                .filter(serde_json::Value::is_object)
                .unwrap_or_else(|| serde_json::json!({}));
            state[KEY] = serde_json::Value::Bool(true);
            let body = state.to_string();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, body).is_ok() {
                let _ = std::fs::rename(tmp, path);
            }
        })
        .detach();
}
