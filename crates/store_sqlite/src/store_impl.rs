//! `impl Store for SqliteStore`: writes run as jobs on the writer thread,
//! reads on the reader threads.

use std::collections::HashMap;
use std::sync::Arc;

use domain::{
    ClientMsgId, ComposedText, Conversation, ConversationId, ConversationKind, DeliveryState,
    DraftKey, ExtCodec, ExtKind, Gap, Member, Message, MessageId, MessageTs, OutboxItem, PersistOp,
    SearchHit, UserId,
};
use futures::future::BoxFuture;
use rusqlite::{Connection, Row, params};
use settings::WorkspaceKey;
use store::{Draft, Store, StoreError};

use crate::err::other as db;
use crate::rows;
use crate::thread::SqliteStore;

type Codec = Arc<dyn ExtCodec>;

/// Numeric sort key (`secs * 1e6 + micros`) matching `MessageTs` ordering for
/// well-formed timestamps; unparsable text sorts first.
fn ts_key(ts: &MessageTs) -> i64 {
    let s: &str = &ts.0;
    let (secs, frac) = s.split_once('.').unwrap_or((s, ""));
    let digits = |t: &str| t.bytes().all(|b| b.is_ascii_digit());
    if secs.is_empty() || !digits(secs) || !digits(frac) || frac.len() > 6 {
        return 0;
    }
    let Ok(secs) = secs.parse::<i64>() else {
        return 0;
    };
    let micros: i64 = format!("{frac:0<6}").parse().unwrap_or(0);
    secs.saturating_mul(1_000_000).saturating_add(micros)
}

fn t(s: &Option<MessageTs>) -> Option<&str> {
    s.as_ref().map(|t| &*t.0)
}

/// The canonical text a timestamp is stored under, so spellings that compare
/// equal (`1.5` and `1.500000`) are one row in a table keyed by text.
fn ts_text(ts: &MessageTs) -> String {
    ts.normalized().0.to_string()
}

fn ts_opt(s: &Option<MessageTs>) -> Option<String> {
    s.as_ref().map(ts_text)
}

fn mts(s: String) -> MessageTs {
    MessageTs(Arc::from(s))
}

fn kind_to_int(k: ConversationKind) -> i64 {
    match k {
        ConversationKind::Channel => 0,
        ConversationKind::PrivateChannel => 1,
        ConversationKind::Direct => 2,
        ConversationKind::Group => 3,
    }
}

fn kind_from_int(i: i64) -> ConversationKind {
    match i {
        1 => ConversationKind::PrivateChannel,
        2 => ConversationKind::Direct,
        3 => ConversationKind::Group,
        _ => ConversationKind::Channel,
    }
}

const MSG_COLS: &str = "conversation_id, ts, thread_ts, broadcast, author, text, plain, \
    mentions, files, reactions, reply_count, latest_reply, reply_users, edited_ts, \
    client_msg_id, ext";

/// The window query: one conversation's messages older than `?2`, newest
/// first. Shared with the query-plan test.
pub(crate) fn window_sql() -> String {
    format!(
        "SELECT {MSG_COLS} FROM messages WHERE conversation_id = ?1 AND ts_key < ?2 \
         AND NOT (thread_ts IS NOT NULL AND thread_ts <> ts AND broadcast = 0) \
         ORDER BY ts_key DESC, ts DESC LIMIT ?3"
    )
}

/// The Files tab query: one conversation's messages with files before
/// `(?2, ?4)` in `(ts_key, ts)` order, newest first, thread replies included.
/// Comparing the pair keeps messages that share a `ts_key` (unparsable ids
/// all have 0) from being skipped at a page boundary. Served by
/// `messages_files`.
pub(crate) fn files_sql() -> String {
    format!(
        "SELECT {MSG_COLS} FROM messages WHERE conversation_id = ?1 \
         AND (ts_key < ?2 OR (ts_key = ?2 AND ts < ?4)) \
         AND files IS NOT NULL ORDER BY ts_key DESC, ts DESC LIMIT ?3"
    )
}

/// The `latest_all` query: the conversation list from `conversations` and one
/// `messages_window` seek per conversation for its latest channel message
/// (never a scan of the whole messages index). The inner query walks back
/// from the newest and stops at the first channel message. Shared with the
/// query-plan test.
pub(crate) fn latest_sql() -> &'static str {
    "SELECT conversations.id, (SELECT m.ts FROM messages m \
       WHERE m.conversation_id = conversations.id \
       AND NOT (m.thread_ts IS NOT NULL AND m.thread_ts <> m.ts AND m.broadcast = 0) \
       ORDER BY m.ts_key DESC, m.ts DESC LIMIT 1) \
     FROM conversations"
}

