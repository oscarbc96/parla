use settings::WorkspaceKey;
use slack_auth::*;
use url::Url;

fn account(id: &str, name: &str) -> Account {
    Account {
        workspace: WorkspaceKey::new("slack", id),
        name: name.to_string(),
        url: Url::parse("https://example.invalid/").unwrap(),
        user_id: "U0000001".to_string(),
        enterprise_id: None,
        enterprise_name: None,
        icon_url: None,
    }
}

#[test]
fn account_index_has_no_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.json");
    let mut idx = AccountIndex::load(&path).unwrap();
    idx.upsert(account("T0000001", "One"));
    idx.save().unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("xox"));
    assert!(text.contains("T0000001"));
}

#[test]
fn upsert_replaces_and_remove_deletes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.json");
    let mut idx = AccountIndex::load(&path).unwrap();
    idx.upsert(account("T0000001", "One"));
    idx.upsert(account("T0000002", "Two"));
    idx.upsert(account("T0000001", "Renamed"));
    idx.save().unwrap();
    let mut idx = AccountIndex::load(&path).unwrap();
    assert_eq!(idx.accounts().len(), 2);
    assert_eq!(idx.accounts()[0].name, "Renamed");
    idx.remove(&WorkspaceKey::new("slack", "T0000001"));
    assert_eq!(idx.accounts().len(), 1);
}

#[test]
fn missing_file_is_empty_and_corrupt_is_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub/accounts.json");
    assert!(AccountIndex::load(&path).unwrap().accounts().is_empty());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{nope").unwrap();
    assert!(AccountIndex::load(&path).is_err());
}

#[cfg(unix)]
#[test]
fn account_index_mode_0600() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub/accounts.json");
    let mut idx = AccountIndex::load(&path).unwrap();
    idx.upsert(account("T0000001", "One"));
    idx.save().unwrap();
    idx.save().unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}
