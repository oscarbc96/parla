//! `parla://` links and inbound provider permalinks.

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use components::sign_in::{Candidate, SignIn, SignInError, SignInOption, SignInResult, SignInText};
use components::workspace_picker::RefreshStep;
use domain::test_support::{FakeFactory, FakeProvider, conv, member, msg, uid};
use domain::{
    ChatProvider, ContentParser, ConversationId, ExtCodec, MessageTs, ParsedPermalink,
    ProviderError, ProviderFactory,
};
use futures::channel::mpsc::UnboundedSender;
use futures::future::BoxFuture;
use gpui::{App, AppContext as _, Task, TestAppContext, Window};
use parla::links::{ParlaLink, ResolvedLink, parse_parla_url, resolve_url};
use parla::providers::ProviderRegistry;
use parla::root_view::RootView;
use settings::{ConfigPaths, ProviderId, WorkspaceKey};
use store::{MemoryStore, Store};

fn provider_id() -> &'static str {
    concat!("sl", "ack")
}

fn init(cx: &mut TestAppContext) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    dir
}

fn store() -> Arc<dyn Store> {
    Arc::new(MemoryStore::default())
}

#[test]
fn parses_open_and_conversation_links() {
    let open =
        parse_parla_url("parla://open?url=https%3A%2F%2Fexample.test%2Fa%3Fb%3D1").expect("open");
    assert_eq!(
        open,
        ParlaLink::Open {
            url: url::Url::parse("https://example.test/a?b=1").unwrap()
        }
    );

    let c = parse_parla_url(&format!(
        "parla://{}/T0000001/conversation/C0000001?ts=1700000000.000100",
        provider_id()
    ))
    .expect("conversation");
    assert_eq!(
        c,
        ParlaLink::Conversation {
            ws: WorkspaceKey::new(provider_id(), "T0000001"),
            id: ConversationId(Arc::from("C0000001")),
            ts: Some(MessageTs(Arc::from("1700000000.000100"))),
        }
    );

    let c2 = parse_parla_url(&format!(
        "parla://{}/T0000001/conversation/C0000001",
        provider_id()
    ))
    .expect("no ts");
    assert_eq!(
        c2,
        ParlaLink::Conversation {
            ws: WorkspaceKey::new(provider_id(), "T0000001"),
            id: ConversationId(Arc::from("C0000001")),
            ts: None,
        }
    );

    let t = parse_parla_url(&format!(
        "parla://{}/T0000001/thread/C0000001/1700000000.000100",
        provider_id()
    ))
    .expect("thread");
    assert_eq!(
        t,
        ParlaLink::Thread {
            ws: WorkspaceKey::new(provider_id(), "T0000001"),
            conversation: ConversationId(Arc::from("C0000001")),
            root: MessageTs(Arc::from("1700000000.000100")),
        }
    );
}

#[test]
fn quick_switcher_paste_intercepts_only_lone_links() {
    use parla::links::lone_pasted_link;

    assert_eq!(
        lone_pasted_link("https://example.test/archives/C1/p1700000000000100"),
        Some("https://example.test/archives/C1/p1700000000000100")
    );
    let parla = format!("parla://{}/T1/conversation/C1", provider_id());
    assert_eq!(lone_pasted_link(&parla), Some(parla.as_str()));
    assert_eq!(
        lone_pasted_link("  http://example.test/path  "),
        Some("http://example.test/path")
    );

    // Text with a URL is not intercepted.
    assert_eq!(
        lone_pasted_link("see https://example.test/archives/C1/p1700000000000100"),
        None
    );
    assert_eq!(
        lone_pasted_link("https://example.test one https://example.test/two"),
        None
    );
    assert_eq!(lone_pasted_link("not a url"), None);
    assert_eq!(lone_pasted_link(""), None);
}

