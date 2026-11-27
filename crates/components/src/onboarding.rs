//! First-run screens around a provider's sign-in: Welcome, the provider
//! picker, the provider's own sign-in screen untouched, then the first-sync
//! progress fed by the workspace sessions. Provider-neutral: providers are
//! shown by the display name their sign-in reports.
//!
//! Each screen is an entity on a [`NavStack`]; [`Onboarding`] owns the flow,
//! its keys and its events, and moves the focus to the page it pushes.

use gpui::{
    AnyElement, App, Context, Div, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    KeyDownEvent, Render, SharedString, Subscription, WeakEntity, Window, div, prelude::*,
    relative,
};
use gpui_base::NavStackState;
use themes::{ActiveTheme, ThemeColors, metrics, parla_tokens};

use crate::controls::{ButtonKind, button_frame, error_banner, on_activate};
use crate::icon::{Icon, IconName};
use crate::nav_stack::NavStack;
use crate::sign_in::{SignInScreen, Step};

/// How many steps the stepper shows: Welcome, provider, sign in, pick
/// workspaces, first sync, tour.
pub const STEPS: usize = 6;

/// The steps of the add-workspace window: Welcome, provider, sign in, pick
/// workspaces. No first sync or tour follows; the main window already shows.
pub const ADDED_STEPS: usize = 4;

/// The screen the first-run flow is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnboardingStep {
    Welcome,
    Provider,
    /// The provider's own sign-in screen (consent, options, workspace picker).
    SignIn,
    FirstSync,
}

/// A provider the picker offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderCard {
    /// The display name the provider's sign-in reports.
    pub name: String,
}

/// How far one workspace's first sync got, from its session's own events: the
/// local store opened, the bootstrap answered, and the catch-up jobs ran.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncProgress {
    pub store_ready: bool,
    /// Conversations and people the bootstrap brought.
    pub directory: Option<(usize, usize)>,
    /// Catch-up jobs done and planned, once the counts answered.
    pub catch_up: Option<(usize, usize)>,
    /// The bootstrap failed or the session could not start.
    pub failed: bool,
}

impl SyncProgress {
    /// Whole percent: the store is a tenth, the bootstrap half, catch-up the rest.
    pub fn percent(&self) -> u8 {
        let mut p = 0usize;
        if self.store_ready {
            p += 10;
        }
        if self.directory.is_some() {
            p += 50;
        }
        if let Some((done, total)) = self.catch_up {
            p += (40 * done.min(total)).checked_div(total).unwrap_or(40);
        }
        p.min(100) as u8
    }

    /// Finished, or stopped by a failure the main window shows.
    pub fn done(&self) -> bool {
        self.failed
            || (self.directory.is_some()
                && matches!(self.catch_up, Some((done, total)) if done >= total))
    }
}

/// One workspace in the first sync.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceSync {
    pub name: String,
    pub progress: SyncProgress,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OnboardingEvent {
    /// The provider picker moved on with this provider (index into the cards).
    ProviderChosen(usize),
    /// "Open Parla" after the first sync.
    Finished,
    /// The language select picked a language: the host writes the `locale`
    /// setting, which applies live.
    LanguageChosen(settings::LocaleSetting),
    /// Back from Welcome: nothing earlier to go to. The add-workspace window
    /// closes on it; the first-run window has nothing to close and ignores it.
    Dismissed,
}

pub struct Onboarding {
    step: OnboardingStep,
    nav: Entity<NavStackState>,
    welcome: Entity<WelcomePage>,
    provider: Entity<ProviderPage>,
    sign_in_page: Entity<SignInPage>,
    first_sync: Entity<FirstSyncPage>,
    language: Entity<LanguagePicker>,
    providers: Vec<ProviderCard>,
    selected: usize,
    sign_in: Entity<SignInScreen>,
    _sign_in_changed: Subscription,
    _nav_changed: Subscription,
    sync: Vec<WorkspaceSync>,
    focus: FocusHandle,
    /// The interface language the select shows (the `locale` setting).
    language_setting: settings::LocaleSetting,
    /// Shown in the add-workspace window: a Cancel button on Welcome and a
    /// stepper without the first-sync and tour steps.
    added: bool,
    /// The page on top takes the focus on the next draw, once every entity of
    /// the push (a newly picked sign-in screen included) exists.
    focus_next_page: bool,
}

impl EventEmitter<OnboardingEvent> for Onboarding {}

impl Onboarding {
    /// Starts on Welcome, ahead of `sign_in` (the screen shown at the sign-in step).
    pub fn new(
        providers: Vec<ProviderCard>,
        sign_in: Entity<SignInScreen>,
        cx: &mut Context<Self>,
    ) -> Self {
        let parent = cx.entity().downgrade();
        let language = cx.new(|cx| LanguagePicker::new(parent.clone(), cx));
        let welcome = cx.new(|cx| WelcomePage::new(parent.clone(), language.clone(), cx));
        let provider = cx.new(|cx| ProviderPage::new(parent.clone(), language.clone(), cx));
        let sign_in_focus = sign_in.read(cx).focus_handle(cx);
        let sign_in_page = cx.new(|cx| SignInPage::new(parent.clone(), sign_in_focus, cx));
        let first_sync = cx.new(|cx| FirstSyncPage::new(parent, cx));
        let nav = cx.new(|_| NavStackState::new());
        nav.update(cx, |nav, cx| {
            nav.push(welcome.clone(), gpui_base::NavMotion::Immediate, cx);
        });
        let sub = cx.observe(&sign_in, |this, _, cx| {
            this.sign_in_page.update(cx, |_, cx| cx.notify());
            cx.notify();
        });
        let nav_sub = cx.observe(&nav, |_, _, cx| cx.notify());
        Self {
            step: OnboardingStep::Welcome,
            nav,
            welcome,
            provider,
            sign_in_page,
            first_sync,
            language,
            providers,
            selected: 0,
            sign_in,
            _sign_in_changed: sub,
            _nav_changed: nav_sub,
            sync: Vec::new(),
            focus: cx.focus_handle(),
            language_setting: settings::LocaleSetting::System,
            added: false,
            focus_next_page: false,
        }
    }

