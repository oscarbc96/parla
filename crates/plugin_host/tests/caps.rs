//! Capability grants, checks, risk and matchers.

use std::path::PathBuf;
use std::sync::Arc;

use domain::{Capabilities, ConversationId, ConversationKind, MessageTs};
use plugin_host::caps::{
    CallContext, CardRow, ConversationRef, Denied, Grant, HostCall, HostCallKind, MessageTarget,
    Need, Origin, Risk, SearchKind, UrlPattern, UserList, ViewId, area_capability, card_rows,
    check, check_result,
};
use plugin_host::manifest::{
    ArgPattern, ArgShape, Manifest, ManifestSource, ReadArea, WriteArea, parse,
};
use settings::WorkspaceKey;
use url::Url;

const HEAD: &str = r#"
id = "test-plugin"
name = "Test plugin"
version = "0.1.0"
api = "0.1"
"#;

fn manifest(rest: &str) -> Manifest {
    let src = format!("{HEAD}{rest}");
    parse(&src, ManifestSource::Package).unwrap_or_else(|e| panic!("{e}\n{src}"))
}

fn grant(caps: &str) -> Grant {
    Grant::from_manifest(&manifest(&format!("[capabilities]\n{caps}")))
}

fn ws() -> WorkspaceKey {
    WorkspaceKey::new("fake", "w1")
}

fn conv(id: &str, kind: ConversationKind) -> ConversationRef {
    ConversationRef {
        workspace: ws(),
        id: ConversationId(Arc::from(id)),
        kind,
    }
}

fn channel(id: &str) -> ConversationRef {
    conv(id, ConversationKind::Channel)
}

fn dm(id: &str) -> ConversationRef {
    conv(id, ConversationKind::Direct)
}

fn ts(s: &str) -> MessageTs {
    MessageTs(Arc::from(s))
}

fn all_caps() -> Capabilities {
    Capabilities {
        threads: true,
        edit: true,
        delete: true,
        presence: true,
        scheduled_send: true,
        search: true,
        reminders: true,
        status: true,
        pins: true,
        saved: true,
        member_lookup: true,
        files: true,
        read_state: true,
        search_messages: true,
        search_files: true,
        ..Capabilities::default()
    }
}

fn ctx(provider: &Capabilities) -> CallContext<'_> {
    CallContext {
        provider,
        conversation: None,
        message: None,
        origin: Origin::UserAction { composer: true },
        views: &[],
        workspaces: None,
    }
}

fn url(s: &str) -> Url {
    Url::parse(s).unwrap()
}

fn history(target: ConversationRef, message: Option<MessageTs>) -> HostCall {
    HostCall::Messages(MessageTarget {
        conversation: target,
        message,
    })
}

const FULL: &str = r#"
workspace_read = ["workspace:read", "conversations:read", "messages:read", "history:read",
  "dms:read", "people:read", "search:read", "files:read", "reminders:read",
  "scheduled:read", "saved:read", "pins:read", "status:read"]
workspace_write = ["messages:write", "messages:edit", "reactions:write", "reminders:write",
  "scheduled:write", "files:write", "status:write", "saved:write", "pins:write",
  "read_state:write"]
network = ["api.example.com"]
storage = true
windows = true
panel = true
notifications = true
clipboard_write = true

[[capabilities.exec]]
program = "gh"
allow = [["pr", "view", "*"]]
"#;

