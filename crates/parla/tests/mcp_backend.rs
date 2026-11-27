//! `AppBackend` over the workspace sessions: tools over the real
//! IPC socket see the session state, sends take the composer path, reads go
//! store first, and nothing waits for a provider on the main thread.

mod support;

use std::sync::Arc;
use std::time::Duration;

use domain::test_support::{cid, conv, member, msg};
use domain::{DeliveryState, ProviderEvent};
use gpui::TestAppContext;
use mcp::backend::Backend as _;
use parla::mcp_backend::{AppBackend, Hop};
use serde_json::json;
use support::mcp_env::{settings_json, start};
use support::{key, settle_until};

#[gpui::test]
async fn get_unreads_over_ipc_matches_session_state(cx: &mut TestAppContext) {
    let m = start(cx, &settings_json(true, ""));
    let listed = m
        .call("list_channels", json!({ "workspace": "fake/w1" }))
        .await;
    assert!(listed.to_string().contains("D1"), "{listed}");
    let session_has = m.w.read_with(cx, |w, cx| {
        w.session(&key("w1"))
            .unwrap()
            .read(cx)
            .state()
            .conversations
            .contains_key(&cid("D1"))
    });
    assert!(session_has);
    let unreads = m.call("get_unreads", json!({})).await;
    assert!(unreads.get("error").is_none(), "{unreads}");
}

fn open_d1(m: &support::mcp_env::Mcp, cx: &mut TestAppContext) {
    let s = m.w.read_with(cx, |w, _| w.session(&key("w1")).unwrap());
    s.update(cx, |s, cx| s.open(cid("D1"), cx));
    // The window loads from the store on another thread: wait for it.
    settle_until(cx, |cx| {
        s.read_with(cx, |s, _| s.state().windows.contains_key(&cid("D1")))
    });
}

fn d1_texts(m: &support::mcp_env::Mcp, cx: &mut TestAppContext) -> Vec<(String, bool)> {
    m.w.read_with(cx, |w, cx| {
        let s = w.session(&key("w1")).unwrap();
        let st = s.read(cx).state();
        st.windows[&cid("D1")]
            .messages()
            .iter()
            .map(|m| {
                (
                    m.text.to_string(),
                    matches!(m.state, DeliveryState::Delivered),
                )
            })
            .collect()
    })
}

#[gpui::test]
async fn agent_send_uses_outbox_and_dedupes_echo(cx: &mut TestAppContext) {
    let m = start(cx, &settings_json(true, ""));
    open_d1(&m, cx);
    let r = m
        .call(
            "send_message",
            json!({ "workspace": "fake/w1", "conversation": "D1", "text": "hello agent" }),
        )
        .await;
    assert!(r.get("error").is_none(), "{r}");
    // The provider's echo arrives as a live event, from another thread: wait
    // until the row is delivered and the outbox is drained.
    settle_until(cx, |cx| {
        d1_texts(&m, cx)
            .iter()
            .any(|(t, ok)| t == "hello agent" && *ok)
            && m.w.read_with(cx, |w, cx| {
                w.session(&key("w1"))
                    .unwrap()
                    .read(cx)
                    .state()
                    .outbox()
                    .is_empty()
            })
    });
    // The provider's echo arrives as a live event.
    let sent: Vec<_> = m.env.providers[0]
        .calls()
        .into_iter()
        .filter(|c| format!("{c:?}").contains("hello agent"))
        .collect();
    assert_eq!(sent.len(), 1, "one provider send");
    let rows = d1_texts(&m, cx);
    assert_eq!(
        rows.iter().filter(|(t, _)| t == "hello agent").count(),
        1,
        "{rows:?}"
    );
    assert!(
        rows.iter().any(|(t, ok)| t == "hello agent" && *ok),
        "{rows:?}"
    );
    m.w.read_with(cx, |w, cx| {
        let s = w.session(&key("w1")).unwrap();
        assert!(s.read(cx).state().outbox().is_empty(), "outbox drained");
    });
}

