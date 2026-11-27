use std::sync::Arc;

use domain::{
    Capabilities, ConversationId, Message, MessageId, MessageTs, PersistOp, UserId,
    test_support::FakeProvider,
};
use mcp::{
    ToolError,
    backend::Backend,
    ids::{Cursor, parse_workspace},
    test_support::FakeBackend,
};
use settings::WorkspaceKey;
use store::Store;

fn ts(n: u64) -> MessageTs {
    MessageTs(Arc::from(format!("{}.000000", 1_700_000_000 + n)))
}

fn msg(c: &ConversationId, n: u64) -> Message {
    Message::new(
        MessageId {
            conversation: c.clone(),
            ts: ts(n),
        },
        Some(UserId(Arc::from("U1"))),
        Arc::from(format!("m{n}")),
    )
}

#[test]
fn cursor_round_trips_and_rejects_garbage() {
    let c = Cursor { before: ts(7) };
    let text = c.encode();
    assert!(!text.contains(['+', '/', '=']));
    assert_eq!(Cursor::decode(&text).unwrap(), c);
    for bad in ["", "!!!", "bm90LWEtdHM", "Li4v", &"QQ".repeat(40)] {
        assert_eq!(Cursor::decode(bad), Err(ToolError::InvalidCursor), "{bad}");
    }
}

#[test]
fn parse_workspace_rejects_bad_keys() {
    assert_eq!(
        parse_workspace("fake/W1").unwrap(),
        WorkspaceKey::new("fake", "W1")
    );
    for bad in ["", "W1", "/W1", "fake/", "fake/a/b"] {
        assert!(matches!(
            parse_workspace(bad),
            Err(ToolError::InvalidWorkspace(_))
        ));
    }
}

#[test]
fn backend_is_object_safe() {
    let b: Arc<dyn Backend> = Arc::new(FakeBackend::new(vec![]));
    assert!(b.workspaces().is_empty());
}

#[test]
fn rmcp_has_no_http_features() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = std::process::Command::new(cargo)
        .args([
            "tree", "-p", "mcp", "-e", "normal", "-f", "{p} {f}", "--prefix", "none",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo tree");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    let rmcp: Vec<&str> = text.lines().filter(|l| l.starts_with("rmcp ")).collect();
    assert!(!rmcp.is_empty(), "rmcp missing from the tree");
    for line in rmcp {
        for bad in ["http", "sse", "reqwest"] {
            assert!(
                !line.split([' ', ',']).any(|f| f.contains(bad)),
                "rmcp feature {bad} enabled: {line}"
            );
        }
    }
}

#[tokio::test]
async fn fake_backend_reads_store_first() {
    let p = Arc::new(
        FakeProvider::new("W1", UserId(Arc::from("U1"))).with_capabilities(Capabilities::default()),
    );
    let c = ConversationId(Arc::from("C1"));
    let all: Vec<Message> = (1..=40).map(|n| msg(&c, n)).collect();
    p.add_messages(&c, all.clone());
    let b = FakeBackend::new(vec![p]);
    let ws = WorkspaceKey::new("fake", "W1");
    // The store holds the 20 newest.
    b.store()
        .apply(
            &ws,
            PersistOp::Messages {
                conversation: c.clone(),
                messages: all[20..].to_vec(),
            },
        )
        .await
        .unwrap();
    let hit = b.messages(&ws, &c, None, 20, true).await.unwrap();
    assert_eq!(hit.provider_calls, 0);
    assert_eq!(hit.value.messages.len(), 20);
    let miss = b.messages(&ws, &c, None, 40, true).await.unwrap();
    assert_eq!(miss.provider_calls, 1);
    assert_eq!(miss.value.messages.len(), 40);
}
