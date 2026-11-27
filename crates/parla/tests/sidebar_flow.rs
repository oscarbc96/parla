//! M1 core flow end to end against the fake provider: sign in, bootstrap, the
//! sidebar lists the conversations and the first one opens. Plus the failure
//! paths that used to leave a silently empty window.

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use components::sign_in::{Candidate, SignIn, SignInError, SignInOption, SignInResult, SignInText};
use components::workspace_picker::RefreshStep;
use components::{SidebarRow, SidebarSource};
use domain::test_support::{FakeFactory, FakeProvider, Op, conv, member, uid};
use domain::{ChatProvider, ContentParser, ExtCodec, ProviderError, ProviderFactory};
use futures::channel::mpsc::UnboundedSender;
use futures::future::BoxFuture;
use gpui::{App, Task, TestAppContext, Window};
use parla::providers::ProviderRegistry;
use parla::root_view::{Phase, RootView};
use settings::{ConfigPaths, ProviderId, WorkspaceKey};
use store::{MemoryStore, Store};

fn init(cx: &mut TestAppContext) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        // Tests never depend on the OS language.
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    dir
}

fn store() -> Arc<dyn Store> {
    Arc::new(MemoryStore::default())
}

/// Accounts appear once the sign-in ran; `start` hands out the provider.
struct LateFactory {
    inner: FakeFactory,
    provider: Arc<FakeProvider>,
    signed_in: Arc<AtomicBool>,
}

impl ProviderFactory for LateFactory {
    fn catalog(&self) -> &'static i18n::Catalog {
        domain::test_support::fake_catalog()
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
            label: "Sign in with browser window".into(),
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

fn registry_with(provider: &Arc<FakeProvider>, signed_in: bool) -> ProviderRegistry {
    let signed_in = Arc::new(AtomicBool::new(signed_in));
    let mut registry = ProviderRegistry::new();
    registry.add(LateFactory {
        inner: FakeFactory::default(),
        provider: provider.clone(),
        signed_in: signed_in.clone(),
    });
    registry.add_sign_in(Rc::new(FakeSignIn {
        key: provider.workspace().clone(),
        signed_in,
    }));
    registry
}

fn provider() -> Arc<FakeProvider> {
    let p = Arc::new(FakeProvider::new("w1", uid("u1")));
    p.add_conversation(conv("C1", "general"));
    p.add_conversation(conv("C2", "random"));
    p.add_member(member("u1", "Me"));
    p
}

fn labels(rows: &[SidebarRow]) -> Vec<String> {
    rows.iter()
        .filter_map(|r| match r {
            SidebarRow::Conversation { label, .. } => Some(label.to_string()),
            SidebarRow::Header { .. } | SidebarRow::ShowMore { .. } => None,
        })
        .collect()
}

#[gpui::test]
async fn sign_in_then_bootstrap_fills_sidebar_and_opens_first_conversation(
    cx: &mut TestAppContext,
) {
    let _dir = init(cx);
    let p = provider();
    let registry = registry_with(&p, false);
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();
    let screen = view.read_with(cx, |v, _| v.sign_in_screen().unwrap().clone());
    screen.update_in(cx, |s, window, cx| s.choose("window", window, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert_eq!(*v.phase(), Phase::Ready));
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    let rows = session.read_with(cx, SidebarSource::rows);
    let got = labels(&rows);
    assert!(got.contains(&"general".to_string()), "rows: {got:?}");
    assert!(got.contains(&"random".to_string()), "rows: {got:?}");
    let first = rows
        .iter()
        .find_map(|r| match r {
            SidebarRow::Conversation { id, .. } => Some(id.clone()),
            SidebarRow::Header { .. } | SidebarRow::ShowMore { .. } => None,
        })
        .expect("a conversation row");
    session.update(cx, |s, cx| s.open(first.clone(), cx));
    cx.run_until_parked();
    view.read_with(cx, |v, cx| {
        assert_eq!(v.shown_conversation(), Some(&first));
        assert!(v.error_banner(cx).is_none());
    });
}

/// An account saved under a provider id the registry does not know (the
/// pre-rename id) used to leave "No accounts yet" and an empty sidebar.
struct ForeignAccounts(FakeFactory);

impl ProviderFactory for ForeignAccounts {
    fn catalog(&self) -> &'static i18n::Catalog {
        domain::test_support::fake_catalog()
    }

    fn id(&self) -> &ProviderId {
        self.0.id()
    }
    fn display_name(&self) -> &str {
        "Fake"
    }
    fn accounts(&self) -> Vec<WorkspaceKey> {
        vec![WorkspaceKey::new("retired", "T1")]
    }
    fn start(
        &self,
        _: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Arc<dyn ChatProvider>, ProviderError>> {
        Box::pin(async { Err(ProviderError::Other("never started".into())) })
    }
    fn ext_codec(&self) -> Arc<dyn ExtCodec> {
        self.0.ext_codec()
    }
    fn content_parser(&self) -> Arc<dyn ContentParser> {
        self.0.content_parser()
    }
}

#[gpui::test]
async fn account_under_unknown_provider_shows_error_banner_and_sign_in(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    let signed_in = Arc::new(AtomicBool::new(false));
    let mut registry = ProviderRegistry::new();
    registry.add(ForeignAccounts(FakeFactory::default()));
    registry.add_sign_in(Rc::new(FakeSignIn {
        key: p.workspace().clone(),
        signed_in,
    }));
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, cx| {
        assert!(matches!(v.phase(), Phase::Failed(_)));
        let banner = v.error_banner(cx).expect("visible error banner");
        assert!(banner.contains("provider"), "banner: {banner}");
        assert!(v.sign_in_screen().is_some(), "sign-in stays reachable");
    });
}

#[gpui::test]
async fn bootstrap_failure_shows_error_banner(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    p.fail_next(Op::Bootstrap, ProviderError::Other("boom".into()));
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(p));
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, cx| {
        assert_eq!(*v.phase(), Phase::Ready);
        let banner = v.error_banner(cx).expect("visible error banner");
        assert!(banner.contains("boom"), "banner: {banner}");
    });
}

