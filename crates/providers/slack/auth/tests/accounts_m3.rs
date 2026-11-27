use settings::WorkspaceKey;
use slack_auth::*;
use url::Url;

fn full(id: &str) -> Account {
    Account {
        workspace: WorkspaceKey::new("slack", id),
        name: "One".to_string(),
        url: Url::parse("https://example.invalid/").unwrap(),
        user_id: "U0000001".to_string(),
        enterprise_id: Some("E0000001".to_string()),
        enterprise_name: Some("Example Org".to_string()),
        icon_url: Some("https://example.com/i.png".to_string()),
    }
}

#[test]
fn index_round_trips_new_fields_and_reads_m1_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.json");
    let mut idx = AccountIndex::load(&path).unwrap();
    idx.upsert(full("T0000001"));
    idx.save().unwrap();
    let idx = AccountIndex::load(&path).unwrap();
    assert_eq!(idx.accounts()[0], full("T0000001"));

    // A file written before these fields existed still loads.
    std::fs::write(
        &path,
        r#"{"v":1,"accounts":[{"workspace":"slack/T0000009","name":"Old","url":"https://example.invalid/","user_id":"U1"}]}"#,
    )
    .unwrap();
    let idx = AccountIndex::load(&path).unwrap();
    let a = &idx.accounts()[0];
    assert_eq!(
        (a.enterprise_name.as_ref(), a.icon_url.as_ref()),
        (None, None)
    );
}

#[test]
fn concurrent_updates_do_not_lose_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.json");
    let threads: Vec<_> = (0..8)
        .map(|i| {
            let path = path.clone();
            std::thread::spawn(move || {
                for j in 0..5 {
                    let acc = full(&format!("T{i}{j}"));
                    AccountIndex::update(&path, |idx| idx.upsert(acc)).unwrap();
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(AccountIndex::load(&path).unwrap().accounts().len(), 40);
}

#[test]
fn update_does_not_save_when_closure_returns_false_for_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.json");
    AccountIndex::update(&path, |idx| idx.upsert(full("T1"))).unwrap();
    AccountIndex::update(&path, |idx| idx.remove(&WorkspaceKey::new("slack", "T1"))).unwrap();
    assert!(AccountIndex::load(&path).unwrap().accounts().is_empty());
}
