//! Plugin lifecycle.
//!
//! [`transition`] is pure: it takes a plugin's [`Lifecycle`], one
//! [`Event`] and the current time, and returns the next lifecycle plus the
//! [`Effect`]s the host must carry out. The clock is an argument, so no
//! timer is involved and tests use a fake one.
//!
//! `app.db` is the source of truth for the state; the user's
//! `plugins.<id>.enabled` setting is only an input. It never revives a
//! Quarantined or Blocked plugin.

use std::time::{Duration, Instant};

use store::PluginRow;

/// `plugins.state` of a plugin Blocked while Quarantined (M8 plan B5 note).
pub const BLOCKED_QUARANTINED_ROW: &str = "blocked-quarantined";
/// `plugins.state` of a row kept after "Uninstall, keep its data": no
/// folder, no grant, only the plugin's storage. Installing the same id
/// again replaces it and finds the data.
pub const KEPT_DATA_ROW: &str = "uninstalled";

/// Whether `row` only keeps an uninstalled plugin's data.
pub fn is_kept_data(row: &PluginRow) -> bool {
    &*row.state == KEPT_DATA_ROW
}

/// Failures within [`QUARANTINE_WINDOW`] that quarantine a plugin (§8.6).
pub const QUARANTINE_FAILURES: usize = 3;

/// The window [`QUARANTINE_FAILURES`] must fall in (§8.6).
pub const QUARANTINE_WINDOW: Duration = Duration::from_secs(10 * 60);

/// The lifecycle states.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PluginState {
    /// In the index, not on this machine.
    Available,
    /// Package downloaded to a temporary file.
    Fetched,
    /// Checksum, signature chain and limits pass; manifest parsed.
    Verified,
    /// Unpacked, grant stored, contributions registered, not loaded.
    Installed,
    /// Component instantiated in its own store.
    Active,
    /// Was active; store dropped; contributions still registered.
    Unloaded,
    /// Contributions removed; data and grant kept.
    Disabled,
    /// Disabled by the host after repeated failures.
    Quarantined,
    /// Disabled because the registry blocklist names it.
    Blocked,
}

impl PluginState {
    /// The text kept in `plugins.state`. Active and Unloaded are runtime
    /// states and persist as `installed`.
    pub fn as_row(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Fetched => "fetched",
            Self::Verified => "verified",
            Self::Installed | Self::Active | Self::Unloaded => "installed",
            Self::Disabled => "disabled",
            Self::Quarantined => "quarantined",
            Self::Blocked => "blocked",
        }
    }

    /// Parses `plugins.state`. Only the states a row can hold parse.
    pub fn from_row(s: &str) -> Option<Self> {
        Some(match s {
            "installed" => Self::Installed,
            "disabled" => Self::Disabled,
            "quarantined" => Self::Quarantined,
            "blocked" | BLOCKED_QUARANTINED_ROW => Self::Blocked,
            _ => return None,
        })
    }

    /// Whether the plugin's contributions are registered in this state.
    pub fn contributes(self) -> bool {
        matches!(self, Self::Installed | Self::Active | Self::Unloaded)
    }

    fn is_installed(self) -> bool {
        !matches!(self, Self::Available | Self::Fetched | Self::Verified)
    }
}

/// Why a plugin's store ended (§8.6); the stopped card names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FailureReason {
    /// It used more than its memory cap.
    MemoryLimit,
    /// It took too long.
    Deadline,
    /// It trapped or exited.
    Crashed,
}

/// A registry blocklist entry naming an installed plugin (§8.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockEntry {
    pub reason: String,
    /// The advisory the banner links to.
    pub advisory: Option<String>,
    /// Flagged `malicious`: no override is ever offered.
    pub malicious: bool,
}

impl BlockEntry {
    /// Whether the card may offer a per-plugin override (§8.7).
    pub fn override_offered(&self) -> bool {
        !self.malicious
    }
}

/// The most recent failures, at most [`QUARANTINE_FAILURES`] (bounded).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FailureWindow {
    recent: [Option<Instant>; QUARANTINE_FAILURES],
}

