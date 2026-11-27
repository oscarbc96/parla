//! `FakeHost` (feature `testing`): a plugin's Rust code runs natively against
//! in-memory host functions with the plugin's grants and the documented
//! limits, and the test reads back what it did.

use parla_plugin_sdk::host::settings::SettingValue;
use parla_plugin_sdk::host::{
    self, composer, exec, http, secrets, settings, storage, ui, workspace,
};
use parla_plugin_sdk::testing::{FakeHost, Grants, Invocation, UiCall, WriteCall};
use parla_plugin_sdk::types::{Content, Context, Inline, MessagePage, MessageRef};
use parla_plugin_sdk::view::{button, card, column, image, text};
use parla_plugin_sdk::{Error, Plugin};

struct Greeter;

impl Plugin for Greeter {
    fn run_command(id: String, _context: Context) -> Result<(), Error> {
        host::log(host::LogLevel::Info, &format!("running {id}"))?;
        ui::toast(&format!("Hello from {id}"), ui::ToastKind::Success)
    }
}

fn ctx() -> Context {
    Context {
        workspace: None,
        conversation: None,
        thread: None,
        message: None,
    }
}

fn is<T: std::fmt::Debug>(r: Result<T, Error>, want: fn(&Error) -> bool) {
    match &r {
        Err(e) if want(e) => {}
        _ => panic!("unexpected {r:?}"),
    }
}
fn denied<T: std::fmt::Debug>(r: Result<T, Error>) {
    is(r, |e| matches!(e, Error::Denied(_)));
}
fn limit<T: std::fmt::Debug>(r: Result<T, Error>) {
    is(r, |e| matches!(e, Error::Limit(_)));
}
fn invalid<T: std::fmt::Debug>(r: Result<T, Error>) {
    is(r, |e| matches!(e, Error::Invalid(_)));
}
fn not_found<T: std::fmt::Debug>(r: Result<T, Error>) {
    is(r, |e| matches!(e, Error::NotFound));
}

const MANIFEST: &str = r#"
id = "demo-plugin"
name = "Demo"
version = "0.1.0"
api = "0.1"

[capabilities]
workspace_read = ["messages:read", "people:read"]
workspace_write = ["messages:write"]
network = ["api.example.com"]
storage = true
panel = true

[[capabilities.exec]]
program = "gh"
allow = [["pr", "view", "*"]]

[[settings]]
key = "max_lines"
type = "integer"
default = 20
title = "Max lines"

[[settings]]
key = "lang"
type = "enum"
options = ["en", "ca"]
title = "Language"
"#;

#[test]
fn fake_host_records_toast() {
    let fake = FakeHost::new();
    Greeter::run_command("wave".into(), ctx()).unwrap();
    assert_eq!(
        fake.toasts(),
        vec![("Hello from wave".to_string(), ui::ToastKind::Success)]
    );
    assert_eq!(
        fake.logs(),
        vec![(host::LogLevel::Info, "running wave".to_string())]
    );
}

#[test]
fn grants_come_from_the_manifest() {
    let fake = FakeHost::from_manifest(MANIFEST);
    storage::set(&storage::Scope::Global, "a", b"1").unwrap();
    ui::open_panel("main", &column().build()).unwrap();
    denied(ui::open_window(
        &ui::WindowSpec {
            title: "W".into(),
            width: None,
            height: None,
        },
        &column().build(),
    ));
    denied(ui::notify("t", "b", &ui::NotifyTarget::View("v".into())));
    denied(ui::copy("x"));
    assert!(
        matches!(settings::setting("max_lines").unwrap(), Some(SettingValue::Number(n)) if n == 20.0)
    );
    assert_eq!(settings::setting("lang").unwrap(), None);
    not_found(settings::setting("other"));
    drop(fake);
    let _none = FakeHost::new();
    denied(storage::get(&storage::Scope::Global, "a"));
}

