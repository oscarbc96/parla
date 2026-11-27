//! Huddles on the session: joining a live huddle starts the app's own call
//! window or native audio (the call manager), else falls back to the
//! provider's app or the browser.

use components::{FileNotice, HuddleSource};
use domain::{ConversationId, Huddle, Member, UserId};
use gpui::{App, Context};
use settings::JoinWith;

use crate::call_manager::CallManagerHandle;
use crate::session::WorkspaceSession;

/// What joining a huddle did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HuddleOpen {
    /// Opened in the default browser.
    Browser(String),
    /// Handed to the provider's own app through the OS.
    App(String),
    /// Native audio was started; the call manager owns it.
    Native,
    /// The call window (helper process) was started; the call manager owns it.
    Window,
    /// The provider gave no URL: a toast tells the user to open it there.
    Toast,
}

/// Which URL `join_with` picks when no call manager is installed (tests and
/// headless builds): `window` is the browser then. The app's `CallManager`
/// builds the full ladder. Pure, so the choice is tested without a window.
pub(crate) fn choose(
    join_with: JoinWith,
    huddle_url: Option<String>,
    app_url: Option<String>,
) -> HuddleOpen {
    match (join_with, huddle_url, app_url) {
        (JoinWith::ProviderApp, _, Some(u)) => HuddleOpen::App(u),
        (JoinWith::Native | JoinWith::Window | JoinWith::Browser, Some(u), _) => {
            HuddleOpen::Browser(u)
        }
        _ => HuddleOpen::Toast,
    }
}

impl WorkspaceSession {
    /// Opens the huddle of `c` per `calls.join_with`: through the call manager
    /// (call window, then the fallback ladder) when the app installed one.
    pub fn open_huddle(&mut self, c: &ConversationId, cx: &mut Context<Self>) -> HuddleOpen {
        let manager = cx.try_global::<CallManagerHandle>().map(|h| h.0.clone());
        let out = match manager {
            Some(m) => {
                let calls = self.settings(cx).calls.clone();
                let (ws, provider) = (self.provider.workspace().clone(), self.provider.clone());
                m.update(cx, |m, cx| m.join(&ws, c.clone(), provider, &calls, cx))
            }
            None => {
                let join_with = self.settings(cx).calls.join_with;
                let out = choose(
                    join_with,
                    self.provider.huddle_url(c),
                    self.provider.app_url(c),
                );
                if let HuddleOpen::Browser(u) | HuddleOpen::App(u) = &out {
                    cx.open_url(u);
                }
                out
            }
        };
        if out == HuddleOpen::Toast {
            self.call_notice(i18n::LocalizedText::new("parla", "huddle-open-in"), cx);
        }
        out
    }

    /// Shows a call notice as the workspace's toast. "Open this huddle in
    /// {provider}" gets the provider's name.
    pub(crate) fn call_notice(&mut self, text: i18n::LocalizedText, cx: &mut Context<Self>) {
        let text = if text.id.as_ref() == "huddle-open-in" {
            text.with(
                "provider",
                components::TimelineSource::provider_name(self).to_string(),
            )
        } else {
            text
        };
        self.files.notice = Some(FileNotice::Error(text));
        cx.notify();
    }

    /// The workspace's huddles changed: lets the call manager end a call whose
    /// huddle is over.
    pub(crate) fn sync_call(&self, cx: &mut Context<Self>) {
        if let Some(m) = cx.try_global::<CallManagerHandle>().map(|h| h.0.clone()) {
            let ws = self.provider.workspace().clone();
            m.update(cx, |m, cx| m.huddles_changed(&ws, &self.state, cx));
        }
    }
}

impl HuddleSource for WorkspaceSession {
    fn huddle(&self, c: &ConversationId) -> Option<&Huddle> {
        self.state.huddles.get(c)
    }

    fn self_user(&self) -> Option<&UserId> {
        self.state.self_user.as_ref()
    }

    fn member(&self, u: &UserId) -> Option<&Member> {
        self.state.members.get(u)
    }

    fn join_huddle(&mut self, c: ConversationId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.apply(domain::Input::JoinHuddle(c), cx));
    }

    fn can_start_huddle(&self) -> bool {
        self.state.caps.huddles
    }

    fn in_call_here(&self, c: &ConversationId, cx: &App) -> bool {
        cx.try_global::<CallManagerHandle>()
            .is_some_and(|h| h.0.read(cx).in_call_for(self.provider.workspace(), c, cx))
    }
}