#[test]
fn rejects_other_schemes_and_hosts_without_a_provider() {
    assert!(parse_parla_url("https://example.test").is_none());
    let foreign = format!("{}://open", provider_id());
    assert!(parse_parla_url(&foreign).is_none());
    assert!(parse_parla_url("parla://other/x").is_none());
    assert!(parse_parla_url("parla://open?url=http://example.test/x").is_none());
    assert!(parse_parla_url("parla://open?url=javascript:alert(1)").is_none());
    assert!(parse_parla_url("parla://open?url=file:///etc/passwd").is_none());
    assert!(parse_parla_url("parla://open?url=ftp://example.test/x").is_none());
}

struct LateFactory {
    inner: FakeFactory,
    provider: Arc<FakeProvider>,
    signed_in: Arc<AtomicBool>,
}

impl ProviderFactory for LateFactory {
    fn catalog(&self) -> &'static i18n::Catalog {
        self.inner.catalog()
    }
    fn id(&self) -> &ProviderId {
        self.inner.id()
    }
    fn display_name(&self) -> &str {
        "Fake"
    }
    fn accounts(&self) -> Vec<WorkspaceKey> {
        if self.signed_in.load(Ordering::SeqCst) {
            vec![self.provider.workspace().clone()]
        } else {
            vec![]
        }
    }
    fn start(
        &self,
        _: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Arc<dyn ChatProvider>, ProviderError>> {
        let p = self.provider.clone() as Arc<dyn ChatProvider>;
        Box::pin(async move { Ok(p) })
    }
    fn ext_codec(&self) -> Arc<dyn ExtCodec> {
        self.inner.ext_codec()
    }
    fn content_parser(&self) -> Arc<dyn ContentParser> {
        self.inner.content_parser()
    }
    fn parse_permalink(&self, url: &url::Url) -> Option<ParsedPermalink> {
        self.inner.parse_permalink(url)
    }
}

struct FakeSignIn {
    key: WorkspaceKey,
    signed_in: Arc<AtomicBool>,
}

impl SignIn for FakeSignIn {
    fn provider_name(&self) -> String {
        "Fake".into()
    }
    fn notice(&self) -> Vec<SignInText> {
        vec![]
    }
    fn options(&self) -> Vec<SignInOption> {
        vec![SignInOption {
            id: "window".into(),
            label: "Sign in".into(),
            fields: vec![],
            help: None,
            copy: None,
        }]
    }
    fn run(
        &self,
        _: &str,
        _: Vec<(String, String)>,
        _: UnboundedSender<RefreshStep>,
        _: &mut Window,
        _: &mut App,
    ) -> Task<Result<SignInResult, SignInError>> {
        self.signed_in.store(true, Ordering::SeqCst);
        Task::ready(Ok(SignInResult::Workspaces(vec![Candidate {
            key: self.key.clone(),
            name: "w1".into(),
            ready: true,
        }])))
    }
    fn sign_out(&self, _: &WorkspaceKey, _: &mut App) -> Task<Result<(), String>> {
        Task::ready(Ok(()))
    }
}

struct FailingFactory {
    key: WorkspaceKey,
}

impl ProviderFactory for FailingFactory {
    fn catalog(&self) -> &'static i18n::Catalog {
        static F: std::sync::OnceLock<FakeFactory> = std::sync::OnceLock::new();
        F.get_or_init(FakeFactory::default).catalog()
    }
    fn id(&self) -> &ProviderId {
        &self.key.provider
    }
    fn display_name(&self) -> &str {
        "Failing"
    }
    fn accounts(&self) -> Vec<WorkspaceKey> {
        vec![self.key.clone()]
    }
    fn start(
        &self,
        _: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Arc<dyn ChatProvider>, ProviderError>> {
        Box::pin(async { Err(ProviderError::SessionExpired) })
    }
    fn ext_codec(&self) -> Arc<dyn ExtCodec> {
        FakeFactory::default().ext_codec()
    }
    fn content_parser(&self) -> Arc<dyn ContentParser> {
        FakeFactory::default().content_parser()
    }
}

fn provider() -> Arc<FakeProvider> {
    let p = Arc::new(FakeProvider::new("w1", uid("u1")));
    p.set_web_origin("https://acme.example.test");
    p.add_conversation(conv("C1", "general"));
    p.add_member(member("u1", "Me"));
    p.add_message(msg("C1", "1700000000.000100", "hello"));
    p
}