#[test]
fn storage_enforces_its_limits() {
    let _fake = FakeHost::with_grants(Grants::none().storage());
    let g = storage::Scope::Global;
    limit(storage::set(&g, &"k".repeat(257), b"x"));
    limit(storage::set(&g, "big", &vec![0; 256 * 1024 + 1]));
    for i in 0..20 {
        storage::set(&g, &format!("v{i:02}"), &vec![0; 256 * 1024 - 8]).unwrap();
    }
    limit(storage::set(&g, "one-more", &[0; 1024]));
    storage::clear(&g).unwrap();
    for i in 0..2_000 {
        storage::set(&g, &format!("{i}"), b"").unwrap();
    }
    limit(storage::set(&g, "2001st", b""));
    invalid(storage::keys(&g, "", None, 0));
}

#[test]
fn storage_pages_keys_per_scope() {
    let _fake = FakeHost::with_grants(Grants::none().storage());
    let global = storage::Scope::Global;
    let ws = storage::Scope::Workspace("w1".into());
    storage::set(&global, "a", b"1").unwrap();
    storage::set(&ws, "a", b"2").unwrap();
    storage::set(&ws, "b", b"3").unwrap();
    assert_eq!(storage::get(&ws, "a").unwrap(), Some(b"2".to_vec()));
    let page = storage::keys(&ws, "", None, 1).unwrap();
    assert_eq!(page.items, vec!["a".to_string()]);
    let page = storage::keys(&ws, "", page.next.as_deref(), 1).unwrap();
    assert_eq!(page.items, vec!["b".to_string()]);
    assert_eq!(page.next, None);
    storage::clear(&ws).unwrap();
    assert_eq!(storage::get(&global, "a").unwrap(), Some(b"1".to_vec()));
}

#[test]
fn secrets_enforce_their_limits() {
    let _fake = FakeHost::new();
    limit(secrets::secret_set(&"k".repeat(129), "v"));
    limit(secrets::secret_set("k", &"v".repeat(4097)));
    for i in 0..32 {
        secrets::secret_set(&format!("k{i}"), "v").unwrap();
    }
    limit(secrets::secret_set("k32", "v"));
    secrets::secret_set("k0", "replaced").unwrap();
    assert_eq!(
        secrets::secret_get("k0").unwrap().as_deref(),
        Some("replaced")
    );
}

#[test]
fn toast_limits_size_rate_and_context() {
    let fake = FakeHost::new();
    limit(ui::toast(&"x".repeat(201), ui::ToastKind::Info));
    for _ in 0..6 {
        ui::toast("hi", ui::ToastKind::Info).unwrap();
    }
    limit(ui::toast("hi", ui::ToastKind::Info));
    fake.advance(60_000);
    ui::toast("hi", ui::ToastKind::Info).unwrap();
    fake.set_invocation(Invocation::Background);
    denied(ui::toast("hi", ui::ToastKind::Info));
    fake.add_view("note-1");
    ui::toast("hi", ui::ToastKind::Info).unwrap();
}

#[test]
fn notify_needs_its_grant_and_limits() {
    let _fake = FakeHost::with_grants(Grants::none().notifications());
    let target = ui::NotifyTarget::View("v".into());
    limit(ui::notify(&"t".repeat(101), "b", &target));
    limit(ui::notify("t", &"b".repeat(501), &target));
    ui::notify("t", "b", &target).unwrap();
}

#[test]
fn windows_and_panels_have_their_caps() {
    let _fake = FakeHost::with_grants(Grants::none().windows().panel());
    let spec = |t: &str| ui::WindowSpec {
        title: t.into(),
        width: None,
        height: None,
    };
    let tree = column().build();
    limit(ui::open_window(&spec(&"t".repeat(101)), &tree));
    let a = ui::open_window(&spec("A"), &tree).unwrap();
    ui::open_window(&spec("B"), &tree).unwrap();
    limit(ui::open_window(&spec("C"), &tree));
    ui::close(&a).unwrap();
    ui::open_window(&spec("C"), &tree).unwrap();
    let p = ui::open_panel("main", &tree).unwrap();
    limit(ui::open_panel("main", &tree));
    ui::close(&p).unwrap();
    ui::open_panel("main", &tree).unwrap();
}