    /// Makes this the add-workspace window's flow (see `added`).
    pub fn for_added_workspace(mut self) -> Self {
        self.added = true;
        self
    }

    /// Whether this is the add-workspace window's flow.
    pub fn is_added_workspace(&self) -> bool {
        self.added
    }

    /// The number of steps the flow shows (four in the add-workspace window).
    pub fn steps(&self) -> usize {
        self.steps_for_added(self.added)
    }

    fn steps_for_added(&self, added: bool) -> usize {
        if added { ADDED_STEPS } else { STEPS }
    }

    /// The interface language the select shows.
    pub fn language(&self) -> &settings::LocaleSetting {
        &self.language_setting
    }

    /// Shows `language` in the select (the host reads the `locale` setting).
    pub fn set_language(&mut self, language: settings::LocaleSetting, cx: &mut Context<Self>) {
        if self.language_setting != language {
            self.language_setting = language.clone();
            self.language
                .update(cx, |picker, cx| picker.set_language(language, cx));
            cx.notify();
        }
    }

    /// Opens the language list (a test and screenshot hook).
    pub fn open_language(&mut self, cx: &mut Context<Self>) {
        self.language.update(cx, |picker, cx| picker.open(cx));
    }

    /// Picks a language: closes the list and tells the host. The select sets
    /// its own copy first, so this never re-enters the picker.
    pub fn choose_language(&mut self, language: settings::LocaleSetting, cx: &mut Context<Self>) {
        self.language_setting = language.clone();
        cx.emit(OnboardingEvent::LanguageChosen(language));
        cx.notify();
    }

    pub fn step(&self) -> OnboardingStep {
        self.step
    }

    /// The focus of the page on top: hosts focus the flow through it.
    pub fn focus_handle_for(&self, cx: &App) -> FocusHandle {
        match self.step {
            OnboardingStep::Welcome => self.welcome.read(cx).focus_handle_for(),
            OnboardingStep::Provider => self.provider.read(cx).focus_handle_for(),
            OnboardingStep::SignIn => self.sign_in.read(cx).focus_handle(cx),
            OnboardingStep::FirstSync => self.first_sync.read(cx).focus_handle_for(),
        }
    }

    /// Index of the highlighted provider card.
    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn sign_in(&self) -> &Entity<SignInScreen> {
        &self.sign_in
    }

    pub fn welcome_page(&self) -> &Entity<WelcomePage> {
        &self.welcome
    }

    pub fn provider_page(&self) -> &Entity<ProviderPage> {
        &self.provider
    }

    pub fn sign_in_page(&self) -> &Entity<SignInPage> {
        &self.sign_in_page
    }

    pub fn first_sync_page(&self) -> &Entity<FirstSyncPage> {
        &self.first_sync
    }

    /// Replaces the screen shown at the sign-in step (another provider).
    pub fn set_sign_in(&mut self, screen: Entity<SignInScreen>, cx: &mut Context<Self>) {
        let focus = screen.read(cx).focus_handle(cx);
        self._sign_in_changed = cx.observe(&screen, |this, _, cx| {
            this.sign_in_page.update(cx, |_, cx| cx.notify());
            cx.notify();
        });
        self.sign_in_page
            .update(cx, |page, cx| page.set_focus(focus, cx));
        self.sign_in = screen;
        cx.notify();
    }

    /// Moves on: Welcome to the provider picker, the picker to the sign-in,
    /// and the first sync to the main window once it is done.
    pub fn next(&mut self, cx: &mut Context<Self>) {
        match self.step {
            OnboardingStep::Welcome => {
                self.step = OnboardingStep::Provider;
                crate::nav_stack::push(&self.nav, self.provider.clone(), cx);
                self.focus_next_page = true;
            }
            OnboardingStep::Provider => {
                self.step = OnboardingStep::SignIn;
                cx.emit(OnboardingEvent::ProviderChosen(self.selected));
                crate::nav_stack::push(&self.nav, self.sign_in_page.clone(), cx);
                self.focus_next_page = true;
            }
            OnboardingStep::SignIn => return,
            OnboardingStep::FirstSync => return self.finish(cx),
        }
        cx.notify();
    }

    /// One screen back: the sign-in's first screen to the picker, the picker
    /// to Welcome, Welcome out of the flow. A running sign-in and the first sync have no way back.
    pub fn back(&mut self, cx: &mut Context<Self>) {
        match self.step {
            OnboardingStep::Welcome => return cx.emit(OnboardingEvent::Dismissed),
            OnboardingStep::Provider => self.step = OnboardingStep::Welcome,
            OnboardingStep::SignIn if matches!(self.sign_in.read(cx).step(), Step::Options) => {
                self.step = OnboardingStep::Provider
            }
            _ => return,
        }
        crate::nav_stack::pop(&self.nav, cx);
        self.focus_next_page = true;
        cx.notify();
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        cx.emit(OnboardingEvent::Dismissed);
    }

    /// The sign-in added these workspaces: show their first sync.
    pub fn start_first_sync(&mut self, names: Vec<String>, cx: &mut Context<Self>) {
        self.step = OnboardingStep::FirstSync;
        self.sync = names
            .into_iter()
            .map(|name| WorkspaceSync {
                name,
                progress: SyncProgress::default(),
            })
            .collect();
        crate::nav_stack::push(&self.nav, self.first_sync.clone(), cx);
        self.first_sync.update(cx, |_, cx| cx.notify());
        self.focus_next_page = true;
        cx.notify();
    }

    /// The latest progress of every workspace being synced.
    pub fn set_sync(&mut self, sync: Vec<WorkspaceSync>, cx: &mut Context<Self>) {
        if self.sync != sync {
            self.sync = sync;
            self.first_sync.update(cx, |_, cx| cx.notify());
            cx.notify();
        }
    }

    pub fn workspaces(&self) -> &[WorkspaceSync] {
        &self.sync
    }

