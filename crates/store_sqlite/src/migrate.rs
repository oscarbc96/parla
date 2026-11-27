//! Forward-only migrations tracked in `PRAGMA user_version`.
//! (`rusqlite_migration` 2.7 pins rusqlite 0.41, which cannot coexist with 0.42.)

use rusqlite::Connection;
use store::StoreError;

const WORKSPACE: &[&str] = &[
    include_str!("../migrations/workspace/0001_init.sql"),
    include_str!("../migrations/workspace/0002_fts.sql"),
    include_str!("../migrations/workspace/0003_group_participants.sql"),
    include_str!("../migrations/workspace/0004_m3.sql"),
    include_str!("../migrations/workspace/0005_topic.sql"),
    include_str!("../migrations/workspace/0006_draft_spans.sql"),
    include_str!("../migrations/workspace/0007_notify_prefs.sql"),
    include_str!("../migrations/workspace/0008_local_reminders.sql"),
    include_str!("../migrations/workspace/0009_reminder_fired.sql"),
    include_str!("../migrations/workspace/0010_reminder_interval.sql"),
    include_str!("../migrations/workspace/0011_open_flag_fts_when.sql"),
    include_str!("../migrations/workspace/0012_drop_emoji_uses.sql"),
    include_str!("../migrations/workspace/0013_local_starred.sql"),
    include_str!("../migrations/workspace/0014_draft_edited_sidebar_collapsed.sql"),
    include_str!("../migrations/workspace/0015_reminder_link.sql"),
    include_str!("../migrations/workspace/0016_member_title_tz.sql"),
    include_str!("../migrations/workspace/0017_user_groups.sql"),
    include_str!("../migrations/workspace/0018_messages_files.sql"),
    include_str!("../migrations/workspace/0019_notify_extras.sql"),
    include_str!("../migrations/workspace/0020_purpose.sql"),
    include_str!("../migrations/workspace/0021_member_avatar.sql"),
    include_str!("../migrations/workspace/0022_opened.sql"),
];

const APP: &[&str] = &[
    include_str!("../migrations/app/0001_init.sql"),
    include_str!("../migrations/app/0002_plugins.sql"),
    include_str!("../migrations/app/0003_plugin_files.sql"),
];

pub(crate) fn migrate_workspace(conn: &mut Connection) -> Result<(), StoreError> {
    migrate(conn, WORKSPACE)
}

pub(crate) fn migrate_app(conn: &mut Connection) -> Result<(), StoreError> {
    migrate(conn, APP)
}

fn migrate(conn: &mut Connection, set: &[&str]) -> Result<(), StoreError> {
    let err = |e: rusqlite::Error| StoreError::Migration(e.to_string());
    let current: usize = conn
        .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
        .map_err(err)? as usize;
    if current > set.len() {
        return Err(StoreError::Migration(format!(
            "database version {current} is newer than this build ({})",
            set.len()
        )));
    }
    for (i, sql) in set.iter().enumerate().skip(current) {
        let tx = conn.transaction().map_err(err)?;
        tx.execute_batch(sql).map_err(err)?;
        tx.execute_batch(&format!("PRAGMA user_version = {}", i + 1))
            .map_err(err)?;
        tx.commit().map_err(err)?;
    }
    Ok(())
}