/// One call of every family, with the capability an empty grant lacks for
/// it (`None`: no capability needed). `every_host_fn_checks_grant` checks
/// the list against `HostCallKind::all()`, so a new variant fails the test
/// until it is listed here.
fn every_family() -> Vec<(HostCall, Option<Need>)> {
    let gh = PathBuf::from("/usr/bin/gh");
    let calls = vec![
        HostCall::Host,
        HostCall::Workspace {
            workspace: Some(ws()),
        },
        HostCall::Conversations { workspace: ws() },
        history(channel("C1"), None),
        HostCall::People { workspace: ws() },
        HostCall::Search {
            workspace: ws(),
            kind: SearchKind::Messages,
        },
        HostCall::Files(MessageTarget {
            conversation: channel("C1"),
            message: None,
        }),
        HostCall::List {
            workspace: ws(),
            list: UserList::Reminders,
        },
        HostCall::Write {
            workspace: ws(),
            area: WriteArea::Messages,
        },
        HostCall::Http {
            url: Url::parse("https://api.example.com/x").unwrap(),
        },
        HostCall::Storage,
        HostCall::Secrets,
        HostCall::Settings,
        HostCall::Panel,
        HostCall::Window,
        HostCall::View { view: ViewId(1) },
        HostCall::Toast,
        HostCall::Notify,
        HostCall::Copy,
        HostCall::OpenUrl {
            url: url("https://example.com/"),
        },
        HostCall::OpenConversation,
        HostCall::Composer,
        HostCall::Exec {
            program: "gh".into(),
            resolved: Some(gh),
            args: vec!["pr".into(), "view".into(), "12".into()],
        },
    ];
    calls
        .into_iter()
        .map(|c| {
            let need = match &c {
                HostCall::Host
                | HostCall::Secrets
                | HostCall::Settings
                | HostCall::View { .. }
                | HostCall::Toast
                | HostCall::OpenUrl { .. }
                | HostCall::OpenConversation
                | HostCall::Composer => None,
                HostCall::Workspace { .. } => Some(Need::Read(ReadArea::Workspace)),
                HostCall::Conversations { .. } => Some(Need::Read(ReadArea::Conversations)),
                // In context, so the narrower area is the one missing.
                HostCall::Messages(_) => Some(Need::Read(ReadArea::Messages)),
                HostCall::People { .. } => Some(Need::Read(ReadArea::People)),
                HostCall::Search { .. } => Some(Need::Read(ReadArea::Search)),
                HostCall::Files(_) => Some(Need::Read(ReadArea::Files)),
                HostCall::List { .. } => Some(Need::Read(ReadArea::Reminders)),
                HostCall::Write { .. } => Some(Need::Write(WriteArea::Messages)),
                HostCall::Http { .. } => Some(Need::Network("api.example.com".into())),
                HostCall::Storage => Some(Need::Storage),
                HostCall::Panel => Some(Need::Panel),
                HostCall::Window => Some(Need::Windows),
                HostCall::Notify => Some(Need::Notifications),
                HostCall::Copy => Some(Need::ClipboardWrite),
                HostCall::Exec { .. } => Some(Need::Exec("gh".into())),
            };
            (c, need)
        })
        .collect()
}

#[test]
fn every_host_fn_checks_grant() {
    let provider = all_caps();
    let empty = Grant::default();
    let mut full = grant(FULL);
    full.set_exec_path("gh", PathBuf::from("/usr/bin/gh"));
    let mut cx = ctx(&provider);
    cx.conversation = Some(channel("C1"));
    cx.views = &[ViewId(1)];
    let family = every_family();
    let listed: std::collections::HashSet<HostCallKind> =
        family.iter().map(|(c, _)| HostCallKind::from(c)).collect();
    for kind in HostCallKind::all() {
        assert!(listed.contains(&kind), "{kind:?} missing from every_family");
    }
    for (call, need) in family {
        match need {
            Some(n) => assert_eq!(
                check(&empty, &call, &cx),
                Err(Denied::Missing(n)),
                "{call:?}"
            ),
            None => assert_eq!(check(&empty, &call, &cx), Ok(()), "{call:?}"),
        }
        assert_eq!(check(&full, &call, &cx), Ok(()), "{call:?}");
    }
}