impl FailureWindow {
    /// Records a failure at `now`; true when it is the
    /// [`QUARANTINE_FAILURES`]th within [`QUARANTINE_WINDOW`].
    pub fn record(&mut self, now: Instant) -> bool {
        // Oldest first. Drop what fell out of the window, then shift the
        // oldest out when full; the window edge is inclusive.
        let within = |t: Instant| now.saturating_duration_since(t) <= QUARANTINE_WINDOW;
        let mut kept = [None; QUARANTINE_FAILURES];
        let mut n = 0;
        for t in self.recent.iter().flatten().copied().filter(|t| within(*t)) {
            kept[n] = Some(t);
            n += 1;
        }
        if n == QUARANTINE_FAILURES {
            kept.rotate_left(1);
            n -= 1;
        }
        kept[n] = Some(now);
        self.recent = kept;
        n + 1 == QUARANTINE_FAILURES
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Failures currently remembered.
    pub fn len(&self) -> usize {
        self.recent.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One plugin's lifecycle: its state plus what the transitions need.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lifecycle {
    pub state: PluginState,
    /// The user's `plugins.<id>.enabled` setting.
    pub enabled: bool,
    pub failures: FailureWindow,
    /// Set while Blocked.
    pub block: Option<BlockEntry>,
    /// Blocked from Quarantined: Unblocked returns to Quarantined, so a
    /// block and unblock never lifts a quarantine.
    pub quarantined_under_block: bool,
}

impl Lifecycle {
    pub fn new(state: PluginState, enabled: bool) -> Self {
        Self {
            state,
            enabled,
            failures: FailureWindow::default(),
            block: None,
            quarantined_under_block: false,
        }
    }

    /// The `plugins.state` text for this lifecycle: [`PluginState::as_row`],
    /// except that Blocked from Quarantined persists as
    /// [`BLOCKED_QUARANTINED_ROW`] so a restart keeps the quarantine.
    pub fn row_text(&self) -> &'static str {
        if self.state == PluginState::Blocked && self.quarantined_under_block {
            BLOCKED_QUARANTINED_ROW
        } else {
            self.state.as_row()
        }
    }
    /// Installed when the setting is on, else Disabled.
    fn by_setting(&self) -> PluginState {
        if self.enabled {
            PluginState::Installed
        } else {
            PluginState::Disabled
        }
    }
}

/// What happened to a plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Consent given and the package committed (§8.2).
    Installed,
    /// The component was instantiated (§8.3).
    Activated,
    /// The host dropped the store while idle (§8.3).
    Unloaded,
    /// The user turned `enabled` off.
    Disabled,
    /// The user turned `enabled` on.
    Enabled,
    /// A trap, memory-cap or deadline hit ended the store (§8.6).
    Failed(FailureReason),
    /// An index refresh found it on the blocklist (§8.7).
    Blocked(BlockEntry),
    /// An index refresh no longer lists it.
    Unblocked,
    /// The user pressed **Restart** (§8.6).
    Restart,
    /// A new version was swapped in (§8.4). The caller replaces the
    /// manifest in the [`crate::registry::InstalledSet`] itself.
    Updated,
}

/// What the host must do after a transition, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Close the plugin's open views.
    CloseViews,
    /// Call `deactivate` and drop the store.
    Unload,
    RemoveContributions,
    RegisterContributions,
    /// Write `plugins.state`.
    Persist(PluginState),
    /// Show the stopped card naming the reason.
    ShowStopped {
        reason: FailureReason,
        quarantined: bool,
    },
    /// Show the blocked banner.
    ShowBlocked(BlockEntry),
}

