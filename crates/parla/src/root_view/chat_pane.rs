//! A conversation's timeline and composer, built the same in the main window
//! and in a pop-out: the bookmarks and huddle bars when the provider has them,
//! and the subscriptions both windows need.

use components::{
    BookmarksBar, Composer, ComposerEvent, ComposerSource, OpenUrl, Timeline, TimelineEvent,
};
use domain::{ConversationId, DraftKey, MessageId};
use gpui::{App, AppContext as _, Context, Entity, Subscription, Window};
use settings::SettingsStore;

use crate::delete_confirm::DeleteConfirmHost;
use crate::session::WorkspaceSession;

pub(crate) struct ChatPane {
    pub conversation: ConversationId,
    pub bookmarks: Option<Entity<BookmarksBar<WorkspaceSession>>>,
    pub huddle: Option<Entity<components::HuddleBar<WorkspaceSession>>>,
    pub timeline: Entity<Timeline<WorkspaceSession>>,
    pub composer: Entity<Composer<WorkspaceSession>>,
    pub subs: Vec<Subscription>,
}

/// Builds the pane for `conversation`. `open_url` decides where links in the
/// timeline go (`None` leaves the timeline's own handling); `open_thread`
/// opens a thread the way the window shows it (a panel or a pop-out).
impl ChatPane {
    pub(crate) fn new<H: DeleteConfirmHost>(
        session: Entity<WorkspaceSession>,
        conversation: ConversationId,
        window: &mut Window,
        cx: &mut Context<H>,
        open_url: Option<OpenUrl>,
        open_thread: impl Fn(&mut H, MessageId, &mut Window, &mut Context<H>) + 'static,
    ) -> ChatPane {
        let clock = cx.global::<SettingsStore>().current().messages.clock;
        let timeline = cx.new(|cx| Timeline::new(session.clone(), conversation.clone(), clock, cx));
        if let Some(open_url) = open_url {
            timeline.update(cx, |t, _| t.set_open_url(open_url));
        }
        let inline = cx
            .global::<SettingsStore>()
            .current()
            .messages
            .inline_images;
        timeline.update(cx, |t, cx| t.set_inline_images(inline, cx));
        let composer = cx.new(|cx| Composer::new(session.clone(), conversation.clone(), cx));
        crate::root_view::restore_draft(&session, &composer, &conversation, cx);
        let bookmarks = session
            .read(cx)
            .state()
            .caps
            .bookmarks
            .then(|| cx.new(|cx| BookmarksBar::new(session.clone(), conversation.clone(), cx)));
        let huddle = session.read(cx).state().caps.huddles.then(|| {
            cx.new(|cx| components::HuddleBar::new(session.clone(), conversation.clone(), cx))
        });
        let mut subs = Vec::new();
        let c = composer.clone();
        let t = timeline.clone();
        subs.push(cx.subscribe_in(
            &timeline,
            window,
            move |this, _, ev: &TimelineEvent, window, cx| {
                c.update(cx, |c, cx| c.handle_timeline_event(ev, cx));
                match ev {
                    TimelineEvent::DeleteRequested { id, text } => crate::delete_confirm::open(
                        this,
                        t.clone(),
                        id.clone(),
                        text.clone(),
                        window,
                        cx,
                    ),
                    TimelineEvent::DeleteCancelled => {
                        crate::delete_confirm::close(this, false, window, cx);
                    }
                    TimelineEvent::OpenThread(root) => open_thread(this, root.clone(), window, cx),
                    TimelineEvent::EditRequested { .. } => {}
                }
            },
        ));
        let s = session.clone();
        let sent = conversation.clone();
        subs.push(
            cx.subscribe(&composer, move |_, _, ev: &ComposerEvent, cx| {
                handle_composer_event(&s, &sent, ev, cx);
            }),
        );
        ChatPane {
            conversation,
            bookmarks,
            huddle,
            timeline,
            composer,
            subs,
        }
    }
}

/// Sends, edits, replies or schedules from the composer. `conversation` is
/// the one the composer belongs to.
pub(crate) fn handle_composer_event(
    session: &Entity<WorkspaceSession>,
    conversation: &ConversationId,
    ev: &ComposerEvent,
    cx: &mut App,
) {
    let draft_key = || DraftKey {
        conversation: conversation.clone(),
        thread: None,
    };
    match ev.clone() {
        ComposerEvent::Submit { body } => {
            let key = draft_key();
            session.update(cx, |s, cx| {
                let text = ComposerSource::encode(s, &body);
                s.set_draft(key, "", cx);
                s.send(conversation.clone(), text, cx)
            });
        }
        ComposerEvent::SubmitEdit { id, body } => {
            session.update(cx, |s, cx| {
                let text = ComposerSource::encode(s, &body);
                s.edit(id, text, cx)
            });
        }
        ComposerEvent::SubmitReply {
            root,
            body,
            broadcast,
        } => {
            session.update(cx, |s, cx| {
                let text = ComposerSource::encode(s, &body);
                s.reply(root, text, broadcast, cx)
            });
        }
        ComposerEvent::Schedule { body, post_at } => {
            let key = draft_key();
            session.update(cx, |s, cx| {
                let text = ComposerSource::encode(s, &body);
                s.set_draft(key, "", cx);
                s.schedule(conversation.clone(), None, text, Vec::new(), post_at, cx)
            });
        }
        ComposerEvent::Cancel => {}
    }
}
