//! The write tools behind confirmation: a scripted confirmer answers from a
//! channel so each test decides when and how the user answers.

use std::{sync::Arc, time::Duration};

use domain::{
    Capabilities,
    test_support::{Call, FakeProvider, cid, conv, member, mid, uid},
};
use futures::future::BoxFuture;
use mcp::{
    confirm::{Confirmation, Confirmer, WriteAction, WriteRequest},
    server::ParlaServer,
};
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};
use store::{AppStore, AuditQuery, WriteDecision};
use tokio::sync::{mpsc, oneshot};

mod common;
use common::*;

type Reply = oneshot::Sender<Confirmation>;
type Prompts = mpsc::UnboundedReceiver<(WriteRequest, Reply)>;

/// Hands each request to the test and waits for its answer.
struct Scripted {
    tx: mpsc::UnboundedSender<(WriteRequest, Reply)>,
}

impl Confirmer for Scripted {
    fn confirm(&self, req: WriteRequest) -> BoxFuture<'static, Confirmation> {
        let (reply, answer) = oneshot::channel();
        let _ = self.tx.send((req, reply));
        Box::pin(async move { answer.await.unwrap_or(Confirmation::Declined) })
    }
}

/// The next prompt; a test that expects one fails instead of hanging.
async fn next(prompts: &mut Prompts) -> (WriteRequest, Reply) {
    tokio::time::timeout(Duration::from_secs(5), prompts.recv())
        .await
        .expect("a prompt within 5 s")
        .expect("confirmer alive")
}

/// A small timestamp: the fake provider numbers new messages from 1 upward
/// and counts up past the newest existing one.
const TARGET: &str = "0.000100";

fn provider() -> Arc<FakeProvider> {
    let p = FakeProvider::new("W1", uid("u1")).with_capabilities(Capabilities {
        threads: true,
        scheduled_send: true,
        status: true,
        ..Capabilities::default()
    });
    p.add_member(member("u1", "Me"));
    p.add_member(member("u2", "Alice"));
    p.add_conversation(conv("C1", "general"));
    p.add_conversation(conv("C2", "vault"));
    p.add_messages(
        &cid("C1"),
        vec![domain::Message::new(
            mid("C1", TARGET),
            Some(uid("u2")),
            Arc::from("the target message"),
        )],
    );
    Arc::new(p)
}

fn ws_settings(w: &World, f: impl FnOnce(&mut settings::McpWorkspaceSettings)) {
    let mut s = w.settings.0.lock().unwrap();
    f(s.workspaces.get_mut(&ws("W1").to_string()).unwrap());
}

/// A world with writes allowed (and confirmed) on W1.
async fn world(lc: Lifecycle) -> (World, Prompts, Arc<FakeProvider>) {
    world_with(lc, None).await
}

async fn world_with(
    lc: Lifecycle,
    timeout: Option<Duration>,
) -> (World, Prompts, Arc<FakeProvider>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let p = provider();
    let w = World::build(
        lc,
        vec![p.clone()],
        &["W1"],
        Arc::new(Scripted { tx }),
        timeout,
    )
    .await;
    ws_settings(&w, |s| {
        s.allow_writes = true;
        s.confirm_writes = true;
    });
    (w, rx, p)
}

fn send_args(text: &str) -> Value {
    json!({"workspace": "fake/W1", "conversation": "C1", "text": text})
}

fn error_code(r: &Value) -> &str {
    error_of(r)["code"].as_str().unwrap()
}