    /// Every workspace finished (or failed, which the main window shows).
    pub fn can_finish(&self) -> bool {
        self.step == OnboardingStep::FirstSync && self.sync.iter().all(|w| w.progress.done())
    }

    /// "Open Parla": ends the flow once the first sync is done.
    pub fn finish(&mut self, cx: &mut Context<Self>) {
        if self.can_finish() {
            cx.emit(OnboardingEvent::Finished);
        }
    }

    fn move_selection(&mut self, by: isize, cx: &mut Context<Self>) {
        let n = self.providers.len() as isize;
        if n > 0 {
            self.selected = (self.selected as isize + by).rem_euclid(n) as usize;
            cx.notify();
        }
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let m = &k.modifiers;
        if m.platform || m.control || m.alt || m.shift {
            return;
        }
        match (k.key.as_str(), self.step) {
            ("enter", OnboardingStep::SignIn) => return,
            ("enter", _) => self.next(cx),
            ("escape", _) => self.back(cx),
            ("left", OnboardingStep::Provider) => self.move_selection(-1, cx),
            ("right", OnboardingStep::Provider) => self.move_selection(1, cx),
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Focusable for Onboarding {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The language select shared by the Welcome and provider headers: the
/// floating settings select, one option per shipped language and "System
/// default".
pub struct LanguagePicker {
    onboarding: WeakEntity<Onboarding>,
    language: settings::LocaleSetting,
    open: bool,
}

impl LanguagePicker {
    fn new(onboarding: WeakEntity<Onboarding>, _cx: &mut Context<Self>) -> Self {
        Self {
            onboarding,
            language: settings::LocaleSetting::System,
            open: false,
        }
    }

    pub fn language(&self) -> &settings::LocaleSetting {
        &self.language
    }

    pub fn set_language(&mut self, language: settings::LocaleSetting, cx: &mut Context<Self>) {
        if self.language != language {
            self.language = language;
            cx.notify();
        }
    }

    pub fn open(&mut self, cx: &mut Context<Self>) {
        self.open = true;
        cx.notify();
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        self.open = !self.open;
        cx.notify();
    }

    fn choose(&mut self, language: settings::LocaleSetting, cx: &mut Context<Self>) {
        self.open = false;
        self.language = language.clone();
        if let Some(onboarding) = self.onboarding.upgrade() {
            onboarding.update(cx, |onboarding, cx| {
                onboarding.choose_language(language, cx)
            });
        }
        cx.notify();
    }
}

impl Render for LanguagePicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::settings_controls::{select, select_option, select_trigger_with};
        let colors = *cx.theme().colors();
        let locales = i18n::LocaleSet::embedded();
        let system = t!(cx, "onboarding-language-system").into_owned();
        let current = match &self.language {
            settings::LocaleSetting::System => system.clone(),
            settings::LocaleSetting::Code(c) => locales
                .locales
                .iter()
                .find(|i| i.code.to_string() == *c)
                .map(|i| i.name.clone())
                .unwrap_or_else(|| c.clone()),
        };
        let trigger = on_activate(
            select_trigger_with(
                "onboarding-language",
                Some(IconName::Globe),
                current,
                self.open,
                &colors,
            ),
            cx,
            |this, _, cx| this.toggle(cx),
        )
        .aria_label(t!(cx, "onboarding-language").into_owned());
        let options = self.open.then(|| {
            let mut options = vec![on_activate(
                select_option(
                    "onboarding-language-system",
                    system,
                    self.language == settings::LocaleSetting::System,
                    &colors,
                ),
                cx,
                |this, _, cx| this.choose(settings::LocaleSetting::System, cx),
            )];
            for info in &locales.locales {
                let code = info.code.to_string();
                let choice = settings::LocaleSetting::Code(code.clone());
                let el = select_option(
                    format!("onboarding-language-{code}"),
                    info.name.clone(),
                    self.language == choice,
                    &colors,
                );
                options.push(on_activate(el, cx, move |this, _, cx| {
                    this.choose(choice.clone(), cx)
                }));
            }
            options
        });
        let this = cx.entity();
        select(
            "onboarding-language",
            trigger,
            options,
            &colors,
            move |_, cx| {
                this.update(cx, |v, cx| {
                    v.open = false;
                    cx.notify();
                })
            },
            window,
            cx,
        )
    }
}

/// The Welcome screen.
pub struct WelcomePage {
    onboarding: WeakEntity<Onboarding>,
    language: Entity<LanguagePicker>,
    focus: FocusHandle,
}

impl WelcomePage {
    fn new(
        onboarding: WeakEntity<Onboarding>,
        language: Entity<LanguagePicker>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            onboarding,
            language,
            focus: cx.focus_handle(),
        }
    }

    pub fn focus_handle_for(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn update(&self, cx: &mut App, f: impl FnOnce(&mut Onboarding, &mut Context<Onboarding>)) {
        if let Some(onboarding) = self.onboarding.upgrade() {
            onboarding.update(cx, f);
        }
    }

    fn next(&mut self, cx: &mut Context<Self>) {
        self.update(cx, |onboarding, cx| onboarding.next(cx));
        cx.notify();
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.update(cx, |onboarding, cx| onboarding.dismiss(cx));
        cx.notify();
    }
}

impl Render for WelcomePage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(onboarding) = self.onboarding.upgrade() else {
            return div().id("onboarding-page-welcome").size_full();
        };
        let (added, steps) = onboarding.read_with(cx, |onboarding, _| {
            (
                onboarding.added,
                onboarding.steps_for_added(onboarding.added),
            )
        });
        let colors = *cx.theme().colors();
        let tokens = parla_tokens(cx.theme());
        let prop = |icon: IconName, title: String, body: String| {
            div()
                .flex()
                .items_start()
                .gap(metrics::SPACE_5)
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(metrics::LAYOUT_WORKSPACE_BADGE)
                        .rounded(metrics::RADIUS_LG)
                        .bg(colors.accent_soft)
                        .child(
                            Icon::new(icon)
                                .size(metrics::ICON_SIZE_LG)
                                .color(tokens.accent_soft_text),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(metrics::SPACE_1)
                        .child(
                            div()
                                .text_size(metrics::FONT_SIZE_LG)
                                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(metrics::FONT_SIZE_SM)
                                .text_color(colors.text_muted)
                                .child(body),
                        ),
                )
        };
        let props = div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_7)
            .p(metrics::SPACE_8)
            .rounded(metrics::RADIUS_XL)
            .border_1()
            .border_color(colors.border)
            .bg(colors.surface_background)
            .shadow_md()
            .child(prop(
                IconName::Zap,
                t!(cx, "onboarding-value-light-title").into_owned(),
                t!(cx, "onboarding-value-light-body").into_owned(),
            ))
            .child(prop(
                IconName::Shield,
                t!(cx, "onboarding-value-encrypted-title").into_owned(),
                t!(cx, "onboarding-value-encrypted-body").into_owned(),
            ))
            .child(prop(
                IconName::Layers,
                t!(cx, "onboarding-value-workspaces-title").into_owned(),
                t!(cx, "onboarding-value-workspaces-body").into_owned(),
            ))
            .child(prop(
                IconName::Palette,
                t!(cx, "onboarding-value-yours-title").into_owned(),
                t!(cx, "onboarding-value-yours-body").into_owned(),
            ));
        let start = on_activate(
            arrow_button(
                "onboarding-next",
                t!(cx, "onboarding-get-started").into_owned(),
                ButtonKind::Primary,
                true,
                false,
                &colors,
            )
            .min_h(metrics::SPACE_11)
            .text_size(metrics::FONT_SIZE_LG),
            cx,
            |this, _, cx| this.next(cx),
        );
        // The add-workspace window has no earlier screen: a visible way out.
        let cancel = added.then(|| {
            let label = t!(cx, "sign-in-cancel").into_owned();
            on_activate(
                button_frame("onboarding-cancel", ButtonKind::Secondary, true, &colors)
                    .debug_selector(|| "onboarding-cancel".into())
                    .aria_label(label.clone())
                    .child(label),
                cx,
                |this, _, cx| this.dismiss(cx),
            )
        });
        let intro = div()
            .flex()
            .flex_col()
            .justify_center()
            .flex_1()
            .min_w_0()
            .gap(metrics::SPACE_8)
            .pl(metrics::SPACE_12)
            .pr(metrics::SPACE_11)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(metrics::SPACE_5)
                    .child(crate::logo::logo(metrics::SPACE_12, cx))
                    .child(
                        div()
                            .text_size(metrics::FONT_SIZE_4XL)
                            .font_weight(metrics::FONT_WEIGHT_BOLD)
                            .child(t!(cx, "sign-in-wordmark").into_owned()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(metrics::SPACE_5)
                    .child(
                        div()
                            .text_size(metrics::FONT_SIZE_4XL)
                            .font_weight(metrics::FONT_WEIGHT_BOLD)
                            .child(t!(cx, "onboarding-welcome-title").into_owned()),
                    )
                    .child(signature(cx))
                    .child(
                        div()
                            .max_w(metrics::LAYOUT_DIALOG_SM)
                            .text_size(metrics::FONT_SIZE_LG)
                            .text_color(colors.text_muted)
                            .child(t!(cx, "onboarding-welcome-body").into_owned()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(metrics::SPACE_5)
                    .max_w(metrics::LAYOUT_POPOVER_LG)
                    .child(start)
                    .children(cancel)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(metrics::SPACE_3)
                            .text_size(metrics::FONT_SIZE_XS)
                            .text_color(tokens.text_subtle)
                            .child(
                                Icon::new(IconName::Layers)
                                    .size(metrics::ICON_SIZE_SM)
                                    .color(tokens.text_subtle),
                            )
                            .child(t!(cx, "onboarding-welcome-next").into_owned()),
                    ),
            );
        div()
            .id("onboarding-page-welcome")
            .debug_selector(|| "onboarding-page-welcome".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.background)
            .child(header(
                1,
                steps,
                Some(self.language.clone().into_any_element()),
                cx,
            ))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .pb(metrics::SPACE_10)
                    .child(intro)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .flex_1()
                            .min_w_0()
                            .pl(metrics::SPACE_8)
                            .pr(metrics::SPACE_12)
                            .child(props),
                    ),
            )
            .child(footer(t!(cx, "onboarding-footer").into_owned(), cx))
    }
}

