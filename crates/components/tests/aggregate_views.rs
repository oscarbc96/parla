mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use components::{
    AggregateEntry, AggregateEvent, AggregateKind, AggregateSource, AggregateTarget, AggregateView,
    MentionItem, MentionLine, PreviewLine, ScheduledItem, SendLaterConfig, UnreadItem,
    WorkspaceTag,
};
use domain::test_support::{cid, ts};
use domain::{FixedNow, LocalScheduleState, MessageId, ScheduleId};
use gpui::{App, AppContext as _, Entity, TestAppContext, VisualTestContext};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use settings::{Clock, WorkspaceKey};

/// 2026-11-15 10:00:00 UTC.
const NOW: i64 = 1_794_736_800;

fn tag(id: &str, name: &str) -> WorkspaceTag {
    WorkspaceTag {
        key: WorkspaceKey::new("fake", id),
        name: name.to_string().into(),
        accent: None,
    }
}

fn unread(ws: &str, name: &str, c: &str, mentions: u32, latest: &str) -> UnreadItem {
    UnreadItem {
        workspace: tag(ws, name),
        conversation: cid(c),
        label: format!("#{c}").into(),
        unread: true,
        mentions,
        latest: Some(ts(latest)),
        last_read: None,
    }
}

#[derive(Default)]
struct Src {
    unreads: Vec<UnreadItem>,
    mentions: Vec<MentionItem>,
    scheduled: Vec<ScheduledItem>,
    marked: u32,
    marked_one: Vec<(WorkspaceKey, domain::ConversationId, domain::MessageTs)>,
    previews: std::collections::HashMap<domain::ConversationId, Vec<PreviewLine>>,
    preview_reads: std::cell::Cell<u32>,
}

impl AggregateSource for Src {
    fn unreads(&self, _: &App) -> Vec<UnreadItem> {
        self.unreads.clone()
    }
    fn mentions(&self, _: &App) -> Vec<MentionItem> {
        self.mentions.clone()
    }
    fn scheduled(&self, _: &App) -> Vec<ScheduledItem> {
        self.scheduled.clone()
    }
    fn mark_all_read(&mut self, _: &mut App) {
        self.marked += 1;
    }
    fn mark_read(
        &mut self,
        workspace: &WorkspaceKey,
        c: &domain::ConversationId,
        latest: domain::MessageTs,
        _: &mut App,
    ) {
        self.marked_one.push((workspace.clone(), c.clone(), latest));
    }
    fn previews(&self, _: &WorkspaceKey, c: &domain::ConversationId, _: &App) -> Vec<PreviewLine> {
        self.preview_reads.set(self.preview_reads.get() + 1);
        self.previews.get(c).cloned().unwrap_or_default()
    }
    fn config(&self, _: &App) -> SendLaterConfig {
        SendLaterConfig {
            presets: vec![],
            zone: TimeZone::get("Europe/Madrid").expect("tz"),
            clock: Clock::H24,
            server: true,
        }
    }
}

type Events = Rc<RefCell<Vec<AggregateEvent>>>;

fn setup(
    cx: &mut TestAppContext,
    kind: AggregateKind,
    src: Src,
) -> (
    Entity<Src>,
    Entity<AggregateView<Src>>,
    Events,
    &mut VisualTestContext,
) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        components::set_now(cx, Arc::new(FixedNow(Timestamp::from_second(NOW).unwrap())));
    });
    let src = cx.new(|_| src);
    let s = src.clone();
    let (view, vcx) = cx.add_window_view(move |_w, cx| AggregateView::new(kind, s, cx));
    let events: Events = Rc::default();
    let sink = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, e: &AggregateEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach();
    });
    (src, view, events, vcx)
}

fn entries(v: &Entity<AggregateView<Src>>, cx: &mut VisualTestContext) -> Vec<AggregateEntry> {
    v.read_with(cx, |v, cx| v.entries(cx))
}