async fn audit_rows(w: &World) -> Vec<store::AuditRow> {
    w.auditor.flush().await;
    w.audit_store
        .audit_page(AuditQuery {
            limit: 50,
            ..AuditQuery::default()
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn writes_off_by_default() {
    for lc in LIFECYCLES {
        let (mut w, mut prompts, _p) = world(lc).await;
        ws_settings(&w, |s| s.allow_writes = false);
        let r = w.client.call("send_message", send_args("hi")).await;
        assert_eq!(error_code(&r), "writes_off", "{lc:?}");
        assert!(w.backend.sent().is_empty());
        assert!(prompts.try_recv().is_err(), "the prompt was not shown");
    }
}

#[tokio::test]
async fn write_waits_for_confirmation() {
    for lc in LIFECYCLES {
        let (mut w, mut prompts, _p) = world(lc).await;
        let backend = w.backend.clone();
        let call = w.client.call("send_message", send_args("hello team"));
        let user = async {
            let (_req, reply) = next(&mut prompts).await;
            tokio::time::sleep(Duration::from_millis(30)).await;
            assert!(backend.sent().is_empty(), "nothing is sent before the yes");
            reply.send(Confirmation::Approved).unwrap();
        };
        let (r, ()) = tokio::join!(call, user);
        let out = payload(&r);
        assert_eq!(out["conversation"], "C1", "{lc:?}");
        assert!(out["ts"].as_str().is_some());
        let sent = w.backend.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(&*sent[0].1.text, "hello team");
    }
}

#[tokio::test]
async fn declined_sends_nothing() {
    for lc in LIFECYCLES {
        let (mut w, mut prompts, _p) = world(lc).await;
        let call = w.client.call("send_message", send_args("no"));
        let user = async {
            let (_req, reply) = next(&mut prompts).await;
            reply.send(Confirmation::Declined).unwrap();
        };
        let (r, ()) = tokio::join!(call, user);
        assert_eq!(error_code(&r), "declined", "{lc:?}");
        assert!(w.backend.sent().is_empty());
        let rows = audit_rows(&w).await;
        assert_eq!(rows[0].decision, Some(WriteDecision::Declined));
    }
}

#[tokio::test]
async fn timeout_is_declined() {
    let (mut w, mut prompts, _p) =
        world_with(Lifecycle::Initialize, Some(Duration::from_millis(80))).await;
    let call = w.client.call("send_message", send_args("late"));
    let user = async {
        // Hold the question open and never answer.
        let (_req, reply) = next(&mut prompts).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        drop(reply);
    };
    let (r, ()) = tokio::join!(call, user);
    assert_eq!(error_code(&r), "timed_out");
    assert!(
        error_of(&r)["message"]
            .as_str()
            .unwrap()
            .contains("nothing was sent")
    );
    assert!(w.backend.sent().is_empty());
    let rows = audit_rows(&w).await;
    assert_eq!(rows[0].decision, Some(WriteDecision::TimedOut));
}

#[tokio::test]
async fn confirm_writes_false_sends_without_prompt_and_audits_auto() {
    for lc in LIFECYCLES {
        let (mut w, mut prompts, _p) = world(lc).await;
        ws_settings(&w, |s| s.confirm_writes = false);
        let r = w.client.call("send_message", send_args("go")).await;
        payload(&r);
        assert_eq!(w.backend.sent().len(), 1, "{lc:?}");
        assert!(prompts.try_recv().is_err());
        let rows = audit_rows(&w).await;
        assert_eq!(rows[0].decision, Some(WriteDecision::Auto));
    }
}

#[tokio::test]
async fn confirm_writes_flip_mid_prompt_still_needs_yes() {
    for lc in LIFECYCLES {
        let (mut w, mut prompts, _p) = world(lc).await;
        let backend = w.backend.clone();
        let settings = w.settings.clone();
        let call = w.client.call("send_message", send_args("flip"));
        let user = async {
            let (_req, reply) = next(&mut prompts).await;
            settings
                .0
                .lock()
                .unwrap()
                .workspaces
                .get_mut(&ws("W1").to_string())
                .unwrap()
                .confirm_writes = false;
            tokio::time::sleep(Duration::from_millis(30)).await;
            assert!(
                backend.sent().is_empty(),
                "the pending prompt is not skipped"
            );
            reply.send(Confirmation::Declined).unwrap();
        };
        let (r, ()) = tokio::join!(call, user);
        assert_eq!(error_code(&r), "declined", "{lc:?}");
        assert!(w.backend.sent().is_empty());
    }
}

#[tokio::test]
async fn writes_turned_off_mid_prompt_denies_after_yes() {
    for lc in LIFECYCLES {
        let (mut w, mut prompts, _p) = world(lc).await;
        let settings = w.settings.clone();
        let call = w.client.call("send_message", send_args("off"));
        let user = async {
            let (_req, reply) = next(&mut prompts).await;
            settings
                .0
                .lock()
                .unwrap()
                .workspaces
                .get_mut(&ws("W1").to_string())
                .unwrap()
                .allow_writes = false;
            reply.send(Confirmation::Approved).unwrap();
        };
        let (r, ()) = tokio::join!(call, user);
        assert_eq!(error_code(&r), "writes_off", "{lc:?}");
        assert!(w.backend.sent().is_empty());
    }
}

#[tokio::test]
async fn hidden_target_never_prompts() {
    for lc in LIFECYCLES {
        let (mut w, mut prompts, _p) = world(lc).await;
        ws_settings(&w, |s| s.deny_channels = vec!["C2".into()]);
        let hidden = w
            .client
            .call(
                "send_message",
                json!({"workspace": "fake/W1", "conversation": "C2", "text": "x"}),
            )
            .await;
        let unknown = w
            .client
            .call(
                "send_message",
                json!({"workspace": "fake/W1", "conversation": "C9", "text": "x"}),
            )
            .await;
        assert_eq!(error_code(&hidden), "not_available", "{lc:?}");
        assert_eq!(error_of(&hidden), error_of(&unknown));
        assert!(
            prompts.try_recv().is_err(),
            "the confirmer was never called"
        );
        assert!(w.backend.sent().is_empty());
    }
}

#[tokio::test]
async fn prompt_carries_exact_text_and_destination() {
    for lc in LIFECYCLES {
        let (mut w, mut prompts, _p) = world(lc).await;
        let text = "  h\u{e9}llo\n*bold* <@U2> :wave:  ";
        let call = w.client.call("send_message", send_args(text));
        let user = async {
            let (req, reply) = next(&mut prompts).await;
            reply.send(Confirmation::Declined).unwrap();
            req
        };
        let (_r, req) = tokio::join!(call, user);
        assert_eq!(req.workspace.key, ws("W1"), "{lc:?}");
        assert_eq!(&*req.workspace.name, "W1");
        assert!(!req.client.name.is_empty());
        match req.action {
            WriteAction::Send {
                conversation_name,
                text: sent,
                thread,
                broadcast,
            } => {
                assert_eq!(&*sent, text);
                assert_eq!(&*conversation_name, "general");
                assert!(thread.is_none() && !broadcast);
            }
            other => panic!("unexpected action {other:?}"),
        }
    }
}

#[tokio::test]
async fn parallel_writes_queue_at_most_three() {
    let (w, mut prompts, _p) = world(Lifecycle::Initialize).await;
    let Client::Init(c) = &w.client else {
        unreachable!()
    };
    let calls = (0..5).map(|i| {
        c.call_tool(
            CallToolRequestParams::new("send_message".to_string())
                .with_arguments(send_args(&format!("m{i}")).as_object().unwrap().clone()),
        )
    });
    let user = async {
        let mut held = Vec::new();
        for _ in 0..3 {
            held.push(next(&mut prompts).await);
        }
        let fourth = tokio::time::timeout(Duration::from_millis(100), prompts.recv()).await;
        assert!(fourth.is_err(), "at most three prompts are pending");
        for (_req, reply) in held {
            reply.send(Confirmation::Approved).unwrap();
        }
    };
    let (results, ()) = tokio::join!(futures::future::join_all(calls), user);
    let values: Vec<Value> = results
        .into_iter()
        .map(|r| serde_json::to_value(r.unwrap()).unwrap())
        .collect();
    let ok = values
        .iter()
        .filter(|v| v["isError"] != json!(true))
        .count();
    let limited = values
        .iter()
        .filter(|v| v["structuredContent"]["error"]["code"] == "rate_limited")
        .count();
    assert_eq!((ok, limited), (3, 2));
    assert_eq!(w.backend.sent().len(), 3);
}

#[tokio::test]
async fn retry_with_same_key_sends_once() {
    for lc in LIFECYCLES {
        let (mut w, mut prompts, _p) = world(lc).await;
        let args = json!({"workspace": "fake/W1", "conversation": "C1", "text": "once", "idempotency_key": "k1"});
        let call = w.client.call("send_message", args.clone());
        let user = async {
            let (_req, reply) = next(&mut prompts).await;
            reply.send(Confirmation::Approved).unwrap();
        };
        let (first, ()) = tokio::join!(call, user);
        let second = w.client.call("send_message", args).await;
        assert_eq!(payload(&first), payload(&second), "{lc:?}");
        assert!(prompts.try_recv().is_err(), "no second prompt");
        assert_eq!(w.backend.sent().len(), 1);
        let rows = audit_rows(&w).await;
        assert_eq!(rows[0].detail.as_deref(), Some("idempotent_replay"));
        // A different key is a new write.
        let other = json!({"workspace": "fake/W1", "conversation": "C1", "text": "once", "idempotency_key": "k2"});
        let call = w.client.call("send_message", other);
        let user = async {
            let (_req, reply) = next(&mut prompts).await;
            reply.send(Confirmation::Approved).unwrap();
        };
        let (_r, ()) = tokio::join!(call, user);
        assert_eq!(w.backend.sent().len(), 2);
    }
}

fn in_days(n: i64) -> String {
    (jiff::Timestamp::now() + jiff::SignedDuration::from_hours(24 * n)).to_string()
}

#[tokio::test]
async fn invalid_schedule_rejected_before_prompt() {
    for lc in LIFECYCLES {
        for at in [
            "2020-01-01T00:00:00Z".to_string(),
            in_days(200),
            "tomorrow".to_string(),
            "2030-01-01T10:00:00".to_string(),
        ] {
            // A fresh client each time: the write burst is three calls.
            let (mut w, mut prompts, _p) = world(lc).await;
            let r = w
                .client
                .call(
                    "schedule_message",
                    json!({"workspace": "fake/W1", "conversation": "C1", "text": "x", "at": at}),
                )
                .await;
            assert_eq!(error_code(&r), "bad_argument", "{lc:?} {at}");
            assert!(prompts.try_recv().is_err(), "the user is never asked");
        }
    }
}

#[tokio::test]
async fn schedule_message_after_confirmation() {
    let (mut w, mut prompts, p) = world(Lifecycle::Initialize).await;
    let at = in_days(3);
    let call = w.client.call(
        "schedule_message",
        json!({"workspace": "fake/W1", "conversation": "C1", "text": "later", "at": at}),
    );
    let user = async {
        let (req, reply) = next(&mut prompts).await;
        assert!(matches!(req.action, WriteAction::Schedule { .. }));
        reply.send(Confirmation::Approved).unwrap();
    };
    let (r, ()) = tokio::join!(call, user);
    assert_eq!(payload(&r)["untrusted_text"], "later");
    assert!(
        p.calls()
            .iter()
            .any(|c| matches!(c, Call::Schedule { text, .. } if text == "later"))
    );
}

#[tokio::test]
async fn broadcast_requires_thread() {
    for lc in LIFECYCLES {
        let (mut w, mut prompts, _p) = world(lc).await;
        let r = w
            .client
            .call(
                "send_message",
                json!({"workspace": "fake/W1", "conversation": "C1", "text": "x", "also_send_to_conversation": true}),
            )
            .await;
        assert_eq!(error_code(&r), "bad_argument", "{lc:?}");
        assert!(prompts.try_recv().is_err());
        // With a thread it is a normal confirmed write.
        let args = json!({"workspace": "fake/W1", "conversation": "C1", "text": "x", "thread_ts": TARGET, "also_send_to_conversation": true});
        let call = w.client.call("send_message", args);
        let user = async {
            let (req, reply) = next(&mut prompts).await;
            reply.send(Confirmation::Approved).unwrap();
            req
        };
        let (r, req) = tokio::join!(call, user);
        payload(&r);
        assert!(matches!(
            req.action,
            WriteAction::Send {
                broadcast: true,
                thread: Some(_),
                ..
            }
        ));
        let sent = w.backend.sent();
        assert!(sent[0].1.broadcast && sent[0].1.thread.is_some());
    }
}

#[tokio::test]
async fn write_text_not_in_audit_without_include_content() {
    for lc in LIFECYCLES {
        let (mut w, mut prompts, _p) = world(lc).await;
        ws_settings(&w, |s| s.confirm_writes = false);
        w.client
            .call("send_message", send_args("secret-sauce"))
            .await;
        w.settings
            .1
            .store(true, std::sync::atomic::Ordering::Relaxed);
        w.client
            .call("send_message", send_args("secret-sauce"))
            .await;
        assert!(prompts.try_recv().is_err());
        let rows = audit_rows(&w).await;
        assert_eq!(rows.len(), 2, "{lc:?}");
        assert_eq!(rows[0].content.as_deref(), Some("secret-sauce"));
        assert!(rows[1].content.is_none());
        assert!(!format!("{:?}", rows[1]).contains("secret-sauce"));
    }
}

#[tokio::test]
async fn add_reaction_shows_the_message_and_reacts() {
    let (mut w, mut prompts, p) = world(Lifecycle::Initialize).await;
    w.warm().await;
    let call = w.client.call(
        "add_reaction",
        json!({"workspace": "fake/W1", "conversation": "C1", "ts": TARGET, "emoji": "tada"}),
    );
    let user = async {
        let (req, reply) = next(&mut prompts).await;
        match &req.action {
            WriteAction::React {
                message_excerpt,
                emoji,
                conversation_name,
            } => {
                assert_eq!(&**message_excerpt, "the target message");
                assert_eq!(&*emoji.shortcode, "tada");
                assert_eq!(&**conversation_name, "general");
            }
            other => panic!("unexpected {other:?}"),
        }
        reply.send(Confirmation::Approved).unwrap();
    };
    let (r, ()) = tokio::join!(call, user);
    payload(&r);
    assert!(
        p.calls()
            .iter()
            .any(|c| matches!(c, Call::React { id, add: true, .. } if id == &mid("C1", TARGET)))
    );
    // Colons and unknown messages are rejected without a prompt.
    let r = w
        .client
        .call(
            "add_reaction",
            json!({"workspace": "fake/W1", "conversation": "C1", "ts": TARGET, "emoji": ":tada:"}),
        )
        .await;
    assert_eq!(error_code(&r), "bad_argument");
    let r = w
        .client
        .call(
            "add_reaction",
            json!({"workspace": "fake/W1", "conversation": "C1", "ts": ts(77), "emoji": "tada"}),
        )
        .await;
    assert_eq!(error_code(&r), "not_available");
    assert!(prompts.try_recv().is_err());
}

#[tokio::test]
async fn set_status_sets_and_clears() {
    let (mut w, mut prompts, p) = world(Lifecycle::Initialize).await;
    let set = w.client.call(
        "set_status",
        json!({"workspace": "fake/W1", "text": "heads down", "emoji": "headphones", "expires": in_days(1)}),
    );
    let user = async {
        let (req, reply) = next(&mut prompts).await;
        match &req.action {
            WriteAction::SetStatus { status: Some(s) } => {
                assert_eq!(&*s.text, "heads down");
                assert!(s.expires.is_some());
            }
            other => panic!("unexpected {other:?}"),
        }
        reply.send(Confirmation::Approved).unwrap();
    };
    let (r, ()) = tokio::join!(set, user);
    payload(&r);
    let clear = w
        .client
        .call("set_status", json!({"workspace": "fake/W1", "clear": true}));
    let user = async {
        let (req, reply) = next(&mut prompts).await;
        assert!(matches!(
            req.action,
            WriteAction::SetStatus { status: None }
        ));
        reply.send(Confirmation::Approved).unwrap();
    };
    let (r, ()) = tokio::join!(clear, user);
    payload(&r);
    let calls = p.calls();
    assert!(calls.iter().any(|c| matches!(c, Call::SetStatus(Some(_)))));
    assert!(calls.iter().any(|c| matches!(c, Call::SetStatus(None))));
}

#[tokio::test]
async fn invalid_status_never_prompts() {
    for bad in [
        json!({"workspace": "fake/W1"}),
        json!({"workspace": "fake/W1", "clear": true, "text": "x"}),
        json!({"workspace": "fake/W1", "text": "x".repeat(101)}),
    ] {
        let (mut w, mut prompts, _p) = world(Lifecycle::Initialize).await;
        let r = w.client.call("set_status", bad).await;
        assert_eq!(error_code(&r), "bad_argument");
        assert!(prompts.try_recv().is_err());
    }
}

#[tokio::test]
async fn write_tools_are_annotated_as_writes() {
    let (mut w, _prompts, _p) = world(Lifecycle::Initialize).await;
    let tools = w.client.tools().await;
    let names: Vec<(&str, bool)> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap(),
                t["annotations"]["readOnlyHint"] == json!(true),
            )
        })
        .collect();
    for write in [
        "send_message",
        "schedule_message",
        "add_reaction",
        "set_status",
    ] {
        let t = tools
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == write)
            .unwrap();
        assert_eq!(t["annotations"]["readOnlyHint"], json!(false), "{write}");
        assert_eq!(t["annotations"]["destructiveHint"], json!(false), "{write}");
        assert_eq!(t["annotations"]["openWorldHint"], json!(true), "{write}");
    }
    assert!(names.iter().filter(|(_, ro)| *ro).count() >= 9);
    let _ = ParlaServer::new;
}