#[test]
fn views_must_be_open_to_change() {
    let fake = FakeHost::with_grants(Grants::none().panel());
    let tree = column().build();
    not_found(ui::update("nope", &tree));
    not_found(ui::patch("nope", "x", &tree));
    not_found(ui::close("nope"));
    let v = ui::open_panel("main", &tree).unwrap();
    ui::update(&v, &tree).unwrap();
    ui::close(&v).unwrap();
    not_found(ui::update(&v, &tree));
    fake.add_view("unfurl-1");
    ui::update("unfurl-1", &tree).unwrap();
    let bad = text("leaf").child(text("child")).build();
    invalid(ui::update("unfurl-1", &bad));
    let calls = fake.ui_calls();
    assert!(matches!(&calls[0], UiCall::OpenPanel { panel, .. } if panel == "main"));
    assert_eq!(calls.len(), 4, "rejected calls are not recorded: {calls:?}");
}

#[test]
fn patch_replaces_a_subtree_and_keeps_ids_unique() {
    let fake = FakeHost::with_grants(Grants::none().panel());
    let base = column()
        .child(card().id("box").child(text("old")))
        .child(button("go", "Go"))
        .build();
    let v = ui::open_panel("main", &base).unwrap();
    invalid(ui::patch(
        &v,
        "box",
        &card().child(button("go", "Again")).build(),
    ));
    not_found(ui::patch(&v, "missing", &card().build()));
    ui::patch(
        &v,
        "box",
        &card()
            .id("box")
            .child(text("new"))
            .child(text("more"))
            .build(),
    )
    .unwrap();
    let now = fake.view_tree(&v).unwrap();
    assert_eq!(now.nodes.len(), 5);
    assert!(
        matches!(&now.nodes[2].kind, parla_plugin_sdk::view::NodeKind::Text(s) if s[0].text == "new")
    );
    assert_eq!(now.nodes[4].id.as_deref(), Some("go"));
}

#[test]
fn refresh_after_rules() {
    let fake = FakeHost::with_grants(Grants::none().windows().panel());
    not_found(ui::refresh_after("nope", 5));
    let v = ui::open_panel("main", &column().build()).unwrap();
    invalid(ui::refresh_after(&v, 4));
    ui::refresh_after(&v, 5).unwrap();
    limit(ui::refresh_after(&v, 5));
    assert!(fake.fire_refresh(&v));
    ui::refresh_after(&v, 5).unwrap();
    for i in 0..3 {
        fake.add_view(&format!("x{i}"));
        ui::refresh_after(&format!("x{i}"), 5).unwrap();
    }
    fake.add_view("x3");
    limit(ui::refresh_after("x3", 5));
}

#[test]
fn image_hosts_must_be_granted() {
    let _fake = FakeHost::with_grants(Grants::none().panel().network("img.example.com"));
    ui::open_panel(
        "main",
        &column()
            .child(image("https://img.example.com/a.png", "A"))
            .build(),
    )
    .unwrap();
    let other = column()
        .child(image("https://other.example.com/a.png", "A"))
        .build();
    denied(ui::update("view-1", &other));
}

#[test]
fn log_cuts_long_messages_and_keeps_debug_in_dev_mode() {
    let fake = FakeHost::new();
    host::log(host::LogLevel::Debug, "hidden").unwrap();
    host::log(host::LogLevel::Info, &"é".repeat(3000)).unwrap();
    let logs = fake.logs();
    assert_eq!(logs.len(), 1);
    assert!(logs[0].1.len() <= 4096 && logs[0].1.len() > 4000);
    fake.set_dev_mode(true);
    host::log(host::LogLevel::Debug, "shown").unwrap();
    assert_eq!(fake.logs()[1].1, "shown");
}