#[gpui::test]
async fn agent_send_offline_is_queued_and_reported(cx: &mut TestAppContext) {
    let m = start(cx, &settings_json(true, ""));
    open_d1(&m, cx);
    m.env.providers[0].push(ProviderEvent::Disconnected);
    settle_until(cx, |cx| {
        m.w.read_with(cx, |w, cx| {
            w.session(&key("w1")).unwrap().read(cx).state().connection
                == domain::Connection::Offline
        })
    });
    let r = m
        .call(
            "send_message",
            json!({ "workspace": "fake/w1", "conversation": "D1", "text": "later please" }),
        )
        .await;
    assert!(r.to_string().contains("queued"), "told it is queued: {r}");
    assert!(r.to_string().contains("offline"), "{r}");
    settle_until(cx, |cx| {
        m.w.read_with(cx, |w, cx| {
            w.session(&key("w1"))
                .unwrap()
                .read(cx)
                .state()
                .outbox()
                .len()
                == 1
        })
    });
    m.w.read_with(cx, |w, cx| {
        let s = w.session(&key("w1")).unwrap();
        let st = s.read(cx).state();
        assert_eq!(st.outbox().len(), 1, "left in the outbox");
        assert_eq!(&*st.outbox()[0].text, "later please");
    });
    assert!(
        !m.env.providers[0]
            .calls()
            .iter()
            .any(|c| format!("{c:?}").contains("later please")),
        "nothing reached the provider"
    );
}

#[gpui::test]
async fn page_reads_store_then_provider_and_persists(cx: &mut TestAppContext) {
    let m = start(cx, &settings_json(true, ""));
    let s = m.w.read_with(cx, |w, _| w.session(&key("w1")).unwrap());
    let before = m.env.providers[0].request_count();
    // The store may hold the seeded message already; ask for more than it has.
    let page = s
        .update(cx, |s, cx| s.page(cid("D1"), None, 30, true, cx))
        .await
        .unwrap();
    assert!(page.value.messages.iter().any(|x| &*x.text == "hi"));
    let again = s
        .update(cx, |s, cx| s.page(cid("D1"), None, 30, false, cx))
        .await
        .unwrap();
    assert_eq!(
        again.provider_calls, 0,
        "local only never calls the provider"
    );
    assert!(
        again.value.messages.iter().any(|x| &*x.text == "hi"),
        "persisted"
    );
    assert!(m.env.providers[0].request_count() >= before);
}

#[gpui::test]
async fn backend_never_blocks_main_thread(cx: &mut TestAppContext) {
    use domain::test_support::mid;
    let m = start(cx, &settings_json(true, ""));
    let main = std::thread::current().id();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let p = m.env.providers[1].clone();
    let c = conv("C9", "slow");
    p.add_conversation(c);
    p.add_member(member("u2", "Other"));
    let release_rx = std::sync::Mutex::new(release_rx);
    p.add_lazy_messages(&cid("C9"), move || {
        entered_tx.send(std::thread::current().id()).unwrap();
        let _ = release_rx
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(20));
        let _ = mid("C9", "1.000000");
        vec![msg("C9", "1.000000", "slow one")]
    });
    let backend = cx.update(|cx| Arc::new(AppBackend::new(Hop::start(cx))));
    let (b, ws, c9) = (backend.clone(), key("w2"), cid("C9"));
    let fut =
        m.rt.spawn(async move { b.messages(&ws, &c9, None, 10, true).await });
    let who =
        m.rt.spawn_blocking(move || entered_rx.recv_timeout(Duration::from_secs(10)))
            .await
            .unwrap();
    assert_ne!(who.unwrap(), main, "the provider ran off the main thread");
    // While the provider is stuck, entities still update on the main thread.
    let s = m.w.read_with(cx, |w, _| w.session(&key("w1")).unwrap());
    s.update(cx, |s, cx| s.open(cid("D1"), cx));
    settle_until(cx, |cx| {
        m.w.read_with(cx, |w, cx| {
            w.session(&key("w1")).unwrap().read(cx).selected() == Some(&cid("D1"))
        })
    });
    m.w.read_with(cx, |w, cx| {
        assert_eq!(
            w.session(&key("w1")).unwrap().read(cx).selected(),
            Some(&cid("D1"))
        );
    });
    release_tx.send(()).unwrap();
    let got = fut.await.unwrap().unwrap();
    assert_eq!(got.provider_calls, 1);
}