#[tokio::test]
async fn idempotency_keeps_64_keys_for_ten_minutes_per_client_name() {
    use mcp::idempotency::Idempotency;
    use std::time::Instant;
    let store = Idempotency::default();
    let t0 = Instant::now();
    let put = |c: &str, k: &str, v: Value| {
        let slot = store.slot(c, k, t0);
        async move {
            let mut h = slot.acquire().await;
            h.store(v, t0);
        }
    };
    let get = |c: &str, k: &str, at: Instant| {
        let slot = store.slot(c, k, at);
        async move { slot.acquire().await.replay(at) }
    };
    put("a", "k", json!({"ts": "1"})).await;
    assert_eq!(get("a", "k", t0).await, Some(json!({"ts": "1"})));
    assert_eq!(get("b", "k", t0).await, None, "keys are per client name");
    assert!(get("a", "k", t0 + Duration::from_secs(599)).await.is_some());
    assert!(get("a", "k", t0 + Duration::from_secs(600)).await.is_none());
    for i in 0..65 {
        put("a", &format!("n{i}"), json!(i)).await;
    }
    assert!(get("a", "n0", t0).await.is_none(), "only the last 64 stay");
    assert!(get("a", "n64", t0).await.is_some());
}

#[tokio::test]
async fn concurrent_calls_with_one_key_run_once_and_the_rest_replay() {
    use mcp::idempotency::Idempotency;
    use std::time::Instant;
    let store = Arc::new(Idempotency::default());
    let first = store.slot("c", "k", Instant::now()).acquire().await;
    let waiter = {
        let store = store.clone();
        tokio::spawn(async move {
            store
                .slot("c", "k", Instant::now())
                .acquire()
                .await
                .replay(Instant::now())
        })
    };
    tokio::task::yield_now().await;
    assert!(!waiter.is_finished(), "the retry waits for the first call");
    let mut first = first;
    first.store(json!({"ts": "7"}), Instant::now());
    drop(first);
    assert_eq!(waiter.await.unwrap(), Some(json!({"ts": "7"})));
}

