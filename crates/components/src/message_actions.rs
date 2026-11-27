//! The message actions menu: which actions a message offers.
//!
//! Each action shows only when the provider has the capability and, for
//! edit and delete, when the message is the user's own. A message that is
//! still sending or failed offers none.

use domain::{Capabilities, DeliveryState, Message};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageAction {
    ReplyInThread,
    AddReaction,
    Save,
    RemoveSaved,
    /// Remind me about this message (Parla keeps the reminder).
    RemindMe,
    Pin,
    Unpin,
    CopyLink,
    MarkUnread,
    Edit,
    Delete,
}

impl MessageAction {
    /// The action's label in the active language.
    pub fn label(self, l: &i18n::Localizer) -> String {
        match self {
            MessageAction::ReplyInThread => t!(l, "message-action-reply"),
            MessageAction::AddReaction => t!(l, "message-action-react"),
            MessageAction::Save => t!(l, "message-action-save"),
            MessageAction::RemoveSaved => t!(l, "message-action-unsave"),
            MessageAction::RemindMe => t!(l, "message-action-remind"),
            MessageAction::Pin => t!(l, "message-action-pin"),
            MessageAction::Unpin => t!(l, "message-action-unpin"),
            MessageAction::CopyLink => t!(l, "message-action-copy-link"),
            MessageAction::MarkUnread => t!(l, "message-action-mark-unread"),
            MessageAction::Edit => t!(l, "message-action-edit"),
            MessageAction::Delete => t!(l, "message-action-delete"),
        }
        .into_owned()
    }
}

/// The actions of `m`, in menu order. `own` is whether the user wrote it and
/// `has_link` whether the provider can make a link to it.
pub fn actions_for(
    m: &Message,
    caps: &Capabilities,
    own: bool,
    has_link: bool,
) -> Vec<MessageAction> {
    if m.state != DeliveryState::Delivered {
        return Vec::new();
    }
    let mut out = Vec::new();
    if caps.threads {
        out.push(MessageAction::ReplyInThread);
    }
    out.push(MessageAction::AddReaction);
    if caps.saved {
        out.push(if m.saved {
            MessageAction::RemoveSaved
        } else {
            MessageAction::Save
        });
    }
    if caps.pins {
        out.push(if m.pinned {
            MessageAction::Unpin
        } else {
            MessageAction::Pin
        });
    }
    if has_link {
        out.push(MessageAction::CopyLink);
    }
    out.push(MessageAction::MarkUnread);
    if own && caps.edit {
        out.push(MessageAction::Edit);
    }
    if own && caps.delete {
        out.push(MessageAction::Delete);
    }
    out
}
