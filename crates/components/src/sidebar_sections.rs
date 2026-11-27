//! Parla's own sidebar sections: local, per workspace, kept under
//! `sidebar.sections` in the workspace's settings file (never sent to the
//! provider). The edits the sidebar, its organise mode and the conversation
//! menu make, as one pure function the host applies before it writes the file.

use settings::CustomSection;

/// The longest section name kept (longer names are cut).
pub const SECTION_NAME_MAX: usize = 80;

/// One edit of the custom sections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SectionOp {
    /// A new, empty section at the end.
    Create {
        name: String,
    },
    Rename {
        id: u32,
        name: String,
    },
    /// Removes the section; its conversations go back to their own sections.
    Delete {
        id: u32,
    },
    /// Moves the section to position `to` among the custom sections.
    Reorder {
        id: u32,
        to: usize,
    },
    /// Puts the conversation in section `to`, or back in its own section
    /// (Channels or Direct messages) for `None`.
    MoveConversation {
        conversation: String,
        to: Option<u32>,
    },
}

/// A name as kept: trimmed, at most [`SECTION_NAME_MAX`] characters; `None`
/// when nothing is left.
fn clean_name(name: &str) -> Option<String> {
    let name: String = name.trim().chars().take(SECTION_NAME_MAX).collect();
    let name = name.trim_end().to_string();
    (!name.is_empty()).then_some(name)
}

/// Applies `op` to `sections`. False when nothing changed (an empty name, an
/// unknown section, a move to where it already is), so the host writes
/// nothing.
pub fn apply_section_op(sections: &mut Vec<CustomSection>, op: SectionOp) -> bool {
    match op {
        SectionOp::Create { name } => {
            let Some(name) = clean_name(&name) else {
                return false;
            };
            let id = sections.iter().map(|s| s.id).max().unwrap_or(0) + 1;
            sections.push(CustomSection {
                id,
                name,
                conversations: Vec::new(),
            });
            true
        }
        SectionOp::Rename { id, name } => {
            let Some(name) = clean_name(&name) else {
                return false;
            };
            match sections.iter_mut().find(|s| s.id == id) {
                Some(s) if s.name != name => {
                    s.name = name;
                    true
                }
                _ => false,
            }
        }
        SectionOp::Delete { id } => {
            let before = sections.len();
            sections.retain(|s| s.id != id);
            sections.len() != before
        }
        SectionOp::Reorder { id, to } => {
            let Some(from) = sections.iter().position(|s| s.id == id) else {
                return false;
            };
            let to = to.min(sections.len() - 1);
            if from == to {
                return false;
            }
            let s = sections.remove(from);
            sections.insert(to, s);
            true
        }
        SectionOp::MoveConversation { conversation, to } => {
            if to.is_some_and(|t| !sections.iter().any(|s| s.id == t)) {
                return false;
            }
            let mut changed = false;
            for s in sections.iter_mut() {
                if Some(s.id) == to {
                    continue;
                }
                let before = s.conversations.len();
                s.conversations.retain(|c| *c != conversation);
                changed |= s.conversations.len() != before;
            }
            if let Some(s) = to.and_then(|t| sections.iter_mut().find(|s| s.id == t))
                && !s.conversations.contains(&conversation)
            {
                s.conversations.push(conversation);
                changed = true;
            }
            changed
        }
    }
}

/// The custom section holding `conversation`, if any (the first one wins).
pub fn section_of(sections: &[CustomSection], conversation: &str) -> Option<u32> {
    sections
        .iter()
        .find(|s| s.conversations.iter().any(|c| c == conversation))
        .map(|s| s.id)
}
