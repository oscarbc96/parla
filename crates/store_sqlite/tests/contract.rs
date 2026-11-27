//! Every `store::contract` case against `SqliteStore`.

use std::sync::Arc;

use domain::NoExtCodec;
use settings::WorkspaceKey;
use store::contract;
use store_sqlite::{MemoryKeySource, SqliteConfig, SqliteStore};

/// The temp dir lives as long as the store.
struct Fixture {
    store: SqliteStore,
    _dir: tempfile::TempDir,
}

impl std::ops::Deref for Fixture {
    type Target = SqliteStore;
    fn deref(&self) -> &SqliteStore {
        &self.store
    }
}

async fn sqlite() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::start(
        SqliteConfig {
            dir: dir.path().to_path_buf(),
            cache_mb: 8,
        },
        Arc::new(MemoryKeySource::default()),
    );
    // The contract uses workspaces "a" and "b".
    for id in ["a", "b"] {
        store
            .open(&WorkspaceKey::new("fake", id), Arc::new(NoExtCodec))
            .await
            .unwrap();
    }
    Fixture { store, _dir: dir }
}

macro_rules! case {
    ($($name:ident),* $(,)?) => {$(
        #[tokio::test]
        async fn $name() {
            let s = sqlite().await;
            contract::$name(&*s).await;
        }
    )*};
}

case!(
    channel_window_hides_unbroadcast_replies,
    thread_returns_root_then_replies_ascending,
    replace_message_updates_in_place,
    reactions_replace_whole_set,
    thread_summary_updates_root,
    gaps_add_and_close_split,
    latest_all_lists_each_conversation,
    latest_all_ignores_unbroadcast_replies,
    outbox_round_trip_ordered,
    drafts_put_and_blank_deletes,
    drafts_keep_the_latest_edit_time,
    sidebar_collapsed_round_trip_and_expand_deletes,
    search_finds_plain_text,
    edit_updates_search_text,
    message_lookup_by_ts,
    equivalent_timestamps_are_one_message,
    wipe_removes_one_workspace_only,
    local_schedules_round_trip_ordered,
    local_notify_prefs_round_trip_and_default_deletes,
    local_notify_extras_round_trip_and_default_deletes,
    local_starred_round_trip_and_unstar_deletes,
    local_reminders_round_trip_and_wipe,
    messages_after_ascending_excludes_replies,
    recent_emoji_most_recent_first_capped,
    search_filtered_by_author_conversation_and_dates,
    search_filtered_empty_text_lists_newest,
    search_filtered_has_file,
    wipe_clears_local_schedules_and_recent_emoji,
    user_groups_replace_whole_set_per_workspace_and_wipe,
    conversation_files_newest_first,
    conversation_files_pages_are_bounded,
    conversation_files_ties_never_skip,
);
