//! Provider-neutral catch-up planning and history gaps.
//!
//! Cold start (from the store) and reconnect share this path: [`plan`] picks
//! the conversations that moved, [`run`] pages `history_between` within a
//! bound, and a [`Gap`] records whatever the bound left unfetched so scrolling
//! up can fill it later ([`next_older`]).

use std::collections::HashMap;

use crate::ids::{ConversationId, MessageTs};
use crate::model::{Message, ProviderError};
use crate::provider::ChatProvider;
use crate::sync_types::{ConversationCounts, Gap};

/// Pages fetched per conversation before the rest becomes a gap.
pub const CATCH_UP_MAX_PAGES: u32 = 5;
/// Messages per page (the web client's page size).
pub const CATCH_UP_PAGE: u32 = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatchUp {
    pub conversation: ConversationId,
    pub after: MessageTs,
}

#[derive(Clone, Debug)]
pub struct CatchUpResult {
    pub conversation: ConversationId,
    /// Ascending by ts.
    pub messages: Vec<Message>,
    pub gap: Option<Gap>,
}

/// Jobs for member conversations with a stored ts and a newer remote `latest`.
/// Conversations with nothing stored load their newest page on open.
pub fn plan(
    stored: &HashMap<ConversationId, MessageTs>,
    remote: &[ConversationCounts],
    member_of: impl Fn(&ConversationId) -> bool,
) -> Vec<CatchUp> {
    remote
        .iter()
        .filter(|c| member_of(&c.conversation))
        .filter_map(|c| {
            let after = stored.get(&c.conversation)?;
            let latest = c.latest.as_ref()?;
            (latest > after).then(|| CatchUp {
                conversation: c.conversation.clone(),
                after: after.clone(),
            })
        })
        .collect()
}

/// Pages newest first until the provider has no more or the bound is hit;
/// what remains below the oldest received message becomes the gap.
pub async fn run(
    provider: &dyn ChatProvider,
    job: CatchUp,
) -> Result<CatchUpResult, ProviderError> {
    let mut messages: Vec<Message> = Vec::new();
    let mut before: Option<MessageTs> = None;
    let mut gap = None;
    for _ in 0..CATCH_UP_MAX_PAGES {
        let page = provider
            .history_between(
                &job.conversation,
                job.after.clone(),
                before.clone(),
                CATCH_UP_PAGE,
            )
            .await?;
        let oldest = page.messages.iter().map(|m| m.id.ts.clone()).min();
        messages.extend(page.messages);
        match oldest {
            Some(o) if page.has_more => {
                gap = Some(Gap {
                    conversation: job.conversation.clone(),
                    after: job.after.clone(),
                    before: o.clone(),
                });
                before = Some(o);
            }
            _ => {
                gap = None;
                break;
            }
        }
    }
    messages.sort_by(|a, b| a.id.ts.cmp(&b.id.ts));
    messages.dedup_by(|a, b| a.id.ts == b.id.ts);
    Ok(CatchUpResult {
        conversation: job.conversation,
        messages,
        gap,
    })
}

/// Where the next page of older messages comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Older {
    Store,
    Remote {
        after: Option<MessageTs>,
        before: MessageTs,
    },
}

/// A gap that ends at the oldest held message is crossed remotely; with
/// nothing older stored the remote is asked for everything older (`after:
/// None`); otherwise the store has it.
pub fn next_older(oldest_held: &MessageTs, store_has_older: bool, gaps: &[Gap]) -> Older {
    if let Some(g) = gaps.iter().find(|g| g.before == *oldest_held) {
        return Older::Remote {
            after: Some(g.after.clone()),
            before: oldest_held.clone(),
        };
    }
    if store_has_older {
        Older::Store
    } else {
        Older::Remote {
            after: None,
            before: oldest_held.clone(),
        }
    }
}

/// The range a gap-fill page covered, to hand to `PersistOp::CloseGap`.
/// A page that exhausted the range covers all of it; a full page covers down
/// to its oldest message. `None` when nothing was covered or no lower bound.
pub(crate) fn covered(
    conversation: &ConversationId,
    after: Option<&MessageTs>,
    before: &MessageTs,
    page_oldest: Option<&MessageTs>,
    has_more: bool,
) -> Option<Gap> {
    let lower = if has_more { page_oldest? } else { after? };
    (lower < before).then(|| Gap {
        conversation: conversation.clone(),
        after: lower.clone(),
        before: before.clone(),
    })
}