#[test]
fn history_in_context_only() {
    let provider = all_caps();
    let g = grant(r#"workspace_read = ["messages:read"]"#);
    let mut cx = ctx(&provider);
    cx.conversation = Some(channel("C1"));
    assert_eq!(check(&g, &history(channel("C1"), None), &cx), Ok(()));
    assert_eq!(
        check(&g, &history(channel("C2"), None), &cx),
        Err(Denied::Missing(Need::Read(ReadArea::History)))
    );
    // Same id in another workspace is not in context.
    let other = ConversationRef {
        workspace: WorkspaceKey::new("fake", "w2"),
        ..channel("C1")
    };
    assert!(check(&g, &history(other, None), &cx).is_err());
    cx.conversation = None;
    assert!(check(&g, &history(channel("C1"), None), &cx).is_err());

    let all = grant(r#"workspace_read = ["history:read"]"#);
    assert_eq!(check(&all, &history(channel("C2"), None), &cx), Ok(()));
}

#[test]
fn history_all_excludes_dms_without_dms_history() {
    let provider = all_caps();
    let cx = ctx(&provider);
    let all = grant(r#"workspace_read = ["history:read"]"#);
    assert_eq!(
        check(&all, &history(dm("D1"), None), &cx),
        Err(Denied::Missing(Need::Read(ReadArea::Dms)))
    );
    let group = conv("G1", ConversationKind::Group);
    assert!(check(&all, &history(group.clone(), None), &cx).is_err());
    let with_dms = grant(r#"workspace_read = ["history:read", "dms:read"]"#);
    assert_eq!(check(&with_dms, &history(dm("D1"), None), &cx), Ok(()));
    assert_eq!(check(&with_dms, &history(group, None), &cx), Ok(()));
}

#[test]
fn dm_invoked_message_is_readable() {
    let provider = all_caps();
    let g = grant(r#"workspace_read = ["messages:read"]"#);
    let mut cx = ctx(&provider);
    cx.conversation = Some(dm("D1"));
    cx.message = Some(ts("100.1"));
    assert_eq!(
        check(&g, &history(dm("D1"), Some(ts("100.1"))), &cx),
        Ok(())
    );
    // Another message, or the DM's history, is not readable.
    assert_eq!(
        check(&g, &history(dm("D1"), Some(ts("100.2"))), &cx),
        Err(Denied::Missing(Need::Read(ReadArea::Dms)))
    );
    assert!(check(&g, &history(dm("D1"), None), &cx).is_err());
    // A DM shown in the window is in context only with dms:read.
    cx.message = None;
    assert!(check(&g, &history(dm("D1"), None), &cx).is_err());
    let dms = grant(r#"workspace_read = ["messages:read", "dms:read"]"#);
    assert_eq!(check(&dms, &history(dm("D1"), None), &cx), Ok(()));
    assert!(check(&dms, &history(dm("D2"), None), &cx).is_err());
}

fn shape(parts: &[&str]) -> ArgShape {
    ArgShape(
        parts
            .iter()
            .map(|p| {
                if p.contains('*') {
                    ArgPattern::Glob((*p).into())
                } else {
                    ArgPattern::Literal((*p).into())
                }
            })
            .collect(),
    )
}

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| (*s).into()).collect()
}

#[test]
fn argv_star_never_matches_dotdot_query_url_or_flag() {
    let repos = shape(&["api", "repos/*"]);
    assert!(repos.matches(&args(&["api", "repos/ada/parla"])));
    for bad in [
        "repos/../user",
        "repos/a?x=1",
        "repos/a#frag",
        "repos/https://evil.example",
    ] {
        assert!(!repos.matches(&args(&["api", bad])), "{bad}");
    }
    let view = shape(&["pr", "view", "*", "--json", "*"]);
    assert!(view.matches(&args(&["pr", "view", "12", "--json", "title"])));
    // `*` never matches a flag.
    assert!(!view.matches(&args(&["pr", "view", "--web", "--json", "title"])));
    assert!(!view.matches(&args(&["pr", "view", "-w", "--json", "title"])));
    // Argument count is exact.
    assert!(!view.matches(&args(&["pr", "view", "12", "--json"])));
    assert!(!view.matches(&args(&["pr", "view", "12", "--json", "title", "x"])));
    // Literals are exact.
    assert!(!view.matches(&args(&["pr", "edit", "12", "--json", "title"])));
    assert!(shape(&["*"]).matches(&args(&[""])));
}

#[test]
fn exec_refused_when_program_moved_or_shape_not_allowed() {
    let provider = all_caps();
    let cx = ctx(&provider);
    let mut g = grant(FULL);
    let call = |resolved: Option<&str>, a: &[&str]| HostCall::Exec {
        program: "gh".into(),
        resolved: resolved.map(PathBuf::from),
        args: args(a),
    };
    // No path stored at consent: refused.
    assert_eq!(
        check(&g, &call(Some("/usr/bin/gh"), &["pr", "view", "1"]), &cx),
        Err(Denied::ProgramMoved)
    );
    g.set_exec_path("gh", PathBuf::from("/usr/bin/gh"));
    assert_eq!(
        check(&g, &call(Some("/usr/bin/gh"), &["pr", "view", "1"]), &cx),
        Ok(())
    );
    assert_eq!(
        check(&g, &call(Some("/tmp/gh"), &["pr", "view", "1"]), &cx),
        Err(Denied::ProgramMoved)
    );
    assert_eq!(
        check(&g, &call(Some("/usr/bin/gh"), &["pr", "merge", "1"]), &cx),
        Err(Denied::Missing(Need::Exec("gh".into())))
    );
}

#[test]
fn url_pattern_star_within_segment() {
    let p = UrlPattern::new("tracker.example.com/*").unwrap();
    let u = |s: &str| Url::parse(s).unwrap();
    assert!(p.matches(&u("https://tracker.example.com/123")));
    assert!(p.matches(&u("https://tracker.example.com/123?tab=1")));
    assert!(!p.matches(&u("https://tracker.example.com/123/comments")));
    assert!(!p.matches(&u("https://evil.example.com/123")));
    assert!(!p.matches(&u("https://tracker.example.com.evil.com/123")));
    assert!(!p.matches(&u("ftp://tracker.example.com/123")));
    let pr = UrlPattern::new("github.com/*/*/pull/*").unwrap();
    assert!(pr.matches(&u("https://github.com/ada/parla/pull/7")));
    assert!(!pr.matches(&u("https://github.com/ada/parla/issues/7")));
    let mid = UrlPattern::new("example.com/issue-*").unwrap();
    assert!(mid.matches(&u("https://example.com/issue-12")));
    assert!(!mid.matches(&u("https://example.com/pull-12")));
    assert!(UrlPattern::new("https://example.com/*").is_none());
    assert!(UrlPattern::new("*.example.com/*").is_none());
}

#[test]
fn network_host_exact_and_https_only() {
    let provider = all_caps();
    let cx = ctx(&provider);
    let g = grant(r#"network = ["api.example.com"]"#);
    let http = |s: &str| HostCall::Http {
        url: Url::parse(s).unwrap(),
    };
    assert_eq!(check(&g, &http("https://API.example.com/v1"), &cx), Ok(()));
    assert!(check(&g, &http("http://api.example.com/v1"), &cx).is_err());
    assert!(check(&g, &http("https://example.com/v1"), &cx).is_err());
    assert!(check(&g, &http("https://x.api.example.com/v1"), &cx).is_err());
    assert!(check(&g, &http("https://127.0.0.1/"), &cx).is_err());
}

fn ids(rows: &[CardRow]) -> Vec<&'static str> {
    rows.iter().map(|r| r.id).collect()
}

#[test]
fn implied_rows_listed() {
    let m = manifest(
        r#"
activation = ["transform:spell", "render:links", "hook:message", "filter",
  "message_action:remind", "slash:/later"]

[[composer_transforms]]
id = "spell"
kind = "check"
title = "Spelling"

[[renderers]]
id = "links"
patterns = ["tracker.example.com/*"]

[hooks]
events = ["message"]
notification_filter = true

[[message_actions]]
id = "remind"
title = "Remind me"

[[slash]]
name = "/later"
description = "Later"
"#,
    );
    let rows = card_rows(&m);
    let ids = ids(&rows);
    for id in [
        "plugin-cap-composer-check",
        "plugin-cap-renderer",
        "plugin-cap-hooks",
        "plugin-cap-notification-filter",
        "plugin-cap-adds-message-actions",
        "plugin-cap-adds-slash-commands",
    ] {
        assert!(ids.contains(&id), "{id} missing from {ids:?}");
    }
    let renderer = rows.iter().find(|r| r.id == "plugin-cap-renderer").unwrap();
    assert_eq!(renderer.risk, Risk::Medium);
    assert_eq!(renderer.args, vec!["tracker.example.com/*".to_owned()]);
    let filter = rows
        .iter()
        .find(|r| r.id == "plugin-cap-notification-filter")
        .unwrap();
    assert_eq!(filter.risk, Risk::Medium);
    // No network or exec: no exfiltration warning.
    assert!(!ids.contains(&"plugin-cap-can-send-what-it-reads"));
}

#[test]
fn risk_levels_follow_the_table() {
    let rows = card_rows(&manifest(&format!("[capabilities]\n{FULL}")));
    let risk = |id: &str| rows.iter().find(|r| r.id == id).map(|r| r.risk);
    assert_eq!(risk("plugin-cap-workspace-read"), Some(Risk::Low));
    assert_eq!(risk("plugin-cap-messages-read"), Some(Risk::Medium));
    assert_eq!(risk("plugin-cap-dms-read"), Some(Risk::Writes));
    assert_eq!(risk("plugin-cap-messages-write"), Some(Risk::Writes));
    assert_eq!(risk("plugin-cap-network"), Some(Risk::Medium));
    assert_eq!(risk("plugin-cap-storage"), Some(Risk::Low));
    assert_eq!(risk("plugin-cap-exec"), Some(Risk::Writes));
}

#[test]
fn exfiltration_warning_with_read_and_network() {
    let warn = "plugin-cap-can-send-what-it-reads";
    let read_net = card_rows(&manifest(
        r#"
[capabilities]
workspace_read = ["messages:read"]
network = ["api.example.com"]
"#,
    ));
    let row = read_net.iter().find(|r| r.id == warn).expect("warning");
    assert_eq!(row.args, vec!["api.example.com".to_owned()]);
    let read_only = card_rows(&manifest(
        "[capabilities]\nworkspace_read = [\"messages:read\"]\n",
    ));
    assert!(!ids(&read_only).contains(&warn));
    let net_only = card_rows(&manifest(
        "[capabilities]\nnetwork = [\"api.example.com\"]\n",
    ));
    assert!(!ids(&net_only).contains(&warn));
    let hooks_exec = card_rows(&manifest(
        r#"
activation = ["hook:message"]
[hooks]
events = ["message"]
[[capabilities.exec]]
program = "gh"
allow = [["pr", "view", "*"]]
"#,
    ));
    assert!(ids(&hooks_exec).contains(&warn));
}

#[test]
fn diff_shows_only_new() {
    let old = grant(
        r#"
workspace_read = ["messages:read"]
network = ["api.example.com"]
storage = true
"#,
    );
    let same = manifest(
        r#"
[capabilities]
workspace_read = ["messages:read"]
network = ["api.example.com"]
storage = true
"#,
    );
    assert!(old.diff(&same).is_empty());
    let grown = manifest(
        r#"
[capabilities]
workspace_read = ["messages:read", "people:read"]
network = ["api.example.com", "cdn.example.com"]
storage = true
panel = true
"#,
    );
    let d = old.diff(&grown);
    let rows = d.rows();
    assert_eq!(
        ids(rows),
        vec![
            "plugin-cap-people-read",
            "plugin-cap-network",
            "plugin-cap-panel",
            "plugin-cap-can-send-what-it-reads",
        ]
    );
    let net = rows.iter().find(|r| r.id == "plugin-cap-network").unwrap();
    assert_eq!(net.args, vec!["cdn.example.com".to_owned()]);
    // Dropping a capability is no growth.
    let shrunk = manifest("[capabilities]\nstorage = true\n");
    assert!(old.diff(&shrunk).is_empty());
}

#[test]
fn grant_round_trips_as_json() {
    let mut g = grant(FULL);
    g.set_exec_path("gh", PathBuf::from("/usr/bin/gh"));
    let json = serde_json::to_string(&g).unwrap();
    let back: Grant = serde_json::from_str(&json).unwrap();
    assert_eq!(back, g);
    assert!(json.contains("\"messages:read\""), "{json}");
}

#[test]
fn unsupported_area_when_provider_lacks_flag() {
    let none = Capabilities::default();
    let g = grant(FULL);
    let cx = ctx(&none);
    let unsupported = [
        HostCall::Files(MessageTarget {
            conversation: channel("C1"),
            message: None,
        }),
        HostCall::Search {
            workspace: ws(),
            kind: SearchKind::Messages,
        },
        HostCall::Search {
            workspace: ws(),
            kind: SearchKind::Files,
        },
        HostCall::Write {
            workspace: ws(),
            area: WriteArea::ReadState,
        },
        HostCall::Write {
            workspace: ws(),
            area: WriteArea::Files,
        },
        HostCall::List {
            workspace: ws(),
            list: UserList::Saved,
        },
    ];
    for call in unsupported {
        assert_eq!(check(&g, &call, &cx), Err(Denied::Unsupported), "{call:?}");
    }
    // Message search alone does not cover file search.
    let msgs = Capabilities {
        search_messages: true,
        ..Capabilities::default()
    };
    let cx = ctx(&msgs);
    let search = |kind| HostCall::Search {
        workspace: ws(),
        kind,
    };
    assert_eq!(check(&g, &search(SearchKind::Messages), &cx), Ok(()));
    assert_eq!(
        check(&g, &search(SearchKind::Files), &cx),
        Err(Denied::Unsupported)
    );
    // The mapping table covers every area.
    for a in ReadArea::ALL {
        let _ = area_capability((*a).into())(&none);
    }
    for a in WriteArea::ALL {
        let _ = area_capability((*a).into())(&none);
    }
    assert!(!area_capability(WriteArea::ReadState.into())(&none));
    assert!(area_capability(WriteArea::ReadState.into())(&all_caps()));
    assert!(area_capability(ReadArea::Files.into())(&all_caps()));
}

#[test]
fn hooks_cannot_write_and_gestures_need_a_user_action() {
    let provider = all_caps();
    let g = grant(FULL);
    let mut cx = ctx(&provider);
    cx.origin = Origin::Hook;
    let send = HostCall::Write {
        workspace: ws(),
        area: WriteArea::Messages,
    };
    assert_eq!(check(&g, &send, &cx), Err(Denied::HooksCannotWrite));
    for call in [
        HostCall::Copy,
        HostCall::OpenUrl {
            url: url("https://example.com/"),
        },
        HostCall::OpenConversation,
        HostCall::Composer,
        HostCall::Toast,
    ] {
        assert_eq!(
            check(&g, &call, &cx),
            Err(Denied::NeedsUserAction),
            "{call:?}"
        );
    }
    cx.origin = Origin::View;
    assert_eq!(check(&g, &HostCall::Toast, &cx), Ok(()));
    assert_eq!(
        check(&g, &HostCall::Copy, &cx),
        Err(Denied::NeedsUserAction)
    );
    cx.origin = Origin::UserAction { composer: false };
    assert_eq!(
        check(&g, &HostCall::Composer, &cx),
        Err(Denied::NeedsUserAction)
    );
    assert_eq!(check(&g, &HostCall::Copy, &cx), Ok(()));
}

#[test]
fn toast_from_hook_or_renderer_needs_an_open_view() {
    let provider = all_caps();
    let g = Grant::default();
    for origin in [Origin::Hook, Origin::Other, Origin::Mcp] {
        let mut cx = ctx(&provider);
        cx.origin = origin;
        assert_eq!(
            check(&g, &HostCall::Toast, &cx),
            Err(Denied::NeedsUserAction),
            "{origin:?}"
        );
        cx.views = &[ViewId(4)];
        assert_eq!(check(&g, &HostCall::Toast, &cx), Ok(()), "{origin:?}");
    }
}

#[test]
fn view_calls_need_the_plugin_to_own_the_view() {
    let provider = all_caps();
    let g = Grant::default();
    let mut cx = ctx(&provider);
    cx.views = &[ViewId(1), ViewId(2)];
    assert_eq!(check(&g, &HostCall::View { view: ViewId(2) }, &cx), Ok(()));
    assert_eq!(
        check(&g, &HostCall::View { view: ViewId(3) }, &cx),
        Err(Denied::NotOwner)
    );
    cx.views = &[];
    assert_eq!(
        check(&g, &HostCall::View { view: ViewId(1) }, &cx),
        Err(Denied::NotOwner)
    );
}

#[test]
fn open_url_is_https_only() {
    let provider = all_caps();
    let g = Grant::default();
    let cx = ctx(&provider);
    let open = |s: &str| HostCall::OpenUrl { url: url(s) };
    assert_eq!(check(&g, &open("https://example.com/a"), &cx), Ok(()));
    for bad in [
        "http://example.com/",
        "file:///etc/passwd",
        "javascript:alert(1)",
    ] {
        assert_eq!(check(&g, &open(bad), &cx), Err(Denied::NotHttps), "{bad}");
    }
}

#[test]
fn http_refuses_plain_http_ports_and_user_info() {
    let provider = all_caps();
    let cx = ctx(&provider);
    let g = grant(r#"network = ["api.example.com", "localhost"]"#);
    let http = |s: &str| HostCall::Http { url: url(s) };
    assert_eq!(
        check(&g, &http("http://api.example.com/"), &cx),
        Err(Denied::NotHttps)
    );
    assert_eq!(check(&g, &http("https://api.example.com./x"), &cx), Ok(()));
    assert_eq!(check(&g, &http("https://localhost/x"), &cx), Ok(()));
    assert_eq!(check(&g, &http("https://localhost./x"), &cx), Ok(()));
    // The default port is the same URL; any other port is refused.
    assert_eq!(
        check(&g, &http("https://api.example.com:443/"), &cx),
        Ok(())
    );
    for bad in [
        "https://api.example.com:8443/",
        "https://localhost:8080/",
        "https://user@api.example.com/",
        "https://user:pw@api.example.com/",
        "https://:pw@api.example.com/",
    ] {
        assert_eq!(check(&g, &http(bad), &cx), Err(Denied::InvalidUrl), "{bad}");
    }
    let no_local = grant(r#"network = ["api.example.com"]"#);
    assert_eq!(
        check(&no_local, &http("https://localhost/"), &cx),
        Err(Denied::Missing(Need::Network("localhost".into())))
    );
}

#[test]
fn search_results_obey_read_rules() {
    let provider = all_caps();
    let mut cx = ctx(&provider);
    cx.conversation = Some(channel("C1"));
    let hit = |c: ConversationRef| MessageTarget {
        conversation: c,
        message: Some(ts("1.1")),
    };
    let search = grant(r#"workspace_read = ["search:read", "messages:read"]"#);
    assert_eq!(check_result(&search, &hit(channel("C1")), &cx), Ok(()));
    assert_eq!(
        check_result(&search, &hit(channel("C2")), &cx),
        Err(Denied::Missing(Need::Read(ReadArea::History)))
    );
    assert_eq!(
        check_result(&search, &hit(dm("D1")), &cx),
        Err(Denied::Missing(Need::Read(ReadArea::Dms)))
    );
    let history = grant(r#"workspace_read = ["search:read", "history:read"]"#);
    assert_eq!(check_result(&history, &hit(channel("C2")), &cx), Ok(()));
    assert!(check_result(&history, &hit(dm("D1")), &cx).is_err());
    assert!(check_result(&history, &hit(conv("G1", ConversationKind::Group)), &cx).is_err());
    let dms = grant(r#"workspace_read = ["search:read", "history:read", "dms:read"]"#);
    assert_eq!(check_result(&dms, &hit(dm("D1")), &cx), Ok(()));
    // A result in a workspace the plugin is not enabled for is dropped.
    cx.workspaces = Some(&[]);
    assert_eq!(
        check_result(&dms, &hit(channel("C2")), &cx),
        Err(Denied::WorkspaceNotEnabled)
    );
}

#[test]
fn calls_outside_enabled_workspaces_are_refused() {
    let provider = all_caps();
    let g = grant(FULL);
    let enabled = [WorkspaceKey::new("fake", "w1")];
    let mut cx = ctx(&provider);
    cx.workspaces = Some(&enabled);
    let other = WorkspaceKey::new("fake", "w2");
    let people = |w: WorkspaceKey| HostCall::People { workspace: w };
    assert_eq!(check(&g, &people(ws()), &cx), Ok(()));
    assert_eq!(
        check(&g, &people(other.clone()), &cx),
        Err(Denied::WorkspaceNotEnabled)
    );
    let elsewhere = ConversationRef {
        workspace: other.clone(),
        ..channel("C1")
    };
    for call in [
        HostCall::Workspace {
            workspace: Some(other.clone()),
        },
        HostCall::Conversations {
            workspace: other.clone(),
        },
        history(elsewhere.clone(), None),
        HostCall::Search {
            workspace: other.clone(),
            kind: SearchKind::Messages,
        },
        HostCall::Files(MessageTarget {
            conversation: elsewhere,
            message: None,
        }),
        HostCall::List {
            workspace: other.clone(),
            list: UserList::Reminders,
        },
        HostCall::Write {
            workspace: other.clone(),
            area: WriteArea::Messages,
        },
    ] {
        assert_eq!(
            check(&g, &call, &cx),
            Err(Denied::WorkspaceNotEnabled),
            "{call:?}"
        );
    }
    // `list-workspaces` itself is allowed; the host filters its answer.
    assert_eq!(
        check(&g, &HostCall::Workspace { workspace: None }, &cx),
        Ok(())
    );
}

#[test]
fn star_refuses_split_forbidden_tokens_and_percent() {
    // A literal and the `*` together spell the token.
    let dot = shape(&["repos/.*"]);
    assert!(dot.matches(&args(&["repos/.x"])));
    assert!(!dot.matches(&args(&["repos/.."])));
    assert!(!dot.matches(&args(&["repos/../x"])));
    let colon = shape(&["*:*"]);
    assert!(colon.matches(&args(&["a:b"])));
    assert!(!colon.matches(&args(&["https://evil.example"])));
    let slash = shape(&["x:/*"]);
    assert!(!slash.matches(&args(&["x://evil"])));
    // Percent escapes never pass through a `*`.
    let repos = shape(&["api", "repos/*"]);
    for bad in ["repos/%2e%2e/user", "repos/a%3Fx", "repos/%2E%2E"] {
        assert!(!repos.matches(&args(&["api", bad])), "{bad}");
    }
    // A literal `%` in the shape is the shape's own.
    assert!(shape(&["100%"]).matches(&args(&["100%"])));
}

#[test]
fn group_dm_invoked_message_is_readable() {
    let provider = all_caps();
    let g = grant(r#"workspace_read = ["messages:read"]"#);
    let group = conv("G1", ConversationKind::Group);
    let mut cx = ctx(&provider);
    cx.conversation = Some(group.clone());
    cx.message = Some(ts("5.5"));
    assert_eq!(
        check(&g, &history(group.clone(), Some(ts("5.5"))), &cx),
        Ok(())
    );
    assert!(check(&g, &history(group.clone(), Some(ts("5.6"))), &cx).is_err());
    assert!(check(&g, &history(group, None), &cx).is_err());
}

#[test]
fn files_in_a_dm_follow_the_dm_rules() {
    let provider = all_caps();
    let file = |c: ConversationRef, m: Option<MessageTs>| {
        HostCall::Files(MessageTarget {
            conversation: c,
            message: m,
        })
    };
    let g = grant(r#"workspace_read = ["files:read", "messages:read"]"#);
    let mut cx = ctx(&provider);
    cx.conversation = Some(dm("D1"));
    cx.message = Some(ts("7.7"));
    assert_eq!(check(&g, &file(dm("D1"), Some(ts("7.7"))), &cx), Ok(()));
    assert_eq!(
        check(&g, &file(dm("D1"), None), &cx),
        Err(Denied::Missing(Need::Read(ReadArea::Dms)))
    );
    let all = grant(r#"workspace_read = ["files:read", "history:read", "dms:read"]"#);
    assert_eq!(check(&all, &file(dm("D2"), None), &cx), Ok(()));
    let no_files = grant(r#"workspace_read = ["messages:read", "dms:read"]"#);
    assert_eq!(
        check(&no_files, &file(dm("D1"), None), &cx),
        Err(Denied::Missing(Need::Read(ReadArea::Files)))
    );
}

#[test]
fn hook_reads_the_window_context_but_never_an_invoked_message() {
    let provider = all_caps();
    let g = grant(r#"workspace_read = ["messages:read"]"#);
    let mut cx = ctx(&provider);
    cx.origin = Origin::Hook;
    cx.conversation = Some(channel("C1"));
    assert_eq!(
        check(&g, &history(channel("C1"), Some(ts("1.1"))), &cx),
        Ok(())
    );
    assert!(check(&g, &history(channel("C2"), Some(ts("1.1"))), &cx).is_err());
    // A hook was not invoked on a message, so a DM in the window is not
    // readable through it.
    cx.conversation = Some(dm("D1"));
    cx.message = Some(ts("1.1"));
    assert_eq!(
        check(&g, &history(dm("D1"), Some(ts("1.1"))), &cx),
        Err(Denied::Missing(Need::Read(ReadArea::Dms)))
    );
    assert!(
        check_result(
            &g,
            &MessageTarget {
                conversation: dm("D1"),
                message: Some(ts("1.1")),
            },
            &cx
        )
        .is_err()
    );
}

#[test]
fn diff_lists_new_exec_shapes_only() {
    let old = grant(
        r#"
[[capabilities.exec]]
program = "gh"
allow = [["pr", "view", "*"]]
"#,
    );
    let grown = manifest(
        r#"
[[capabilities.exec]]
program = "gh"
allow = [["pr", "view", "*"], ["issue", "view", "*"]]

[[capabilities.exec]]
program = "git"
allow = [["log", "-1"]]
"#,
    );
    let d = old.diff(&grown);
    let exec: Vec<&CardRow> = d
        .rows()
        .iter()
        .filter(|r| r.id == "plugin-cap-exec")
        .collect();
    assert_eq!(exec.len(), 2);
    assert_eq!(
        exec[0].args,
        vec!["gh".to_owned(), "issue view *".to_owned()]
    );
    assert_eq!(exec[1].args, vec!["git".to_owned(), "log -1".to_owned()]);
    assert!(exec.iter().all(|r| r.risk == Risk::Writes));
}

#[test]
fn diff_warns_when_read_is_added_to_a_networked_plugin() {
    let old = grant(r#"network = ["api.example.com"]"#);
    let m = manifest(
        r#"
[capabilities]
network = ["api.example.com"]
workspace_read = ["people:read"]
"#,
    );
    let d = old.diff(&m);
    let warn = d
        .rows()
        .iter()
        .find(|r| r.id == "plugin-cap-can-send-what-it-reads")
        .expect("warning");
    assert_eq!(warn.args, vec!["api.example.com".to_owned()]);
    assert!(ids(d.rows()).contains(&"plugin-cap-people-read"));
    assert!(!ids(d.rows()).contains(&"plugin-cap-network"));
}
