//! The permission card as data: one row
//! per capability with its risk and arguments. Rows carry ids only; the
//! words are Parla's own, translated by Parla.

use super::grant::{Adds, Grant};
use crate::manifest::{Manifest, ReadArea, WriteArea};

/// Risk levels as designed; `Writes` is drawn in the danger colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Risk {
    Low,
    Medium,
    Writes,
}

/// One row of the permission card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardRow {
    /// Parla's message id for the row text.
    pub id: &'static str,
    pub risk: Risk,
    /// Values the text shows: hosts, patterns, hook events, or a program
    /// followed by its shapes (space-joined).
    pub args: Vec<String>,
}

impl CardRow {
    fn new(id: &'static str, risk: Risk) -> Self {
        Self {
            id,
            risk,
            args: Vec::new(),
        }
    }
}

/// The rows of the card for `m`, implied rows and the combined
/// "can send what it reads" warning included.
pub fn card_rows(m: &Manifest) -> Vec<CardRow> {
    Grant::from_manifest(m).rows()
}

/// Card order: read areas, write areas, network, flags, exec, implied,
/// "adds", then the warning when `warn`.
pub(super) fn rows(g: &Grant, warn: bool) -> Vec<CardRow> {
    let mut out = Vec::new();
    out.extend(
        g.read
            .iter()
            .map(|a| CardRow::new(read_id(*a), read_risk(*a))),
    );
    out.extend(
        g.write
            .iter()
            .map(|a| CardRow::new(write_id(*a), Risk::Writes)),
    );
    if !g.network.is_empty() {
        out.push(CardRow {
            args: g.network.iter().map(|h| h.as_str().to_owned()).collect(),
            ..CardRow::new("plugin-cap-network", Risk::Medium)
        });
    }
    for (on, id) in [
        (g.storage, "plugin-cap-storage"),
        (g.windows, "plugin-cap-windows"),
        (g.panel, "plugin-cap-panel"),
        (g.notifications, "plugin-cap-notifications"),
        (g.clipboard_write, "plugin-cap-clipboard-write"),
    ] {
        if on {
            out.push(CardRow::new(id, Risk::Low));
        }
    }
    for e in &g.exec {
        let mut args = vec![e.program.clone()];
        args.extend(
            e.allow
                .iter()
                .map(|s| s.0.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(" ")),
        );
        out.push(CardRow {
            args,
            ..CardRow::new("plugin-cap-exec", Risk::Writes)
        });
    }
    let i = &g.implied;
    if i.composer_check {
        out.push(CardRow::new("plugin-cap-composer-check", Risk::Medium));
    }
    if i.notification_filter {
        out.push(CardRow::new("plugin-cap-notification-filter", Risk::Medium));
    }
    if !i.renderer_patterns.is_empty() {
        out.push(CardRow {
            args: i
                .renderer_patterns
                .iter()
                .map(|p| p.as_str().to_owned())
                .collect(),
            ..CardRow::new("plugin-cap-renderer", Risk::Medium)
        });
    }
    if !i.hook_events.is_empty() {
        out.push(CardRow {
            args: i.hook_events.iter().cloned().collect(),
            ..CardRow::new("plugin-cap-hooks", Risk::Medium)
        });
    }
    out.extend(i.adds.iter().map(|a| CardRow::new(adds_id(*a), Risk::Low)));
    if warn {
        out.extend(warning(g));
    }
    out
}

/// "Can send what it reads to <host>" (§18): any read area, a check
/// transform, a notification filter or hooks, plus network or exec. Args
/// are the hosts, then the programs.
pub(super) fn warning(g: &Grant) -> Option<CardRow> {
    (g.reads() && g.sends()).then(|| CardRow {
        args: g
            .network
            .iter()
            .map(|h| h.as_str().to_owned())
            .chain(g.exec.iter().map(|e| e.program.clone()))
            .collect(),
        ..CardRow::new("plugin-cap-can-send-what-it-reads", Risk::Writes)
    })
}

/// Read area risk (§7.3); `dms:read` carries the Writes-level warning.
pub(super) fn read_risk(a: ReadArea) -> Risk {
    match a {
        ReadArea::Messages | ReadArea::History | ReadArea::Search | ReadArea::Files => Risk::Medium,
        ReadArea::Dms => Risk::Writes,
        ReadArea::Workspace
        | ReadArea::Conversations
        | ReadArea::People
        | ReadArea::Reminders
        | ReadArea::Scheduled
        | ReadArea::Saved
        | ReadArea::Pins
        | ReadArea::Status => Risk::Low,
    }
}

fn read_id(a: ReadArea) -> &'static str {
    match a {
        ReadArea::Workspace => "plugin-cap-workspace-read",
        ReadArea::Conversations => "plugin-cap-conversations-read",
        ReadArea::Messages => "plugin-cap-messages-read",
        ReadArea::History => "plugin-cap-history-read",
        ReadArea::Dms => "plugin-cap-dms-read",
        ReadArea::People => "plugin-cap-people-read",
        ReadArea::Search => "plugin-cap-search-read",
        ReadArea::Files => "plugin-cap-files-read",
        ReadArea::Reminders => "plugin-cap-reminders-read",
        ReadArea::Scheduled => "plugin-cap-scheduled-read",
        ReadArea::Saved => "plugin-cap-saved-read",
        ReadArea::Pins => "plugin-cap-pins-read",
        ReadArea::Status => "plugin-cap-status-read",
    }
}

fn write_id(a: WriteArea) -> &'static str {
    match a {
        WriteArea::Messages => "plugin-cap-messages-write",
        WriteArea::MessagesEdit => "plugin-cap-messages-edit",
        WriteArea::Reactions => "plugin-cap-reactions-write",
        WriteArea::Reminders => "plugin-cap-reminders-write",
        WriteArea::Scheduled => "plugin-cap-scheduled-write",
        WriteArea::Files => "plugin-cap-files-write",
        WriteArea::Status => "plugin-cap-status-write",
        WriteArea::Saved => "plugin-cap-saved-write",
        WriteArea::Pins => "plugin-cap-pins-write",
        WriteArea::ReadState => "plugin-cap-read-state-write",
    }
}

fn adds_id(a: Adds) -> &'static str {
    match a {
        Adds::Commands => "plugin-cap-adds-commands",
        Adds::SlashCommands => "plugin-cap-adds-slash-commands",
        Adds::MessageActions => "plugin-cap-adds-message-actions",
        Adds::MediaProviders => "plugin-cap-adds-media-providers",
        Adds::McpTools => "plugin-cap-adds-mcp-tools",
    }
}