/// The provider picker.
pub struct ProviderPage {
    onboarding: WeakEntity<Onboarding>,
    language: Entity<LanguagePicker>,
    focus: FocusHandle,
}

impl ProviderPage {
    fn new(
        onboarding: WeakEntity<Onboarding>,
        language: Entity<LanguagePicker>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            onboarding,
            language,
            focus: cx.focus_handle(),
        }
    }

    pub fn focus_handle_for(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn update(&self, cx: &mut App, f: impl FnOnce(&mut Onboarding, &mut Context<Onboarding>)) {
        if let Some(onboarding) = self.onboarding.upgrade() {
            onboarding.update(cx, f);
        }
    }

    fn choose(&mut self, i: usize, cx: &mut Context<Self>) {
        self.update(cx, |onboarding, _| onboarding.selected = i);
        cx.notify();
    }

    fn next(&mut self, cx: &mut Context<Self>) {
        self.update(cx, |onboarding, cx| onboarding.next(cx));
        cx.notify();
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        self.update(cx, |onboarding, cx| onboarding.back(cx));
        cx.notify();
    }

    fn choose_and_next(&mut self, i: usize, cx: &mut Context<Self>) {
        self.update(cx, |onboarding, cx| {
            onboarding.selected = i;
            onboarding.next(cx);
        });
        cx.notify();
        cx.stop_propagation();
    }
}

