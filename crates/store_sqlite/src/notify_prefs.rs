//! Per-conversation local notification preferences.

use std::sync::Arc;

use domain::ConversationId;
use rusqlite::{Connection, params};
use store::StoreError;

use crate::err::other as db;

fn level_code(l: domain::NotifyLevel) -> i64 {
    match l {
        domain::NotifyLevel::Default => 0,
        domain::NotifyLevel::All => 1,
        domain::NotifyLevel::Mentions => 2,
        domain::NotifyLevel::Nothing => 3,
    }
}

fn level_from(code: i64) -> domain::NotifyLevel {
    match code {
        1 => domain::NotifyLevel::All,
        2 => domain::NotifyLevel::Mentions,
        3 => domain::NotifyLevel::Nothing,
        _ => domain::NotifyLevel::Default,
    }
}

/// The default row (level `Default`, not muted) deletes the conversation's row.
pub(crate) fn put_notify_prefs(
    c: &Connection,
    conversation: &ConversationId,
    prefs: domain::ConversationPrefs,
) -> Result<(), StoreError> {
    if prefs == domain::ConversationPrefs::default() {
        c.execute(
            "DELETE FROM local_notify_prefs WHERE conversation_id = ?1",
            [&*conversation.0],
        )
        .map_err(db)?;
    } else {
        c.execute(
            "INSERT OR REPLACE INTO local_notify_prefs (conversation_id, level, muted) \
             VALUES (?1, ?2, ?3)",
            params![&*conversation.0, level_code(prefs.level), prefs.muted],
        )
        .map_err(db)?;
    }
    Ok(())
}

/// The default extras delete the conversation's row.
pub(crate) fn put_notify_extras(
    c: &Connection,
    conversation: &ConversationId,
    extras: domain::NotifyExtras,
) -> Result<(), StoreError> {
    if extras == domain::NotifyExtras::default() {
        c.execute(
            "DELETE FROM local_notify_extras WHERE conversation_id = ?1",
            [&*conversation.0],
        )
        .map_err(db)?;
    } else {
        c.execute(
            "INSERT OR REPLACE INTO local_notify_extras \
             (conversation_id, threads_off, sound_off, mute_until) VALUES (?1, ?2, ?3, ?4)",
            params![
                &*conversation.0,
                extras.threads_off,
                extras.sound_off,
                extras.mute_until
            ],
        )
        .map_err(db)?;
    }
    Ok(())
}

pub(crate) fn load_notify_extras(
    c: &Connection,
) -> Result<std::collections::HashMap<ConversationId, domain::NotifyExtras>, StoreError> {
    let mut st = c
        .prepare_cached(
            "SELECT conversation_id, threads_off, sound_off, mute_until FROM local_notify_extras",
        )
        .map_err(db)?;
    let mut rows = st.query([]).map_err(db)?;
    let mut out = std::collections::HashMap::new();
    while let Some(r) = rows.next().map_err(db)? {
        out.insert(
            ConversationId(Arc::from(r.get::<_, String>(0).map_err(db)?)),
            domain::NotifyExtras {
                threads_off: r.get(1).map_err(db)?,
                sound_off: r.get(2).map_err(db)?,
                mute_until: r.get(3).map_err(db)?,
            },
        );
    }
    Ok(out)
}

/// Stars (`true`) or unstars (`false`) a conversation.
pub(crate) fn put_starred(
    c: &Connection,
    conversation: &ConversationId,
    starred: bool,
) -> Result<(), StoreError> {
    if starred {
        c.execute(
            "INSERT OR IGNORE INTO local_starred (conversation_id) VALUES (?1)",
            [&*conversation.0],
        )
    } else {
        c.execute(
            "DELETE FROM local_starred WHERE conversation_id = ?1",
            [&*conversation.0],
        )
    }
    .map_err(db)?;
    Ok(())
}

/// Collapses (`true`) or expands (`false`) a sidebar section.
pub(crate) fn put_collapsed(
    c: &Connection,
    section: &str,
    collapsed: bool,
) -> Result<(), StoreError> {
    if collapsed {
        c.execute(
            "INSERT OR IGNORE INTO local_sidebar_collapsed (section) VALUES (?1)",
            [section],
        )
    } else {
        c.execute(
            "DELETE FROM local_sidebar_collapsed WHERE section = ?1",
            [section],
        )
    }
    .map_err(db)?;
    Ok(())
}

pub(crate) fn load_collapsed(
    c: &Connection,
) -> Result<std::collections::BTreeSet<String>, StoreError> {
    let mut st = c
        .prepare_cached("SELECT section FROM local_sidebar_collapsed")
        .map_err(db)?;
    let mut rows = st.query([]).map_err(db)?;
    let mut out = std::collections::BTreeSet::new();
    while let Some(r) = rows.next().map_err(db)? {
        out.insert(r.get::<_, String>(0).map_err(db)?);
    }
    Ok(out)
}

pub(crate) fn load_starred(
    c: &Connection,
) -> Result<std::collections::HashSet<ConversationId>, StoreError> {
    let mut st = c
        .prepare_cached("SELECT conversation_id FROM local_starred")
        .map_err(db)?;
    let mut rows = st.query([]).map_err(db)?;
    let mut out = std::collections::HashSet::new();
    while let Some(r) = rows.next().map_err(db)? {
        out.insert(ConversationId(Arc::from(
            r.get::<_, String>(0).map_err(db)?,
        )));
    }
    Ok(out)
}

pub(crate) fn load_notify_prefs(
    c: &Connection,
) -> Result<std::collections::HashMap<ConversationId, domain::ConversationPrefs>, StoreError> {
    let mut st = c
        .prepare_cached("SELECT conversation_id, level, muted FROM local_notify_prefs")
        .map_err(db)?;
    let mut rows = st.query([]).map_err(db)?;
    let mut out = std::collections::HashMap::new();
    while let Some(r) = rows.next().map_err(db)? {
        out.insert(
            ConversationId(Arc::from(r.get::<_, String>(0).map_err(db)?)),
            domain::ConversationPrefs {
                level: level_from(r.get(1).map_err(db)?),
                muted: r.get(2).map_err(db)?,
            },
        );
    }
    Ok(out)
}