fn registry_for(p: Arc<FakeProvider>) -> ProviderRegistry {
    let signed_in = Arc::new(AtomicBool::new(true));
    let inner = FakeFactory::default()
        .with_provider(p.clone())
        .with_permalink_parser(|url| {
            let host = url.host_str()?;
            if host != "acme.example.test" && host != "nomatch.example.test" {
                return None;
            }
            let rest = url.path().strip_prefix("/archives/")?;
            let (c, ts) = rest.split_once("/p")?;
            if ts.len() != 16 || !ts.chars().all(|d| d.is_ascii_digit()) {
                return None;
            }
            let (sec, frac) = ts.split_at(10);
            let thread = url
                .query_pairs()
                .find_map(|(k, v)| (k == "thread_ts").then(|| v.into_owned()))
                .filter(|v| !v.is_empty())
                .map(|v| MessageTs(Arc::from(v)));
            // Hint is the host; only acme.example.test matches the signed-in session.
            Some(ParsedPermalink {
                workspace_hint: Arc::from(host),
                conversation: ConversationId(Arc::from(c)),
                ts: Some(MessageTs(Arc::from(format!("{sec}.{frac}")))),
                thread,
            })
        });
    let mut registry = ProviderRegistry::new();
    registry.add(LateFactory {
        inner,
        provider: p.clone(),
        signed_in: signed_in.clone(),
    });
    registry.add_sign_in(Rc::new(FakeSignIn {
        key: p.workspace().clone(),
        signed_in,
    }));
    registry
}

#[gpui::test]
async fn permalink_for_signed_in_workspace_jumps(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    let registry = registry_for(p.clone());
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();

    // Sanity: session started and signed in.
    view.read_with(cx, |v, _| assert!(v.session().is_some()));

    // An https permalink for the signed-in workspace jumps in-app.
    view.update_in(cx, |v, window, cx| {
        v.open_link_url(
            "https://acme.example.test/archives/C1/p1700000000000100".into(),
            window,
            cx,
        );
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.shown_conversation(),
            Some(&ConversationId(Arc::from("C1")))
        );
    });
    // Reuse of the existing jump-to-message path sets a jump target.
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.read_with(cx, |s, _| {
        assert!(
            s.state().jump.is_some(),
            "expected a jump target for the permalink message"
        );
    });
}

#[gpui::test]
async fn permalink_for_unknown_workspace_opens_browser(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    let registry = registry_for(p.clone());
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(v.session().is_some()));

    // Not parsed by any provider -> opens in browser as today.
    let resolved = cx.update(|_, cx| {
        resolve_url(
            "https://elsewhere.example.test/archives/C9/p1700000000000100",
            cx,
        )
    });
    assert!(
        matches!(resolved, ResolvedLink::OpenBrowser(_)),
        "{resolved:?}"
    );

    // Parsed by a provider but the workspace is not signed in (hint does not
    // match any signed-in session's known origin) -> opens in browser.
    let resolved = cx.update(|_, cx| {
        resolve_url(
            "https://nomatch.example.test/archives/C1/p1700000000000100",
            cx,
        )
    });
    assert!(
        matches!(resolved, ResolvedLink::OpenBrowser(_)),
        "{resolved:?}"
    );
}

#[gpui::test]
async fn reply_permalink_opens_thread_at_reply(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    // Root and a reply in C1.
    p.add_message(msg("C1", "1700000000.000050", "root"));
    let mut reply = msg("C1", "1700000000.000100", "reply");
    reply.thread_root = Some(domain::MessageTs(Arc::from("1700000000.000050")));
    p.add_message(reply);
    let registry = registry_for(p.clone());
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(v.session().is_some()));

    // A reply permalink (message p...0100 in thread thread_ts=...0050) must
    // open the thread for the root, at the reply.
    view.update_in(cx, |v, window, cx| {
        v.open_link_url(
            "https://acme.example.test/archives/C1/p1700000000000100?thread_ts=1700000000.000050"
                .into(),
            window,
            cx,
        );
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.shown_conversation(),
            Some(&ConversationId(Arc::from("C1")))
        );
        let panel = v.thread_panel().expect("thread panel opened");
        panel.read_with(cx, |panel, _| {
            assert_eq!(
                panel.root(),
                &domain::MessageId {
                    conversation: ConversationId(Arc::from("C1")),
                    ts: domain::MessageTs(Arc::from("1700000000.000050")),
                }
            );
            assert_eq!(
                panel.last_revealed(),
                Some(&domain::MessageId {
                    conversation: ConversationId(Arc::from("C1")),
                    ts: domain::MessageTs(Arc::from("1700000000.000100")),
                }),
                "reply permalink should reveal the reply in the thread"
            );
        });
    });
}

