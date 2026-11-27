//! Hosts the channel admin dialogs over the window: create a channel, browse
//! and join channels, edit a channel, invite people and the leave and
//! archive confirms. The session runs the calls; its `AdminFinished` answer
//! closes a dialog or tells why not.

use std::sync::Arc;

use components::{
    BrowseRow, ChannelBrowser, ChannelBrowserEvent, ConfirmDialog, ConfirmEvent,
    CreateChannelDialog, CreateChannelEvent, EditChannelDialog, EditChannelEvent, InviteDialog,
    InviteEvent, InviteRow,
};
use domain::{AdminCall, Conversation, ConversationId, ConversationKind};
use gpui::{
    Context, Entity, Focusable as _, IntoElement, ParentElement as _, Styled as _, Subscription,
    Window, div, prelude::*,
};

use super::RootView;
use crate::session::WorkspaceSession;

pub(super) enum AdminDialog {
    Create(Entity<CreateChannelDialog>),
    Browse(Entity<ChannelBrowser>),
    Edit(Entity<EditChannelDialog>),
    Invite(Entity<InviteDialog>),
    /// Leave or archive, sent only once the person confirms.
    Confirm(Entity<ConfirmDialog>, AdminCall),
}

/// The open admin dialog and the session it acts on.
pub(super) struct AdminPane {
    dialog: AdminDialog,
    session: Entity<WorkspaceSession>,
    /// The browser's text and where its next page starts.
    query: String,
    next: Option<Arc<str>>,
    /// Bumped by each new search, so a late page of an old one is dropped.
    generation: u64,
    /// The page on its way; a newer query aborts it.
    loading: Option<futures::future::AbortHandle>,
    _sub: Subscription,
}

fn browse_row(c: &Conversation) -> BrowseRow {
    BrowseRow {
        id: c.id.clone(),
        name: c.name.to_string().into(),
        private: c.kind == ConversationKind::PrivateChannel,
        purpose: c.purpose.to_string().into(),
        members: c.member_count,
        is_member: c.is_member,
    }
}

fn is_channel(c: &Conversation) -> bool {
    matches!(
        c.kind,
        ConversationKind::Channel | ConversationKind::PrivateChannel
    )
}

impl RootView {
    /// The active session when its provider has channel admin.
    fn admin_session(&self, cx: &Context<Self>) -> Option<Entity<WorkspaceSession>> {
        let s = self.ready.as_ref()?.session.clone();
        s.read(cx).can_admin().then_some(s)
    }

    fn show_admin(
        &mut self,
        session: Entity<WorkspaceSession>,
        dialog: AdminDialog,
        sub: Subscription,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = match &dialog {
            AdminDialog::Create(d) => d.read(cx).focus_handle(cx),
            AdminDialog::Browse(d) => d.read(cx).focus_handle(cx),
            AdminDialog::Edit(d) => d.read(cx).focus_handle(cx),
            AdminDialog::Invite(d) => d.read(cx).focus_handle(cx),
            AdminDialog::Confirm(d, _) => d.read(cx).focus_handle(cx),
        };
        window.focus(&focus, cx);
        self.admin = Some(AdminPane {
            dialog,
            session,
            query: String::new(),
            next: None,
            generation: 0,
            loading: None,
            _sub: sub,
        });
        cx.notify();
    }

    /// Closes the admin dialog and gives the focus back to the composer.
    pub fn close_admin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.admin.take().is_none() {
            return;
        }
        match self.ready.as_ref().and_then(|r| r.chat.as_ref()) {
            Some(chat) => {
                let focus = chat.composer.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
            None => window.focus(&self.focus, cx),
        }
        cx.notify();
    }