#[gpui::test]
fn unreads_from_three_workspaces_ordered(cx: &mut TestAppContext) {
    let src = Src {
        unreads: vec![
            unread("w2", "Beta", "C2", 3, "9.000001"),
            unread("w1", "Alpha", "C1", 0, "8.000001"),
            unread("w3", "Gamma", "C3", 0, "2.000001"),
        ],
        ..Default::default()
    };
    let (_s, v, _e, vcx) = setup(cx, AggregateKind::Unreads, src);
    let got = entries(&v, vcx);
    let names: Vec<&str> = got.iter().map(|e| &*e.workspace.name).collect();
    assert_eq!(names, ["Beta", "Alpha", "Gamma"], "source order is kept");
    assert_eq!(got[0].title, "#C2");
    assert_eq!(got[0].mentions, 3);
    assert_eq!(
        got[1].target,
        AggregateTarget::Conversation(cid("C1")),
        "an unread row opens the conversation"
    );
    vcx.run_until_parked();
    for i in 0..got.len() {
        let sel: &'static str = Box::leak(format!("aggregate-row-{i}").into_boxed_str());
        assert!(vcx.debug_bounds(sel).is_some(), "{sel} is drawn");
    }
}

#[gpui::test]
fn row_opens_in_its_workspace(cx: &mut TestAppContext) {
    let src = Src {
        unreads: vec![
            unread("w1", "Alpha", "C1", 0, "8.000001"),
            unread("w2", "Beta", "C2", 0, "7.000001"),
        ],
        ..Default::default()
    };
    let (_s, v, events, vcx) = setup(cx, AggregateKind::Unreads, src);
    v.update(vcx, |v, cx| v.open(1, cx));
    assert_eq!(
        *events.borrow(),
        [AggregateEvent::Open {
            workspace: WorkspaceKey::new("fake", "w2"),
            target: AggregateTarget::Conversation(cid("C2")),
        }]
    );
}

#[gpui::test]
fn mentions_show_held_messages(cx: &mut TestAppContext) {
    let id = |t: &str| MessageId {
        conversation: cid("C1"),
        ts: ts(t),
    };
    let src = Src {
        mentions: vec![MentionItem {
            row: unread("w1", "Alpha", "C1", 2, "4.000001"),
            messages: vec![
                MentionLine {
                    id: id("4.000001"),
                    author: "Ana".into(),
                    text: "ping @me please".into(),
                },
                MentionLine {
                    id: id("2.000001"),
                    author: "Ben".into(),
                    text: "and again".into(),
                },
            ],
        }],
        ..Default::default()
    };
    let (_s, v, events, vcx) = setup(cx, AggregateKind::Mentions, src);
    let got = entries(&v, vcx);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].lines.len(), 2);
    assert!(got[0].lines[0].contains("Ana") && got[0].lines[0].contains("ping @me please"));
    assert_eq!(got[0].tag, None);
    assert_eq!(got[0].target, AggregateTarget::Message(id("4.000001")));
    v.update(vcx, |v, cx| v.open(0, cx));
    assert!(matches!(
        &events.borrow()[0],
        AggregateEvent::Open {
            target: AggregateTarget::Message(_),
            ..
        }
    ));
}

#[gpui::test]
fn lean_workspace_row_says_open_to_load(cx: &mut TestAppContext) {
    let src = Src {
        mentions: vec![MentionItem {
            row: unread("w2", "Beta", "C9", 4, "4.000001"),
            messages: vec![],
        }],
        ..Default::default()
    };
    let (_s, v, _e, vcx) = setup(cx, AggregateKind::Mentions, src);
    let got = entries(&v, vcx);
    assert_eq!(got[0].mentions, 4);
    assert_eq!(got[0].tag.as_deref(), Some("open to load"));
    assert_eq!(got[0].target, AggregateTarget::Conversation(cid("C9")));
}

