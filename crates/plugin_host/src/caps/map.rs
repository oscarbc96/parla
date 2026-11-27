//! Which provider feature (`domain::Capabilities`) each workspace area
//! needs. A provider without it makes the call
//! return `error.unsupported`.

use domain::Capabilities;

use super::check::SearchKind;
use crate::manifest::{ReadArea, WriteArea};

/// A read or write area.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Area {
    Read(ReadArea),
    Write(WriteArea),
}

impl From<ReadArea> for Area {
    fn from(a: ReadArea) -> Self {
        Self::Read(a)
    }
}

impl From<WriteArea> for Area {
    fn from(a: WriteArea) -> Self {
        Self::Write(a)
    }
}

/// The provider feature `area` needs. Workspace, conversations, messages,
/// people, sending and reactions are what every provider has.
pub fn area_capability(area: Area) -> fn(&Capabilities) -> bool {
    match area {
        Area::Read(a) => match a {
            ReadArea::Workspace
            | ReadArea::Conversations
            | ReadArea::Messages
            | ReadArea::History
            | ReadArea::Dms
            | ReadArea::People => |_| true,
            ReadArea::Search => |c| c.search || c.search_messages || c.search_files,
            ReadArea::Files => |c| c.files,
            ReadArea::Reminders => |c| c.reminders,
            ReadArea::Scheduled => |c| c.scheduled_send,
            ReadArea::Saved => |c| c.saved,
            ReadArea::Pins => |c| c.pins,
            ReadArea::Status => |c| c.status,
        },
        Area::Write(a) => match a {
            WriteArea::Messages | WriteArea::Reactions => |_| true,
            WriteArea::MessagesEdit => |c| c.edit || c.delete,
            WriteArea::Reminders => |c| c.reminders,
            WriteArea::Scheduled => |c| c.scheduled_send,
            WriteArea::Files => |c| c.files,
            WriteArea::Status => |c| c.status,
            WriteArea::Saved => |c| c.saved,
            WriteArea::Pins => |c| c.pins,
            WriteArea::ReadState => |c| c.read_state,
        },
    }
}

/// The provider feature one kind of search needs (`search:read` covers
/// all three).
pub fn search_capability(kind: SearchKind) -> fn(&Capabilities) -> bool {
    match kind {
        SearchKind::Messages => |c| c.search_messages,
        SearchKind::Files => |c| c.search_files,
        SearchKind::People => |c| c.search,
    }
}