impl Render for ProviderPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(onboarding) = self.onboarding.upgrade() else {
            return div().id("onboarding-page-provider").size_full();
        };
        let (providers, selected, steps) = onboarding.read_with(cx, |onboarding, _| {
            (
                onboarding.providers.clone(),
                onboarding.selected,
                onboarding.steps_for_added(onboarding.added),
            )
        });
        let colors = *cx.theme().colors();
        let tokens = parla_tokens(cx.theme());
        let hover = colors.element_hover;
        let mut grid = div()
            .id("provider-cards")
            .role(gpui::Role::RadioGroup)
            .aria_label(t!(cx, "onboarding-provider-title").into_owned())
            .flex()
            .flex_wrap()
            .gap(metrics::SPACE_5)
            .debug_selector(|| "provider-cards".into());
        for (i, p) in providers.iter().enumerate() {
            let on = i == selected;
            let card = div()
                .id(SharedString::from(format!("provider-{i}")))
                .debug_selector(move || format!("provider-card-{i}"))
                .flex()
                .flex_col()
                .gap(metrics::SPACE_5)
                .w(metrics::LAYOUT_PROVIDER_CARD)
                .p(metrics::SPACE_6)
                .rounded(metrics::RADIUS_LG)
                .border_1()
                .border_color(if on {
                    gpui::transparent_black()
                } else {
                    colors.border
                })
                .when(on, |d| d.shadow(crate::controls::ring(&colors)))
                .bg(colors.surface_background)
                .hover(move |d| d.bg(hover))
                .cursor_pointer()
                .role(gpui::Role::RadioButton)
                .aria_label(p.name.clone())
                .aria_selected(on)
                .tab_stop(true)
                .focusable()
                .focus_visible(crate::controls::focus_style(&colors))
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap(metrics::SPACE_5)
                        .child(mono(initials(&p.name), metrics::LAYOUT_WORKSPACE_BADGE, cx))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(metrics::FONT_SIZE_MD)
                                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                                .truncate()
                                .child(p.name.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .px(metrics::SPACE_4)
                                .rounded_full()
                                .bg(colors.accent_soft)
                                .text_color(tokens.accent_soft_text)
                                .text_size(metrics::FONT_SIZE_2XS)
                                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                                .child(t!(cx, "onboarding-provider-available").into_owned()),
                        )
                        .when(on, |d| {
                            d.child(
                                div()
                                    .debug_selector(|| "provider-card-check".into())
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .justify_center()
                                    .size(metrics::SPACE_7)
                                    .rounded_full()
                                    .bg(colors.element_background)
                                    .child(
                                        Icon::new(IconName::Check)
                                            .size(metrics::ICON_SIZE_SM)
                                            .color(colors.text_on_accent),
                                    ),
                            )
                        }),
                );
            // Click or Space picks the card; Enter picks it and moves on.
            let card = card
                .on_click(cx.listener(move |this, _, _, cx| this.choose(i, cx)))
                .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _window, cx| {
                    let k = &ev.keystroke;
                    if k.modifiers.platform || k.modifiers.control || k.modifiers.alt {
                        return;
                    }
                    match k.key.as_str() {
                        "space" => this.choose(i, cx),
                        "enter" => this.choose_and_next(i, cx),
                        _ => {}
                    }
                }));
            grid = grid.child(card);
        }
        let name = providers
            .get(selected)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let back = on_activate(
            arrow_button(
                "onboarding-back",
                t!(cx, "onboarding-back").into_owned(),
                ButtonKind::Secondary,
                true,
                true,
                &colors,
            ),
            cx,
            |this, _, cx| this.back(cx),
        );
        let next = on_activate(
            arrow_button(
                "onboarding-next",
                t!(cx, "onboarding-continue", provider = name).into_owned(),
                ButtonKind::Primary,
                !providers.is_empty(),
                false,
                &colors,
            ),
            cx,
            |this, _, cx| this.next(cx),
        );
        let main = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(metrics::SPACE_7)
            .px(metrics::SPACE_12)
            .pt(metrics::SPACE_5)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(metrics::SPACE_4)
                    .child(crate::logo::logo(metrics::SPACE_9, cx))
                    .child(
                        div()
                            .text_size(metrics::FONT_SIZE_3XL)
                            .font_weight(metrics::FONT_WEIGHT_BOLD)
                            .child(t!(cx, "onboarding-provider-title").into_owned()),
                    )
                    .child(
                        div()
                            .text_size(metrics::FONT_SIZE_MD)
                            .text_color(colors.text_muted)
                            .child(t!(cx, "onboarding-provider-body").into_owned()),
                    ),
            )
            .child(grid)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(metrics::SPACE_6)
                    .child(back)
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap(metrics::SPACE_4)
                            .text_size(metrics::FONT_SIZE_SM)
                            .text_color(colors.text_muted)
                            .child(
                                Icon::new(IconName::Info)
                                    .size(metrics::ICON_SIZE_SM)
                                    .color(colors.text_muted),
                            )
                            .child(t!(cx, "onboarding-provider-later").into_owned()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(metrics::FONT_SIZE_XS)
                            .text_color(tokens.text_subtle)
                            .child(t!(cx, "onboarding-provider-keys").into_owned()),
                    )
                    .child(next),
            );
        div()
            .id("onboarding-page-provider")
            .debug_selector(|| "onboarding-page-provider".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.background)
            .child(header(
                2,
                steps,
                Some(self.language.clone().into_any_element()),
                cx,
            ))
            .child(main)
            .child(footer(
                t!(cx, "onboarding-provider-footer").into_owned(),
                cx,
            ))
    }
}

/// The provider's sign-in on the flow's stack, with the stepper and Back above
/// its first screens. While it runs (a provider window over the screen, the
/// picker) it shows alone, so its layout is exactly what the provider expects.
pub struct SignInPage {
    onboarding: WeakEntity<Onboarding>,
    focus: FocusHandle,
}

impl SignInPage {
    fn new(
        onboarding: WeakEntity<Onboarding>,
        focus: FocusHandle,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self { onboarding, focus }
    }

