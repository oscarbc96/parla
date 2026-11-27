use settings::update_value;

#[test]
fn replaces_existing_value_and_keeps_comments() {
    let text = "{\n  // keep me\n  \"theme\": { \"mode\": \"light\", /* c */ \"dark\": \"Ember\" }, // tail\n}\n";
    let out = update_value(text, &["theme", "mode"], "\"dark\"").unwrap();
    assert_eq!(
        out,
        "{\n  // keep me\n  \"theme\": { \"mode\": \"dark\", /* c */ \"dark\": \"Ember\" }, // tail\n}\n"
    );
}

#[test]
fn inserts_into_commented_template() {
    let text = "// head\n{\n  // \"theme\": { \"mode\": \"system\" },\n}\n";
    let out = update_value(text, &["theme", "mode"], "\"dark\"").unwrap();
    assert!(out.starts_with("// head\n{\n  \"theme\": {"));
    assert!(out.contains("// \"theme\": { \"mode\": \"system\" },"));
    let v: serde_json::Value = serde_json_lenient::from_str(&out).unwrap();
    assert_eq!(v["theme"]["mode"], "dark");
}

#[test]
fn inserts_member_into_existing_object_with_members() {
    let text = "{ \"theme\": { \"dark\": \"Ember\" } }";
    let out = update_value(text, &["theme", "mode"], "\"dark\"").unwrap();
    let v: serde_json::Value = serde_json_lenient::from_str(&out).unwrap();
    assert_eq!(v["theme"]["mode"], "dark");
    assert_eq!(v["theme"]["dark"], "Ember");
}

#[test]
fn empty_text_and_invalid_text() {
    let out = update_value("", &["theme", "mode"], "\"dark\"").unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["theme"]["mode"], "dark");
    assert_eq!(update_value("{", &["a"], "1"), None);
    assert_eq!(update_value("[1]", &["a"], "1"), None);
}

#[cfg(unix)]
#[test]
fn unreadable_settings_file_is_rejected_not_defaulted() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let paths = settings::ConfigPaths::at(dir.path());
    std::fs::write(&paths.settings, "{\"ui_font_size\": 20}").unwrap();
    let mut store = settings::SettingsStore::load(paths.clone());
    let before = store.current().clone();
    std::fs::set_permissions(&paths.settings, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read_to_string(&paths.settings).is_ok() {
        return; // running as root
    }
    assert!(matches!(
        store.reload(),
        settings::ReloadOutcome::Rejected(_)
    ));
    assert_eq!(store.current(), &before);
}

#[test]
fn reset_file_writes_the_commented_template() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.json");
    std::fs::write(&file, "{ \"density\": \"cozy\" }").unwrap();
    settings::reset_file(&file).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("//"), "{text}");
    assert!(!text.contains("\"density\": \"cozy\""), "{text}");
    let v: serde_json::Value = serde_json_lenient::from_str(&text).unwrap();
    assert!(v.is_object());
}

#[test]
fn removes_a_key_and_keeps_comments_and_other_keys() {
    let text = "{\n  // keep me\n  \"mcp\": {\n    \"enabled\": true, // on\n    // old\n    \"transport\": \"http\",\n    \"x\": 1\n  }\n}\n";
    let out = settings::remove_key(text, &["mcp", "transport"]).unwrap();
    assert_eq!(
        out,
        "{\n  // keep me\n  \"mcp\": {\n    \"enabled\": true, // on\n    // old\n    \"x\": 1\n  }\n}\n"
    );
}

#[test]
fn removes_a_last_key_without_leaving_a_comma_problem() {
    let text = "{ \"mcp\": { \"enabled\": true, \"transport\": \"http\" } }";
    let out = settings::remove_key(text, &["mcp", "transport"]).unwrap();
    let v: serde_json::Value = serde_json_lenient::from_str(&out).unwrap();
    assert_eq!(v["mcp"]["enabled"], true);
    assert!(v["mcp"].get("transport").is_none());
}

#[test]
fn removing_a_missing_key_changes_nothing() {
    let text = "{ \"mcp\": { \"enabled\": true } }";
    assert_eq!(
        settings::remove_key(text, &["mcp", "transport"]).as_deref(),
        Some(text)
    );
    assert_eq!(settings::remove_key("[1]", &["mcp", "transport"]), None);
}

#[test]
fn removes_the_same_line_trailing_comment_with_the_member() {
    let text =
        "{\n  \"mcp\": {\n    \"transport\": \"http\", // old\n    \"enabled\": true\n  }\n}\n";
    let out = settings::remove_key(text, &["mcp", "transport"]).unwrap();
    assert_eq!(out, "{\n  \"mcp\": {\n    \"enabled\": true\n  }\n}\n");
}

#[test]
fn removes_every_duplicate_in_one_pass() {
    let text = "{ \"mcp\": { \"transport\": \"a\", \"enabled\": true, \"transport\": \"b\" } }";
    let out = settings::remove_key(text, &["mcp", "transport"]).unwrap();
    let v: serde_json::Value = serde_json_lenient::from_str(&out).unwrap();
    assert_eq!(v["mcp"], serde_json::json!({ "enabled": true }));
    assert!(!out.contains("transport"), "{out}");
}

#[test]
fn removes_with_a_trailing_jsonc_comma() {
    let text = "{\n  \"mcp\": {\n    \"enabled\": true,\n    \"transport\": \"http\",\n  },\n}\n";
    let out = settings::remove_key(text, &["mcp", "transport"]).unwrap();
    let v: serde_json::Value = serde_json_lenient::from_str(&out).unwrap();
    assert_eq!(v["mcp"], serde_json::json!({ "enabled": true }));
}

#[test]
fn removes_the_only_member_of_mcp() {
    let text =
        "{\n  \"mcp\": {\n    \"transport\": \"http\"\n  },\n  \"density\": \"compact\"\n}\n";
    let out = settings::remove_key(text, &["mcp", "transport"]).unwrap();
    let v: serde_json::Value = serde_json_lenient::from_str(&out).unwrap();
    assert_eq!(v["mcp"], serde_json::json!({}));
    assert_eq!(v["density"], "compact");
}

#[cfg(unix)]
#[test]
fn an_atomic_rewrite_keeps_the_file_permissions() {
    use std::os::unix::fs::PermissionsExt as _;
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("settings.json");
    std::fs::write(&f, r#"{ "mcp": { "transport": "http" } }"#).unwrap();
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(settings::remove_key_in_file(&f, &["mcp", "transport"]).unwrap());
    let mode = std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert!(!std::fs::read_to_string(&f).unwrap().contains("transport"));
}