/// The pure lifecycle transition. An event that does not apply to the
/// current state leaves it unchanged and has no effects.
pub fn transition(current: &Lifecycle, event: Event, now: Instant) -> (Lifecycle, Vec<Effect>) {
    use PluginState as S;
    let mut next = current.clone();
    let mut extra = Vec::new();
    // The store already ended (Unloaded, Failed): no Unload is asked for.
    let mut store_ended = false;
    match event {
        // Only from Verified; Fetched, Verified, Rejected and uninstall
        // are the install pipeline's (Task B6).
        Event::Installed => {
            if current.state == S::Verified {
                next.state = next.by_setting();
            }
        }
        Event::Activated => {
            if matches!(current.state, S::Installed | S::Unloaded) {
                next.state = S::Active;
            }
        }
        Event::Unloaded => {
            store_ended = true;
            if current.state == S::Active {
                next.state = S::Unloaded;
            }
        }
        Event::Disabled => {
            next.enabled = false;
            if current.state.contributes() {
                next.state = S::Disabled;
            }
        }
        Event::Enabled => {
            next.enabled = true;
            if current.state == S::Disabled {
                next.state = S::Installed;
            }
        }
        Event::Failed(reason) => {
            store_ended = true;
            if current.state.contributes() {
                let quarantined = next.failures.record(now);
                // A failure before activation (instantiate or `activate`
                // failed) leaves it Installed: there was no store to drop.
                next.state = if quarantined {
                    S::Quarantined
                } else if current.state == S::Installed {
                    S::Installed
                } else {
                    S::Unloaded
                };
                extra.push(Effect::ShowStopped {
                    reason,
                    quarantined,
                });
            }
        }
        Event::Blocked(entry) => {
            if current.state.is_installed() {
                if current.state != S::Blocked {
                    next.quarantined_under_block = current.state == S::Quarantined;
                }
                next.state = S::Blocked;
                next.block = Some(entry.clone());
                if current.block.as_ref() != Some(&entry) {
                    extra.push(Effect::ShowBlocked(entry));
                }
            }
        }
        Event::Unblocked => {
            if current.state == S::Blocked {
                next.block = None;
                next.state = if next.quarantined_under_block {
                    S::Quarantined
                } else {
                    next.by_setting()
                };
                next.quarantined_under_block = false;
            }
        }
        Event::Restart => match current.state {
            S::Quarantined => {
                next.failures.clear();
                next.state = next.by_setting();
            }
            // From the stopped card while not quarantined: the window is
            // kept, so repeated crashes still quarantine. Installed and
            // Unloaded have no store, so nothing changes.
            S::Active => next.state = S::Unloaded,
            _ => {}
        },
        Event::Updated => match current.state {
            S::Quarantined => {
                next.failures.clear();
                next.state = next.by_setting();
            }
            // The window is kept; an active plugin is unloaded for the swap.
            S::Active => next.state = S::Unloaded,
            _ => {}
        },
    }

    let mut effects = Vec::new();
    let (was, is) = (current.state, next.state);
    if was.contributes() && !is.contributes() {
        effects.push(Effect::CloseViews);
    }
    if was == S::Active && is != S::Active && !store_ended {
        effects.push(Effect::Unload);
    }
    if was.contributes() && !is.contributes() {
        effects.push(Effect::RemoveContributions);
    }
    if !was.contributes() && is.contributes() {
        effects.push(Effect::RegisterContributions);
    }
    if was.as_row() != is.as_row() {
        effects.push(Effect::Persist(is));
    }
    effects.extend(extra);
    (next, effects)
}

/// A plugin found on startup by comparing `plugins/` with `app.db`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Discovered {
    /// A folder with a row.
    Installed { id: String, state: PluginState },
    /// A folder copied in by hand, without a row: listed as "Found in the
    /// plugins folder · not installed", never loaded (§14).
    FoundNotInstalled(String),
    /// A row whose folder is gone.
    MissingFolder(String),
}

/// Matches the plugin folders on disk with the `plugins` rows, sorted by
/// id. A row whose state text does not parse counts as Disabled, so an
/// unknown state never runs code. A [`KEPT_DATA_ROW`] row is not an
/// installed plugin and is skipped.
pub fn discover<'a>(
    folders: impl IntoIterator<Item = &'a str>,
    rows: &[PluginRow],
) -> Vec<Discovered> {
    let rows: Vec<&PluginRow> = rows.iter().filter(|r| !is_kept_data(r)).collect();
    let mut folders: Vec<&str> = folders.into_iter().collect();
    folders.sort_unstable();
    folders.dedup();
    let mut out: Vec<Discovered> = folders
        .iter()
        .map(|id| match rows.iter().find(|r| &*r.id == *id) {
            Some(r) => Discovered::Installed {
                id: (*id).to_owned(),
                state: PluginState::from_row(&r.state).unwrap_or(PluginState::Disabled),
            },
            None => Discovered::FoundNotInstalled((*id).to_owned()),
        })
        .collect();
    out.extend(
        rows.iter()
            .filter(|r| folders.binary_search(&&*r.id).is_err())
            .map(|r| Discovered::MissingFolder(r.id.to_string())),
    );
    out.sort_by(|a, b| key(a).cmp(key(b)));
    out
}

fn key(d: &Discovered) -> &str {
    match d {
        Discovered::Installed { id, .. }
        | Discovered::FoundNotInstalled(id)
        | Discovered::MissingFolder(id) => id,
    }
}