    /// The create a channel dialog.
    pub fn create_channel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.admin_session(cx) else {
            return;
        };
        let check = session.read(cx).name_check();
        let dialog = cx.new(|cx| CreateChannelDialog::new(check, cx));
        let target = session.clone();
        let sub = cx.subscribe_in(
            &dialog,
            window,
            move |this, _, ev: &CreateChannelEvent, window, cx| match ev {
                CreateChannelEvent::Create { name, private } => {
                    let call = AdminCall::Create {
                        name: name.as_str().into(),
                        private: *private,
                    };
                    target.update(cx, |s, cx| s.admin(call, cx));
                }
                CreateChannelEvent::Cancel => this.close_admin(window, cx),
            },
        );
        self.show_admin(session, AdminDialog::Create(dialog), sub, window, cx);
    }

    /// The channel browser (the sidebar's Channels "+").
    pub fn browse_channels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.admin_session(cx) else {
            // Without channel admin the switcher lists the known channels.
            self.open_switcher(components::SwitchScope::Channels, window, cx);
            return;
        };
        let browser = cx.new(ChannelBrowser::new);
        let target = session.clone();
        let sub = cx.subscribe_in(
            &browser,
            window,
            move |this, _, ev: &ChannelBrowserEvent, window, cx| match ev {
                ChannelBrowserEvent::Query(q) => {
                    if let Some(p) = this.admin.as_mut() {
                        p.query = q.clone();
                    }
                    this.load_browse(true, cx);
                }
                ChannelBrowserEvent::More => this.load_browse(false, cx),
                ChannelBrowserEvent::Join(c) => {
                    let call = AdminCall::Join(c.clone());
                    target.update(cx, |s, cx| s.admin(call, cx));
                }
                ChannelBrowserEvent::Open(c) => {
                    let c = c.clone();
                    target.update(cx, |s, cx| s.open(c, cx));
                    this.close_admin(window, cx);
                }
                ChannelBrowserEvent::Create => {
                    this.close_admin(window, cx);
                    this.create_channel(window, cx);
                }
                ChannelBrowserEvent::Cancel => this.close_admin(window, cx),
            },
        );
        self.show_admin(session, AdminDialog::Browse(browser), sub, window, cx);
        self.load_browse(true, cx);
    }

    /// Loads the first page for the browser's text, or the next one.
    fn load_browse(&mut self, first: bool, cx: &mut Context<Self>) {
        let Some(p) = self.admin.as_mut() else { return };
        if !matches!(p.dialog, AdminDialog::Browse(_)) {
            return;
        }
        if first {
            p.generation += 1;
            p.next = None;
            if let Some(a) = p.loading.take() {
                a.abort();
            }
        } else if p.next.is_none() {
            return;
        }
        let (generation, query, cursor) = (p.generation, p.query.clone(), p.next.clone());
        let root = cx.entity().downgrade();
        let abort = p.session.update(cx, |s, cx| {
            s.browse_channels(query, cursor, cx, move |_, r, cx| {
                root.update(cx, |this, cx| this.browse_loaded(generation, first, r, cx))
                    .ok();
            })
        });
        p.loading = Some(abort);
    }

    fn browse_loaded(
        &mut self,
        generation: u64,
        first: bool,
        r: Result<domain::Page<Conversation>, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(p) = self.admin.as_mut() else { return };
        let AdminDialog::Browse(b) = &p.dialog else {
            return;
        };
        if p.generation != generation {
            return;
        }
        let b = b.clone();
        p.loading = None;
        match r {
            Ok(page) => {
                p.next = page.next.clone();
                let rows = page
                    .items
                    .iter()
                    .filter(|c| is_channel(c) && !c.is_archived)
                    .map(browse_row)
                    .collect();
                let more = page.next.is_some();
                b.update(cx, |b, cx| b.set_page(rows, first, more, cx));
            }
            Err(why) => b.update(cx, |b, cx| b.set_error(Some(why), cx)),
        }
    }

    /// Edit a channel's name, topic and description.
    pub fn edit_channel(&mut self, c: ConversationId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.admin_session(cx) else {
            return;
        };
        let Some(conv) = session.read(cx).state().conversations.get(&c).cloned() else {
            return;
        };
        let check = session.read(cx).name_check();
        let dialog = cx.new(|cx| {
            EditChannelDialog::new(
                &conv.name,
                &conv.topic,
                &conv.purpose,
                is_channel(&conv),
                check,
                cx,
            )
        });
        let target = session.clone();
        let sub = cx.subscribe_in(
            &dialog,
            window,
            move |this, _, ev: &EditChannelEvent, window, cx| {
                if let EditChannelEvent::Save {
                    name,
                    topic,
                    purpose,
                } = ev
                {
                    let conversation = c.clone();
                    let mut calls = Vec::new();
                    if let Some(n) = name {
                        calls.push(AdminCall::Rename {
                            conversation: conversation.clone(),
                            name: n.as_str().into(),
                        });
                    }
                    if let Some(t) = topic {
                        calls.push(AdminCall::SetTopic {
                            conversation: conversation.clone(),
                            topic: t.as_str().into(),
                        });
                    }
                    if let Some(p) = purpose {
                        calls.push(AdminCall::SetPurpose {
                            conversation,
                            purpose: p.as_str().into(),
                        });
                    }
                    target.update(cx, |s, cx| {
                        for call in calls {
                            s.admin(call, cx);
                        }
                    });
                }
                this.close_admin(window, cx);
            },
        );
        self.show_admin(session, AdminDialog::Edit(dialog), sub, window, cx);
    }

    /// Pick people to add to a channel: asks the provider who is in it
    /// first, so the picker offers only people who are not.
    pub fn invite_people(
        &mut self,
        c: ConversationId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.admin_session(cx) else {
            return;
        };
        let root = cx.entity().downgrade();
        let handle = window.window_handle();
        session.update(cx, |s, cx| {
            s.conversation_members(c.clone(), cx, move |_, r, cx| {
                // Without the list, what is known here still filters.
                let inside = r.unwrap_or_default();
                // After this session update: the dialog reads the session.
                cx.defer(move |cx| {
                    handle
                        .update(cx, |_, window, cx| {
                            root.update(cx, |this, cx| this.show_invite(c, inside, window, cx))
                                .ok();
                        })
                        .ok();
                });
            })
        });
    }

    fn show_invite(
        &mut self,
        c: ConversationId,
        members: Vec<domain::UserId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.admin_session(cx) else {
            return;
        };
        let l = i18n::gpui::localizer(cx);
        let (title, people) = {
            let s = session.read(cx);
            let st = s.state();
            let Some(conv) = st.conversations.get(&c) else {
                return;
            };
            let (mut inside, _) = components::channel_details::conversation_people(st, conv);
            inside.extend(members);
            let mut people: Vec<InviteRow> = st
                .members
                .values()
                .filter(|m| {
                    !m.deleted
                        && !m.is_bot
                        && Some(&m.id) != st.self_user.as_ref()
                        && !inside.contains(&m.id)
                })
                .map(|m| {
                    let name = [&m.display_name, &m.real_name, &m.handle]
                        .into_iter()
                        .find(|n| !n.is_empty())
                        .map_or_else(String::new, |n| n.to_string());
                    let detail = if m.handle.is_empty() || *m.handle == *name {
                        String::new()
                    } else {
                        format!("@{}", m.handle)
                    };
                    InviteRow {
                        id: m.id.clone(),
                        name: name.into(),
                        detail: detail.into(),
                    }
                })
                .collect();
            people.sort_by_key(|a| a.name.to_lowercase());
            let title = t!(l, "shell-invite-title", channel = conv.name.to_string()).into_owned();
            (title, people)
        };
        let dialog = cx.new(|cx| InviteDialog::new(title, people, cx));
        let target = session.clone();
        let sub = cx.subscribe_in(
            &dialog,
            window,
            move |this, _, ev: &InviteEvent, window, cx| {
                if let InviteEvent::Invite(users) = ev {
                    let call = AdminCall::Invite {
                        conversation: c.clone(),
                        users: users.clone(),
                    };
                    target.update(cx, |s, cx| s.admin(call, cx));
                }
                this.close_admin(window, cx);
            },
        );
        self.show_admin(session, AdminDialog::Invite(dialog), sub, window, cx);
    }

    /// Asks before leaving (`archive` false) or archiving a channel.
    pub fn confirm_admin(
        &mut self,
        c: ConversationId,
        archive: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.admin_session(cx) else {
            return;
        };
        let Some(conv) = session.read(cx).state().conversations.get(&c).cloned() else {
            return;
        };
        let l = i18n::gpui::localizer(cx);
        let name = conv.name.to_string();
        let private = conv.kind == ConversationKind::PrivateChannel;
        let (title, body, ok, call) = if archive {
            (
                t!(l, "shell-archive-title", channel = name.clone()).into_owned(),
                t!(l, "shell-archive-body").into_owned(),
                t!(l, "shell-archive-ok").into_owned(),
                AdminCall::Archive(c),
            )
        } else {
            (
                t!(l, "shell-leave-title", channel = name.clone()).into_owned(),
                if private {
                    t!(l, "shell-leave-body-private")
                } else {
                    t!(l, "shell-leave-body")
                }
                .into_owned(),
                t!(l, "shell-leave-ok").into_owned(),
                AdminCall::Leave(c),
            )
        };
        // Archive, and leaving a private channel (no way back without an
        // invite), read as danger.
        let danger = archive || private;
        let dialog = cx.new(|cx| ConfirmDialog::new(title, body, ok, cx).danger(danger));
        let target = session.clone();
        let sub = cx.subscribe_in(
            &dialog,
            window,
            move |this, _, ev: &ConfirmEvent, window, cx| {
                if *ev == ConfirmEvent::Confirm {
                    let call = match this.admin.as_ref().map(|p| &p.dialog) {
                        Some(AdminDialog::Confirm(_, call)) => call.clone(),
                        _ => return,
                    };
                    target.update(cx, |s, cx| s.admin(call, cx));
                }
                this.close_admin(window, cx);
            },
        );
        self.show_admin(session, AdminDialog::Confirm(dialog, call), sub, window, cx);
    }

    /// A channel admin call finished: close the dialog that asked for it, or
    /// show why it failed (in the dialog when it is still open).
    pub(super) fn admin_finished(
        &mut self,
        call: &AdminCall,
        result: &Result<(), String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open = self.admin.as_ref().map(|p| &p.dialog);
        match (open, call, result) {
            (Some(AdminDialog::Create(_)), AdminCall::Create { .. }, Ok(()))
            | (Some(AdminDialog::Browse(_)), AdminCall::Join(_), Ok(())) => {
                self.close_admin(window, cx);
            }
            (Some(AdminDialog::Create(d)), AdminCall::Create { .. }, Err(why)) => {
                let (d, why) = (d.clone(), why.clone());
                d.update(cx, |d, cx| d.set_error(Some(why), cx));
            }
            (Some(AdminDialog::Browse(b)), AdminCall::Join(_), Err(why)) => {
                let (b, why) = (b.clone(), why.clone());
                b.update(cx, |b, cx| b.set_error(Some(why), cx));
            }
            // The composer's Join: a toast says why.
            (_, AdminCall::Join(_), Err(why)) => {
                let l = i18n::gpui::localizer(cx);
                components::Toasts::push(
                    cx,
                    components::Toast::new(
                        components::ToastKind::Error,
                        t!(l, "shell-join-failed").into_owned(),
                    )
                    .detail(why.clone()),
                );
            }
            (_, _, Err(why)) => {
                let l = i18n::gpui::localizer(cx);
                self.notice = Some(t!(l, "shell-admin-failed", reason = why.clone()).into_owned());
                cx.notify();
            }
            _ => {}
        }
    }

    /// The open channel browser, if any.
    pub fn channel_browser(&self) -> Option<&Entity<ChannelBrowser>> {
        match self.admin.as_ref().map(|p| &p.dialog) {
            Some(AdminDialog::Browse(b)) => Some(b),
            _ => None,
        }
    }

    /// The open create a channel dialog, if any.
    pub fn create_channel_dialog(&self) -> Option<&Entity<CreateChannelDialog>> {
        match self.admin.as_ref().map(|p| &p.dialog) {
            Some(AdminDialog::Create(d)) => Some(d),
            _ => None,
        }
    }

    /// The open edit channel dialog, if any.
    pub fn edit_channel_dialog(&self) -> Option<&Entity<EditChannelDialog>> {
        match self.admin.as_ref().map(|p| &p.dialog) {
            Some(AdminDialog::Edit(d)) => Some(d),
            _ => None,
        }
    }

    /// The open invite dialog, if any.
    pub fn invite_dialog(&self) -> Option<&Entity<InviteDialog>> {
        match self.admin.as_ref().map(|p| &p.dialog) {
            Some(AdminDialog::Invite(d)) => Some(d),
            _ => None,
        }
    }

    /// The open leave or archive confirm, if any.
    pub fn admin_confirm(&self) -> Option<&Entity<ConfirmDialog>> {
        match self.admin.as_ref().map(|p| &p.dialog) {
            Some(AdminDialog::Confirm(d, _)) => Some(d),
            _ => None,
        }
    }

    /// Whether a channel row can be left and archived here.
    pub(super) fn admin_menu_for(&self, c: &ConversationId, cx: &Context<Self>) -> bool {
        self.admin_session(cx).is_some_and(|s| {
            s.read(cx)
                .state()
                .conversations
                .get(c)
                .is_some_and(|c| is_channel(c) && c.is_member && !c.is_archived)
        })
    }

    /// The scrim and the open dialog, over the window.
    pub(super) fn render_admin(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let p = self.admin.as_ref()?;
        let (body, width) = match &p.dialog {
            AdminDialog::Create(d) => (
                d.clone().into_any_element(),
                themes::metrics::LAYOUT_DIALOG_SM,
            ),
            AdminDialog::Browse(d) => (
                d.clone().into_any_element(),
                themes::metrics::LAYOUT_DIALOG_MD,
            ),
            AdminDialog::Edit(d) => (
                d.clone().into_any_element(),
                themes::metrics::LAYOUT_DIALOG_SM,
            ),
            AdminDialog::Invite(d) => (
                d.clone().into_any_element(),
                themes::metrics::LAYOUT_DIALOG_SM,
            ),
            AdminDialog::Confirm(d, _) => (
                d.clone().into_any_element(),
                themes::metrics::LAYOUT_DIALOG_SM,
            ),
        };
        Some(
            div()
                .id("admin-scrim")
                .debug_selector(|| "admin-dialog".into())
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .px(themes::metrics::SPACE_6)
                .pt(themes::metrics::LAYOUT_DIALOG_TOP)
                .bg(themes::parla_tokens(themes::ActiveTheme::theme(&**cx)).bg_overlay)
                .occlude()
                .child(
                    div()
                        .w_full()
                        .max_w(width)
                        .on_mouse_down_out(
                            cx.listener(|this, _, window, cx| this.close_admin(window, cx)),
                        )
                        .child(body),
                ),
        )
    }
}
