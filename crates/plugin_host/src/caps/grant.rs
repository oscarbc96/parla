//! What the user granted: built from the manifest at
//! consent, stored with the plugin's rows, checked on every host call.
//! An update that asks for more shows only the growth ([`Grant::diff`]).

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::risk::{CardRow, rows};
use super::url_pattern::UrlPattern;
use crate::manifest::{ArgShape, Manifest, NetworkHost, ReadArea, TransformKind, WriteArea};

/// The capabilities a user accepted for one plugin.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Grant {
    pub read: BTreeSet<ReadArea>,
    pub write: BTreeSet<WriteArea>,
    pub network: BTreeSet<NetworkHost>,
    pub storage: bool,
    pub windows: bool,
    pub panel: bool,
    pub notifications: bool,
    pub clipboard_write: bool,
    pub exec: Vec<ExecGrant>,
    /// What contributions imply (§7.2 "implied" rows).
    pub implied: Implied,
}

/// One granted program and its argument shapes (§7.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecGrant {
    pub program: String,
    /// Absolute path resolved on the login `PATH` at consent; a call that
    /// resolves elsewhere is refused. `None` until consent resolves it.
    pub path: Option<PathBuf>,
    pub allow: Vec<ArgShape>,
}

/// Access a plugin has through what it contributes, not a capability.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Implied {
    /// A `kind = "check"` composer transform reads the draft (§10.7).
    pub composer_check: bool,
    /// The notification filter reads every notification (§10.6).
    pub notification_filter: bool,
    /// Renderer link patterns (§10.8).
    pub renderer_patterns: BTreeSet<UrlPattern>,
    /// Hook events, manifest spelling (§10.5).
    pub hook_events: BTreeSet<String>,
    /// Contributions with no host function behind them, listed for
    /// clarity.
    pub adds: BTreeSet<Adds>,
}

/// A kind of contribution the card lists as "Adds ...".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Adds {
    Commands,
    SlashCommands,
    MessageActions,
    MediaProviders,
    McpTools,
}

impl Grant {
    /// Everything `m` declares, as the consent card offers it.
    pub fn from_manifest(m: &Manifest) -> Self {
        let caps = &m.capabilities;
        let c = &m.contributions;
        let mut adds = BTreeSet::new();
        for (has, kind) in [
            (!c.commands.is_empty(), Adds::Commands),
            (!c.slash.is_empty(), Adds::SlashCommands),
            (!c.message_actions.is_empty(), Adds::MessageActions),
            (!c.media_providers.is_empty(), Adds::MediaProviders),
            (!c.mcp_tools.is_empty(), Adds::McpTools),
        ] {
            if has {
                adds.insert(kind);
            }
        }
        let mut exec: Vec<ExecGrant> = Vec::new();
        for rule in &caps.exec {
            match exec.iter_mut().find(|e| e.program == rule.program) {
                Some(e) => {
                    for s in &rule.allow {
                        if !e.allow.contains(s) {
                            e.allow.push(s.clone());
                        }
                    }
                }
                None => exec.push(ExecGrant {
                    program: rule.program.clone(),
                    path: None,
                    allow: rule.allow.clone(),
                }),
            }
        }
        Self {
            read: caps.workspace_read.iter().copied().collect(),
            write: caps.workspace_write.iter().copied().collect(),
            network: caps.network.iter().cloned().collect(),
            storage: caps.storage,
            windows: caps.windows,
            panel: caps.panel,
            notifications: caps.notifications,
            clipboard_write: caps.clipboard_write,
            exec,
            implied: Implied {
                composer_check: c
                    .composer_transforms
                    .iter()
                    .any(|t| t.kind == TransformKind::Check),
                notification_filter: c.hooks.as_ref().is_some_and(|h| h.notification_filter),
                renderer_patterns: c
                    .renderers
                    .iter()
                    .flat_map(|r| &r.patterns)
                    .filter_map(|p| UrlPattern::new(p))
                    .collect(),
                hook_events: c
                    .hooks
                    .iter()
                    .flat_map(|h| &h.events)
                    .map(|e| e.as_str().to_owned())
                    .collect(),
                adds,
            },
        }
    }

