use futures::executor::block_on;
use store::{AuditOutcome, MemoryAppStore, WriteDecision, app_contract};

macro_rules! case {
    ($($name:ident),* $(,)?) => {$(
        #[test]
        fn $name() {
            block_on(app_contract::$name(&MemoryAppStore::default()));
        }
    )*};
}

case!(
    append_then_page_newest_first,
    page_filters_workspace_tool_and_writes,
    page_continues_before_id,
    prune_drops_only_older,
    forget_workspace_drops_only_its_rows,
    clear_empties,
    plugin_row_round_trips,
    writes_need_a_plugin_row,
    grants_replace_whole_set,
    write_allow_add_and_revoke,
    kv_scopes_keys_and_clear,
    kv_quota_5mb_2000_keys,
    kv_value_max_256kb,
    cwasm_hash_round_trips,
    delete_plugin_cascades,
    kv_keys_prefix_is_case_sensitive,
    kv_empty_key_refused,
    files_digest_survives_set_grants,
);

#[test]
fn outcome_codes_are_stable() {
    use AuditOutcome as O;
    let all = [
        O::Ok,
        O::Denied,
        O::NotAvailable,
        O::RateLimited,
        O::Declined,
        O::TimedOut,
        O::Failed,
    ];
    for (i, o) in all.into_iter().enumerate() {
        assert_eq!(o.code(), i as i64);
        assert_eq!(AuditOutcome::from_code(i as i64), Some(o));
    }
    assert_eq!(AuditOutcome::from_code(7), None);
    use WriteDecision as D;
    for (i, d) in [
        D::Confirmed,
        D::Auto,
        D::Declined,
        D::TimedOut,
        D::SessionAllowed,
        D::ConfirmedForSession,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(d.code(), i as i64);
        assert_eq!(WriteDecision::from_code(i as i64), Some(d));
    }
}