fn message_from_row(r: &Row, codec: &Codec) -> Result<Message, StoreError> {
    let s = |i: usize| r.get::<_, String>(i).map_err(db);
    let os = |i: usize| r.get::<_, Option<String>>(i).map_err(db);
    let blob = |i: usize| r.get::<_, Option<Vec<u8>>>(i).map_err(db);
    let text: Arc<str> = Arc::from(s(5)?);
    Ok(Message {
        id: MessageId {
            conversation: ConversationId(Arc::from(s(0)?)),
            ts: mts(s(1)?),
        },
        author: os(4)?.map(|a| UserId(Arc::from(a))),
        text,
        thread_root: os(2)?.map(mts),
        reply_count: r.get::<_, Option<u32>>(10).map_err(db)?.unwrap_or(0),
        edited: os(13)?.map(mts),
        client_msg_id: os(14)?.map(|c| ClientMsgId(Arc::from(c))),
        state: DeliveryState::Delivered,
        ext: rows::decode_ext(codec, ExtKind::Message, blob(15)?),
        plain: Arc::from(s(6)?),
        mentions: rows::decode_mentions(blob(7)?)?,
        reactions: rows::decode_reactions(blob(9)?)?,
        files: rows::decode_files(blob(8)?, codec)?,
        latest_reply: os(11)?.map(mts),
        reply_users: rows::decode_users(blob(12)?)?,
        broadcast: r.get::<_, i64>(3).map_err(db)? != 0,
        pinned: false,
        saved: false,
    })
}

fn query_messages(
    c: &Connection,
    codec: &Codec,
    sql: &str,
    p: impl rusqlite::Params,
) -> Result<Vec<Message>, StoreError> {
    let mut st = c.prepare_cached(sql).map_err(db)?;
    let mut rows = st.query(p).map_err(db)?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().map_err(db)? {
        out.push(message_from_row(r, codec)?);
    }
    Ok(out)
}

fn upsert_message(
    c: &Connection,
    codec: &Codec,
    conversation: &ConversationId,
    m: &Message,
) -> Result<(), StoreError> {
    let mut st = c
        .prepare_cached(
            "INSERT INTO messages (conversation_id, ts, ts_key, thread_ts, broadcast, author, \
             text, plain, mentions, files, reactions, reply_count, latest_reply, reply_users, \
             edited_ts, client_msg_id, ext) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17) \
             ON CONFLICT (conversation_id, ts) DO UPDATE SET \
             thread_ts = excluded.thread_ts, broadcast = excluded.broadcast, \
             author = excluded.author, text = excluded.text, plain = excluded.plain, \
             mentions = excluded.mentions, files = excluded.files, \
             reactions = excluded.reactions, reply_count = excluded.reply_count, \
             latest_reply = excluded.latest_reply, reply_users = excluded.reply_users, \
             edited_ts = excluded.edited_ts, \
             client_msg_id = COALESCE(excluded.client_msg_id, client_msg_id), \
             ext = excluded.ext",
        )
        .map_err(db)?;
    st.execute(params![
        &*conversation.0,
        ts_text(&m.id.ts),
        ts_key(&m.id.ts),
        ts_opt(&m.thread_root),
        m.broadcast,
        m.author.as_ref().map(|a| &*a.0),
        &*m.text,
        &*m.plain,
        rows::encode_mentions(&m.mentions)?,
        rows::encode_files(&m.files, codec)?,
        rows::encode_reactions(&m.reactions)?,
        m.reply_count,
        ts_opt(&m.latest_reply),
        rows::encode_users(&m.reply_users)?,
        ts_opt(&m.edited),
        m.client_msg_id.as_ref().map(|c| &*c.0),
        rows::encode_ext(codec, ExtKind::Message, &m.ext),
    ])
    .map_err(db)?;
    Ok(())
}

fn load_gaps(c: &Connection, conv: &ConversationId) -> Result<Vec<Gap>, StoreError> {
    let mut st = c
        .prepare_cached("SELECT after_ts, before_ts FROM gaps WHERE conversation_id = ?1")
        .map_err(db)?;
    let mut rows = st.query([&*conv.0]).map_err(db)?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().map_err(db)? {
        out.push(Gap {
            conversation: conv.clone(),
            after: mts(r.get::<_, String>(0).map_err(db)?).normalized(),
            before: mts(r.get::<_, String>(1).map_err(db)?).normalized(),
        });
    }
    Ok(out)
}

