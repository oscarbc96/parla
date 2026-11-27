//! `search_messages`, `get_user`, `search_users` and `list_scheduled`, in both
//! client lifecycles.

use std::sync::{Arc, atomic::Ordering};

use domain::{
    Capabilities, HitSource, MessageHit, PersistOp,
    test_support::{FakeProvider, cid, conv, member, uid},
};
use serde_json::json;
use store::{AppStore, AuditQuery, Store};

mod common;
use common::*;

fn search_provider(id: &str) -> Arc<FakeProvider> {
    let p = FakeProvider::new(id, uid("u1")).with_capabilities(Capabilities {
        search: true,
        scheduled_send: true,
        ..Capabilities::default()
    });
    p.add_member(member("u1", "Me"));
    p.add_member(member("u2", "Alice"));
    Arc::new(p)
}

async fn store_messages(w: &World, c: &str, msgs: Vec<domain::Message>) {
    w.backend
        .store()
        .apply(
            &ws("W1"),
            PersistOp::Messages {
                conversation: cid(c),
                messages: msgs,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn search_local_first_then_remote() {
    for lc in LIFECYCLES {
        let p = search_provider("W1");
        p.add_conversation(conv("C1", "general"));
        let remote = text_msg("C1", 9, "needle from the provider");
        p.set_search_hits(vec![MessageHit {
            message: remote,
            snippet: Arc::from("needle from the provider"),
            source: HitSource::Remote,
        }]);
        let mut w = World::new(lc, vec![p.clone()], &["W1"]).await;
        w.warm().await;
        store_messages(&w, "C1", vec![text_msg("C1", 3, "a local needle")]).await;

        // Enough local hits: the provider is not asked.
        let before = p.request_count();
        let r = w
            .client
            .call("search_messages", json!({"query": "needle", "limit": 1}))
            .await;
        let hits = payload(&r)["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 1, "{lc:?}");
        assert_eq!(hits[0]["untrusted_snippet"], "a local needle");
        assert_eq!(p.request_count(), before, "{lc:?}");

        // Too few local hits: one provider request, merged newest first.
        let r = w
            .client
            .call("search_messages", json!({"query": "needle", "limit": 5}))
            .await;
        let hits = payload(&r)["hits"].as_array().unwrap();
        let texts: Vec<&str> = hits
            .iter()
            .map(|h| h["untrusted_snippet"].as_str().unwrap())
            .collect();
        assert_eq!(
            texts,
            ["needle from the provider", "a local needle"],
            "{lc:?}"
        );
        assert_eq!(p.request_count(), before + 1, "{lc:?}");

        // remote: false stays local.
        let r = w
            .client
            .call(
                "search_messages",
                json!({"query": "needle", "limit": 5, "remote": false}),
            )
            .await;
        assert_eq!(payload(&r)["hits"].as_array().unwrap().len(), 1);
        assert_eq!(p.request_count(), before + 1, "{lc:?}");
    }
}

#[tokio::test]
async fn search_drops_hidden_hits_before_limit() {
    for lc in LIFECYCLES {
        let p = search_provider("W1");
        p.add_conversation(conv("C1", "general"));
        p.add_conversation(conv("C2", "secret"));
        let mut w = World::new(lc, vec![p], &["W1"]).await;
        w.warm().await;
        // The two newest hits are in the hidden conversation.
        store_messages(
            &w,
            "C1",
            vec![text_msg("C1", 1, "needle a"), text_msg("C1", 2, "needle b")],
        )
        .await;
        store_messages(
            &w,
            "C2",
            vec![text_msg("C2", 8, "needle x"), text_msg("C2", 9, "needle y")],
        )
        .await;
        w.settings
            .0
            .lock()
            .unwrap()
            .workspaces
            .get_mut(&ws("W1").to_string())
            .unwrap()
            .deny_channels = vec!["C2".into()];
        let r = w
            .client
            .call(
                "search_messages",
                json!({"query": "needle", "limit": 2, "remote": false}),
            )
            .await;
        let hits = payload(&r)["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 2, "{lc:?}");
        assert!(hits.iter().all(|h| h["conversation"] == "C1"));
        assert!(!r.to_string().contains("secret"));
        assert!(!r.to_string().contains("needle x"));
    }
}

#[tokio::test]
async fn search_query_not_in_audit_without_include_content() {
    for lc in LIFECYCLES {
        let p = search_provider("W1");
        p.add_conversation(conv("C1", "general"));
        let mut w = World::new(lc, vec![p], &["W1"]).await;
        w.warm().await;
        w.client
            .call(
                "search_messages",
                json!({"query": "tomato-soup", "workspace": "fake/W1", "remote": false}),
            )
            .await;
        w.settings.1.store(true, Ordering::Relaxed);
        w.client
            .call(
                "search_messages",
                json!({"query": "tomato-soup", "workspace": "fake/W1", "remote": false}),
            )
            .await;
        w.auditor.flush().await;
        let rows = w
            .audit_store
            .audit_page(AuditQuery {
                limit: 10,
                ..AuditQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(rows.len(), 2, "{lc:?}");
        // Newest first: the second call logged content, the first did not.
        assert!(
            rows[0]
                .content
                .as_deref()
                .is_some_and(|c| c.contains("tomato-soup"))
        );
        assert!(rows[1].content.is_none(), "{lc:?}");
        assert!(!format!("{:?}", rows[1]).contains("tomato-soup"));
    }
}

#[tokio::test]
async fn users_lookup_and_search() {
    for lc in LIFECYCLES {
        let p = search_provider("W1");
        let mut bob = member("u3", "Bob");
        bob.status = Some(domain::Status {
            emoji: Some(domain::EmojiRef {
                shortcode: Arc::from("palm_tree"),
                unicode: None,
                skin_tone: None,
            }),
            text: Arc::from("on holiday"),
            expires: None,
        });
        p.add_member(bob);
        let mut w = World::new(lc, vec![p], &["W1"]).await;
        w.warm().await;
        let r = w
            .client
            .call("get_user", json!({"workspace": "fake/W1", "user": "u3"}))
            .await;
        let u = payload(&r);
        assert_eq!(u["user"], "u3", "{lc:?}");
        assert_eq!(u["untrusted_display_name"], "Bob");
        assert_eq!(u["untrusted_status_text"], "on holiday");
        assert_eq!(u["status_emoji"], "palm_tree");
        assert_eq!(r["_meta"]["parla/untrusted_content"], json!(true));

        let r = w
            .client
            .call(
                "search_users",
                json!({"workspace": "fake/W1", "query": "ali"}),
            )
            .await;
        let users = payload(&r)["users"].as_array().unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0]["user"], "u2");

        // Unknown user and unknown workspace are the same kind of error.
        let r = w
            .client
            .call("get_user", json!({"workspace": "fake/W1", "user": "nope"}))
            .await;
        assert_eq!(error_of(&r)["code"], "not_available");
        let r = w
            .client
            .call("get_user", json!({"workspace": "fake/W9", "user": "u3"}))
            .await;
        assert_eq!(error_of(&r)["code"], "not_available");
    }
}

#[tokio::test]
async fn scheduled_filtered_by_policy() {
    for lc in LIFECYCLES {
        let p = search_provider("W1");
        p.add_conversation(conv("C1", "general"));
        p.add_conversation(conv("C2", "secret"));
        domain::ChatProvider::schedule(&*p, &cid("C1"), None, "visible plan", 1_800_000_000)
            .await
            .unwrap();
        domain::ChatProvider::schedule(&*p, &cid("C2"), None, "hidden plan", 1_800_000_100)
            .await
            .unwrap();
        let mut w = World::new(lc, vec![p], &["W1"]).await;
        w.warm().await;
        w.settings
            .0
            .lock()
            .unwrap()
            .workspaces
            .get_mut(&ws("W1").to_string())
            .unwrap()
            .deny_channels = vec!["C2".into()];
        let r = w.client.call("list_scheduled", json!({})).await;
        let list = payload(&r)["scheduled"].as_array().unwrap();
        assert_eq!(list.len(), 1, "{lc:?}");
        assert_eq!(list[0]["conversation"], "C1");
        assert_eq!(list[0]["untrusted_text"], "visible plan");
        assert_eq!(list[0]["post_at"], 1_800_000_000);
        assert!(!r.to_string().contains("hidden plan"));
    }
}

#[tokio::test]
async fn scheduled_is_partial_when_one_workspace_is_over_budget() {
    for lc in LIFECYCLES {
        let (p1, p2) = (search_provider("W1"), search_provider("W2"));
        p2.add_conversation(conv("C1", "general"));
        domain::ChatProvider::schedule(&*p2, &cid("C1"), None, "plan", 1_800_000_000)
            .await
            .unwrap();
        let mut w = World::new(lc, vec![p1, p2], &["W1", "W2"]).await;
        w.warm().await;
        w.limits.provider_reads(&ws("W1"), 20).unwrap();
        let r = w.client.call("list_scheduled", json!({})).await;
        let page = payload(&r);
        assert_eq!(page["partial"], json!(true), "{lc:?}");
        assert_eq!(page["scheduled"].as_array().unwrap().len(), 1, "{lc:?}");
        // Asking for the exhausted workspace by name still fails loudly.
        let r = w
            .client
            .call("list_scheduled", json!({"workspace": "fake/W1"}))
            .await;
        assert_eq!(error_of(&r)["code"], "rate_limited", "{lc:?}");
    }
}