#[gpui::test]
async fn parla_thread_link_opens_thread(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    p.add_message(msg("C1", "1700000000.000050", "root"));
    let registry = registry_for(p.clone());
    let ws = p.workspace().clone();
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(v.session().is_some()));
    view.update_in(cx, |v, window, cx| {
        v.open_link_url(
            format!(
                "parla://{}/{}/thread/C1/1700000000.000050",
                ws.provider.0, ws.id.0
            ),
            window,
            cx,
        );
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.shown_conversation(),
            Some(&ConversationId(Arc::from("C1")))
        );
        let panel = v.thread_panel().expect("thread panel opened");
        panel.read_with(cx, |panel, _| {
            assert_eq!(
                panel.root(),
                &domain::MessageId {
                    conversation: ConversationId(Arc::from("C1")),
                    ts: domain::MessageTs(Arc::from("1700000000.000050")),
                }
            );
        });
    });
}

#[gpui::test]
async fn handle_open_url_delivers_to_window(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    let ws = p.workspace().clone();
    let registry = registry_for(p.clone());
    let handle = cx.update(|cx| {
        let r = registry.clone();
        cx.open_window(Default::default(), |window, cx| {
            cx.new(|cx| RootView::new(&r, store(), window, cx))
        })
        .unwrap()
    });
    cx.run_until_parked();
    let shown = std::rc::Rc::new(std::cell::Cell::new(false));
    let shown2 = shown.clone();
    cx.update(|cx| {
        parla::links::handle_open_url(
            format!("parla://{}/{}/conversation/C1", ws.provider.0, ws.id.0),
            &|_| shown2.set(true),
            Some(handle),
            cx,
        );
    });
    cx.run_until_parked();
    assert!(shown.get(), "show callback should run");
    handle
        .update(cx, |v, _, _| {
            assert_eq!(
                v.shown_conversation(),
                Some(&ConversationId(Arc::from("C1")))
            );
        })
        .ok();
}

#[gpui::test]
async fn handle_open_url_with_no_window_queues_link(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    p.add_conversation(conv("C2", "random"));
    let (registry, start) = gated_registry_for(p.clone());
    let ws = p.workspace().clone();
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    let shown = std::rc::Rc::new(std::cell::Cell::new(false));
    let shown2 = shown.clone();
    cx.update(|_, cx| {
        parla::links::handle_open_url(
            format!("parla://{}/{}/conversation/C2", ws.provider.0, ws.id.0),
            &|_| shown2.set(true),
            None,
            cx,
        );
    });
    assert!(shown.get(), "show callback should run");
    let _ = start.send(());
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.shown_conversation(),
            Some(&ConversationId(Arc::from("C2"))),
            "queued link should open once session is ready"
        );
    });
}

struct PendingFactory {
    key: WorkspaceKey,
}

impl ProviderFactory for PendingFactory {
    fn catalog(&self) -> &'static i18n::Catalog {
        static F: std::sync::OnceLock<FakeFactory> = std::sync::OnceLock::new();
        F.get_or_init(FakeFactory::default).catalog()
    }
    fn id(&self) -> &ProviderId {
        &self.key.provider
    }
    fn display_name(&self) -> &str {
        "Pending"
    }
    fn accounts(&self) -> Vec<WorkspaceKey> {
        vec![self.key.clone()]
    }
    fn start(
        &self,
        _: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Arc<dyn ChatProvider>, ProviderError>> {
        Box::pin(std::future::pending())
    }
    fn ext_codec(&self) -> Arc<dyn ExtCodec> {
        FakeFactory::default().ext_codec()
    }
    fn content_parser(&self) -> Arc<dyn ContentParser> {
        FakeFactory::default().content_parser()
    }
}