    /// Records where `program` resolved at consent.
    pub fn set_exec_path(&mut self, program: &str, path: PathBuf) {
        if let Some(e) = self.exec.iter_mut().find(|e| e.program == program) {
            e.path = Some(path);
        }
    }

    /// The card rows for this grant, in card order.
    pub fn rows(&self) -> Vec<CardRow> {
        rows(self, self.reads_and_sends())
    }

    /// What `m` asks for beyond this grant (an update's consent).
    pub fn diff(&self, m: &Manifest) -> GrantDiff {
        let new = Self::from_manifest(m);
        let added = new.minus(self);
        // The combined warning shows when the new grant reads and sends
        // and the update adds to either side.
        let warn = new.reads_and_sends() && (added.reads() || added.sends());
        let rows = if added == Self::default() {
            Vec::new()
        } else {
            let mut r = rows(&added, false);
            if warn {
                r.extend(super::risk::warning(&new));
            }
            r
        };
        GrantDiff { added, rows }
    }

    /// Reads workspace data, drafts or notifications.
    pub(super) fn reads(&self) -> bool {
        !self.read.is_empty()
            || self.implied.composer_check
            || self.implied.notification_filter
            || !self.implied.hook_events.is_empty()
    }

    /// Can reach something outside Parla.
    pub(super) fn sends(&self) -> bool {
        !self.network.is_empty() || !self.exec.is_empty()
    }

    fn reads_and_sends(&self) -> bool {
        self.reads() && self.sends()
    }

    /// What `self` has that `old` lacks.
    fn minus(&self, old: &Self) -> Self {
        fn set<T: Ord + Clone>(a: &BTreeSet<T>, b: &BTreeSet<T>) -> BTreeSet<T> {
            a.difference(b).cloned().collect()
        }
        let exec = self
            .exec
            .iter()
            .filter_map(|e| {
                let had = old.exec.iter().find(|o| o.program == e.program);
                let allow: Vec<ArgShape> = e
                    .allow
                    .iter()
                    .filter(|s| !had.is_some_and(|o| o.allow.contains(s)))
                    .cloned()
                    .collect();
                (!allow.is_empty()).then(|| ExecGrant {
                    program: e.program.clone(),
                    path: e.path.clone(),
                    allow,
                })
            })
            .collect();
        Self {
            read: set(&self.read, &old.read),
            write: set(&self.write, &old.write),
            network: set(&self.network, &old.network),
            storage: self.storage && !old.storage,
            windows: self.windows && !old.windows,
            panel: self.panel && !old.panel,
            notifications: self.notifications && !old.notifications,
            clipboard_write: self.clipboard_write && !old.clipboard_write,
            exec,
            implied: Implied {
                composer_check: self.implied.composer_check && !old.implied.composer_check,
                notification_filter: self.implied.notification_filter
                    && !old.implied.notification_filter,
                renderer_patterns: set(
                    &self.implied.renderer_patterns,
                    &old.implied.renderer_patterns,
                ),
                hook_events: set(&self.implied.hook_events, &old.implied.hook_events),
                adds: set(&self.implied.adds, &old.implied.adds),
            },
        }
    }

    /// Whether `area` is granted (`read` or `write`).
    pub fn has_read(&self, area: ReadArea) -> bool {
        self.read.contains(&area)
    }

    pub fn has_write(&self, area: WriteArea) -> bool {
        self.write.contains(&area)
    }
}

/// What an update adds to a grant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantDiff {
    /// Only the new parts.
    pub added: Grant,
    rows: Vec<CardRow>,
}

impl GrantDiff {
    /// Nothing new: the update installs without asking.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The card rows of the growth, with the combined warning when it
    /// now applies.
    pub fn rows(&self) -> &[CardRow] {
        &self.rows
    }
}