fn sched(ws: &str, name: &str, id: &str, at: i64, local: bool) -> ScheduledItem {
    ScheduledItem {
        workspace: tag(ws, name),
        id: ScheduleId(Arc::from(id)),
        conversation: "#general".into(),
        text: format!("text {id}").into(),
        post_at: at,
        local,
        state: local.then_some(LocalScheduleState::Pending),
    }
}

#[gpui::test]
fn scheduled_merged_by_time_with_local_tag(cx: &mut TestAppContext) {
    let src = Src {
        scheduled: vec![
            sched("w1", "Alpha", "a", NOW + 100, false),
            sched("w2", "Beta", "b", NOW + 200, true),
            sched("w1", "Alpha", "c", NOW + 300, false),
        ],
        ..Default::default()
    };
    let (_s, v, events, vcx) = setup(cx, AggregateKind::Scheduled, src);
    let got = entries(&v, vcx);
    let names: Vec<&str> = got.iter().map(|e| &*e.workspace.name).collect();
    assert_eq!(names, ["Alpha", "Beta", "Alpha"]);
    assert_eq!(got[0].tag, None);
    assert_eq!(got[1].tag.as_deref(), Some("local, needs app open"));
    assert!(got[1].lines.iter().any(|l| l.contains("text b")));
    v.update(vcx, |v, cx| v.open(1, cx));
    assert_eq!(
        events.borrow()[0],
        AggregateEvent::Open {
            workspace: WorkspaceKey::new("fake", "w2"),
            target: AggregateTarget::Schedule(ScheduleId(Arc::from("b"))),
        }
    );
}

#[gpui::test]
fn mark_all_read_confirms(cx: &mut TestAppContext) {
    let src = Src {
        unreads: vec![unread("w1", "Alpha", "C1", 0, "8.000001")],
        ..Default::default()
    };
    let (s, v, _e, vcx) = setup(cx, AggregateKind::Unreads, src);
    v.update(vcx, |v, cx| v.request_mark_all_read(cx));
    assert!(v.read_with(vcx, |v, _| v.confirming()));
    assert_eq!(
        s.read_with(vcx, |s, _| s.marked),
        0,
        "nothing before confirm"
    );
    v.update(vcx, |v, cx| v.cancel_mark_all_read(cx));
    assert!(!v.read_with(vcx, |v, _| v.confirming()));
    assert_eq!(s.read_with(vcx, |s, _| s.marked), 0);
    v.update(vcx, |v, cx| {
        v.request_mark_all_read(cx);
        v.confirm_mark_all_read(cx);
    });
    assert_eq!(s.read_with(vcx, |s, _| s.marked), 1);
    assert!(!v.read_with(vcx, |v, _| v.confirming()));
}

#[gpui::test]
fn long_lists_draw_a_page_then_show_more(cx: &mut TestAppContext) {
    let src = Src {
        unreads: (0..250)
            .map(|i| unread("w1", "Alpha", &format!("C{i}"), 0, "8.000001"))
            .collect(),
        ..Default::default()
    };
    let (_s, v, _e, vcx) = setup(cx, AggregateKind::Unreads, src);
    vcx.run_until_parked();
    assert_eq!(v.read_with(vcx, |v, _| v.shown()), components::PAGE);
    assert!(vcx.debug_bounds("aggregate-row-0").is_some());
    assert!(
        vcx.debug_bounds("aggregate-row-150").is_none(),
        "rows past the page are not laid out"
    );
    assert!(vcx.debug_bounds("aggregate-show-more").is_some());
    v.update(vcx, |v, cx| v.show_more(cx));
    vcx.run_until_parked();
    assert_eq!(v.read_with(vcx, |v, _| v.shown()), 2 * components::PAGE);
    assert!(vcx.debug_bounds("aggregate-row-150").is_some());
}

