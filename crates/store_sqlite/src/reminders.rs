//! Local reminders.

use std::sync::Arc;

use rusqlite::{Connection, params};
use store::StoreError;

use crate::err::other as db;

pub(crate) fn put_reminder(c: &Connection, r: &domain::Reminder) -> Result<(), StoreError> {
    let link = r.link.as_ref();
    c.execute(
        "INSERT OR REPLACE INTO local_reminders (id, text, due, recurring, completed, for_user, fired, interval_secs, \
         link_conversation, link_ts, link_author, link_excerpt, snoozed_until) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            &*r.id.0,
            &*r.text,
            r.due,
            r.recurring,
            r.completed,
            &*r.for_user.0,
            r.fired,
            r.interval_secs,
            link.map(|l| &*l.message.conversation.0),
            link.map(|l| &*l.message.ts.0),
            link.and_then(|l| l.author.as_ref()).map(|u| &*u.0),
            link.map(|l| &*l.excerpt),
            r.snoozed_until,
        ],
    )
    .map_err(db)?;
    Ok(())
}

pub(crate) fn remove_reminder(c: &Connection, id: &domain::ReminderId) -> Result<(), StoreError> {
    c.execute("DELETE FROM local_reminders WHERE id = ?1", [&*id.0])
        .map_err(db)?;
    Ok(())
}

/// By due time (none last), then id.
pub(crate) fn load_reminders(c: &Connection) -> Result<Vec<domain::Reminder>, StoreError> {
    let mut st = c
        .prepare_cached(
            "SELECT id, text, due, recurring, completed, for_user, fired, interval_secs, \
             link_conversation, link_ts, link_author, link_excerpt, snoozed_until FROM local_reminders \
             ORDER BY due IS NULL, due, id",
        )
        .map_err(db)?;
    let mut rows = st.query([]).map_err(db)?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().map_err(db)? {
        out.push(domain::Reminder {
            id: domain::ReminderId(Arc::from(r.get::<_, String>(0).map_err(db)?)),
            text: Arc::from(r.get::<_, String>(1).map_err(db)?),
            due: r.get(2).map_err(db)?,
            recurring: r.get(3).map_err(db)?,
            completed: r.get(4).map_err(db)?,
            for_user: domain::UserId(Arc::from(r.get::<_, String>(5).map_err(db)?)),
            fired: r.get(6).map_err(db)?,
            interval_secs: r.get(7).map_err(db)?,
            link: link_of(r)?,
            snoozed_until: r.get(12).map_err(db)?,
        });
    }
    Ok(out)
}

/// The stored link of a reminder row (columns 8 to 11); `None` without one.
fn link_of(r: &rusqlite::Row<'_>) -> Result<Option<domain::ReminderLink>, StoreError> {
    let conversation: Option<String> = r.get(8).map_err(db)?;
    let ts: Option<String> = r.get(9).map_err(db)?;
    let (Some(conversation), Some(ts)) = (conversation, ts) else {
        return Ok(None);
    };
    let author: Option<String> = r.get(10).map_err(db)?;
    let excerpt: Option<String> = r.get(11).map_err(db)?;
    Ok(Some(domain::ReminderLink {
        message: domain::MessageId {
            conversation: domain::ConversationId(Arc::from(conversation)),
            ts: domain::MessageTs(Arc::from(ts)),
        },
        author: author.map(|a| domain::UserId(Arc::from(a))),
        excerpt: Arc::from(excerpt.unwrap_or_default()),
    }))
}
