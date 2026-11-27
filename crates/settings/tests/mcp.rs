use std::path::Path;

use settings::{
    Layer, McpSettings, SettingsError, WorkspaceKey, default_layer, parse_layers, paths,
};

fn user(text: &str) -> Layer<'_> {
    Layer {
        path: Path::new("settings.json"),
        text,
    }
}

#[test]
fn mcp_defaults_are_off() {
    let s = parse_layers(&[default_layer()]).unwrap().mcp;
    assert!(!s.enabled);
    assert_eq!(s.audit_retention_days, 90);
    let w = s.workspace(&WorkspaceKey::new("fake", "W1"));
    assert!(!w.enabled);
    assert!(!w.allow_writes);
    assert!(w.confirm_writes);
    assert!(!w.include_dms);
    assert!(w.allow_channels.is_empty() && w.deny_channels.is_empty());
    assert_eq!(s, McpSettings::default());
}

#[test]
fn workspace_lookup_uses_provider_slash_id() {
    let text =
        r#"{"mcp":{"enabled":true,"workspaces":{"fake/W1":{"enabled":true,"allow_writes":true}}}}"#;
    let s = parse_layers(&[default_layer(), user(text)]).unwrap().mcp;
    assert!(s.workspace(&WorkspaceKey::new("fake", "W1")).enabled);
    assert!(s.workspace(&WorkspaceKey::new("fake", "W1")).allow_writes);
    assert!(!s.workspace(&WorkspaceKey::new("fake", "W2")).enabled);
    assert!(!s.workspace(&WorkspaceKey::new("other", "W1")).enabled);
}

#[test]
fn bad_workspace_key_is_reported_not_fatal() {
    let text = r#"{"mcp":{"workspaces":{"T0123":{"enabled":true}}}}"#;
    match parse_layers(&[default_layer(), user(text)]).unwrap_err() {
        SettingsError::Type { path, pointer, .. } => {
            assert_eq!(path, Path::new("settings.json"));
            assert_eq!(pointer, "/mcp/workspaces/T0123");
        }
        other => panic!("expected Type, got {other:?}"),
    }
    // A map built in code with a bad key never matches a real workspace.
    let mut s = McpSettings::default();
    s.workspaces.insert("T0123".into(), Default::default());
    assert!(!s.workspace(&WorkspaceKey::new("fake", "T0123")).enabled);
}

#[test]
fn default_settings_name_no_provider() {
    for text in [
        include_str!("../../../assets/settings/default.json"),
        include_str!("../../../assets/settings/initial_user_settings.jsonc"),
    ] {
        let lower = text.to_lowercase();
        let dirs = std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../providers"))
            .map(|d| {
                d.flatten()
                    .map(|e| e.file_name().to_string_lossy().to_lowercase())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for d in dirs {
            assert!(!lower.contains(&d), "settings text names provider {d}");
        }
    }
}

#[test]
fn run_dir_under_data_dir() {
    // Single test: it mutates process env.
    unsafe { std::env::set_var("PARLA_DATA_DIR", "/tmp/parla-data-mcp") };
    assert_eq!(
        paths::run_dir(),
        Path::new("/tmp/parla-data-mcp").join("run")
    );
    #[cfg(unix)]
    assert_eq!(
        paths::mcp_socket_path(),
        Path::new("/tmp/parla-data-mcp/run/mcp.sock")
    );
    unsafe { std::env::remove_var("PARLA_DATA_DIR") };
}

#[test]
fn windows_pipe_name_is_per_user_and_stable() {
    let a = paths::windows_pipe_name("S-1-5-21-1-2-3-1001");
    assert!(a.starts_with(r"\\.\pipe\parla-mcp-"));
    assert_eq!(a.len(), r"\\.\pipe\parla-mcp-".len() + 16);
    assert_eq!(a, paths::windows_pipe_name("S-1-5-21-1-2-3-1001"));
    assert_ne!(a, paths::windows_pipe_name("S-1-5-21-1-2-3-1002"));
}

#[cfg(unix)]
#[test]
fn long_socket_path_does_not_fit() {
    assert!(paths::socket_path_fits(Path::new(
        "/tmp/parla/run/mcp.sock"
    )));
    let long = format!("/{}/mcp.sock", "a".repeat(120));
    assert!(!paths::socket_path_fits(Path::new(&long)));
}
