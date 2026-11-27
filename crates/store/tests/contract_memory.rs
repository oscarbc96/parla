use futures::executor::block_on;
use store::{MemoryStore, contract};

macro_rules! case {
    ($($name:ident),* $(,)?) => {$(
        #[test]
        fn $name() {
            block_on(contract::$name(&MemoryStore::default()));
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
