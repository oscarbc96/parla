//! The MCP confirmation window: one prompt window per request,
//! queued one at a time, closed by the user, the app or the server's timeout,
//! also without a main window.

mod support;

use std::sync::Arc;
use std::time::Duration;

use components::{ApproveWrite, ApproveWriteForSession, ConfirmWriteView, DeclineWrite};
use domain::Capabilities;
use futures::FutureExt as _;
use gpui::{AnyWindowHandle, TestAppContext};
use mcp::audit::ClientInfo;
use mcp::backend::WorkspaceInfo;
use mcp::confirm::{Confirmation, Confirmer, WriteAction, WriteRequest};
use mcp::limits::ClientId;
use parla::mcp_backend::Hop;
use parla::write_confirm_window::{ConfirmQueue, WindowConfirmer};
use settings::WorkspaceKey;

fn request(text: &str) -> WriteRequest {
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
        as_user: None,
        action: WriteAction::Send {
            conversation_name: Arc::from("general"),
            thread: None,
            broadcast: false,
            text: Arc::from(text),
        },
    }
}

fn confirmer(cx: &mut TestAppContext) -> WindowConfirmer {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        WindowConfirmer::new(Hop::start(cx))
    })
}

fn settle(cx: &mut TestAppContext) {
    for _ in 0..5 {
        cx.run_until_parked();
    }
}

fn window(cx: &mut TestAppContext) -> Option<AnyWindowHandle> {
    cx.update(|cx| ConfirmQueue::window(cx))
}

fn prompt_text(cx: &mut TestAppContext) -> String {
    let w = window(cx)
        .expect("a prompt")
        .downcast::<ConfirmWriteView>()
        .unwrap();
    cx.update(|cx| w.read(cx).unwrap().body())
}

fn answer(cx: &mut TestAppContext, action: impl gpui::Action) {
    let w = window(cx).expect("a prompt");
    cx.dispatch_action(w, action);
    settle(cx);
}

/// Gives the prompt focus and waits out the arming delay.
fn arm(cx: &mut TestAppContext) {
    let w = window(cx).expect("a prompt");
    cx.update(|cx| {
        w.update(cx, |_, window, _| window.activate_window())
            .unwrap()
    });
    settle(cx);
    cx.executor().advance_clock(Duration::from_millis(1100));
    settle(cx);
}

#[gpui::test]
async fn closing_window_declines(cx: &mut TestAppContext) {
    let c = confirmer(cx);
    let mut f = c.confirm(request("hi"));
    settle(cx);
    let w = window(cx).expect("the prompt opened");
    assert_eq!(prompt_text(cx), "hi");
    cx.update(|cx| w.update(cx, |_, window, _| window.remove_window()).unwrap());
    settle(cx);
    assert_eq!((&mut f).now_or_never(), Some(Confirmation::Declined));
    assert!(window(cx).is_none());
}

#[gpui::test]
async fn requests_queue_one_window_at_a_time(cx: &mut TestAppContext) {
    let c = confirmer(cx);
    let mut first = c.confirm(request("first"));
    let mut second = c.confirm(request("second"));
    settle(cx);
    assert_eq!(cx.update(|cx| cx.windows().len()), 1);
    assert_eq!(cx.update(|cx| ConfirmQueue::waiting(cx)), 1);
    assert_eq!(prompt_text(cx), "first");
    assert!((&mut second).now_or_never().is_none());
    answer(cx, DeclineWrite);
    assert_eq!((&mut first).now_or_never(), Some(Confirmation::Declined));
    // The oldest goes first, the next one takes its place.
    assert_eq!(cx.update(|cx| cx.windows().len()), 1);
    assert_eq!(prompt_text(cx), "second");
    arm(cx);
    answer(cx, ApproveWrite);
    assert_eq!((&mut second).now_or_never(), Some(Confirmation::Approved));
    assert!(window(cx).is_none());
    assert_eq!(cx.update(|cx| cx.windows().len()), 0);
}

#[gpui::test]
async fn timeout_closes_window(cx: &mut TestAppContext) {
    let c = confirmer(cx);
    let f = c.confirm(request("hi"));
    let queued = c.confirm(request("later"));
    settle(cx);
    assert!(window(cx).is_some());
    // The server drops the wait at its timeout.
    drop(f);
    settle(cx);
    assert_eq!(prompt_text(cx), "later");
    drop(queued);
    settle(cx);
    assert!(window(cx).is_none());
    assert_eq!(cx.update(|cx| cx.windows().len()), 0);
}