struct GatedFactory {
    inner: FakeFactory,
    provider: Arc<FakeProvider>,
    signed_in: Arc<AtomicBool>,
    gate: std::sync::Mutex<Option<futures::channel::oneshot::Receiver<()>>>,
}

impl ProviderFactory for GatedFactory {
    fn catalog(&self) -> &'static i18n::Catalog {
        self.inner.catalog()
    }
    fn id(&self) -> &ProviderId {
        self.inner.id()
    }
    fn display_name(&self) -> &str {
        "Fake"
    }
    fn accounts(&self) -> Vec<WorkspaceKey> {
        if self.signed_in.load(Ordering::SeqCst) {
            vec![self.provider.workspace().clone()]
        } else {
            vec![]
        }
    }
    fn start(
        &self,
        _: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Arc<dyn ChatProvider>, ProviderError>> {
        let p = self.provider.clone() as Arc<dyn ChatProvider>;
        let rx = self.gate.lock().unwrap().take();
        Box::pin(async move {
            if let Some(rx) = rx {
                let _ = rx.await;
            }
            Ok(p)
        })
    }
    fn ext_codec(&self) -> Arc<dyn ExtCodec> {
        self.inner.ext_codec()
    }
    fn content_parser(&self) -> Arc<dyn ContentParser> {
        self.inner.content_parser()
    }
    fn parse_permalink(&self, url: &url::Url) -> Option<ParsedPermalink> {
        self.inner.parse_permalink(url)
    }
}

fn gated_registry_for(
    p: Arc<FakeProvider>,
) -> (ProviderRegistry, futures::channel::oneshot::Sender<()>) {
    let (tx, rx) = futures::channel::oneshot::channel();
    let signed_in = Arc::new(AtomicBool::new(true));
    let inner = FakeFactory::default()
        .with_provider(p.clone())
        .with_permalink_parser(|url| {
            let host = url.host_str()?;
            if host != "acme.example.test" && host != "nomatch.example.test" {
                return None;
            }
            let rest = url.path().strip_prefix("/archives/")?;
            let (c, ts) = rest.split_once("/p")?;
            if ts.len() != 16 || !ts.chars().all(|d| d.is_ascii_digit()) {
                return None;
            }
            let (sec, frac) = ts.split_at(10);
            let thread = url
                .query_pairs()
                .find_map(|(k, v)| (k == "thread_ts").then(|| v.into_owned()))
                .filter(|v| !v.is_empty())
                .map(|v| MessageTs(Arc::from(v)));
            Some(ParsedPermalink {
                workspace_hint: Arc::from(host),
                conversation: ConversationId(Arc::from(c)),
                ts: Some(MessageTs(Arc::from(format!("{sec}.{frac}")))),
                thread,
            })
        });
    let mut registry = ProviderRegistry::new();
    registry.add(GatedFactory {
        inner,
        provider: p.clone(),
        signed_in: signed_in.clone(),
        gate: std::sync::Mutex::new(Some(rx)),
    });
    registry.add_sign_in(Rc::new(FakeSignIn {
        key: p.workspace().clone(),
        signed_in,
    }));
    (registry, tx)
}

#[gpui::test]
async fn plain_https_link_while_starting_opens_immediately(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let key = WorkspaceKey::new("fake", "w1");
    let mut registry = ProviderRegistry::new();
    registry.add(PendingFactory { key });
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    view.update_in(cx, |v, window, cx| {
        assert!(
            cx.global::<parla::workspaces::WorkspacesHandle>()
                .0
                .read(cx)
                .is_starting(),
            "workspace start must still be outstanding"
        );
        v.open_link_url("https://example.com/plain".into(), window, cx);
    });
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://example.com/plain"),
        "plain links must open at once even while workspaces start"
    );
}

