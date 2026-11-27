//! Executes the generic `Effect::Call` and `Effect::Load` on the provider and
//! feeds the answer back as `Input::CallFinished` / `Input::Loaded`.

use domain::{
    AdminCall, CallOutput, ChatProvider, Input, Load, Loaded, ProviderCall, ProviderError,
};
use gpui::Context;

use crate::session::WorkspaceSession;

impl WorkspaceSession {
    pub(crate) fn run_call(&mut self, call: ProviderCall, cx: &mut Context<Self>) {
        let p = self.provider.clone();
        let c = call.clone();
        self.run(
            cx,
            async move {
                match c {
                    ProviderCall::Schedule {
                        conversation,
                        thread,
                        text,
                        post_at,
                    } => p
                        .schedule(&conversation, thread, &text, post_at)
                        .await
                        .map(CallOutput::Scheduled),
                    ProviderCall::DeleteScheduled(s) => {
                        p.delete_scheduled(&s).await.map(|()| CallOutput::None)
                    }
                    ProviderCall::AddReminder { text, when, user } => p
                        .add_reminder(&text, &when, user.as_ref())
                        .await
                        .map(CallOutput::Reminder),
                    ProviderCall::CompleteReminder(id) => {
                        p.complete_reminder(&id).await.map(|()| CallOutput::None)
                    }
                    ProviderCall::DeleteReminder(id) => {
                        p.delete_reminder(&id).await.map(|()| CallOutput::None)
                    }
                    ProviderCall::SetStatus(s) => p.set_status(s).await.map(|()| CallOutput::None),
                    ProviderCall::SetPresence(s) => {
                        p.set_presence(s).await.map(|()| CallOutput::None)
                    }
                    ProviderCall::WatchPresence(users) => {
                        p.watch_presence(&users).await.map(|()| CallOutput::None)
                    }
                    ProviderCall::OpenDirect(u) => {
                        p.open_direct(&u).await.map(CallOutput::Conversation)
                    }
                    ProviderCall::Snooze(m) => p.snooze(m).await.map(CallOutput::Dnd),
                    ProviderCall::EndSnooze => p.end_snooze().await.map(CallOutput::Dnd),
                    ProviderCall::SetNotifyPrefs {
                        conversation,
                        prefs,
                    } => p
                        .set_notify_prefs(&conversation, prefs)
                        .await
                        .map(|()| CallOutput::None),
                    ProviderCall::DeclineHuddle { conversation, id } => p
                        .decline_huddle(&conversation, &id)
                        .await
                        .map(|()| CallOutput::None),
                    ProviderCall::SetPinned { id, pinned } => {
                        p.set_pinned(&id, pinned).await.map(|()| CallOutput::None)
                    }
                    ProviderCall::SetSaved { id, saved } => {
                        p.set_saved(&id, saved).await.map(|()| CallOutput::None)
                    }
                    ProviderCall::SetThreadFollowed {
                        root,
                        follow,
                        last_read,
                    } => p
                        .set_thread_followed(&root, follow, last_read.as_ref())
                        .await
                        .map(|()| CallOutput::None),
                    ProviderCall::AddBookmark {
                        conversation,
                        title,
                        link,
                    } => p
                        .add_bookmark(&conversation, &title, &link)
                        .await
                        .map(CallOutput::Bookmark),
                    ProviderCall::RemoveBookmark { conversation, id } => p
                        .remove_bookmark(&conversation, &id)
                        .await
                        .map(|()| CallOutput::None),
                    ProviderCall::SetConversationStarred {
                        conversation,
                        starred,
                    } => p
                        .set_conversation_starred(&conversation, starred)
                        .await
                        .map(|()| CallOutput::None),
                    ProviderCall::Admin(a) => run_admin(&*p, a).await,
                }
            },
            move |s, result, cx| {
                s.feed_back(Input::CallFinished { call, result }, cx);
            },
        );
    }

    pub(crate) fn run_load(&mut self, load: Load, cx: &mut Context<Self>) {
        let p = self.provider.clone();
        let l = load.clone();
        self.run(
            cx,
            async move {
                match l {
                    Load::Scheduled => p.scheduled().await.map(Loaded::Scheduled),
                    Load::Reminders => p.reminders().await.map(Loaded::Reminders),
                    Load::Dnd => p.dnd().await.map(Loaded::Dnd),
                    Load::DndUsers(users) => p.dnd_users(&users).await.map(Loaded::DndUsers),
                    Load::NotifyPrefs => p.notify_prefs().await.map(Loaded::NotifyPrefs),
                    Load::Profile(u) => p.profile(&u).await.map(Loaded::Profile),
                    Load::Huddles => p.huddles().await.map(Loaded::Huddles),
                    Load::Pins(c) => p.pins(&c).await.map(|v| Loaded::Pins(c, v)),
                    Load::Bookmarks(c) => p.bookmarks(&c).await.map(|v| Loaded::Bookmarks(c, v)),
                    Load::Saved { cursor } => p.saved(cursor).await.map(Loaded::Saved),
                    Load::ThreadInbox { cursor } => {
                        p.thread_inbox(cursor).await.map(Loaded::ThreadInbox)
                    }
                    Load::Message(id) => p.message(&id).await.map(Loaded::Message),
                    Load::Conversation(id) => p.conversation(&id).await.map(Loaded::Conversation),
                    Load::Members(ids) => p
                        .members(&ids)
                        .await
                        .map(|found| Loaded::Members { asked: ids, found }),
                }
            },
            move |s, result: Result<Loaded, ProviderError>, cx| {
                s.feed_back(Input::Loaded(result.map_err(|e| (load, e))), cx);
            },
        );
    }