#[tokio::test]
async fn a_key_survives_the_clients_reconnect() {
    for lc in LIFECYCLES {
        let (mut w, mut prompts, _p) = world(lc).await;
        let args = json!({"workspace": "fake/W1", "conversation": "C1", "text": "once", "idempotency_key": "kr"});
        let call = w.client.call("send_message", args.clone());
        let user = async {
            let (_req, reply) = next(&mut prompts).await;
            reply.send(Confirmation::Approved).unwrap();
        };
        let (first, ()) = tokio::join!(call, user);
        // The same client name on a new connection (the shim reconnected).
        let mut again = w.reconnect(lc).await;
        let second = again.call("send_message", args).await;
        assert_eq!(payload(&first), payload(&second), "{lc:?}");
        assert!(prompts.try_recv().is_err(), "no second prompt");
        assert_eq!(w.backend.sent().len(), 1);
    }
}

#[tokio::test]
async fn concurrent_send_with_one_key_prompts_and_sends_once() {
    for lc in LIFECYCLES {
        let (mut w, mut prompts, _p) = world(lc).await;
        let mut second_client = w.reconnect(lc).await;
        let args = json!({"workspace": "fake/W1", "conversation": "C1", "text": "once", "idempotency_key": "kc"});
        let a = w.client.call("send_message", args.clone());
        let b = async {
            // Let the first call reach its prompt before the retry starts.
            tokio::time::sleep(Duration::from_millis(100)).await;
            second_client.call("send_message", args.clone()).await
        };
        let user = async {
            let (_req, reply) = next(&mut prompts).await;
            tokio::time::sleep(Duration::from_millis(300)).await;
            reply.send(Confirmation::Approved).unwrap();
        };
        let (first, second, ()) = tokio::join!(a, b, user);
        assert_eq!(payload(&first), payload(&second), "{lc:?}");
        assert!(prompts.try_recv().is_err(), "the retry never prompts");
        assert_eq!(w.backend.sent().len(), 1, "{lc:?}");
    }
}