fn save_gaps(c: &Connection, conv: &ConversationId, gaps: &[Gap]) -> Result<(), StoreError> {
    c.execute("DELETE FROM gaps WHERE conversation_id = ?1", [&*conv.0])
        .map_err(db)?;
    let mut st = c
        .prepare_cached("INSERT INTO gaps (conversation_id, after_ts, before_ts) VALUES (?1,?2,?3)")
        .map_err(db)?;
    for g in gaps {
        st.execute(params![&*conv.0, ts_text(&g.after), ts_text(&g.before)])
            .map_err(db)?;
    }
    Ok(())
}

fn conversation_from_row(r: &Row, codec: &Codec) -> Result<Conversation, StoreError> {
    let os = |i: usize| r.get::<_, Option<String>>(i).map_err(db);
    let flag = |i: usize| r.get::<_, Option<i64>>(i).map(|v| v.unwrap_or(0) != 0);
    Ok(Conversation {
        id: ConversationId(Arc::from(r.get::<_, String>(0).map_err(db)?)),
        kind: kind_from_int(r.get(1).map_err(db)?),
        name: Arc::from(r.get::<_, String>(2).map_err(db)?),
        peer: os(3)?.map(|p| UserId(Arc::from(p))),
        is_member: flag(4).map_err(db)?,
        is_archived: flag(5).map_err(db)?,
        is_open: r
            .get::<_, Option<i64>>(15)
            .map_err(db)?
            .is_none_or(|v| v != 0),
        is_muted: flag(6).map_err(db)?,
        last_read: os(7)?.map(mts),
        latest: os(8)?.map(mts),
        unread: flag(9).map_err(db)?,
        mentions: r.get::<_, Option<u32>>(10).map_err(db)?.unwrap_or(0),
        participants: os(12)?
            .filter(|p| !p.is_empty())
            .map(|p| p.split('\n').map(Arc::<str>::from).collect())
            .unwrap_or_else(|| Arc::from([])),
        member_ids: Arc::from([]),
        topic: os(13)?.map(Arc::<str>::from).unwrap_or_default(),
        member_count: r.get::<_, Option<u32>>(14).map_err(db)?,
        purpose: os(16)?.map(Arc::<str>::from).unwrap_or_default(),
        ext: rows::decode_ext(codec, ExtKind::Conversation, r.get(11).map_err(db)?),
    })
}

fn member_from_row(r: &Row, codec: &Codec) -> Result<Member, StoreError> {
    let flag = |i: usize| r.get::<_, Option<i64>>(i).map(|v| v.unwrap_or(0) != 0);
    let text = |i: usize| {
        r.get::<_, Option<String>>(i)
            .map(|v| Arc::<str>::from(v.unwrap_or_default()))
    };
    let opt = |i: usize| {
        r.get::<_, Option<String>>(i)
            .map(|v| v.map(Arc::<str>::from))
    };
    Ok(Member {
        id: UserId(Arc::from(r.get::<_, String>(0).map_err(db)?)),
        display_name: text(1).map_err(db)?,
        real_name: text(2).map_err(db)?,
        handle: text(6).map_err(db)?,
        is_bot: flag(3).map_err(db)?,
        deleted: flag(4).map_err(db)?,
        status: None,
        title: opt(7).map_err(db)?,
        tz: opt(8).map_err(db)?,
        avatar: {
            let get = |i| -> Result<Option<domain::ImageRef>, StoreError> {
                Ok(r.get::<_, Option<String>>(i)
                    .map_err(db)?
                    .and_then(|v| rows::decode_avatar(&v)))
            };
            domain::Avatar::new(get(9)?, get(10)?)
        },
        ext: rows::decode_ext(codec, ExtKind::Member, r.get(5).map_err(db)?),
    })
}

fn outbox_from_row(r: &Row) -> Result<OutboxItem, StoreError> {
    Ok(OutboxItem {
        id: ClientMsgId(Arc::from(r.get::<_, String>(0).map_err(db)?)),
        conversation: ConversationId(Arc::from(r.get::<_, String>(1).map_err(db)?)),
        thread: r.get::<_, Option<String>>(2).map_err(db)?.map(mts),
        broadcast: r.get::<_, Option<i64>>(3).map_err(db)?.unwrap_or(0) != 0,
        text: Arc::from(r.get::<_, String>(4).map_err(db)?),
        created_ms: r.get(5).map_err(db)?,
        attempts: r.get::<_, Option<u32>>(6).map_err(db)?.unwrap_or(0),
        state: rows::decode_outbox_state(r.get(7).map_err(db)?)?,
    })
}