    pub fn focus_handle_for(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn set_focus(&mut self, focus: FocusHandle, cx: &mut Context<Self>) {
        self.focus = focus;
        cx.notify();
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        if let Some(onboarding) = self.onboarding.upgrade() {
            onboarding.update(cx, |onboarding, cx| onboarding.back(cx));
        }
        cx.notify();
    }
}

impl Render for SignInPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(onboarding) = self.onboarding.upgrade() else {
            return div().id("onboarding-page-sign-in").size_full();
        };
        let (sign_in, steps) = onboarding.read_with(cx, |onboarding, _| {
            (
                onboarding.sign_in.clone(),
                onboarding.steps_for_added(onboarding.added),
            )
        });
        let screen = sign_in.read(cx).step().clone();
        let n = match screen {
            Step::Options | Step::Form(_) | Step::Email | Step::Code => 3,
            Step::Pick(_) => 4,
            Step::Waiting | Step::PickWorkspaces(_) => {
                return div()
                    .id("onboarding-page-sign-in")
                    .debug_selector(|| "onboarding-page-sign-in".into())
                    .track_focus(&self.focus)
                    .size_full()
                    .bg(cx.theme().colors().background)
                    .child(sign_in);
            }
        };
        let colors = *cx.theme().colors();
        let back = (n == 3 && matches!(screen, Step::Options)).then(|| {
            on_activate(
                arrow_button(
                    "onboarding-back",
                    t!(cx, "onboarding-back").into_owned(),
                    ButtonKind::Secondary,
                    true,
                    true,
                    &colors,
                ),
                cx,
                |this, _, cx| this.back(cx),
            )
            .min_h(metrics::LAYOUT_TITLEBAR_BUTTON)
            .px(metrics::SPACE_5)
            .text_size(metrics::FONT_SIZE_XS)
        });
        div()
            .id("onboarding-page-sign-in")
            .debug_selector(|| "onboarding-page-sign-in".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.background)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(metrics::SPACE_10)
                    .px(metrics::SPACE_6)
                    .child(div().flex_1().children(back))
                    .child(stepper(n, steps, cx))
                    .child(div().flex_1()),
            )
            .child(div().flex_1().min_h_0().child(sign_in))
    }
}

/// The first-sync progress of the workspaces the sign-in added.
pub struct FirstSyncPage {
    onboarding: WeakEntity<Onboarding>,
    focus: FocusHandle,
}

impl FirstSyncPage {
    fn new(onboarding: WeakEntity<Onboarding>, cx: &mut Context<Self>) -> Self {
        Self {
            onboarding,
            focus: cx.focus_handle(),
        }
    }

    pub fn focus_handle_for(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        if let Some(onboarding) = self.onboarding.upgrade() {
            onboarding.update(cx, |onboarding, cx| onboarding.finish(cx));
        }
        cx.notify();
    }
}

