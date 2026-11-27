//! Recently used emoji.

use std::sync::Arc;

use domain::EmojiRef;
use rusqlite::{Connection, params};
use store::StoreError;

use crate::err::other as db;

pub(crate) fn recent_emoji(c: &Connection, limit: usize) -> Result<Vec<EmojiRef>, StoreError> {
    let mut st = c
        .prepare_cached(
            "SELECT shortcode, unicode, skin_tone FROM recent_emoji \
             ORDER BY used_ms DESC, shortcode LIMIT ?1",
        )
        .map_err(db)?;
    let mut rows = st
        .query([i64::try_from(limit).unwrap_or(i64::MAX)])
        .map_err(db)?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().map_err(db)? {
        out.push(EmojiRef {
            shortcode: Arc::from(r.get::<_, String>(0).map_err(db)?),
            unicode: r.get::<_, Option<String>>(1).map_err(db)?.map(Arc::from),
            skin_tone: r
                .get::<_, Option<i64>>(2)
                .map_err(db)?
                .and_then(|t| u8::try_from(t).ok()),
        });
    }
    Ok(out)
}

pub(crate) fn emoji_used(c: &Connection, e: &EmojiRef, at_ms: i64) -> Result<(), StoreError> {
    c.execute(
        "INSERT INTO recent_emoji (shortcode, unicode, skin_tone, used_ms) \
         VALUES (?1,?2,?3,?4) \
         ON CONFLICT (shortcode) DO UPDATE SET unicode = excluded.unicode, \
         skin_tone = excluded.skin_tone, used_ms = excluded.used_ms",
        params![
            &*e.shortcode,
            e.unicode.as_deref(),
            e.skin_tone.map(i64::from),
            at_ms
        ],
    )
    .map_err(db)?;
    c.execute(
        "DELETE FROM recent_emoji WHERE shortcode NOT IN \
         (SELECT shortcode FROM recent_emoji ORDER BY used_ms DESC, shortcode LIMIT ?1)",
        [store::RECENT_EMOJI_CAP as i64],
    )
    .map_err(db)?;
    Ok(())
}
