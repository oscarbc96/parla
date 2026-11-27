//! Local scheduled sends.

use std::sync::Arc;

use domain::{ConversationId, LocalSchedule, LocalScheduleState, MessageTs, ScheduleId, Upload};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use store::StoreError;

use crate::err::other;
use crate::err::other as db;

#[derive(Serialize, Deserialize)]
struct UploadRow {
    path: std::path::PathBuf,
    name: String,
    mime: String,
    size: u64,
    #[serde(default)]
    filetype: Option<String>,
}

#[derive(Serialize, Deserialize)]
enum StateRow {
    Pending,
    Sending,
    Failed(String),
}

pub(crate) fn put_schedule(c: &Connection, l: &LocalSchedule) -> Result<(), StoreError> {
    let files: Vec<UploadRow> = l
        .files
        .iter()
        .map(|u| UploadRow {
            path: u.path.clone(),
            name: u.name.to_string(),
            mime: u.mime.to_string(),
            size: u.size,
            filetype: u.filetype.as_deref().map(str::to_string),
        })
        .collect();
    let files = if files.is_empty() {
        None
    } else {
        Some(serde_json::to_vec(&files).map_err(other)?)
    };
    let state = serde_json::to_vec(&match &l.state {
        LocalScheduleState::Pending => StateRow::Pending,
        LocalScheduleState::Sending => StateRow::Sending,
        LocalScheduleState::Failed(e) => StateRow::Failed(e.clone()),
    })
    .map_err(other)?;
    c.execute(
        "INSERT OR REPLACE INTO local_schedules \
         (id, conversation_id, thread_ts, text, files, post_at, created_ms, state) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            &*l.id.0,
            &*l.conversation.0,
            l.thread.as_ref().map(|t| &*t.0),
            &*l.text,
            files,
            l.post_at,
            l.created_ms,
            state,
        ],
    )
    .map_err(db)?;
    Ok(())
}

pub(crate) fn remove_schedule(c: &Connection, id: &ScheduleId) -> Result<(), StoreError> {
    c.execute("DELETE FROM local_schedules WHERE id = ?1", [&*id.0])
        .map_err(db)?;
    Ok(())
}

pub(crate) fn load_schedules(c: &Connection) -> Result<Vec<LocalSchedule>, StoreError> {
    let mut st = c
        .prepare_cached(
            "SELECT id, conversation_id, thread_ts, text, files, post_at, created_ms, state \
             FROM local_schedules ORDER BY post_at, id",
        )
        .map_err(db)?;
    let mut rows = st.query([]).map_err(db)?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().map_err(db)? {
        let files: Vec<UploadRow> = match r.get::<_, Option<Vec<u8>>>(4).map_err(db)? {
            Some(b) if !b.is_empty() => serde_json::from_slice(&b).map_err(other)?,
            _ => Vec::new(),
        };
        let state = match r.get::<_, Option<Vec<u8>>>(7).map_err(db)? {
            Some(b) if !b.is_empty() => match serde_json::from_slice(&b).map_err(other)? {
                StateRow::Pending => LocalScheduleState::Pending,
                StateRow::Sending => LocalScheduleState::Sending,
                StateRow::Failed(e) => LocalScheduleState::Failed(e),
            },
            _ => LocalScheduleState::Pending,
        };
        out.push(LocalSchedule {
            id: ScheduleId(Arc::from(r.get::<_, String>(0).map_err(db)?)),
            conversation: ConversationId(Arc::from(r.get::<_, String>(1).map_err(db)?)),
            thread: r
                .get::<_, Option<String>>(2)
                .map_err(db)?
                .map(|t| MessageTs(Arc::from(t))),
            text: Arc::from(r.get::<_, String>(3).map_err(db)?),
            files: files
                .into_iter()
                .map(|u| Upload {
                    path: u.path,
                    name: Arc::from(u.name),
                    mime: Arc::from(u.mime),
                    size: u.size,
                    filetype: u.filetype.map(Arc::from),
                })
                .collect(),
            post_at: r.get(5).map_err(db)?,
            created_ms: r.get(6).map_err(db)?,
            state,
        });
    }
    Ok(out)
}