#[gpui::test]
async fn stalled_bootstrap_shows_connecting_then_error_with_retry(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    p.hang_bootstrap();
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(p));
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, cx| {
        assert_eq!(*v.phase(), Phase::Ready);
        assert!(
            v.connecting(cx),
            "an empty layout must say it is connecting"
        );
        assert!(v.error_banner(cx).is_none());
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("connecting-banner").is_some());
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(60));
    cx.run_until_parked();
    view.read_with(cx, |v, cx| {
        assert!(!v.connecting(cx));
        assert!(v.error_banner(cx).is_some(), "a stall surfaces an error");
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("retry-bootstrap").is_some());
}

/// A real window must paint the rows: the sidebar list used to sit in a
/// wrapper with no height, so it laid out at zero height and drew nothing
/// although the session held every conversation.
#[gpui::test]
async fn sidebar_paints_its_rows_with_height(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(p));
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();
    let sidebar = view.read_with(cx, |v, _| v.sidebar().expect("ready").clone());
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(sidebar.read_with(cx, |s, _| s.rendered_rows()) > 0);
}

/// Group names show the other members' display names, and follow members that
/// arrive after the bootstrap (the directory loads in the background).
#[gpui::test]
async fn group_dm_label_follows_members_arriving_later(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    let mut g = conv("G1", "mpdm-raw-name-1");
    g.kind = domain::ConversationKind::Group;
    g.participants = ["amy", "me", "bob"]
        .iter()
        .map(|h| Arc::<str>::from(*h))
        .collect();
    p.add_conversation(g);
    let mut me = member("u1", "Me");
    me.handle = Arc::from("me");
    p.add_member(me);
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(p.clone()));
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    let group_label = |cx: &mut gpui::VisualTestContext| {
        labels(&session.read_with(cx, SidebarSource::rows))
            .into_iter()
            .find(|l| l.contains("amy") || l.contains("Amy"))
            .expect("group row")
    };
    assert_eq!(group_label(cx), "amy, bob");
    let mut amy = member("u2", "Amy A");
    amy.handle = Arc::from("amy");
    p.push_event(domain::ProviderEvent::MemberUpdated(amy));
    cx.run_until_parked();
    assert_eq!(group_label(cx), "Amy A, bob");
}

/// The "still connecting" strip must go once bootstrap answered, on a fresh
/// start and on a start that painted the cache first.
#[gpui::test]
async fn connecting_banner_clears_after_bootstrap(cx: &mut TestAppContext) {
    for cached in [false, true] {
        let _dir = init(cx);
        let p = provider();
        let st = store();
        if cached {
            let ws = p.workspace().clone();
            st.apply(
                &ws,
                domain::PersistOp::Conversations(vec![conv("C1", "general")]),
            )
            .await
            .unwrap();
        }
        let mut registry = ProviderRegistry::new();
        registry.add(FakeFactory::default().with_provider(p.clone()));
        let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, st, window, cx));
        cx.run_until_parked();
        view.read_with(cx, |v, cx| assert!(!v.connecting(cx), "cached: {cached}"));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(
            cx.debug_bounds("connecting-banner").is_none(),
            "banner still painted, cached: {cached}"
        );
    }
}