#[test]
fn t_fills_arguments_within_limits() {
    let fake = FakeHost::new();
    fake.set_string("hello", "Hello, { $name }!");
    assert_eq!(host::t("hello", &[("name", "Ada")]).unwrap(), "Hello, Ada!");
    assert_eq!(host::t("missing", &[]).unwrap(), "missing");
    limit(host::t(&"k".repeat(129), &[]));
    let args: Vec<(&str, &str)> = (0..17).map(|_| ("a", "b")).collect();
    limit(host::t("hello", &args));
}

#[test]
fn exec_follows_its_grant_and_limits() {
    let fake = FakeHost::with_grants(Grants::none().exec("gh", &[&["pr", "view", "*"]]));
    fake.on_exec(|_, args| {
        Ok(exec::ExecOutput {
            exit_code: 0,
            stdout: args.join(" ").into_bytes(),
            stderr: vec![],
        })
    });
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        exec::run("gh", &args(&["pr", "view", "12"]))
            .unwrap()
            .stdout,
        b"pr view 12"
    );
    denied(exec::run("gh", &args(&["pr", "merge", "12"])));
    denied(exec::run("git", &args(&["status"])));
    limit(exec::run("gh", &args(&["pr", "view", &"x".repeat(1025)])));
    limit(exec::run("gh", &vec!["a".to_string(); 33]));
    for _ in 1..30 {
        exec::run("gh", &args(&["pr", "view", "1"])).unwrap();
    }
    limit(exec::run("gh", &args(&["pr", "view", "1"])));
}

fn get(url: &str, headers: &[(&str, &str)]) -> http::Request {
    http::Request {
        method: http::Method::Get,
        url: url.into(),
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body: None,
        timeout_ms: 1_000,
    }
}

#[test]
fn http_follows_its_grant_and_rules() {
    let fake = FakeHost::with_grants(Grants::none().network("api.example.com"));
    fake.on_http(|req| {
        Ok(http::Response {
            status: 200,
            headers: vec![],
            body: req.url.clone().into_bytes(),
        })
    });
    let ok = http::fetch(&get(
        "https://api.example.com/x",
        &[("Accept", "text/plain")],
    ))
    .unwrap();
    assert_eq!(ok.body, b"https://api.example.com/x");
    invalid(http::fetch(&get("http://api.example.com/x", &[])));
    denied(http::fetch(&get("https://other.example.com/x", &[])));
    for h in ["User-Agent", "cookie", "Host", "Proxy-Authorization"] {
        invalid(http::fetch(&get("https://api.example.com/x", &[(h, "v")])));
    }
    let mut slow = get("https://api.example.com/x", &[]);
    slow.timeout_ms = 30_001;
    limit(http::fetch(&slow));
    assert_eq!(
        fake.http_requests().len(),
        1,
        "only accepted requests are recorded"
    );
    for _ in 1..120 {
        http::fetch(&get("https://api.example.com/x", &[])).unwrap();
    }
    limit(http::fetch(&get("https://api.example.com/x", &[])));
}

#[test]
fn errors_can_be_injected_per_call() {
    let fake = FakeHost::with_grants(Grants::none().network("api.example.com"));
    fake.inject("http.fetch", Error::Offline);
    is(http::fetch(&get("https://api.example.com/x", &[])), |e| {
        matches!(e, Error::Offline)
    });
    is(http::fetch(&get("https://api.example.com/x", &[])), |e| {
        matches!(e, Error::Offline)
    });
    fake.clear_injections();
    fake.inject_once("ui.toast", Error::Limit("toast".into()));
    limit(ui::toast("a", ui::ToastKind::Info));
    ui::toast("a", ui::ToastKind::Info).unwrap();
    fake.inject_once("workspace.history", Error::Denied("history:read".into()));
    denied(workspace::history("w", "c", None, 10));
}

