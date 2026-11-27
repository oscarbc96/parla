use std::{
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, channel},
    time::Duration,
};

use notify::{Event, EventKind, RecursiveMode, Watcher};

use crate::{ConfigPaths, WorkspaceKey};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangedFile {
    Settings,
    Keymap,
    Theme(PathBuf),
    Workspace(WorkspaceKey),
}

/// Keeps the watcher alive; drop to stop watching.
pub struct WatchHandle {
    _watcher: notify::RecommendedWatcher,
}

const QUIET: Duration = Duration::from_millis(100);

/// Blocks until an event arrives, then collects events until none arrives for
/// `quiet`. Returns `None` once the sender is gone. The wait for the first
/// event has no timeout, so an idle watcher never wakes its thread (the
/// `notify-debouncer-full` tick loop woke ~40 times a second).
fn next_batch(rx: &Receiver<notify::Result<Event>>, quiet: Duration) -> Option<Vec<Event>> {
    let mut batch = Vec::new();
    let mut take = |res: notify::Result<Event>| {
        if let Ok(ev) = res
            && !matches!(ev.kind, EventKind::Access(_))
        {
            batch.push(ev);
        }
    };
    take(rx.recv().ok()?);
    while let Ok(res) = rx.recv_timeout(quiet) {
        take(res);
    }
    Some(batch)
}

fn is_temp(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_none_or(|n| n.ends_with('~') || n.ends_with(".swp") || n.ends_with(".tmp"))
}

/// Map an event path (already relative to the config root) to a changed file.
fn classify(paths: &ConfigPaths, rel: &Path) -> Option<ChangedFile> {
    if is_temp(rel) {
        return None;
    }
    let comps: Vec<_> = rel.components().map(|c| c.as_os_str()).collect();
    match comps.as_slice() {
        [f] if *f == "settings.json" => Some(ChangedFile::Settings),
        [f] if *f == "keymap.json" => Some(ChangedFile::Keymap),
        [d, f] if *d == "themes" && Path::new(f).extension().is_some_and(|e| e == "json") => {
            Some(ChangedFile::Theme(paths.themes_dir.join(f)))
        }
        [d, provider, f] if *d == "workspaces" => {
            let p = Path::new(f);
            if p.extension().is_some_and(|e| e == "json") {
                let stem = p.file_stem()?.to_str()?;
                Some(ChangedFile::Workspace(WorkspaceKey::new(
                    provider.to_str()?,
                    stem,
                )))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Watch the config root (not individual files, so atomic renames are seen),
/// debounced to 100 ms. The root is created if missing.
pub fn watch(
    paths: &ConfigPaths,
    on_change: impl Fn(ChangedFile) + Send + 'static,
) -> notify::Result<WatchHandle> {
    std::fs::create_dir_all(&paths.root)?;
    // Events carry canonical paths (e.g. /private/var on macOS).
    let canon = std::fs::canonicalize(&paths.root).unwrap_or_else(|_| paths.root.clone());
    let cfg = paths.clone();
    let (tx, rx) = channel();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;
    watcher.watch(&paths.root, RecursiveMode::Recursive)?;
    std::thread::Builder::new()
        .name("parla-settings-watch".into())
        .spawn(move || {
            while let Some(events) = next_batch(&rx, QUIET) {
                let mut seen: Vec<ChangedFile> = Vec::new();
                for ev in events {
                    for p in &ev.paths {
                        let rel = p
                            .strip_prefix(&canon)
                            .or_else(|_| p.strip_prefix(&cfg.root));
                        if let Some(c) = rel.ok().and_then(|r| classify(&cfg, r))
                            && !seen.contains(&c)
                        {
                            seen.push(c);
                        }
                    }
                }
                for c in seen {
                    on_change(c);
                }
            }
        })
        .map_err(notify::Error::io)?;
    Ok(WatchHandle { _watcher: watcher })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(name: &str) -> notify::Result<Event> {
        Ok(Event::new(EventKind::Any).add_path(PathBuf::from(name)))
    }

    #[test]
    fn next_batch_blocks_without_a_timer_and_groups_bursts() {
        let (tx, rx) = channel();
        let h = std::thread::spawn(move || {
            let first = next_batch(&rx, Duration::from_millis(50));
            let second = next_batch(&rx, Duration::from_millis(50));
            (first, second)
        });
        // Idle longer than the quiet window: nothing may be returned yet.
        std::thread::sleep(Duration::from_millis(200));
        assert!(!h.is_finished());
        tx.send(ev("a")).unwrap();
        tx.send(ev("b")).unwrap();
        drop(tx);
        let (first, second) = h.join().unwrap();
        assert_eq!(first.unwrap().len(), 2);
        assert!(second.is_none());
    }

    #[test]
    fn classifies_workspace_files_by_provider_and_id() {
        let paths = ConfigPaths::at("/cfg");
        assert_eq!(
            classify(&paths, Path::new("workspaces/fake/W1.json")),
            Some(ChangedFile::Workspace(WorkspaceKey::new("fake", "W1")))
        );
        assert_eq!(classify(&paths, Path::new("workspaces/W1.json")), None);
        assert_eq!(
            classify(&paths, Path::new("workspaces/fake/W1.json~")),
            None
        );
        assert_eq!(
            classify(&paths, Path::new("settings.json")),
            Some(ChangedFile::Settings)
        );
    }
}
