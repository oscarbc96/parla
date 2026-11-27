use std::sync::{Arc, Mutex};

use domain::{ConversationKind, test_support::conv};
use mcp::policy::{Deny, Policy, SettingsSource, WriteMode};
use settings::{McpSettings, McpWorkspaceSettings, WorkspaceKey};

fn ws() -> WorkspaceKey {
    WorkspaceKey::new("fake", "W1")
}

fn settings(enabled: bool, w: McpWorkspaceSettings) -> McpSettings {
    let mut s = McpSettings {
        enabled,
        ..McpSettings::default()
    };
    s.workspaces.insert(ws().to_string(), w);
    s
}

fn on() -> McpWorkspaceSettings {
    McpWorkspaceSettings {
        enabled: true,
        ..McpWorkspaceSettings::default()
    }
}

fn policy(s: McpSettings) -> Policy {
    Policy::new(Arc::new(s))
}

fn kind(id: &str, k: ConversationKind) -> domain::Conversation {
    let mut c = conv(id, id);
    c.kind = k;
    c
}

#[test]
fn everything_denied_when_mcp_off() {
    let p = policy(settings(
        false,
        McpWorkspaceSettings {
            allow_writes: true,
            ..on()
        },
    ));
    assert_eq!(p.workspace(&ws()).unwrap_err(), Deny::McpOff);
    assert_eq!(
        p.conversation(&ws(), &conv("C1", "general")),
        Err(Deny::McpOff)
    );
    assert_eq!(p.write(&ws()).unwrap_err(), Deny::McpOff);
    assert!(p.visible(&ws(), &[conv("C1", "general")]).is_empty());
}

#[test]
fn workspace_off_by_default() {
    let p = policy(McpSettings {
        enabled: true,
        ..McpSettings::default()
    });
    assert_eq!(p.workspace(&ws()).unwrap_err(), Deny::WorkspaceOff);
    let p = policy(settings(true, McpWorkspaceSettings::default()));
    assert_eq!(p.workspace(&ws()).unwrap_err(), Deny::WorkspaceOff);
    assert!(policy(settings(true, on())).workspace(&ws()).is_ok());
}

#[test]
fn dm_and_group_hidden_unless_include_dms() {
    let cs = [
        kind("C1", ConversationKind::Channel),
        kind("G1", ConversationKind::PrivateChannel),
        kind("D1", ConversationKind::Direct),
        kind("M1", ConversationKind::Group),
    ];
    let p = policy(settings(true, on()));
    let ids: Vec<_> = p
        .visible(&ws(), &cs)
        .iter()
        .map(|c| c.id.0.to_string())
        .collect();
    assert_eq!(ids, ["C1", "G1"]);
    assert_eq!(p.conversation(&ws(), &cs[2]), Err(Deny::DmsExcluded));
    let p = policy(settings(
        true,
        McpWorkspaceSettings {
            include_dms: true,
            ..on()
        },
    ));
    assert_eq!(p.visible(&ws(), &cs).len(), 4);
}

#[test]
fn deny_beats_allow() {
    let w = McpWorkspaceSettings {
        allow_channels: vec!["C1".into(), "C2".into()],
        deny_channels: vec!["C1".into()],
        ..on()
    };
    let p = policy(settings(true, w));
    assert_eq!(
        p.conversation(&ws(), &conv("C1", "a")),
        Err(Deny::DenyListed)
    );
    assert_eq!(p.conversation(&ws(), &conv("C2", "b")), Ok(()));
}

#[test]
fn non_empty_allow_list_hides_rest() {
    let w = McpWorkspaceSettings {
        allow_channels: vec!["C1".into()],
        ..on()
    };
    let p = policy(settings(true, w));
    assert_eq!(p.conversation(&ws(), &conv("C1", "a")), Ok(()));
    assert_eq!(
        p.conversation(&ws(), &conv("C2", "b")),
        Err(Deny::NotAllowListed)
    );
}

#[test]
fn names_never_match_lists() {
    let w = McpWorkspaceSettings {
        deny_channels: vec!["general".into()],
        ..on()
    };
    let p = policy(settings(true, w));
    assert_eq!(p.conversation(&ws(), &conv("C1", "general")), Ok(()));
    let w = McpWorkspaceSettings {
        allow_channels: vec!["general".into()],
        ..on()
    };
    let p = policy(settings(true, w));
    assert_eq!(
        p.conversation(&ws(), &conv("C1", "general")),
        Err(Deny::NotAllowListed)
    );
}