#[tokio::test]
async fn oversized_ids_are_capped_in_the_audit_target_and_error_text() {
    for lc in LIFECYCLES {
        let (mut w, _prompts, _p) = world(lc).await;
        let huge = "x".repeat(10_000);
        let r = w
            .client
            .call(
                "send_message",
                json!({"workspace": "fake/W1", "conversation": huge, "text": "hi"}),
            )
            .await;
        assert!(
            r.to_string().len() < 1_000,
            "{lc:?}: {}",
            r.to_string().len()
        );
        let r = w
            .client
            .call(
                "send_message",
                json!({"workspace": huge, "conversation": "C1", "text": "hi"}),
            )
            .await;
        assert_eq!(error_code(&r), "bad_argument", "{lc:?}");
        assert!(r.to_string().len() < 1_000, "{lc:?}");
        for row in audit_rows(&w).await {
            assert!(
                row.target.as_deref().is_none_or(|t| t.len() <= 300),
                "{lc:?}: audit target not capped"
            );
        }
    }
}

/// Answers the first prompt "Always for this session" and checks it was sent.
async fn allow_for_session(w: &mut World, prompts: &mut Prompts, lc: Lifecycle) {
    let call = w.client.call("send_message", send_args("first"));
    let user = async {
        let (req, reply) = next(prompts).await;
        assert_eq!(req.action.tool(), "send_message");
        reply.send(Confirmation::ApprovedForSession).unwrap();
    };
    let (r, ()) = tokio::join!(call, user);
    assert!(payload(&r)["ts"].as_str().is_some(), "{lc:?}: {r}");
}

