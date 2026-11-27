//! The MCP confirmation types round trip through the shared `write_confirm`
//! ones without changing what MCP sees.

use std::sync::Arc;

use domain::Capabilities;
use mcp::audit::ClientInfo;
use mcp::backend::WorkspaceInfo;
use mcp::confirm::{Confirmation, WriteAction, WriteRequest};
use mcp::limits::ClientId;
use settings::WorkspaceKey;

fn request() -> WriteRequest {
    WriteRequest {
        client: ClientInfo {
            id: ClientId(1),
            name: Arc::from("claude-code"),
            peer_pid: Some(7),
        },
        workspace: WorkspaceInfo {
            key: WorkspaceKey::new("fake", "W1"),
            name: Arc::from("Acme"),
            provider_display: Arc::from("Fakechat"),
            self_user: None,
            caps: Capabilities::default(),
        },
        as_user: Some(Arc::from("Marta Vidal")),
        action: WriteAction::Send {
            conversation_name: Arc::from("general"),
            thread: None,
            broadcast: false,
            text: Arc::from("hi"),
        },
    }
}

#[test]
fn shared_answers_map_to_the_mcp_answers() {
    use write_confirm::Confirmation as Shared;
    let pairs = [
        (Shared::Approved { remember: false }, Confirmation::Approved),
        (
            Shared::Approved { remember: true },
            Confirmation::ApprovedForSession,
        ),
        (Shared::Declined, Confirmation::Declined),
        (Shared::TimedOut, Confirmation::TimedOut),
    ];
    for (shared, mcp) in pairs {
        assert_eq!(Confirmation::from(shared), mcp);
    }
}

#[test]
fn mcp_request_keeps_its_fields_and_offers_nothing() {
    let req = request();
    let shared = write_confirm::WriteRequest::from(req.clone());
    assert_eq!(shared.requester.name(), "claude-code");
    assert_eq!(shared.workspace.key, req.workspace.key);
    assert_eq!(shared.as_user.as_deref(), Some("Marta Vidal"));
    assert_eq!(shared.action, req.action);
    assert_eq!(shared.remember_offer(), &write_confirm::AllowOffer::None);
    assert_eq!(shared.conversation_kind, None);
    assert_eq!(shared.details(), req.details());
}