#[gpui::test]
async fn timeline_link_handler_falls_back_to_browser(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    let registry = registry_for(p.clone());
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();
    let before = view.read_with(cx, |v, _| v.shown_conversation().cloned());
    view.update_in(cx, |v, window, cx| {
        v.open_link_url("https://example.com/docs".into(), window, cx);
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(v.shown_conversation().cloned(), before);
        assert!(v.thread_panel().is_none());
    });
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/docs"));
}

#[gpui::test]
async fn back_forward_after_permalink_jump(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    p.add_conversation(conv("C2", "random"));
    let ws = p.workspace().clone();
    let registry = registry_for(p.clone());
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();

    view.update_in(cx, |v, window, cx| {
        v.navigate(
            ws.clone(),
            ConversationId(Arc::from("C2")),
            None,
            window,
            cx,
        );
    });
    cx.run_until_parked();
    view.update_in(cx, |v, window, cx| {
        v.open_link_url(
            "https://acme.example.test/archives/C1/p1700000000000100".into(),
            window,
            cx,
        );
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.shown_conversation(),
            Some(&ConversationId(Arc::from("C1")))
        );
    });

    view.update_in(cx, |v, window, cx| v.go_back(window, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.shown_conversation(),
            Some(&ConversationId(Arc::from("C2")))
        );
    });

    view.update_in(cx, |v, window, cx| v.go_forward(window, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.shown_conversation(),
            Some(&ConversationId(Arc::from("C1")))
        );
    });
}

#[gpui::test]
async fn parla_conversation_link_received_before_session_ready_is_queued(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    p.add_conversation(conv("C2", "random"));
    let (registry, start) = gated_registry_for(p.clone());
    let ws = p.workspace().clone();
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    // Delivered before the workspace session has started: must be queued and
    // resolved once the session is added (no timers).
    let still_starting = cx.update(|_, cx| {
        cx.global::<parla::workspaces::WorkspacesHandle>()
            .0
            .read(cx)
            .is_starting()
    });
    assert!(still_starting, "workspace start must still be outstanding");
    view.update_in(cx, |v, window, cx| {
        v.open_link_url(
            format!("parla://{}/{}/conversation/C2", ws.provider.0, ws.id.0),
            window,
            cx,
        );
    });
    let _ = start.send(());
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.shown_conversation(),
            Some(&ConversationId(Arc::from("C2"))),
            "queued link should open once session is ready"
        );
    });
}

#[gpui::test]
async fn parla_link_for_workspace_that_never_starts_is_dropped_after_start(
    cx: &mut TestAppContext,
) {
    let _dir = init(cx);
    let key = WorkspaceKey::new("fake", "w1");
    let mut registry = ProviderRegistry::new();
    registry.add(FailingFactory { key: key.clone() });
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    view.update_in(cx, |v, window, cx| {
        v.open_link_url(
            format!("parla://{}/{}/conversation/C1", key.provider.0, key.id.0),
            window,
            cx,
        );
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert!(v.shown_conversation().is_none());
        assert!(v.session().is_none(), "workspace never started");
        assert!(v.thread_panel().is_none());
    });
}

#[gpui::test]
async fn queued_https_permalink_with_no_matching_workspace_opens_in_browser_after_starts_finish(
    cx: &mut TestAppContext,
) {
    let _dir = init(cx);
    let p = provider();
    let (registry, start) = gated_registry_for(p.clone());
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    view.update_in(cx, |v, window, cx| {
        assert!(
            cx.global::<parla::workspaces::WorkspacesHandle>()
                .0
                .read(cx)
                .is_starting(),
            "workspace start must still be outstanding"
        );
        v.open_link_url(
            "https://nomatch.example.test/archives/C1/p1700000000000100".into(),
            window,
            cx,
        );
    });
    assert_eq!(
        cx.opened_url(),
        None,
        "unmatched permalink must not open in browser before starts finish"
    );
    let _ = start.send(());
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert!(v.shown_conversation().is_none());
        assert!(v.thread_panel().is_none());
    });
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://nomatch.example.test/archives/C1/p1700000000000100")
    );
}
