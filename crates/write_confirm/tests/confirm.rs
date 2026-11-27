//! The shared write request: who asks, what the prompt offers to remember,
//! and the Details text.

use std::process::Command;
use std::sync::Arc;

use domain::ConversationKind;
use futures::future::{BoxFuture, ready};
use settings::WorkspaceKey;
use write_confirm::{
    AllowOffer, ClientId, ClientInfo, Confirmation, Confirmer, DenyAllConfirmer, PluginSource,
    ReminderKind, Requester, Workspace, WriteAction, WriteRequest,
};

fn workspace() -> Workspace {
    Workspace {
        key: WorkspaceKey::new("fake", "W1"),
        name: Arc::from("Acme"),
    }
}

fn plugin() -> Requester {
    Requester::Plugin {
        id: Arc::from("standup-bot"),
        name: Arc::from("Standup Bot"),
        source: PluginSource::Registry,
    }
}

fn mcp() -> Requester {
    Requester::Mcp(ClientInfo {
        id: ClientId(1),
        name: Arc::from("claude-code"),
        peer_pid: Some(42),
    })
}

fn send() -> WriteAction {
    WriteAction::Send {
        conversation_name: Arc::from("general"),
        thread: None,
        broadcast: false,
        text: Arc::from("hello"),
    }
}

fn delete() -> WriteAction {
    WriteAction::Delete {
        conversation_name: Arc::from("general"),
        message_excerpt: Arc::from("old text"),
    }
}

#[test]
fn plugin_request_shows_plugin_name() {
    let req = WriteRequest::new(plugin(), workspace(), None, send());
    assert_eq!(req.requester.name(), "Standup Bot");
    let details: serde_json::Value = serde_json::from_str(&req.details()).unwrap();
    assert_eq!(details["plugin"], "standup-bot");
    assert_eq!(details["action"], "send_message");
    assert_eq!(details["text"], "hello");
    assert!(details.get("tool").is_none());
}

#[test]
fn delete_never_offers_remember() {
    assert_eq!(AllowOffer::for_action(&delete()), AllowOffer::None);
    let mut req = WriteRequest::new(plugin(), workspace(), None, delete());
    assert_eq!(req.offer, AllowOffer::None);
    // Even a caller that asks for it gets no remember offer on a delete.
    req.offer = AllowOffer::ForPlugin;
    assert_eq!(req.remember_offer(), &AllowOffer::None);
}

#[test]
fn remember_offered_in_conversation_for_send() {
    let req = WriteRequest::new(plugin(), workspace(), None, send());
    assert_eq!(req.offer, AllowOffer::InConversation(Arc::from("general")));
    assert_eq!(req.remember_offer(), &req.offer);
    for action in [
        WriteAction::Edit {
            conversation_name: Arc::from("general"),
            message_excerpt: Arc::from("a"),
            text: Arc::from("b"),
        },
        WriteAction::Upload {
            conversation_name: Arc::from("general"),
            name: Arc::from("a.png"),
            bytes: 10,
        },
        WriteAction::Schedule {
            conversation_name: Arc::from("general"),
            thread: None,
            text: Arc::from("later"),
            at: jiff::Timestamp::UNIX_EPOCH,
        },
    ] {
        assert_eq!(
            AllowOffer::for_action(&action),
            AllowOffer::InConversation(Arc::from("general")),
            "{action:?}"
        );
    }
    // Other areas are remembered for the plugin, not per conversation.
    for action in [
        WriteAction::Reminder {
            kind: ReminderKind::Add,
            text: Arc::from("x"),
            at: None,
        },
        WriteAction::MarkRead {
            conversation_name: Arc::from("general"),
        },
        WriteAction::Presence { away: true },
        WriteAction::Snooze { minutes: Some(30) },
    ] {
        assert_eq!(AllowOffer::for_action(&action), AllowOffer::ForPlugin);
    }
}

#[test]
fn mcp_requests_never_offer_remember() {
    let mut req = WriteRequest::new(mcp(), workspace(), None, send());
    assert_eq!(req.offer, AllowOffer::None);
    req.offer = AllowOffer::ForPlugin;
    assert_eq!(req.remember_offer(), &AllowOffer::None);
}

#[test]
fn mcp_details_keep_the_tool_shape() {
    let req = WriteRequest::new(mcp(), workspace(), None, send());
    let details: serde_json::Value = serde_json::from_str(&req.details()).unwrap();
    assert_eq!(
        details,
        serde_json::json!({
            "tool": "send_message",
            "workspace": "fake/W1",
            "conversation": "general",
            "thread_ts": null,
            "also_send_to_conversation": false,
            "text": "hello",
        })
    );
}

#[test]
fn every_action_has_a_name() {
    let names: Vec<&str> = [
        delete(),
        WriteAction::Pin {
            conversation_name: Arc::from("g"),
            message_excerpt: Arc::from("m"),
            on: true,
        },
        WriteAction::Save {
            conversation_name: Arc::from("g"),
            message_excerpt: Arc::from("m"),
            on: false,
        },
        WriteAction::Snooze { minutes: None },
    ]
    .iter()
    .map(WriteAction::tool)
    .collect();
    assert_eq!(
        names,
        ["delete_message", "set_pinned", "set_saved", "end_snooze"]
    );
}

/// Approves, remembering whenever the prompt offered it (what a user who
/// ticks the box does).
struct TicksTheBox;

impl Confirmer for TicksTheBox {
    fn confirm(&self, req: WriteRequest) -> BoxFuture<'static, Confirmation> {
        let remember = *req.remember_offer() != AllowOffer::None;
        Box::pin(ready(Confirmation::Approved { remember }))
    }
}

#[test]
fn confirmation_round_trip_carries_remember() {
    let c: &dyn Confirmer = &TicksTheBox;
    let answer = |req| futures::executor::block_on(c.confirm(req));
    assert_eq!(
        answer(WriteRequest::new(plugin(), workspace(), None, send())),
        Confirmation::Approved { remember: true }
    );
    assert_eq!(
        answer(WriteRequest::new(plugin(), workspace(), None, delete())),
        Confirmation::Approved { remember: false }
    );
    assert_eq!(
        answer(WriteRequest::new(mcp(), workspace(), None, send())),
        Confirmation::Approved { remember: false }
    );
    assert_eq!(
        futures::executor::block_on(DenyAllConfirmer.confirm(WriteRequest::new(
            plugin(),
            workspace(),
            None,
            send()
        ))),
        Confirmation::Declined
    );
}

#[test]
fn conversation_kind_defaults_to_unknown() {
    let req = WriteRequest::new(plugin(), workspace(), None, send());
    assert_eq!(req.conversation_kind, None);
    let req = req.with_conversation_kind(ConversationKind::Direct);
    assert_eq!(req.conversation_kind, Some(ConversationKind::Direct));
}

#[test]
fn write_confirm_depends_on_no_gpui() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args([
            "tree",
            "--offline",
            "-p",
            "write_confirm",
            "-e",
            "normal",
            "--prefix",
            "none",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo tree runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let tree = String::from_utf8_lossy(&out.stdout);
    assert!(
        !tree.lines().any(|l| l.starts_with("gpui")),
        "write_confirm must not depend on GPUI:\n{tree}"
    );
}