fn apply_op(c: &Connection, codec: &Codec, op: PersistOp) -> Result<(), StoreError> {
    match op {
        PersistOp::Conversations(cs) => {
            // Upsert so `last_opened_ms`, which the model does not carry, survives.
            let mut st = c
                .prepare_cached(
                    "INSERT INTO conversations (id, kind, name, peer, is_member, is_archived, \
                     is_muted, last_read, latest, unread, mentions, ext, participants, topic, \
                     member_count, is_open, purpose) \
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17) \
                     ON CONFLICT (id) DO UPDATE SET kind = excluded.kind, name = excluded.name, \
                     peer = excluded.peer, is_member = excluded.is_member, \
                     is_archived = excluded.is_archived, is_muted = excluded.is_muted, \
                     last_read = excluded.last_read, latest = excluded.latest, \
                     unread = excluded.unread, mentions = excluded.mentions, ext = excluded.ext, \
                     participants = excluded.participants, topic = excluded.topic, \
                     member_count = excluded.member_count, is_open = excluded.is_open, \
                     purpose = excluded.purpose",
                )
                .map_err(db)?;
            for v in cs {
                st.execute(params![
                    &*v.id.0,
                    kind_to_int(v.kind),
                    &*v.name,
                    v.peer.as_ref().map(|p| &*p.0),
                    v.is_member,
                    v.is_archived,
                    v.is_muted,
                    ts_opt(&v.last_read),
                    ts_opt(&v.latest),
                    v.unread,
                    v.mentions,
                    rows::encode_ext(codec, ExtKind::Conversation, &v.ext),
                    v.participants.join("\n"),
                    &*v.topic,
                    v.member_count,
                    v.is_open,
                    &*v.purpose,
                ])
                .map_err(db)?;
            }
        }
        PersistOp::Members(ms) => {
            let mut st = c
                .prepare_cached(
                    "INSERT OR REPLACE INTO members \
                     (id, display_name, real_name, is_bot, deleted, ext, handle, title, tz, avatar, avatar_lg) \
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                )
                .map_err(db)?;
            for m in ms {
                st.execute(params![
                    &*m.id.0,
                    &*m.display_name,
                    &*m.real_name,
                    m.is_bot,
                    m.deleted,
                    rows::encode_ext(codec, ExtKind::Member, &m.ext),
                    &*m.handle,
                    m.title.as_deref(),
                    m.tz.as_deref(),
                    m.avatar.as_ref().map(|a| rows::encode_avatar(&a.row)),
                    m.avatar.as_ref().map(|a| rows::encode_avatar(&a.large)),
                ])
                .map_err(db)?;
            }
        }
        PersistOp::Messages {
            conversation,
            messages,
        } => {
            for m in &messages {
                upsert_message(c, codec, &conversation, m)?;
            }
        }
        PersistOp::EditMessage {
            id,
            text,
            plain,
            edited,
        } => {
            c.execute(
                "UPDATE messages SET text = ?3, edited_ts = ?4, plain = ?5 \
                 WHERE conversation_id = ?1 AND ts = ?2",
                params![
                    &*id.conversation.0,
                    ts_text(&id.ts),
                    &*text,
                    ts_text(&edited),
                    &*plain
                ],
            )
            .map_err(db)?;
        }
        PersistOp::DeleteMessage(id) => {
            c.execute(
                "DELETE FROM messages WHERE conversation_id = ?1 AND ts = ?2",
                params![&*id.conversation.0, ts_text(&id.ts)],
            )
            .map_err(db)?;
        }
        PersistOp::ReadMarker { conversation, ts } => {
            c.execute(
                "UPDATE conversations SET last_read = ?2 WHERE id = ?1",
                params![&*conversation.0, ts_text(&ts)],
            )
            .map_err(db)?;
        }
        PersistOp::ReplaceMessage(m) => upsert_message(c, codec, &m.id.conversation, &m)?,
        PersistOp::Reactions { id, reactions } => {
            c.execute(
                "UPDATE messages SET reactions = ?3 WHERE conversation_id = ?1 AND ts = ?2",
                params![
                    &*id.conversation.0,
                    ts_text(&id.ts),
                    rows::encode_reactions(&reactions)?
                ],
            )
            .map_err(db)?;
        }
        PersistOp::ThreadSummary {
            root,
            reply_count,
            latest_reply,
        } => {
            c.execute(
                "UPDATE messages SET reply_count = ?3, latest_reply = ?4 \
                 WHERE conversation_id = ?1 AND ts = ?2",
                params![
                    &*root.conversation.0,
                    ts_text(&root.ts),
                    reply_count,
                    ts_opt(&latest_reply)
                ],
            )
            .map_err(db)?;
        }
        PersistOp::AddGap(g) => {
            let mut gaps = load_gaps(c, &g.conversation)?;
            if !gaps.contains(&g) {
                gaps.push(g.clone());
                save_gaps(c, &g.conversation, &gaps)?;
            }
        }
        PersistOp::CloseGap(g) => {
            let (conv, stored) = (&g.conversation, load_gaps(c, &g.conversation)?);
            let gaps: Vec<_> = stored.into_iter().flat_map(|s| s.subtract(&g)).collect();
            save_gaps(c, conv, &gaps)?;
        }
        PersistOp::Opened {
            conversation,
            at_ms,
        } => {
            // Upsert, not an update: the conversation's row may not exist yet
            // (the app can open it before the bootstrap has stored it), and a
            // no-op would let retention prune the page fetched for it.
            c.execute(
                "INSERT INTO opened (conversation_id, at_ms) VALUES (?1, ?2) \
                 ON CONFLICT (conversation_id) DO UPDATE SET at_ms = excluded.at_ms",
                params![&*conversation.0, at_ms],
            )
            .map_err(db)?;
            // Kept for databases and tools that read the column; retention
            // also reads `opened`, so the marker survives the row's absence.
            c.execute(
                "UPDATE conversations SET last_opened_ms = ?2 WHERE id = ?1",
                params![&*conversation.0, at_ms],
            )
            .map_err(db)?;
        }
        PersistOp::LocalSchedulePut(l) => crate::schedules::put_schedule(c, &l)?,
        PersistOp::LocalScheduleRemove(id) => crate::schedules::remove_schedule(c, &id)?,
        PersistOp::LocalNotifyPrefsPut {
            conversation,
            prefs,
        } => crate::notify_prefs::put_notify_prefs(c, &conversation, prefs)?,
        PersistOp::LocalNotifyExtrasPut {
            conversation,
            extras,
        } => crate::notify_prefs::put_notify_extras(c, &conversation, extras)?,
        PersistOp::LocalStarredPut {
            conversation,
            starred,
        } => crate::notify_prefs::put_starred(c, &conversation, starred)?,
        PersistOp::LocalReminderPut(r) => crate::reminders::put_reminder(c, &r)?,
        PersistOp::LocalReminderRemove(id) => crate::reminders::remove_reminder(c, &id)?,
        PersistOp::UserGroups(all) => {
            c.execute("DELETE FROM user_groups", []).map_err(db)?;
            let mut st = c
                .prepare_cached(
                    "INSERT OR REPLACE INTO user_groups \
                     (id, handle, name, member_count, is_member) VALUES (?1,?2,?3,?4,?5)",
                )
                .map_err(db)?;
            for g in all.iter().take(domain::MAX_USER_GROUPS) {
                st.execute(params![
                    &*g.id,
                    &*g.handle,
                    &*g.name,
                    g.member_count,
                    g.is_member
                ])
                .map_err(db)?;
            }
        }
        PersistOp::FileDeleted(file) => {
            // The blob is JSON, so a message holding the file contains its quoted id.
            // That prefilter keeps the writer from decoding every message with files.
            let needle = serde_json::to_vec(&*file.0).map_err(crate::err::other)?;
            let hits: Vec<(String, String, Option<Vec<u8>>)> = {
                let mut st = c
                    .prepare_cached(
                        "SELECT conversation_id, ts, files FROM messages \
                         WHERE files IS NOT NULL AND instr(files, ?1) > 0",
                    )
                    .map_err(db)?;
                st.query_map([needle], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .map_err(db)?
                    .collect::<Result<_, _>>()
                    .map_err(db)?
            };
            for (conv, ts, blob) in hits {
                let mut files = rows::decode_files(blob, codec)?;
                if !domain::tombstone_file(&mut files, &file) {
                    continue;
                }
                c.execute(
                    "UPDATE messages SET files = ?3 WHERE conversation_id = ?1 AND ts = ?2",
                    params![conv, ts, rows::encode_files(&files, codec)?],
                )
                .map_err(db)?;
            }
        }
    }
    Ok(())
}

impl Store for SqliteStore {
    fn apply(&self, ws: &WorkspaceKey, op: PersistOp) -> BoxFuture<'_, Result<(), StoreError>> {
        self.write(ws, move |c, codec| apply_op(c, codec, op))
    }

    fn conversations(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<Conversation>, StoreError>> {
        self.read(ws, |c, codec| {
            let mut st = c
                .prepare_cached(
                    "SELECT id, kind, name, peer, is_member, is_archived, is_muted, last_read, \
                     latest, unread, mentions, ext, participants, topic, member_count, is_open, \
                     purpose FROM conversations ORDER BY id",
                )
                .map_err(db)?;
            let mut rows = st.query([]).map_err(db)?;
            let mut out = Vec::new();
            while let Some(r) = rows.next().map_err(db)? {
                out.push(conversation_from_row(r, codec)?);
            }
            Ok(out)
        })
    }

    fn members(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<Vec<Member>, StoreError>> {
        self.read(ws, |c, codec| {
            let mut st = c
                .prepare_cached(
                    "SELECT id, display_name, real_name, is_bot, deleted, ext, handle, title, tz, avatar, avatar_lg \
                     FROM members ORDER BY id",
                )
                .map_err(db)?;
            let mut rows = st.query([]).map_err(db)?;
            let mut out = Vec::new();
            while let Some(r) = rows.next().map_err(db)? {
                out.push(member_from_row(r, codec)?);
            }
            Ok(out)
        })
    }

    fn messages_before(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        before: Option<MessageTs>,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<Message>, StoreError>> {
        let conv = c.0.to_string();
        // `i64::MAX` keeps the statement identical for the "newest" window.
        let upper = before.as_ref().map_or(i64::MAX, ts_key);
        let sql = window_sql();
        self.read(ws, move |c, codec| {
            query_messages(c, codec, &sql, params![conv, upper, limit as i64])
        })
    }

    #[cfg(feature = "test-support")]
    fn latest_ts(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
    ) -> BoxFuture<'_, Result<Option<MessageTs>, StoreError>> {
        use rusqlite::OptionalExtension;
        let conv = c.0.to_string();
        self.read(ws, move |c, _| {
            c.query_row(
                "SELECT ts FROM messages WHERE conversation_id = ?1 \
                 ORDER BY ts_key DESC, ts DESC LIMIT 1",
                [conv],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map(|v| v.map(mts))
            .map_err(db)
        })
    }

    fn thread(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        root: &MessageTs,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<Message>, StoreError>> {
        let (conv, root_ts, key) = (c.0.to_string(), ts_text(root), ts_key(root));
        let sql = format!(
            "SELECT {MSG_COLS} FROM messages WHERE conversation_id = ?1 \
             AND (ts_key = ?3 OR thread_ts = ?2) ORDER BY ts_key ASC, ts ASC LIMIT ?4"
        );
        self.read(ws, move |c, codec| {
            query_messages(c, codec, &sql, params![conv, root_ts, key, limit as i64])
        })
    }

    fn latest_all(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<HashMap<ConversationId, MessageTs>, StoreError>> {
        self.read(ws, |c, _| {
            let mut st = c
                .prepare_cached(crate::store_impl::latest_sql())
                .map_err(db)?;
            let mut rows = st.query([]).map_err(db)?;
            let mut out = HashMap::new();
            while let Some(r) = rows.next().map_err(db)? {
                let Some(ts) = r.get::<_, Option<String>>(1).map_err(db)? else {
                    continue;
                };
                out.insert(
                    ConversationId(Arc::from(r.get::<_, String>(0).map_err(db)?)),
                    mts(ts),
                );
            }
            Ok(out)
        })
    }

    fn gaps(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
    ) -> BoxFuture<'_, Result<Vec<Gap>, StoreError>> {
        let conv = c.clone();
        self.read(ws, move |c, _| {
            let mut v = load_gaps(c, &conv)?;
            v.sort_by(|a, b| a.after.cmp(&b.after));
            Ok(v)
        })
    }

    fn outbox(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<Vec<OutboxItem>, StoreError>> {
        self.read(ws, |c, _| {
            let mut st = c
                .prepare_cached(
                    "SELECT id, conversation_id, thread_ts, broadcast, text, created_ms, \
                     attempts, state FROM outbox ORDER BY created_ms, id",
                )
                .map_err(db)?;
            let mut rows = st.query([]).map_err(db)?;
            let mut out = Vec::new();
            while let Some(r) = rows.next().map_err(db)? {
                out.push(outbox_from_row(r)?);
            }
            Ok(out)
        })
    }

    fn outbox_put(
        &self,
        ws: &WorkspaceKey,
        item: OutboxItem,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        self.write(ws, move |c, _| {
            c.execute(
                "INSERT OR REPLACE INTO outbox \
                 (id, conversation_id, thread_ts, broadcast, text, created_ms, attempts, state) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    &*item.id.0,
                    &*item.conversation.0,
                    t(&item.thread),
                    item.broadcast,
                    &*item.text,
                    item.created_ms,
                    item.attempts,
                    rows::encode_outbox_state(&item.state)?,
                ],
            )
            .map(|_| ())
            .map_err(db)
        })
    }

    fn outbox_remove(
        &self,
        ws: &WorkspaceKey,
        id: &ClientMsgId,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        let id = id.0.to_string();
        self.write(ws, move |c, _| {
            c.execute("DELETE FROM outbox WHERE id = ?1", [id])
                .map(|_| ())
                .map_err(db)
        })
    }

    fn drafts(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<Vec<Draft>, StoreError>> {
        self.read(ws, |c, _| {
            let mut st = c
                .prepare_cached(
                    "SELECT conversation_id, thread_ts, body, spans, edited_at FROM drafts \
                     ORDER BY conversation_id, thread_ts",
                )
                .map_err(db)?;
            let mut rows = st.query([]).map_err(db)?;
            let mut out = Vec::new();
            while let Some(r) = rows.next().map_err(db)? {
                let thread: String = r.get(1).map_err(db)?;
                out.push((
                    DraftKey {
                        conversation: ConversationId(Arc::from(r.get::<_, String>(0).map_err(db)?)),
                        thread: (!thread.is_empty()).then(|| mts(thread)),
                    },
                    {
                        let body: String = r.get(2).map_err(db)?;
                        let spans = r
                            .get::<_, Option<String>>(3)
                            .map_err(db)?
                            .map(|j| domain::spans_from_json(&j, &body))
                            .unwrap_or_default();
                        ComposedText::new(&body, spans)
                    },
                    r.get::<_, i64>(4).map_err(db)?,
                ));
            }
            Ok(out)
        })
    }

    fn draft_put(
        &self,
        ws: &WorkspaceKey,
        key: DraftKey,
        body: Option<ComposedText>,
        edited: i64,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        self.write(ws, move |c, _| {
            let thread = key.thread.as_ref().map_or("", |t| &*t.0);
            match body {
                Some(b) if !b.text.trim().is_empty() => c.execute(
                    "INSERT OR REPLACE INTO drafts \
                     (conversation_id, thread_ts, body, spans, edited_at) VALUES (?1,?2,?3,?4,?5)",
                    params![
                        &*key.conversation.0,
                        thread,
                        &*b.text,
                        domain::spans_to_json(&b.spans),
                        edited
                    ],
                ),
                _ => c.execute(
                    "DELETE FROM drafts WHERE conversation_id = ?1 AND thread_ts = ?2",
                    params![&*key.conversation.0, thread],
                ),
            }
            .map(|_| ())
            .map_err(db)
        })
    }

    fn message(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        ts: &MessageTs,
    ) -> BoxFuture<'_, Result<Option<Message>, StoreError>> {
        let (conv, ts) = (c.0.to_string(), ts_text(ts));
        let sql = format!("SELECT {MSG_COLS} FROM messages WHERE conversation_id = ?1 AND ts = ?2");
        self.read(ws, move |c, codec| {
            Ok(query_messages(c, codec, &sql, params![conv, ts])?
                .into_iter()
                .next())
        })
    }

    /// FTS5 match on `plain`, best `bm25` first. `rank` is `-bm25` (higher is better).
    fn search(
        &self,
        ws: &WorkspaceKey,
        query: &str,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<SearchHit>, StoreError>> {
        let user = query.to_owned();
        self.read(ws, move |c, _| {
            let Some(q) = crate::search::fts_query(&user) else {
                return Ok(Vec::new());
            };
            let mut st = c
                .prepare_cached(
                    "SELECT m.conversation_id, m.ts, m.plain, bm25(messages_fts) AS r \
                     FROM messages_fts \
                     JOIN messages_fts_map f ON f.rowid = messages_fts.rowid \
                     JOIN messages m ON m.conversation_id = f.conversation_id AND m.ts = f.ts \
                     WHERE messages_fts MATCH ?1 ORDER BY r, m.ts_key DESC LIMIT ?2",
                )
                .map_err(db)?;
            let mut rows = st.query(params![q, limit as i64]).map_err(db)?;
            let mut out = Vec::new();
            while let Some(r) = rows.next().map_err(db)? {
                let plain: String = r.get(2).map_err(db)?;
                out.push(SearchHit {
                    id: MessageId {
                        conversation: ConversationId(Arc::from(r.get::<_, String>(0).map_err(db)?)),
                        ts: mts(r.get(1).map_err(db)?),
                    },
                    snippet: Arc::from(crate::search::snippet(&plain, &user)),
                    rank: -r.get::<_, f64>(3).map_err(db)?,
                });
            }
            Ok(out)
        })
    }

    fn local_schedules(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<domain::LocalSchedule>, StoreError>> {
        self.read(ws, |c, _| crate::schedules::load_schedules(c))
    }

    fn messages_after(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        after: &MessageTs,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<Message>, StoreError>> {
        let (conv, key) = (c.0.to_string(), ts_key(after));
        let sql = format!(
            "SELECT {MSG_COLS} FROM messages WHERE conversation_id = ?1 AND ts_key > ?2 \
             AND NOT (thread_ts IS NOT NULL AND thread_ts <> ts AND broadcast = 0) \
             ORDER BY ts_key ASC, ts ASC LIMIT ?3"
        );
        self.read(ws, move |c, codec| {
            query_messages(
                c,
                codec,
                &sql,
                params![conv, key, i64::try_from(limit).unwrap_or(i64::MAX)],
            )
        })
    }

    fn local_notify_prefs(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<
        '_,
        Result<std::collections::HashMap<ConversationId, domain::ConversationPrefs>, StoreError>,
    > {
        self.read(ws, |c, _| crate::notify_prefs::load_notify_prefs(c))
    }

    fn sidebar_collapsed(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<std::collections::BTreeSet<String>, StoreError>> {
        self.read(ws, |c, _| crate::notify_prefs::load_collapsed(c))
    }

    fn sidebar_collapsed_put(
        &self,
        ws: &WorkspaceKey,
        section: String,
        collapsed: bool,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        self.write(ws, move |c, _| {
            crate::notify_prefs::put_collapsed(c, &section, collapsed)
        })
    }

    fn local_notify_extras(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<
        '_,
        Result<std::collections::HashMap<ConversationId, domain::NotifyExtras>, StoreError>,
    > {
        self.read(ws, |c, _| crate::notify_prefs::load_notify_extras(c))
    }

    fn local_starred(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<std::collections::HashSet<ConversationId>, StoreError>> {
        self.read(ws, |c, _| crate::notify_prefs::load_starred(c))
    }

    fn local_reminders(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<domain::Reminder>, StoreError>> {
        self.read(ws, |c, _| crate::reminders::load_reminders(c))
    }

    fn recent_emoji(
        &self,
        ws: &WorkspaceKey,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<domain::EmojiRef>, StoreError>> {
        self.read(ws, move |c, _| crate::emoji::recent_emoji(c, limit))
    }

    fn emoji_used(
        &self,
        ws: &WorkspaceKey,
        e: domain::EmojiRef,
        at_ms: i64,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        self.write(ws, move |c, _| crate::emoji::emoji_used(c, &e, at_ms))
    }

    fn search_filtered(
        &self,
        ws: &WorkspaceKey,
        q: &domain::SearchQuery,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<SearchHit>, StoreError>> {
        let q = q.clone();
        self.read(ws, move |c, _| crate::search::search_filtered(c, &q, limit))
    }

    fn user_groups(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<domain::UserGroup>, StoreError>> {
        self.read(ws, |c, _| {
            let mut st = c
                .prepare_cached(
                    "SELECT id, handle, name, member_count, is_member FROM user_groups \
                     ORDER BY handle, id LIMIT ?1",
                )
                .map_err(db)?;
            let rows = st
                .query_map([domain::MAX_USER_GROUPS as i64], |r| {
                    Ok(domain::UserGroup {
                        id: r.get::<_, String>(0)?.into(),
                        handle: r.get::<_, String>(1)?.into(),
                        name: r.get::<_, String>(2)?.into(),
                        member_count: r.get(3)?,
                        is_member: r.get(4)?,
                    })
                })
                .map_err(db)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(db)
        })
    }

    fn conversation_files(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        before: Option<MessageTs>,
        limit: usize,
    ) -> BoxFuture<'_, Result<domain::Page<domain::FileHit>, StoreError>> {
        let conv = c.0.to_string();
        let upper = before.as_ref().map_or(i64::MAX, ts_key);
        let upper_ts = before.as_ref().map(ts_text).unwrap_or_default();
        let limit = limit.clamp(1, store::FILES_PAGE_MAX);
        let sql = files_sql();
        self.read(ws, move |c, codec| {
            query_messages(c, codec, &sql, params![conv, upper, limit as i64, upper_ts])
                .map(|v| store::files_page(v, limit))
        })
    }

    #[cfg(feature = "test-support")]
    fn wipe(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<(), StoreError>> {
        self.write(ws, |c, _| {
            c.execute_batch(
                "DELETE FROM messages; DELETE FROM conversations; DELETE FROM members; \
                 DELETE FROM gaps; DELETE FROM outbox; DELETE FROM drafts; \
                 DELETE FROM opened; DELETE FROM local_schedules; DELETE FROM recent_emoji; \
                 DELETE FROM local_notify_prefs; DELETE FROM local_notify_extras; DELETE FROM local_starred; \
                 DELETE FROM local_sidebar_collapsed; DELETE FROM local_reminders; \
                 DELETE FROM user_groups;",
            )
            .map_err(db)
        })
    }
}