#[test]
fn reads_are_stubbed_per_method_and_default_to_empty() {
    let fake = FakeHost::with_grants(Grants::none().read("messages:read").read("people:read"));
    fake.set_context(Context {
        workspace: Some("w".into()),
        conversation: Some("c".into()),
        thread: None,
        message: None,
    });
    // Unset reads: an empty page, or not-found for one item.
    assert!(
        workspace::history("w", "c", None, 10)
            .unwrap()
            .items
            .is_empty()
    );
    assert!(workspace::find_users("w", "ada", 10).unwrap().is_empty());
    not_found(workspace::get_user("w", "u1"));
    // `messages:read` covers only the conversation in context.
    denied(workspace::history("w", "elsewhere", None, 10));
    denied(workspace::list_conversations("w", &filter(), None, 10));
    invalid(workspace::history("w", "c", None, 201));
    fake.on_history(|_ws, conv, cursor, _limit| {
        assert_eq!(cursor, Some("next"));
        Ok(MessagePage {
            items: vec![],
            next: Some(format!("{conv}-2")),
        })
    });
    assert_eq!(
        workspace::history("w", "c", Some("next"), 10)
            .unwrap()
            .next
            .as_deref(),
        Some("c-2")
    );
    for _ in 4..60 {
        workspace::find_users("w", "ada", 10).unwrap();
    }
    limit(workspace::find_users("w", "ada", 10));
}

fn filter() -> parla_plugin_sdk::types::ConversationFilter {
    parla_plugin_sdk::types::ConversationFilter {
        kinds: vec![],
        unread_only: false,
        query: None,
    }
}

fn hello() -> Content {
    Content {
        spans: vec![Inline::Text("hi".into())],
        image_url: None,
    }
}

#[test]
fn writes_need_their_area_and_are_recorded() {
    let fake = FakeHost::with_grants(Grants::none().write("messages:write"));
    workspace::send("w", "c", &hello()).unwrap();
    workspace::reply("w", "c", "t1", &hello(), true).unwrap();
    let sent = fake.sent();
    assert_eq!(
        (sent[0].thread.as_deref(), sent[0].also_to_channel),
        (None, false)
    );
    assert_eq!(
        (sent[1].thread.as_deref(), sent[1].also_to_channel),
        (Some("t1"), true)
    );
    let msg = MessageRef {
        conversation: "c".into(),
        message: "m".into(),
    };
    denied(workspace::react("w", &msg, "tada"));
    assert!(matches!(&fake.writes()[1], WriteCall::Reply(s) if s.also_to_channel));
    fake.set_invocation(Invocation::EventHook);
    denied(workspace::send("w", "c", &hello()));
    fake.set_invocation(Invocation::UserAction);
    let long = Content {
        spans: vec![Inline::Text("x".repeat(16 * 1024 + 1))],
        image_url: None,
    };
    limit(workspace::send("w", "c", &long));
    for _ in 2..20 {
        workspace::send("w", "c", &hello()).unwrap();
    }
    limit(workspace::send("w", "c", &hello()));
}

#[test]
fn composer_needs_a_draft_and_limits_text() {
    let fake = FakeHost::new();
    is(composer::draft(), |e| matches!(e, Error::Unsupported));
    fake.set_draft("hi");
    composer::insert(" there").unwrap();
    assert_eq!(composer::draft().unwrap(), "hi there");
    limit(composer::replace_draft(&"x".repeat(40 * 1024 + 1)));
}

#[test]
fn clicks_only_actions_need_a_user_action() {
    let fake = FakeHost::with_grants(Grants::none().clipboard_write());
    ui::copy("x").unwrap();
    ui::open_url("https://example.com").unwrap();
    invalid(ui::open_url("http://example.com"));
    fake.set_invocation(Invocation::Background);
    denied(ui::copy("x"));
    denied(ui::open_url("https://example.com"));
    denied(ui::open_conversation("w", "c", None));
}
