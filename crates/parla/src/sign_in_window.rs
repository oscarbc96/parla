//! The window that adds a workspace: the provider's sign-in screen, behind the
//! first-run Welcome and provider screens when it adds a workspace, so the main
//! window keeps working. It is opened by the flow, closed when the flow ends,
//! and never shows the sidebar.

use components::onboarding::Onboarding;
use components::sign_in::SignInScreen;
use gpui::{
    App, Bounds, Context, Entity, IntoElement, Render, Window, WindowHandle, div, prelude::*, size,
};
use themes::ActiveTheme;

/// Content of the sign-in window.
pub struct SignInWindow {
    screen: Entity<SignInScreen>,
    /// The Welcome and provider screens, when the window starts with them.
    onboarding: Option<Entity<Onboarding>>,
}

impl SignInWindow {
    pub fn screen(&self) -> &Entity<SignInScreen> {
        &self.screen
    }

    /// Shows `screen` at the sign-in step (another provider), dropping the old one.
    pub fn set_screen(&mut self, screen: Entity<SignInScreen>) {
        self.screen = screen;
    }

    pub fn onboarding(&self) -> Option<&Entity<Onboarding>> {
        self.onboarding.as_ref()
    }
}

impl Render for SignInWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.theme().colors();
        div()
            .debug_selector(|| "sign-in-window".into())
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .child(match &self.onboarding {
                Some(ob) => crate::titlebar::column(ob.clone()),
                None => crate::titlebar::column(self.screen.clone()),
            })
    }
}

/// Opens the window over a screen (and the first-run screens ahead of it, when
/// given) made by `make`, which runs inside the new window so a provider's
/// webview attaches to it. The first-run screens are laid out for the main
/// window's size, so the window opens that large.
pub fn open(
    cx: &mut App,
    welcome: bool,
    make: impl FnOnce(&mut Window, &mut App) -> (Entity<SignInScreen>, Option<Entity<Onboarding>>),
) -> Option<WindowHandle<SignInWindow>> {
    let (w, h) = if welcome {
        (
            themes::metrics::LAYOUT_WINDOW_WIDTH,
            themes::metrics::LAYOUT_WINDOW_HEIGHT,
        )
    } else {
        (
            themes::metrics::LAYOUT_SIGN_IN_WIDTH,
            themes::metrics::LAYOUT_SIGN_IN_HEIGHT,
        )
    };
    let bounds = Bounds::centered(None, size(w, h), cx);
    let options = if welcome {
        crate::window_options::sign_in_welcome(bounds)
    } else {
        crate::window_options::sign_in(bounds)
    };
    cx.open_window(options, |window, cx| {
        let (screen, onboarding) = make(window, cx);
        let focus = match &onboarding {
            Some(ob) => ob.read(cx).focus_handle_for(cx),
            None => gpui::Focusable::focus_handle(screen.read(cx), cx),
        };
        window.focus(&focus, cx);
        cx.new(|_| SignInWindow { screen, onboarding })
    })
    .ok()
}