#[test]
fn writes_need_allow_writes() {
    let p = policy(settings(true, on()));
    assert_eq!(p.write(&ws()).unwrap_err(), Deny::WritesOff);
    let p = policy(settings(
        true,
        McpWorkspaceSettings {
            allow_writes: true,
            ..on()
        },
    ));
    assert_eq!(p.write(&ws()), Ok(WriteMode::Confirm));
}

#[test]
fn confirm_writes_false_is_auto() {
    let w = McpWorkspaceSettings {
        allow_writes: true,
        confirm_writes: false,
        ..on()
    };
    assert_eq!(policy(settings(true, w)).write(&ws()), Ok(WriteMode::Auto));
}

#[test]
fn deny_codes_are_stable() {
    let all = [
        (Deny::McpOff, "mcp_off"),
        (Deny::WorkspaceOff, "workspace_off"),
        (Deny::DmsExcluded, "dms_excluded"),
        (Deny::NotAllowListed, "not_allow_listed"),
        (Deny::DenyListed, "deny_listed"),
        (Deny::WritesOff, "writes_off"),
    ];
    for (d, c) in all {
        assert_eq!(d.code(), c);
    }
}

struct Fake(Mutex<Arc<McpSettings>>);
impl SettingsSource for Fake {
    fn mcp(&self) -> Arc<McpSettings> {
        self.0.lock().unwrap().clone()
    }
    fn include_content(&self) -> bool {
        false
    }
}

#[test]
fn settings_change_applies_to_next_call() {
    let src = Fake(Mutex::new(Arc::new(settings(true, on()))));
    let c = conv("C1", "general");
    assert_eq!(Policy::from_source(&src).conversation(&ws(), &c), Ok(()));
    *src.0.lock().unwrap() = Arc::new(settings(
        true,
        McpWorkspaceSettings {
            deny_channels: vec!["C1".into()],
            ..on()
        },
    ));
    assert_eq!(
        Policy::from_source(&src).conversation(&ws(), &c),
        Err(Deny::DenyListed)
    );
    *src.0.lock().unwrap() = Arc::new(McpSettings::default());
    assert_eq!(
        Policy::from_source(&src).conversation(&ws(), &c),
        Err(Deny::McpOff)
    );
}

mod session_allows {
    use domain::ConversationId;
    use mcp::policy::{MAX_SESSION_ALLOWS, SessionAllows, SessionScope};
    use settings::WorkspaceKey;

    fn scope(tool: &'static str, ws: &str, c: Option<&str>) -> SessionScope {
        SessionScope {
            tool,
            workspace: WorkspaceKey::new("fake", ws),
            conversation: c.map(|c| ConversationId(c.into())),
        }
    }

    #[test]
    fn always_for_this_session_allows_that_tool_and_destination_only() {
        let allows = SessionAllows::default();
        let send = scope("send_message", "W1", Some("C1"));
        assert!(!allows.allows(&send), "nothing is allowed up front");
        assert!(allows.allow(send.clone()));
        assert!(allows.allows(&send));
        assert!(allows.allows(&send), "and keeps allowing it");
        for other in [
            scope("send_message", "W1", Some("C2")),
            scope("send_message", "W2", Some("C1")),
            scope("add_reaction", "W1", Some("C1")),
            scope("schedule_message", "W1", Some("C1")),
            scope("send_message", "W1", None),
        ] {
            assert!(!allows.allows(&other), "{other:?} still asks");
        }
        let status = scope("set_status", "W1", None);
        allows.allow(status.clone());
        assert!(allows.allows(&status));
        assert!(!allows.allows(&scope("set_status", "W2", None)));
    }

    #[test]
    fn a_new_session_starts_empty() {
        let first = SessionAllows::default();
        first.allow(scope("send_message", "W1", Some("C1")));
        // A new connection (or a restarted app) builds a new set: nothing
        // is read from disk or carried over.
        let next = SessionAllows::default();
        assert!(!next.allows(&scope("send_message", "W1", Some("C1"))));
        assert_eq!(next.len(), 0);
    }

    #[test]
    fn the_set_is_bounded() {
        let allows = SessionAllows::default();
        for i in 0..MAX_SESSION_ALLOWS {
            let c = format!("C{i}");
            assert!(allows.allow(scope("send_message", "W1", Some(&c))));
        }
        assert_eq!(allows.len(), MAX_SESSION_ALLOWS);
        // A full set keeps asking instead of growing.
        let extra = scope("send_message", "W1", Some("extra"));
        assert!(!allows.allow(extra.clone()));
        assert!(!allows.allows(&extra));
        assert_eq!(allows.len(), MAX_SESSION_ALLOWS);
        // Allowing a scope it already holds is still fine.
        assert!(allows.allow(scope("send_message", "W1", Some("C0"))));
    }
}
