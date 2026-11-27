//! Resources: no enumeration, two neutral templates, reads through the same
//! policy, limits and audit as the tools.

use std::sync::Arc;

use domain::test_support::{FakeProvider, cid, conv, member, uid};
use serde_json::{Value, json};
use store::{AppStore, AuditQuery};

mod common;
use common::*;

fn seeded() -> Arc<FakeProvider> {
    let p = FakeProvider::new("W1", uid("u1"));
    p.add_member(member("u1", "Me"));
    p.add_member(member("u2", "Alice"));
    p.add_conversation(conv("C1", "general"));
    p.add_conversation(conv("C2", "secret"));
    p.add_messages(
        &cid("C1"),
        (1..=3)
            .map(|n| text_msg("C1", n, &format!("m{n}")))
            .collect(),
    );
    p.add_messages(&cid("C2"), vec![text_msg("C2", 1, "hidden text")]);
    Arc::new(p)
}

#[tokio::test]
async fn resource_list_is_empty() {
    for lc in LIFECYCLES {
        let mut w = World::new(lc, vec![seeded()], &["W1"]).await;
        assert_eq!(w.client.resources().await, json!([]), "{lc:?}");
        let t = w.client.templates().await;
        let uris: Vec<&str> = t
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["uriTemplate"].as_str().unwrap())
            .collect();
        assert_eq!(
            uris,
            [
                "parla://{provider}/{workspace}/conversation/{conversation}",
                "parla://{provider}/{workspace}/thread/{conversation}/{ts}",
            ],
            "{lc:?}"
        );
    }
}

#[tokio::test]
async fn resource_read_matches_read_channel() {
    for lc in LIFECYCLES {
        let mut w = World::new(lc, vec![seeded()], &["W1"]).await;
        let tool = w
            .client
            .call(
                "read_channel",
                json!({"workspace": "fake/W1", "conversation": "C1"}),
            )
            .await;
        let text = w
            .client
            .read_resource("parla://fake/W1/conversation/C1")
            .await
            .unwrap();
        let got: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(&got, payload(&tool), "{lc:?}");

        let root = ts(1);
        let tool = w
            .client
            .call(
                "read_thread",
                json!({"workspace": "fake/W1", "conversation": "C1", "ts": root}),
            )
            .await;
        let text = w
            .client
            .read_resource(&format!("parla://fake/W1/thread/C1/{root}"))
            .await
            .unwrap();
        let got: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(&got, payload(&tool), "{lc:?}");

        // Hidden, unknown and malformed all fail without telling which.
        w.settings
            .0
            .lock()
            .unwrap()
            .workspaces
            .get_mut(&ws("W1").to_string())
            .unwrap()
            .deny_channels = vec!["C2".into()];
        let hidden = w
            .client
            .read_resource("parla://fake/W1/conversation/C2")
            .await
            .unwrap_err();
        let unknown = w
            .client
            .read_resource("parla://fake/W1/conversation/C9")
            .await
            .unwrap_err();
        assert_eq!(hidden, unknown, "{lc:?}");
        assert!(!hidden.contains("hidden text"));
        assert!(
            w.client
                .read_resource("https://example.com/x")
                .await
                .is_err()
        );

        w.auditor.flush().await;
        let rows = w
            .audit_store
            .audit_page(AuditQuery {
                limit: 20,
                ..AuditQuery::default()
            })
            .await
            .unwrap();
        assert!(rows.iter().any(|r| &*r.tool == "resource_read"), "{lc:?}");
    }
}

#[tokio::test]
async fn resource_read_counts_against_the_read_limit() {
    let mut w = World::new(Lifecycle::Initialize, vec![seeded()], &["W1"]).await;
    let mut limited = false;
    for _ in 0..40 {
        if w.client
            .read_resource("parla://fake/W1/conversation/C1")
            .await
            .is_err()
        {
            limited = true;
            break;
        }
    }
    assert!(limited, "the read burst applies to resources too");
}
