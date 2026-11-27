//! Cross-tool neutrality and policy checks.

use std::sync::Arc;

use domain::{
    Capabilities, HitSource, MessageHit, PersistOp,
    test_support::{FakeProvider, cid, conv, member, uid},
};
use serde_json::{Value, json};
use store::Store;

mod common;
use common::*;

/// Names of the directories under `crates/providers/`.
fn provider_dirs() -> Vec<String> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../providers");
    std::fs::read_dir(root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_lowercase())
        .collect()
}

#[tokio::test]
async fn tool_schemas_name_no_provider() {
    for lc in LIFECYCLES {
        let mut w = World::new(lc, vec![member_provider()], &["W1"]).await;
        let all =
            format!("{}{}", w.client.tools().await, w.client.templates().await).to_lowercase();
        // Spelled in two pieces so this file itself names no provider.
        assert!(!all.contains(concat!("sla", "ck")), "{lc:?}");
        let dirs = provider_dirs();
        assert!(!dirs.is_empty());
        for d in dirs {
            assert!(
                !all.contains(&d),
                "{d} appears in the tool or template JSON"
            );
        }
    }
}

fn member_provider() -> Arc<FakeProvider> {
    let p = FakeProvider::new("W1", uid("u1")).with_capabilities(Capabilities {
        search: true,
        scheduled_send: true,
        threads: true,
        ..Capabilities::default()
    });
    p.add_member(member("u1", "Me"));
    p.add_member(member("u2", "Alice"));
    Arc::new(p)
}

#[tokio::test]
async fn deny_list_hides_from_every_tool() {
    for lc in LIFECYCLES {
        let p = member_provider();
        let mut secret = conv("C2", "vault");
        secret.unread = true;
        secret.mentions = 4;
        secret.latest = Some(domain::test_support::ts(&ts(5)));
        p.add_conversation(conv("C1", "general"));
        p.add_conversation(secret);
        let mut root = text_msg("C2", 2, "THREADROOT classified");
        root.reply_count = 1;
        p.add_messages(
            &cid("C2"),
            vec![text_msg("C2", 1, "classified unread"), root],
        );
        p.set_search_hits(vec![MessageHit {
            message: text_msg("C2", 3, "classified remote hit"),
            snippet: Arc::from("classified remote hit"),
            source: HitSource::Remote,
        }]);
        domain::ChatProvider::schedule(&*p, &cid("C2"), None, "classified plan", 1_800_000_000)
            .await
            .unwrap();
        let mut w = World::new(lc, vec![p], &["W1"]).await;
        w.warm().await;
        w.backend
            .store()
            .apply(
                &ws("W1"),
                PersistOp::Messages {
                    conversation: cid("C2"),
                    messages: vec![text_msg("C2", 4, "classified stored")],
                },
            )
            .await
            .unwrap();
        w.settings
            .0
            .lock()
            .unwrap()
            .workspaces
            .get_mut(&ws("W1").to_string())
            .unwrap()
            .deny_channels = vec!["C2".into()];

        let calls: Vec<(&str, Value)> = vec![
            ("list_channels", json!({})),
            ("get_unreads", json!({})),
            (
                "read_channel",
                json!({"workspace": "fake/W1", "conversation": "C2"}),
            ),
            (
                "read_thread",
                json!({"workspace": "fake/W1", "conversation": "C2", "ts": ts(2)}),
            ),
            ("search_messages", json!({"query": "classified"})),
            ("list_scheduled", json!({})),
        ];
        for (tool, args) in calls {
            let r = w.client.call(tool, args).await;
            let text = r.to_string();
            assert!(!text.contains("classified"), "{tool} {lc:?}: {text}");
            assert!(!text.contains("vault"), "{tool} {lc:?}: {text}");
        }
        for uri in [
            "parla://fake/W1/conversation/C2".to_string(),
            format!("parla://fake/W1/thread/C2/{}", ts(2)),
        ] {
            let e = w.client.read_resource(&uri).await.unwrap_err();
            assert!(!e.contains("classified") && !e.contains("vault"), "{uri}");
        }
    }
}