#[gpui::test]
fn only_a_change_of_rows_redraws(cx: &mut TestAppContext) {
    let src = Src {
        unreads: vec![unread("w1", "Alpha", "C1", 0, "8.000001")],
        ..Default::default()
    };
    let (s, v, _e, vcx) = setup(cx, AggregateKind::Unreads, src);
    let redraws = Rc::new(std::cell::Cell::new(0u32));
    let n = redraws.clone();
    vcx.update(|_, cx| {
        cx.observe(&v, move |_, _| n.set(n.get() + 1)).detach();
    });
    // The manager notifies on every session change; the rows did not change.
    s.update(vcx, |_, cx| cx.notify());
    vcx.run_until_parked();
    assert_eq!(redraws.get(), 0, "same rows: no redraw");
    s.update(vcx, |s, cx| {
        s.unreads.push(unread("w2", "Beta", "C2", 1, "9.000001"));
        cx.notify();
    });
    vcx.run_until_parked();
    assert_eq!(redraws.get(), 1, "new row: one redraw");
    assert_eq!(entries(&v, vcx).len(), 2);
}

#[gpui::test]
fn unread_rows_show_previews_and_mark_one_read(cx: &mut TestAppContext) {
    let mut src = Src {
        unreads: vec![
            unread("w1", "Alpha", "C1", 0, "8.000001"),
            unread("w2", "Beta", "C2", 0, "6.000001"),
        ],
        ..Default::default()
    };
    src.previews.insert(
        cid("C1"),
        vec![PreviewLine {
            id: domain::test_support::mid("C1", "7.000001"),
            author: "Ana Reyes".into(),
            avatar: None,
            time: "10:02".into(),
            text: "Idle CPU is back under budget".into(),
        }],
    );
    let (s, v, events, vcx) = setup(cx, AggregateKind::Unreads, src);
    vcx.run_until_parked();
    let rows = v.read_with(vcx, |v, _| v.rows().to_vec());
    assert_eq!(rows[0].previews.len(), 1);
    assert_eq!(&*rows[0].previews[0].text, "Idle CPU is back under budget");
    assert!(rows[1].previews.is_empty());
    assert!(vcx.debug_bounds("aggregate-preview-0-0").is_some());
    let button = vcx
        .debug_bounds("aggregate-mark-read-1")
        .expect("each unread row has Mark read");
    vcx.simulate_click(button.center(), gpui::Modifiers::none());
    assert_eq!(
        s.read_with(vcx, |s, _| s.marked_one.clone()),
        [(WorkspaceKey::new("fake", "w2"), cid("C2"), ts("6.000001"))],
        "the painted row's conversation at its painted newest message"
    );
    assert!(
        events.borrow().is_empty(),
        "Mark read does not open the row"
    );
}

#[gpui::test]
fn previews_are_read_for_shown_rows_once_per_change(cx: &mut TestAppContext) {
    let src = Src {
        unreads: (0..250)
            .map(|i| unread("w1", "Alpha", &format!("C{i}"), 0, "8.000001"))
            .collect(),
        ..Default::default()
    };
    let (s, v, _e, vcx) = setup(cx, AggregateKind::Unreads, src);
    vcx.run_until_parked();
    let reads = |vcx: &mut VisualTestContext| s.read_with(vcx, |s, _| s.preview_reads.get());
    assert_eq!(reads(vcx), components::PAGE as u32, "only the shown page");
    s.update(vcx, |_, cx| cx.notify());
    vcx.run_until_parked();
    assert_eq!(
        reads(vcx),
        components::PAGE as u32,
        "unchanged rows are not read again"
    );
    s.update(vcx, |s, cx| {
        s.unreads[0].latest = Some(ts("9.000001"));
        cx.notify();
    });
    vcx.run_until_parked();
    assert_eq!(
        reads(vcx),
        components::PAGE as u32 + 1,
        "a changed row is read again"
    );
    v.update(vcx, |v, cx| v.show_more(cx));
    vcx.run_until_parked();
    assert_eq!(reads(vcx), 2 * components::PAGE as u32 + 1);
}
