//! Retention pruning and incremental vacuum.

use settings::WorkspaceKey;
use store::StoreError;

use crate::thread::SqliteStore;

const DAY_MS: i64 = 86_400_000;
/// Conversations opened within this window are never pruned.
const RECENT_OPEN_DAYS: i64 = 30;

impl SqliteStore {
    /// Deletes messages older than `retention_days` in conversations not
    /// opened in the last 30 days (or never), then frees the pages with
    /// `PRAGMA incremental_vacuum`. Outbox and drafts are untouched. Returns
    /// the number of messages deleted. `retention_days` is clamped to at least
    /// one day, so `0` cannot delete everything older than now.
    pub async fn prune(
        &self,
        ws: &WorkspaceKey,
        now_ms: i64,
        retention_days: u32,
    ) -> Result<u64, StoreError> {
        let db = crate::err::other;
        let cutoff_ts_key = now_ms
            .saturating_sub(i64::from(retention_days.max(1)).saturating_mul(DAY_MS))
            .saturating_mul(1000);
        let recent_open = now_ms.saturating_sub(RECENT_OPEN_DAYS * DAY_MS);
        let n = self
            .write(ws, move |c, _| {
                // A conversation opened since `recent_open` keeps its old
                // messages; the marker lives in `opened` as well, because the
                // open can land before its `conversations` row does.
                let n = c
                    .execute(
                        "DELETE FROM messages WHERE ts_key < ?1 \
                         AND NOT EXISTS \
                      (SELECT 1 FROM conversations c WHERE c.id = messages.conversation_id \
                       AND c.last_opened_ms >= ?2) \
                         AND NOT EXISTS \
                      (SELECT 1 FROM opened o WHERE o.conversation_id = messages.conversation_id \
                       AND o.at_ms >= ?2)",
                        rusqlite::params![cutoff_ts_key, recent_open],
                    )
                    .map_err(db)?;
                // Older than the window that can keep a message, so it cannot
                // matter again: dropping it keeps the table as small as the
                // conversations opened in the last month.
                c.execute(
                    "DELETE FROM opened WHERE at_ms < ?1",
                    rusqlite::params![recent_open],
                )
                .map_err(db)?;
                Ok(n as u64)
            })
            .await?;
        // A separate job: inside the delete's transaction it leaves pages on the freelist.
        self.write(ws, move |c, _| {
            let mut st = c.prepare("PRAGMA incremental_vacuum").map_err(db)?;
            let mut rows = st.query([]).map_err(db)?;
            while rows.next().map_err(db)?.is_some() {}
            Ok(())
        })
        .await?;
        Ok(n)
    }
}