impl Render for FirstSyncPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(onboarding) = self.onboarding.upgrade() else {
            return div().id("onboarding-page-first-sync").size_full();
        };
        let (sync, can_finish) = onboarding.read_with(cx, |onboarding, _| {
            (onboarding.sync.clone(), onboarding.can_finish())
        });
        let colors = *cx.theme().colors();
        let tokens = parla_tokens(cx.theme());
        let error = cx.theme().status().error;
        let success = cx.theme().status().success;
        let current = sync.iter().position(|w| !w.progress.done()).unwrap_or(0);
        let ws = sync.get(current).cloned().unwrap_or_default();
        let p = &ws.progress;
        let pct = p.percent();
        #[derive(Clone, Copy, PartialEq)]
        enum State {
            Done,
            Now,
            Todo,
        }
        let row = |state: State, icon: IconName, label: String, detail: String, last: bool| {
            let (ground, fg) = match state {
                State::Done => (tokens.success_soft, success),
                State::Now => (tokens.signal_soft, tokens.signal_text),
                State::Todo => (tokens.bg_sunken, tokens.text_subtle),
            };
            let glyph = if state == State::Done {
                IconName::Check
            } else {
                icon
            };
            div()
                .flex()
                .items_center()
                .gap(metrics::SPACE_5)
                .h(metrics::SPACE_11)
                .px(metrics::SPACE_2)
                .when(!last, |d| d.border_b_1().border_color(tokens.border_subtle))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(metrics::LAYOUT_SYNC_STEP_ICON)
                        .rounded_full()
                        .bg(ground)
                        .when(state == State::Todo, |d| {
                            d.border_1().border_color(colors.border)
                        })
                        .child(Icon::new(glyph).size(metrics::ICON_SIZE_BASE).color(fg)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_weight(if state == State::Now {
                            metrics::FONT_WEIGHT_SEMIBOLD
                        } else {
                            metrics::FONT_WEIGHT_MEDIUM
                        })
                        .text_color(if state == State::Todo {
                            colors.text_muted
                        } else {
                            colors.text
                        })
                        .child(label),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(metrics::FONT_SIZE_XS)
                        .when(state == State::Now, |d| {
                            d.text_color(tokens.signal_text)
                                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                        })
                        .when(state != State::Now, |d| d.text_color(tokens.text_subtle))
                        .child(detail),
                )
        };
        let store = if p.store_ready {
            State::Done
        } else {
            State::Now
        };
        let directory = match (p.directory, p.store_ready && !p.failed) {
            (Some(_), _) => State::Done,
            (None, true) => State::Now,
            (None, false) => State::Todo,
        };
        let messages = match (p.catch_up, p.directory.is_some() && !p.failed) {
            (Some((d, t)), _) if d >= t => State::Done,
            (_, true) => State::Now,
            _ => State::Todo,
        };
        let waiting = t!(cx, "onboarding-sync-waiting").into_owned();
        let store_detail = if p.store_ready {
            t!(cx, "onboarding-sync-store-ready").into_owned()
        } else {
            t!(cx, "onboarding-sync-store-opening").into_owned()
        };
        let directory_detail = match p.directory {
            Some((c, m)) => t!(
                cx,
                "onboarding-sync-directory-count",
                conversations = c,
                people = m
            )
            .into_owned(),
            None => waiting.clone(),
        };
        let messages_detail = match p.catch_up {
            Some((d, t)) => t!(
                cx,
                "onboarding-sync-messages-count",
                done = d.min(t),
                total = t
            )
            .into_owned(),
            None => waiting.clone(),
        };
        let steps = div()
            .flex()
            .flex_col()
            .px(metrics::SPACE_6)
            .py(metrics::SPACE_2)
            .rounded(metrics::RADIUS_LG)
            .border_1()
            .border_color(colors.border)
            .bg(colors.surface_background)
            .text_size(metrics::FONT_SIZE_MD)
            .child(row(
                store,
                IconName::Lock,
                t!(cx, "onboarding-sync-store").into_owned(),
                store_detail,
                false,
            ))
            .child(row(
                directory,
                IconName::Users,
                t!(cx, "onboarding-sync-directory").into_owned(),
                directory_detail,
                false,
            ))
            .child(row(
                messages,
                IconName::Refresh,
                t!(cx, "onboarding-sync-messages").into_owned(),
                messages_detail,
                true,
            ));
        let bar = |fraction: f32, tall: bool| {
            div()
                .flex_1()
                .h(if tall {
                    metrics::SPACE_4
                } else {
                    metrics::SPACE_3
                })
                .rounded_full()
                .bg(tokens.bg_sunken)
                .overflow_hidden()
                .child(
                    div()
                        .h_full()
                        .w(relative(fraction.clamp(0.0, 1.0)))
                        .rounded_full()
                        .bg(tokens.signal),
                )
        };
        let mut col = div()
            .flex()
            .flex_col()
            .w(metrics::LAYOUT_FIRST_SYNC_WIDTH)
            .max_w_full()
            .h_full()
            .gap(metrics::SPACE_7)
            .pt(metrics::SPACE_7)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(metrics::SPACE_6)
                    .child(crate::logo::logo(metrics::SPACE_11, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(metrics::SPACE_1)
                            .child(
                                div()
                                    .text_size(metrics::FONT_SIZE_3XL)
                                    .font_weight(metrics::FONT_WEIGHT_BOLD)
                                    .truncate()
                                    .child(
                                        t!(
                                            cx,
                                            "onboarding-sync-title",
                                            workspace = ws.name.clone()
                                        )
                                        .into_owned(),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(metrics::FONT_SIZE_SM)
                                    .text_color(colors.text_muted)
                                    .child(t!(cx, "onboarding-sync-body").into_owned()),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(metrics::SPACE_4)
                    .child(
                        div()
                            .debug_selector(|| "first-sync-percent".into())
                            .text_size(metrics::FONT_SIZE_3XL)
                            .font_weight(metrics::FONT_WEIGHT_BOLD)
                            .child(t!(cx, "onboarding-sync-percent", percent = pct).into_owned()),
                    )
                    .child(div().flex().child(bar(f32::from(pct) / 100.0, true))),
            );
        if p.failed {
            col = col.child(error_banner(
                t!(cx, "onboarding-sync-failed", workspace = ws.name.clone()).into_owned(),
                error,
                &colors,
            ));
        }
        col = col.child(steps);
        let others: Vec<&WorkspaceSync> = sync
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != current)
            .map(|(_, w)| w)
            .collect();
        if !others.is_empty() {
            let mut list = div().flex().flex_col().gap(metrics::SPACE_5).child(
                div()
                    .text_size(metrics::FONT_SIZE_2XS)
                    .font_weight(metrics::FONT_WEIGHT_BOLD)
                    .text_color(tokens.text_subtle)
                    .child(t!(cx, "onboarding-sync-next").into_owned()),
            );
            for w in others {
                let status = if w.progress.done() {
                    t!(cx, "onboarding-sync-done").into_owned()
                } else if w.progress.percent() == 0 {
                    waiting.clone()
                } else {
                    t!(
                        cx,
                        "onboarding-sync-percent",
                        percent = w.progress.percent()
                    )
                    .into_owned()
                };
                list = list.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(metrics::SPACE_5)
                        .child(
                            mono(initials(&w.name), metrics::SPACE_8, cx)
                                .text_size(metrics::FONT_SIZE_2XS),
                        )
                        .child(
                            div()
                                .w(metrics::LAYOUT_FIELD_XS)
                                .flex_none()
                                .truncate()
                                .text_size(metrics::FONT_SIZE_SM)
                                .child(w.name.clone()),
                        )
                        .child(bar(f32::from(w.progress.percent()) / 100.0, false))
                        .child(
                            div()
                                .flex_none()
                                .text_size(metrics::FONT_SIZE_XS)
                                .text_color(tokens.text_subtle)
                                .child(status),
                        ),
                );
            }
            col = col.child(list);
        }
        let open = on_activate(
            arrow_button(
                "first-sync-open",
                t!(cx, "onboarding-open").into_owned(),
                ButtonKind::Primary,
                can_finish,
                false,
                &colors,
            ),
            cx,
            |this, _, cx| this.finish(cx),
        );
        col = col.child(div().flex_1()).child(
            div()
                .flex()
                .items_center()
                .gap(metrics::SPACE_5)
                .pb(metrics::SPACE_8)
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap(metrics::SPACE_3)
                        .text_size(metrics::FONT_SIZE_XS)
                        .text_color(tokens.text_subtle)
                        .child(
                            Icon::new(IconName::Info)
                                .size(metrics::ICON_SIZE_SM)
                                .color(tokens.text_subtle),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(t!(cx, "onboarding-sync-note").into_owned()),
                        ),
                )
                .child(open),
        );
        div()
            .id("onboarding-page-first-sync")
            .debug_selector(|| "onboarding-page-first-sync".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.background)
            .child(header(5, STEPS, None, cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .justify_center()
                    .px(metrics::SPACE_6)
                    .child(col),
            )
    }
}