#[gpui::test]
async fn headless_confirmation_opens_only_the_prompt(cx: &mut TestAppContext) {
    use parla::instance::{Hooks, Instance, LaunchMode};
    cx.executor().allow_parking();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let dir = tempfile::Builder::new()
        .prefix("t")
        .tempdir_in("/tmp")
        .unwrap();
    let socket = dir.path().join("run").join("mcp.sock");
    let handle = rt.handle().clone();
    cx.update(|cx| {
        let hooks = Hooks {
            show: std::rc::Rc::new(|_| {}),
            quit: std::rc::Rc::new(|_| {}),
        };
        Instance::install(cx, &handle, &socket, LaunchMode::Headless, hooks).unwrap();
        assert!(Instance::is_headless(cx));
    });
    let c = confirmer(cx);
    let mut f = c.confirm(request("hi"));
    settle(cx);
    assert_eq!(cx.update(|cx| cx.windows().len()), 1, "only the prompt");
    assert!(cx.update(|cx| Instance::is_headless(cx)), "still headless");
    answer(cx, DeclineWrite);
    assert_eq!((&mut f).now_or_never(), Some(Confirmation::Declined));
    assert_eq!(cx.update(|cx| cx.windows().len()), 0);
}

#[gpui::test]
async fn always_for_this_session_answers_for_the_session(cx: &mut TestAppContext) {
    let c = confirmer(cx);
    let mut f = c.confirm(request("hi"));
    settle(cx);
    arm(cx);
    answer(cx, ApproveWriteForSession);
    assert_eq!(
        (&mut f).now_or_never(),
        Some(Confirmation::ApprovedForSession)
    );
    assert!(window(cx).is_none());
}

#[gpui::test]
async fn the_idle_lock_declines_the_open_and_the_queued_prompts(cx: &mut TestAppContext) {
    let c = confirmer(cx);
    let mut first = c.confirm(request("first"));
    let mut second = c.confirm(request("second"));
    settle(cx);
    assert!(window(cx).is_some());
    cx.update(parla::write_confirm_window::decline_all);
    settle(cx);
    assert_eq!((&mut first).now_or_never(), Some(Confirmation::Declined));
    assert_eq!((&mut second).now_or_never(), Some(Confirmation::Declined));
    assert!(window(cx).is_none());
    assert_eq!(cx.update(|cx| ConfirmQueue::waiting(cx)), 0);
}

fn plugin_request(text: &str) -> write_confirm::WriteRequest {
    write_confirm::WriteRequest::new(
        write_confirm::Requester::Plugin {
            id: Arc::from("standup-bot"),
            name: Arc::from("Standup Bot"),
            source: write_confirm::PluginSource::Registry,
        },
        write_confirm::Workspace {
            key: WorkspaceKey::new("fake", "W1"),
            name: Arc::from("Acme"),
        },
        None,
        WriteAction::Send {
            conversation_name: Arc::from("general"),
            thread: None,
            broadcast: false,
            text: Arc::from(text),
        },
    )
}

#[gpui::test]
async fn plugin_and_mcp_requests_share_one_window_queue(cx: &mut TestAppContext) {
    let c = confirmer(cx);
    let mut agent = Confirmer::confirm(&c, request("from the agent"));
    let mut plugin = write_confirm::Confirmer::confirm(&c, plugin_request("from the plugin"));
    settle(cx);
    assert_eq!(cx.update(|cx| cx.windows().len()), 1);
    assert_eq!(prompt_text(cx), "from the agent");
    answer(cx, DeclineWrite);
    assert_eq!((&mut agent).now_or_never(), Some(Confirmation::Declined));
    assert_eq!(prompt_text(cx), "from the plugin");
    arm(cx);
    // Tick "Allow without asking in #general", then allow.
    let w = window(cx).unwrap().downcast::<ConfirmWriteView>().unwrap();
    let mut vcx = gpui::VisualTestContext::from_window(w.into(), cx);
    let at = vcx
        .debug_bounds("confirm-write-remember")
        .expect("drawn")
        .center();
    vcx.simulate_click(at, gpui::Modifiers::none());
    answer(cx, ApproveWrite);
    assert_eq!(
        (&mut plugin).now_or_never(),
        Some(write_confirm::Confirmation::Approved { remember: true })
    );
    assert!(window(cx).is_none());
}