    fn feed_back(&mut self, input: Input, cx: &mut Context<Self>) {
        #[cfg(feature = "fixture")]
        {
            self.last_feedback = Some(input.clone());
        }
        // The direct message the person asked for exists now: go to it.
        let opened = match &input {
            Input::CallFinished {
                call: ProviderCall::OpenDirect(u),
                result,
            } => Some((
                u.clone(),
                match result {
                    Ok(CallOutput::Conversation(c)) => Ok(c.id.clone()),
                    Ok(_) => Err(crate::error_text::provider_error(
                        &ProviderError::Unsupported,
                    )),
                    Err(e) => Err(crate::error_text::provider_error(e)),
                },
            )),
            _ => None,
        };
        // A channel admin answer: the host closes or updates its dialog.
        let admin = match &input {
            Input::CallFinished {
                call: ProviderCall::Admin(a),
                result,
            } => Some((
                a.clone(),
                match result {
                    Ok(CallOutput::Conversation(c)) => Ok(Some(c.id.clone())),
                    // Some people could not be added: say who and why.
                    Ok(CallOutput::Invite(r)) if !r.failed.is_empty() => {
                        let l = i18n::gpui::localizer(cx);
                        Err(r
                            .failed
                            .iter()
                            .map(|(u, e)| {
                                let who = components::user_name(self.state.members.get(u), &l);
                                let why = l.text(&crate::error_text::provider_error(e));
                                t!(
                                    l,
                                    "shell-invite-person-failed",
                                    person = who,
                                    reason = why.into_owned()
                                )
                                .into_owned()
                            })
                            .collect::<Vec<_>>()
                            .join("; "))
                    }
                    Ok(_) => Ok(None),
                    Err(e) => Err(i18n::gpui::localizer(cx)
                        .text(&crate::error_text::provider_error(e))
                        .into_owned()),
                },
            )),
            _ => None,
        };
        if let Some((AdminCall::Join(c), _)) = &admin {
            self.join_finished(c);
        }
        self.apply(input, cx);
        if let Some((call, result)) = admin {
            match (&call, &result) {
                // The new or joined channel is where the person goes next.
                (_, Ok(Some(id))) => self.open(id.clone(), cx),
                // Left or archived: nothing staged for it can be sent any more.
                (AdminCall::Leave(c) | AdminCall::Archive(c), Ok(None)) => {
                    self.prune_conversation(c, cx);
                }
                _ => {}
            }
            cx.emit(crate::session::SessionEvent::AdminFinished {
                call,
                result: result.map(|_| ()),
            });
        }
        match opened {
            Some((u, Ok(id))) => {
                self.open(id, cx);
                cx.emit(crate::session::SessionEvent::DirectOpened(u));
            }
            Some((u, Err(why))) => {
                let why = i18n::gpui::localizer(cx).text(&why).into_owned();
                cx.emit(crate::session::SessionEvent::DirectFailed(u, why));
            }
            None => {}
        }
    }
}

/// Runs one channel admin write; `Create` and `Join` answer the conversation.
async fn run_admin(p: &dyn ChatProvider, a: AdminCall) -> Result<CallOutput, ProviderError> {
    let none = |r: Result<(), ProviderError>| r.map(|()| CallOutput::None);
    match a {
        AdminCall::Create { name, private } => p
            .create_channel(&name, private)
            .await
            .map(CallOutput::Conversation),
        AdminCall::Join(c) => p.join_conversation(&c).await.map(CallOutput::Conversation),
        AdminCall::Leave(c) => none(p.leave_conversation(&c).await),
        AdminCall::Archive(c) => none(p.archive_conversation(&c).await),
        AdminCall::Rename { conversation, name } => {
            none(p.rename_conversation(&conversation, &name).await)
        }
        AdminCall::SetTopic {
            conversation,
            topic,
        } => none(p.set_topic(&conversation, &topic).await),
        AdminCall::SetPurpose {
            conversation,
            purpose,
        } => none(p.set_purpose(&conversation, &purpose).await),
        AdminCall::Invite {
            conversation,
            users,
        } => p
            .invite(&conversation, &users)
            .await
            .map(CallOutput::Invite),
    }
}