/// The setup stepper (board Stepper): "Step n of 6", the step's name and one
/// segment per step, done ones in the accent, the current one in the signal.
pub fn stepper(current: usize, total: usize, cx: &App) -> Div {
    let colors = cx.theme().colors();
    let tokens = parla_tokens(cx.theme());
    let current = current.clamp(1, total);
    let l = i18n::gpui::localizer(cx);
    let name = match current {
        1 => t!(l, "onboarding-step-welcome"),
        2 => t!(l, "onboarding-step-provider"),
        3 => t!(l, "onboarding-step-sign-in"),
        4 => t!(l, "onboarding-step-pick"),
        5 => t!(l, "onboarding-step-sync"),
        _ => t!(l, "onboarding-step-tour"),
    }
    .into_owned();
    let mut segs = div().flex().items_center().gap(metrics::SPACE_2);
    for i in 1..=total {
        let (w, bg) = match i.cmp(&current) {
            std::cmp::Ordering::Less => (metrics::LAYOUT_STEPPER_SEGMENT, colors.text_accent),
            std::cmp::Ordering::Equal => (metrics::LAYOUT_STEPPER_SEGMENT_ACTIVE, tokens.signal),
            std::cmp::Ordering::Greater => (metrics::LAYOUT_STEPPER_SEGMENT, colors.border_strong),
        };
        segs = segs.child(div().w(w).h(metrics::SPACE_2).rounded_full().bg(bg));
    }
    div()
        .debug_selector(|| "onboarding-stepper".into())
        .flex()
        .items_center()
        .gap(metrics::SPACE_5)
        .text_size(metrics::FONT_SIZE_XS)
        .child(
            div()
                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                .text_color(colors.text_muted)
                .child(t!(l, "onboarding-step", step = current, total = total).into_owned()),
        )
        .child(
            div()
                .font_weight(metrics::FONT_WEIGHT_MEDIUM)
                .text_color(tokens.text_subtle)
                .child(name),
        )
        .child(segs)
}

/// A button with a label and a trailing or leading arrow.
fn arrow_button(
    id: &'static str,
    label: String,
    kind: ButtonKind,
    enabled: bool,
    leading: bool,
    colors: &ThemeColors,
) -> gpui::Stateful<Div> {
    let glyph = if kind == ButtonKind::Primary {
        colors.text_on_accent
    } else {
        colors.text
    };
    let icon = Icon::new(if leading {
        IconName::ArrowLeft
    } else {
        IconName::ArrowRight
    })
    .size(metrics::ICON_SIZE_BASE)
    .color(glyph);
    let frame = button_frame(id, kind, enabled, colors)
        .gap(metrics::SPACE_4)
        .debug_selector(move || id.into());
    if leading {
        frame.child(icon).child(label)
    } else {
        frame.child(label).child(icon)
    }
}

/// The footer line with an info glyph, along the bottom edge.
fn footer(text: String, cx: &App) -> Div {
    let tokens = parla_tokens(cx.theme());
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(metrics::SPACE_4)
        .px(metrics::SPACE_12)
        .py(metrics::SPACE_5)
        .border_t_1()
        .border_color(tokens.border_subtle)
        .text_size(metrics::FONT_SIZE_XS)
        .text_color(tokens.text_subtle)
        .child(
            Icon::new(IconName::Info)
                .size(metrics::ICON_SIZE_SM)
                .color(tokens.text_subtle),
        )
        .child(div().flex_1().min_w_0().child(text))
}

/// The header row: the stepper centred, the language select (when given) at
/// the right.
fn header(step: usize, total: usize, language: Option<AnyElement>, cx: &App) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(metrics::SPACE_10)
        .px(metrics::SPACE_6)
        .child(div().flex_1())
        .child(stepper(step, total, cx))
        .child(div().flex_1().flex().justify_end().children(language))
}

/// The brand signature: the three logo accents in a row.
fn signature(cx: &App) -> Div {
    let logo = parla_tokens(cx.theme()).logo;
    div()
        .flex()
        .flex_none()
        .w(metrics::SPACE_12)
        .h(metrics::LAYOUT_SIGNATURE_BAR)
        .child(div().flex_1().bg(logo.accent_1))
        .child(div().flex_1().bg(logo.accent_2))
        .child(div().flex_1().bg(logo.accent_3))
}

/// Two letters for a provider or workspace tile.
fn initials(name: &str) -> String {
    let mut words = name.split_whitespace();
    match (words.next(), words.next()) {
        (Some(a), Some(b)) => a.chars().take(1).chain(b.chars().take(1)).collect(),
        (Some(a), None) => {
            let mut c = a.chars();
            let first: String = c.next().into_iter().collect();
            let second: String = c
                .next()
                .map(|c| c.to_lowercase().collect())
                .unwrap_or_default();
            first + &second
        }
        _ => String::new(),
    }
}

/// A square tile with initials on the sunken ground.
fn mono(text: String, size: gpui::Pixels, cx: &App) -> Div {
    let colors = cx.theme().colors();
    let tokens = parla_tokens(cx.theme());
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(size)
        .rounded(metrics::RADIUS_MD)
        .bg(tokens.bg_sunken)
        .border_1()
        .border_color(tokens.border_subtle)
        .text_color(colors.text)
        .text_size(metrics::FONT_SIZE_MD)
        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
        .child(text)
}

impl Render for Onboarding {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_next_page) {
            let focus = self.focus_handle_for(cx);
            window.focus(&focus, cx);
        }
        let colors = *cx.theme().colors();
        div()
            .id("onboarding")
            .debug_selector(|| "onboarding".into())
            .key_context("Onboarding")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .child(NavStack::new(&self.nav).size_full())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_take_two_letters() {
        assert_eq!(initials("Example"), "Ex");
        assert_eq!(initials("North Wind"), "NW");
        assert_eq!(initials(""), "");
    }

    #[test]
    fn percent_weights_each_stage() {
        let mut p = SyncProgress::default();
        assert_eq!(p.percent(), 0);
        p.store_ready = true;
        assert_eq!(p.percent(), 10);
        p.directory = Some((1, 1));
        assert_eq!(p.percent(), 60);
        p.catch_up = Some((1, 2));
        assert_eq!(p.percent(), 80);
        assert!(!p.done());
        p.catch_up = Some((2, 2));
        assert_eq!(p.percent(), 100);
        assert!(p.done());
        assert!(
            SyncProgress {
                failed: true,
                ..Default::default()
            }
            .done()
        );
    }
}
