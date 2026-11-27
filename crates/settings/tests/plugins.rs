use std::path::Path;

use settings::{
    Layer, PluginEntry, PluginsSettings, SecretSettingRejected, WorkspaceKey, default_layer,
    parse_layers, settings_schema, unknown_keys, update_plugin_config,
};

fn user(text: &str) -> Layer<'_> {
    Layer {
        path: Path::new("settings.json"),
        text,
    }
}

const DESIGN_EXAMPLE: &str = r#"{
  "plugins": {
    "auto_update": false,
    "dev_paths": ["~/code/release-dashboard"],
    "reminders-panel": {
      "enabled": true,
      "workspaces": ["fake/W1"], // optional
      "config": { "max_lines": 20 }
    }
  }
}"#;

#[test]
fn design_example_round_trips() {
    let s = parse_layers(&[default_layer(), user(DESIGN_EXAMPLE)])
        .unwrap()
        .plugins;
    assert!(!s.auto_update);
    assert_eq!(s.dev_paths, vec!["~/code/release-dashboard".to_string()]);
    let entry = s.entry("reminders-panel").expect("entry");
    assert!(entry.enabled);
    assert_eq!(
        entry.workspaces.as_deref(),
        Some(&["fake/W1".to_string()][..])
    );
    assert_eq!(entry.config.get("max_lines"), Some(&serde_json::json!(20)));

    // The JSON shape stays the same: entries sit beside the two keys.
    let json = serde_json::to_value(&s).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "auto_update": false,
            "dev_paths": ["~/code/release-dashboard"],
            "reminders-panel": {
                "enabled": true,
                "workspaces": ["fake/W1"],
                "config": { "max_lines": 20 }
            }
        })
    );
    let back: PluginsSettings = serde_json::from_value(json).unwrap();
    assert_eq!(back, s);
    assert!(unknown_keys(DESIGN_EXAMPLE).is_empty());
}

#[test]
fn defaults_are_empty_and_off() {
    let s = parse_layers(&[default_layer()]).unwrap().plugins;
    assert_eq!(s, PluginsSettings::default());
    assert!(!s.auto_update);
    assert!(s.dev_paths.is_empty());
    assert!(s.entries.is_empty());
}

#[test]
fn unknown_plugin_entry_kept() {
    // An entry for a plugin that is not installed loads and survives a write.
    let text = r#"{"plugins":{"not-installed":{"enabled":false,"config":{"a":[1,2]}}}}"#;
    let s = parse_layers(&[default_layer(), user(text)])
        .unwrap()
        .plugins;
    let entry = s.entry("not-installed").expect("kept");
    assert!(!entry.enabled);
    let json = serde_json::to_value(&s).unwrap();
    assert_eq!(
        json["not-installed"],
        serde_json::json!({"enabled": false, "config": {"a": [1, 2]}})
    );
    // Today's untyped files keep loading: an entry with only some keys.
    let s = parse_layers(&[default_layer(), user(r#"{"plugins":{"any":{}}}"#)])
        .unwrap()
        .plugins;
    assert_eq!(s.entry("any"), Some(&PluginEntry::default()));
}

#[test]
fn workspaces_absent_means_all() {
    let text =
        r#"{"plugins":{"all":{},"some":{"workspaces":["fake/W1"]},"none":{"workspaces":[]}}}"#;
    let s = parse_layers(&[default_layer(), user(text)])
        .unwrap()
        .plugins;
    let w1 = WorkspaceKey::new("fake", "W1");
    let w2 = WorkspaceKey::new("fake", "W2");
    let all = s.entry("all").unwrap();
    assert!(all.workspaces.is_none());
    assert!(all.applies_to(&w1) && all.applies_to(&w2));
    let some = s.entry("some").unwrap();
    assert!(some.applies_to(&w1) && !some.applies_to(&w2));
    let none = s.entry("none").unwrap();
    assert!(!none.applies_to(&w1));
    // No entry at all: every workspace, enabled.
    assert!(PluginEntry::default().enabled);
    assert!(PluginEntry::default().applies_to(&w2));
}

#[test]
fn secret_settings_never_serialised() {
    let mut entry = PluginEntry::default();
    entry
        .set_config("max_lines", serde_json::json!(30), false)
        .unwrap();
    let err = entry
        .set_config("api_key", serde_json::json!("sk-123"), true)
        .unwrap_err();
    assert_eq!(
        err,
        SecretSettingRejected {
            key: "api_key".into()
        }
    );
    assert!(!entry.config.contains_key("api_key"));
    assert!(!serde_json::to_string(&entry).unwrap().contains("sk-123"));

    // File edits: a secret key is refused, other keys are written in place.
    let text = "{\n  // mine\n  \"plugins\": {}\n}\n";
    assert_eq!(
        update_plugin_config(text, "gif-x", "api_key", "\"sk-123\"", true),
        Err(SecretSettingRejected {
            key: "api_key".into()
        })
    );
    let out = update_plugin_config(text, "gif-x", "max_lines", "30", false)
        .unwrap()
        .unwrap();
    assert!(out.contains("// mine"));
    let s = parse_layers(&[default_layer(), user(&out)])
        .unwrap()
        .plugins;
    assert_eq!(
        s.entry("gif-x").unwrap().config.get("max_lines"),
        Some(&serde_json::json!(30))
    );
}

#[test]
fn schema_has_plugins_section() {
    let schema = settings_schema();
    let plugins = &schema["properties"]["plugins"];
    assert!(plugins.is_object(), "plugins in schema");
    let def = plugins["$ref"]
        .as_str()
        .and_then(|r| r.strip_prefix("#"))
        .and_then(|p| schema.pointer(p))
        .unwrap_or(plugins);
    assert_eq!(def["properties"]["auto_update"]["type"], "boolean");
    assert_eq!(def["properties"]["dev_paths"]["type"], "array");
    let entry = def["additionalProperties"]["$ref"]
        .as_str()
        .and_then(|r| r.strip_prefix("#"))
        .and_then(|p| schema.pointer(p))
        .expect("entries typed");
    for key in ["enabled", "workspaces", "config"] {
        assert!(entry["properties"][key].is_object(), "{key}");
    }
    // A typo inside an entry is a warning, an unknown plugin id is not.
    assert_eq!(
        unknown_keys(r#"{"plugins":{"x-y":{"enabeld":true}}}"#),
        vec!["/plugins/x-y/enabeld"]
    );
}

#[test]
fn bad_entry_is_a_type_error_in_the_plugins_section() {
    let text = r#"{"plugins":{"x-y":{"enabled":"yes"}}}"#;
    match parse_layers(&[default_layer(), user(text)]).unwrap_err() {
        settings::SettingsError::Type { pointer, .. } => {
            assert!(pointer.starts_with("/plugins"), "{pointer}")
        }
        other => panic!("expected Type, got {other:?}"),
    }
}