/// Expects one prompt for `tool` and declines it.
async fn asks_again(w: &mut World, prompts: &mut Prompts, tool: &str, args: Value) {
    let call = w.client.call(tool, args);
    let user = async {
        let (_req, reply) = next(prompts).await;
        reply.send(Confirmation::Declined).unwrap();
    };
    let (r, ()) = tokio::join!(call, user);
    assert_eq!(error_code(&r), "declined", "{tool}");
}

// Writes are rate limited per connection (a burst of three), so each test
// below makes at most three writes on one connection.

#[tokio::test]
async fn always_for_this_session_skips_later_prompts_for_that_tool_and_destination() {
    for lc in LIFECYCLES {
        let (mut w, mut prompts, _p) = world(lc).await;
        let before = w.settings.0.lock().unwrap().clone();
        allow_for_session(&mut w, &mut prompts, lc).await;

        // The same tool to the same conversation goes without a prompt.
        let r = w.client.call("send_message", send_args("second")).await;
        assert!(payload(&r)["ts"].as_str().is_some(), "{lc:?}: {r}");
        assert!(prompts.try_recv().is_err(), "{lc:?}: no second prompt");
        assert_eq!(w.backend.sent().len(), 2);

        // Another conversation still asks.
        let other = json!({"workspace": "fake/W1", "conversation": "C2", "text": "x"});
        asks_again(&mut w, &mut prompts, "send_message", other).await;

        // The audit log tells the two yeses apart.
        let rows = audit_rows(&w).await;
        let decisions: Vec<_> = rows.iter().rev().map(|r| r.decision).collect();
        assert_eq!(
            decisions,
            [
                Some(WriteDecision::ConfirmedForSession),
                Some(WriteDecision::SessionAllowed),
                Some(WriteDecision::Declined),
            ],
            "{lc:?}"
        );

        // Nothing was written to the settings.
        assert_eq!(*w.settings.0.lock().unwrap(), before, "{lc:?}");

        // A new session (the client reconnects, or the app restarts) asks again.
        let mut again = w.reconnect(lc).await;
        let call = again.call("send_message", send_args("new session"));
        let user = async {
            let (_req, reply) = next(&mut prompts).await;
            reply.send(Confirmation::Declined).unwrap();
        };
        let (r, ()) = tokio::join!(call, user);
        assert_eq!(error_code(&r), "declined", "{lc:?}");
        assert_eq!(w.backend.sent().len(), 2, "{lc:?}");
    }
}

#[tokio::test]
async fn always_for_this_session_keeps_the_policy_and_other_tools_asking() {
    for lc in LIFECYCLES {
        let (mut w, mut prompts, _p) = world(lc).await;
        allow_for_session(&mut w, &mut prompts, lc).await;

        // Turning writes off still refuses the write.
        ws_settings(&w, |s| s.allow_writes = false);
        let r = w.client.call("send_message", send_args("third")).await;
        assert_eq!(error_code(&r), "writes_off", "{lc:?}");
        ws_settings(&w, |s| s.allow_writes = true);

        // Another tool to the same conversation still asks.
        let react =
            json!({"workspace": "fake/W1", "conversation": "C1", "ts": TARGET, "emoji": "tada"});
        asks_again(&mut w, &mut prompts, "add_reaction", react).await;
        assert_eq!(w.backend.sent().len(), 1, "{lc:?}");
    }
}
