use std::path::Path;

use settings::*;

fn user(text: &str) -> Layer<'_> {
    Layer {
        path: Path::new("settings.json"),
        text,
    }
}

#[test]
fn defaults_match_spec() {
    let s = parse_layers(&[default_layer()]).unwrap();
    assert_eq!(s.performance.message_window, 200);
    assert_eq!(s.storage.cache_mb, 8);
    assert!(!s.mcp.enabled);
    assert_eq!(s.gifs.rating, "pg-13");
    assert_eq!(s.theme.mode, ThemeMode::System);
    assert_eq!(s.density, Density::Compact);
    assert!(s.calls.ring && s.calls.mute_on_join && !s.calls.camera_on_join);
    assert_eq!(s.calls.ringtone, "parla-chime");
    assert_eq!(s.calls.join_with, JoinWith::Native);
    assert_eq!(s.calls.input_device, None);
    assert_eq!(s.calls.output_device, None);
    assert!(s.updates.check);
    assert_eq!(s.dock_icon, DockIcon::Always);
    assert!(!s.launch_at_login);
    assert!(s.windows.restore);
    assert!(s.notifications.show_preview);
}

#[test]
fn dock_icon_never_and_window_restore_parse() {
    let u = user(r#"{"dock_icon":"never","windows":{"restore":false}}"#);
    let s = parse_layers(&[default_layer(), u]).unwrap();
    assert_eq!(s.dock_icon, DockIcon::Never);
    assert!(!s.windows.restore);
}

#[test]
fn quiet_hours_days_parse_and_default_to_every_day() {
    let u = user(
        r#"{"notifications":{"show_preview":false,"quiet_hours":{"from":"20:00","to":"08:00","days":["mon","sun"]}}}"#,
    );
    let s = parse_layers(&[default_layer(), u]).unwrap();
    assert!(!s.notifications.show_preview);
    let q = s.notifications.quiet_hours.unwrap();
    assert_eq!(q.days, vec![Weekday::Mon, Weekday::Sun]);
    let u = user(r#"{"notifications":{"quiet_hours":{"from":"20:00","to":"08:00"}}}"#);
    let s = parse_layers(&[default_layer(), u]).unwrap();
    assert!(s.notifications.quiet_hours.unwrap().days.is_empty());
}

#[test]
fn schema_pointer_is_a_known_key() {
    assert!(unknown_keys(r#"{"$schema":"./settings.schema.json"}"#).is_empty());
}

#[test]
fn join_with_default_is_native_and_explicit_choices_are_kept() {
    // A settings file is layered over the defaults, so a user who never set
    // `calls.join_with` (the old default was `window`) moves to `native`.
    let none = parse_layers(&[default_layer(), user(r#"{"calls":{"ring":false}}"#)]).unwrap();
    assert_eq!(none.calls.join_with, JoinWith::Native);
    for (text, want) in [
        ("window", JoinWith::Window),
        ("browser", JoinWith::Browser),
        ("provider_app", JoinWith::ProviderApp),
        ("native", JoinWith::Native),
    ] {
        let body = format!(r#"{{"calls":{{"join_with":"{text}"}}}}"#);
        let s = parse_layers(&[default_layer(), user(&body)]).unwrap();
        assert_eq!(s.calls.join_with, want);
    }
}

#[test]
fn dock_icon_parses_with_window() {
    let u = user(r#"{"dock_icon":"with_window"}"#);
    let s = parse_layers(&[default_layer(), u]).unwrap();
    assert_eq!(s.dock_icon, DockIcon::WithWindow);
}

#[test]
fn default_json_equals_rust_defaults() {
    assert_eq!(
        parse_layers(&[default_layer()]).unwrap(),
        Settings::default()
    );
}

#[test]
fn comments_and_trailing_commas_allowed() {
    let u = user("{ // c\n \"ui_font_size\": 16, }");
    let s = parse_layers(&[default_layer(), u]).unwrap();
    assert_eq!(s.ui_font_size, 16.0);
}

#[test]
fn objects_merge_arrays_replace() {
    let u = user(r#"{"notifications":{"keywords":["b"]}}"#);
    let s = parse_layers(&[default_layer(), u]).unwrap();
    assert_eq!(s.notifications.keywords, vec!["b".to_string()]);
    assert_eq!(s.notifications.desktop, DesktopNotifications::Mentions);
    let a = user(r#"{"notifications":{"keywords":["a"]}}"#);
    let b = user(r#"{"notifications":{"keywords":["b"]}}"#);
    let s = parse_layers(&[default_layer(), a, b]).unwrap();
    assert_eq!(s.notifications.keywords, vec!["b".to_string()]);
}

#[test]
fn workspace_layer_overrides_user() {
    let base = [default_layer(), user(r#"{"density":"cozy"}"#)];
    let ws = Layer {
        path: Path::new("workspaces/fake/W1.json"),
        text: r#"{"density":"compact"}"#,
    };
    let w1 = WorkspaceKey::new("fake", "W1");
    let s = effective_for(Some(&w1), &base, Some(ws)).unwrap();
    assert_eq!(s.density, Density::Compact);
    let s = effective_for(None, &base, Some(ws)).unwrap();
    assert_eq!(s.density, Density::Cozy);
}

#[test]
fn parse_error_has_line_and_col() {
    let err = parse_layers(&[default_layer(), user("{\n  \"a\": }")]).unwrap_err();
    match &err {
        SettingsError::Parse {
            path, line, col, ..
        } => {
            assert_eq!(path, Path::new("settings.json"));
            assert_eq!((*line, *col), (2, 8));
        }
        other => panic!("expected Parse, got {other:?}"),
    }
    assert!(err.to_string().starts_with("settings.json:2:8: "));
}

#[test]
fn type_error_has_pointer() {
    let err = parse_layers(&[default_layer(), user(r#"{"ui_font_size":"big"}"#)]).unwrap_err();
    match err {
        SettingsError::Type { path, pointer, .. } => {
            assert_eq!(path, Path::new("settings.json"));
            assert_eq!(pointer, "/ui_font_size");
        }
        other => panic!("expected Type, got {other:?}"),
    }
}

#[test]
fn unknown_keys_are_warnings() {
    let text = r#"{"nope":1,"gifs":{"x":2},"plugins":{"any":{"free":1}},"mcp":{"workspaces":{"fake/W1":{"bad":true}}}}"#;
    parse_layers(&[default_layer(), user(text)]).unwrap();
    assert_eq!(
        unknown_keys(text),
        vec![
            "/gifs/x",
            "/mcp/workspaces/fake~1W1/bad",
            "/nope",
            "/plugins/any/free"
        ]
    );
    assert!(unknown_keys("{").is_empty());
}

#[test]
fn call_devices_are_names_or_null_for_the_system_default() {
    let s = parse_layers(&[
        default_layer(),
        user(r#"{ "calls": { "input_device": "USB Mic", "output_device": null } }"#),
    ])
    .unwrap();
    assert_eq!(s.calls.input_device.as_deref(), Some("USB Mic"));
    assert_eq!(s.calls.output_device, None);
}

#[test]
fn ui_font_size_is_clamped_to_its_bounds() {
    for (given, want) in [(2.0, 10.0), (14.0, 14.0), (80.0, 24.0)] {
        let text = format!("{{\"ui_font_size\": {given}}}");
        let u = user(&text);
        let s = parse_layers(&[default_layer(), u]).unwrap();
        assert_eq!(s.ui_font_size, want);
    }
}

#[test]
fn schema_bounds_ui_font_size() {
    let schema = settings::settings_schema();
    let p = &schema["properties"]["ui_font_size"];
    assert_eq!(p["minimum"], 10.0);
    assert_eq!(p["maximum"], 24.0);
}

#[test]
fn log_level_defaults_to_info_and_reads_the_file() {
    let s = parse_layers(&[default_layer()]).unwrap();
    assert_eq!(s.log.level, LogLevel::Info);
    let user = Layer {
        path: std::path::Path::new("settings.json"),
        text: r#"{"log":{"level":"debug"}}"#,
    };
    assert_eq!(
        parse_layers(&[default_layer(), user]).unwrap().log.level,
        LogLevel::Debug
    );
}

#[test]
fn idle_lock_is_off_and_crash_reports_ask_by_default() {
    let s = parse_layers(&[default_layer()]).unwrap();
    assert_eq!(s.storage.lock_after_minutes, 0, "off by default");
    assert_eq!(s.crash_reports, CrashReports::Ask);
    let user = Layer {
        path: std::path::Path::new("settings.json"),
        text: r#"{"storage":{"lock_after_minutes":15},"crash_reports":"local"}"#,
    };
    let s = parse_layers(&[default_layer(), user]).unwrap();
    assert_eq!(s.storage.lock_after_minutes, 15);
    assert_eq!(s.crash_reports, CrashReports::Local);
    let off = Layer {
        path: std::path::Path::new("settings.json"),
        text: r#"{"crash_reports":"off"}"#,
    };
    assert_eq!(
        parse_layers(&[default_layer(), off]).unwrap().crash_reports,
        CrashReports::Off
    );
}

#[test]
fn sidebar_sections_parse_from_a_workspace_layer() {
    let base = [default_layer()];
    let ws = Layer {
        path: Path::new("workspaces/fake/w1.json"),
        text: r#"{ "sidebar": { "sections": [
            { "id": 2, "name": "Focus", "conversations": ["C1", "D1"] },
            { "id": 5, "name": "Clients" }
        ] } }"#,
    };
    let key = WorkspaceKey::new("fake", "w1");
    let s = effective_for(Some(&key), &base, Some(ws)).unwrap();
    assert_eq!(
        s.sidebar.sections,
        vec![
            CustomSection {
                id: 2,
                name: "Focus".into(),
                conversations: vec!["C1".into(), "D1".into()],
            },
            CustomSection {
                id: 5,
                name: "Clients".into(),
                conversations: vec![],
            },
        ]
    );
    // The user-level options stay as they were.
    assert!(s.sidebar.show_unreads_section);
    assert!(parse_layers(&base).unwrap().sidebar.sections.is_empty());
}

#[test]
fn layer_value_reads_one_file_only() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("w1.json");
    assert_eq!(layer_value(&f, &["sidebar", "sort"]), None);
    std::fs::write(&f, "{ // note\n \"sidebar\": { \"sort\": \"recent\", } }").unwrap();
    assert_eq!(
        layer_value(&f, &["sidebar", "sort"]),
        Some(serde_json::json!("recent"))
    );
    assert_eq!(layer_value(&f, &["sidebar", "sections"]), None);
    std::fs::write(&f, "not json").unwrap();
    assert_eq!(layer_value(&f, &["sidebar"]), None);
}
