//! A workspace layout over a [`Scenario`]: sidebar, timeline, composer and
//! thread panel as the app composes them, driven by [`ScenarioSession`].

use std::rc::Rc;
use std::sync::Arc;

use anyhow::Result;
use components::code_sign_in::{CodeAlphabet, CodeError, CodeSent, CodeSignIn, CodeSpec};
use components::media::{ImageCache, MediaError, MediaFetcher};
use components::sign_in::{
    SignIn, SignInError, SignInOption, SignInResult, SignInScreen, SignInText,
};
use components::workspace_picker::{
    RefreshStep, WorkspaceAdder, WorkspacePicker, WorkspacePickerModel,
};
use components::{Composer, HeaderActions, SearchPanel, Sidebar, ThreadPanel, Timeline};
use domain::{ConversationId, ImageRef, MessageId};
use futures::future::BoxFuture;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, Task, Window, div, px,
};
use image::RgbaImage;
use settings::{Clock, WorkspaceKey};
use themes::{ActiveTheme as _, parla_tokens};

use crate::harness::UiHarness;
use crate::scenario::{Chrome, Scenario, SignInStage};
use crate::source::ScenarioSession;

/// Draws a banner strip; the host supplies it because banners live in the app crate.
pub type BannerFn = Box<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// Builds the toasts the stage shows (the host localises their text).
pub type ToastsFn = Box<dyn Fn(&mut App) -> Vec<components::Toast>>;

/// What the stage shows on top of the scenario's own state.
#[derive(Default)]
pub struct StageOptions {
    /// Open the emoji picker on the newest message, searching "kissing".
    pub picker: bool,
    /// Strip drawn above the timeline.
    pub banner: Option<BannerFn>,
    /// Toasts in the stack, bottom left of the main pane.
    pub toasts: Option<ToastsFn>,
    /// Open the quick switcher over the window with these items, typed with the query.
    pub switcher: Option<(Vec<components::SwitchItem>, String)>,
    /// Open the command palette over the window with these actions, typed with the query.
    pub command_palette: Option<(Vec<components::CommandItem>, String)>,
    /// Open a notification menu at a point (logical pixels from the top left).
    pub menu: Option<(components::MenuKind, (f32, f32))>,
    /// Show the scenario's thread as the pop-out window does: `banner` is the
    /// window header, the panel's own header is off.
    pub popout: bool,
}

/// A sign-in provider with two options and a consent notice, for the sign-in screen.
struct ExampleSignIn {
    /// Strings about 40% longer, as in a pseudo-locale.
    long: bool,
    /// The email-code sign-in, for the `sign_in_stage` scenarios.
    code: Option<Rc<ExampleCodeSignIn>>,
}

/// A scripted email-code sign-in: each call takes its answer; `None` never answers.
struct ExampleCodeSignIn {
    send: std::cell::RefCell<Option<Result<CodeSent, CodeError>>>,
    confirm: std::cell::RefCell<Option<Result<SignInResult, CodeError>>>,
}

/// Six digits in two groups of three, as the Example service sends.
fn example_spec() -> CodeSpec {
    CodeSpec {
        length: 6,
        alphabet: CodeAlphabet::Digits,
        group: Some(3),
    }
}

fn example_sent(resend_after: u64) -> CodeSent {
    CodeSent {
        spec: example_spec(),
        resend_after: std::time::Duration::from_secs(resend_after),
    }
}

fn never<T: 'static>(cx: &mut App) -> Task<T> {
    cx.spawn(async move |_| futures::future::pending::<T>().await)
}

impl CodeSignIn for ExampleCodeSignIn {
    fn send_code(&self, _: &str, _: bool, cx: &mut App) -> Task<Result<CodeSent, CodeError>> {
        match self.send.borrow_mut().take() {
            Some(r) => Task::ready(r),
            None => never(cx),
        }
    }
    fn confirm_code(
        &self,
        _: &str,
        _: domain::SecretText,
        _: futures::channel::mpsc::UnboundedSender<RefreshStep>,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<SignInResult, CodeError>> {
        match self.confirm.borrow_mut().take() {
            Some(r) => Task::ready(r),
            None => never(cx),
        }
    }
    fn forget(&self) {}
}

/// Drives a sign-in screen with a code sign-in to `stage`.
fn stage_code(
    screen: &Entity<SignInScreen>,
    code: &ExampleCodeSignIn,
    stage: SignInStage,
    window: &mut Window,
    cx: &mut App,
) {
    use SignInStage::*;
    const EMAIL: &str = "person@example.test";
    let send = match stage {
        EmailNeedsBrowser => Some(Err(CodeError::NeedsBrowser {
            option: "window".into(),
            reason: "This workspace signs in with single sign-on.".into(),
        })),
        EmailRateLimited => Some(Err(CodeError::RateLimited {
            retry_after: Some(std::time::Duration::from_secs(150)),
        })),
        EmailOffline => Some(Err(CodeError::Offline)),
        CodeResent => Some(Ok(example_sent(30))),
        _ => None,
    };
    *code.send.borrow_mut() = send;
    *code.confirm.borrow_mut() = match stage {
        CodeWrong => Some(Err(CodeError::WrongCode {
            attempts_left: Some(2),
        })),
        CodeExpired => Some(Err(CodeError::Expired)),
        CodeTooMany => Some(Err(CodeError::TooManyAttempts)),
        CodeRateLimited => Some(Err(CodeError::RateLimited {
            retry_after: Some(std::time::Duration::from_secs(150)),
        })),
        _ => None,
    };
    screen.update(cx, |s, cx| {
        s.set_consent(true, cx);
        s.start_email(window, cx);
        match stage {
            Email => {}
            EmailInvalid => {
                s.set_code_email("person@example", cx);
                s.submit_email(window, cx);
            }
            EmailSending | EmailNeedsBrowser | EmailRateLimited | EmailOffline => {
                s.set_code_email(EMAIL, cx);
                s.submit_email(window, cx);
            }
            Code | CodeResent => {
                s.show_code(EMAIL, example_sent(0), cx);
                if stage == CodeResent {
                    s.resend(window, cx);
                }
            }
            CodeResendWait => s.show_code(EMAIL, example_sent(30), cx),
            CodePartial => {
                s.show_code(EMAIL, example_sent(30), cx);
                s.type_code("123", window, cx);
            }
            CodeVerifying | CodeWrong | CodeExpired | CodeTooMany | CodeRateLimited => {
                s.show_code(EMAIL, example_sent(30), cx);
                s.type_code("123456", window, cx);
            }
        }
    });
}

impl SignIn for ExampleSignIn {
    fn provider_name(&self) -> String {
        "Example".into()
    }
    fn email_code(&self) -> Option<Rc<dyn CodeSignIn>> {
        self.code.clone().map(|c| c as Rc<dyn CodeSignIn>)
    }
    fn notice(&self) -> Vec<SignInText> {
        vec![
            if self.long {
                "This is an unofficial client, written independently of the service it connects to."
            } else {
                "This is an unofficial client."
            }
            .into(),
            if self.long {
                "It uses your own session and stores it safely in the keychain of your operating system."
            } else {
                "It uses your own session and stores it in the system keychain."
            }
            .into(),
        ]
    }
    fn options(&self) -> Vec<SignInOption> {
        vec![
            SignInOption {
                id: "window".into(),
                label: if self.long {
                    "Sign in with a separate browser window on this computer"
                } else {
                    "Sign in with a browser window"
                }
                .into(),
                fields: vec![],
                help: None,
                copy: None,
            },
            SignInOption {
                id: "paste".into(),
                label: if self.long {
                    "Paste a session token and cookie from your browser"
                } else {
                    "Paste a token"
                }
                .into(),
                fields: vec![components::sign_in::SignInField {
                    id: "token".into(),
                    label: if self.long {
                        "Session token copied from your browser"
                    } else {
                        "Session token"
                    }
                    .into(),
                    secret: true,
                }],
                help: Some(
                    if self.long {
                        "Copy the token from a browser where you are already signed in, then paste it into the field below."
                    } else {
                        "Copy the token from a browser where you are signed in."
                    }
                    .into(),
                ),
                copy: None,
            },
        ]
    }
    fn run(
        &self,
        _option: &str,
        _values: Vec<(String, String)>,
        _: futures::channel::mpsc::UnboundedSender<RefreshStep>,
        _: &mut Window,
        _: &mut App,
    ) -> Task<Result<SignInResult, SignInError>> {
        Task::ready(Ok(SignInResult::Workspaces(Vec::new())))
    }
    fn sign_out(&self, _: &WorkspaceKey, _: &mut App) -> Task<Result<(), String>> {
        Task::ready(Ok(()))
    }
}

/// A decoder that shows no frames and stays where it is put, for clip scenarios.
struct StillPlayer(std::cell::Cell<u64>);

impl components::platform::VideoPlayer for StillPlayer {
    fn open(&mut self, _: &std::path::Path) -> Result<(), components::platform::ClipError> {
        Ok(())
    }
    fn play(&mut self) {}
    fn pause(&mut self) {}
    fn seek(&mut self, ms: u64) {
        self.0.set(ms);
    }
    fn frame(&mut self) -> Option<components::platform::Frame> {
        None
    }
    fn position_ms(&self) -> u64 {
        self.0.get()
    }
}

/// Opens the scenario's clip as a click and a download would, then pauses it
/// at the staged position with the staged speed, volume and captions.
fn stage_clip(
    timeline: &Entity<Timeline<ScenarioSession>>,
    session: &Entity<ScenarioSession>,
    open: &ConversationId,
    c: &crate::scenario::StagedClip,
    window: &mut Window,
    cx: &mut Context<Stage>,
) {
    let (file, msg) = session
        .read(cx)
        .state()
        .windows
        .get(open)
        .and_then(|w| {
            w.messages().iter().find_map(|m| {
                m.files
                    .iter()
                    .find(|f| &*f.id.0 == c.file)
                    .map(|f| (f.clone(), m.id.clone()))
            })
        })
        .expect("the staged clip is in the open conversation");
    // Nothing is written: the click only computes the cache path.
    let mut ctl = components::clip::ClipController::new(
        &file,
        &std::env::temp_dir(),
        Box::new(|| Box::new(StillPlayer(std::cell::Cell::new(0)))),
    );
    ctl.click();
    ctl.downloaded();
    // Seek while playing, then pause: a paused seek would wait for a frame.
    ctl.seek_to(c.position_ms);
    ctl.click();
    for _ in 0..c.speed_steps {
        ctl.cycle_speed();
    }
    ctl.set_volume(c.volume);
    if let Some(vtt) = c.captions {
        ctl.toggle_captions();
        ctl.captions_loaded(components::vtt::parse(vtt));
    }
    timeline.update(cx, |t, cx| t.insert_clip(file, msg, ctl, cx));
    if c.focus_seek {
        timeline.update(cx, |t, cx| t.focus_clip_seek(window, cx));
        // A key press makes the focus ring show, as after Tab. Deferred: the
        // window has no root view until the stage is built.
        let handle = window.window_handle();
        cx.defer(move |cx| {
            handle
                .update(cx, |_, window, cx| {
                    window.dispatch_event(
                        gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                            keystroke: gpui::Keystroke::parse("f19").expect("a key"),
                            is_held: false,
                            prefer_character_input: false,
                        }),
                        cx,
                    );
                })
                .ok();
        });
    }
}

/// Adds nothing: the visual scenario only captures the list in its ticked state.
struct ExampleAdder;

impl WorkspaceAdder for ExampleAdder {
    fn add(
        &self,
        _: Vec<String>,
        _: futures::channel::mpsc::UnboundedSender<components::workspace_picker::AddStep>,
        _: &mut Window,
        _: &mut App,
    ) -> Task<Result<Vec<components::sign_in::Candidate>, SignInError>> {
        Task::ready(Ok(Vec::new()))
    }
}

// One short-lived stage per scenario: boxing the workspace variant buys nothing.
#[allow(clippy::large_enum_variant)]
enum Body {
    SignIn(Entity<SignInScreen>),
    WorkspacePicker(Entity<WorkspacePicker>),
    /// The pop-out thread window: the host's header over the panel.
    Popout(Entity<ThreadPanel<ScenarioSession>>),
    Workspace {
        sidebar: Entity<Sidebar<ScenarioSession>>,
        timeline: Entity<Timeline<ScenarioSession>>,
        composer: Entity<Composer<ScenarioSession>>,
        thread: Option<Entity<ThreadPanel<ScenarioSession>>>,
        scheduled: Option<Entity<components::ScheduledView<ScenarioSession>>>,
        reminders: Option<Entity<components::RemindersView<ScenarioSession>>>,
        drafts: Option<Entity<components::DraftsView<ScenarioSession>>>,
        people: Option<Entity<components::PeopleDirectory<ScenarioSession>>>,
        /// The open conversation's Files tab, shown instead of the messages.
        files: Option<Entity<components::FilesTab<ScenarioSession>>>,
        /// Later or Threads in place of the conversation.
        collection: Option<StageCollection>,
        /// The pins popover under the header.
        pins: Option<Entity<components::PinsPopover<ScenarioSession>>>,
        /// The bookmarks bar under the header.
        bookmarks: Option<Entity<components::BookmarksBar<ScenarioSession>>>,
        /// The huddle bar above the timeline.
        huddle: Option<Entity<components::HuddleBar<ScenarioSession>>>,
        /// The call bar of a running call.
        call_bar: Option<Entity<components::CallBar>>,
        profile: Option<Entity<components::ProfilePanel<ScenarioSession>>>,
        details: Option<Entity<components::ChannelDetailsPanel<ScenarioSession>>>,
        /// The status dialog over the window.
        dialog: Option<Entity<components::StatusDialog>>,
        /// A channel admin dialog over the window, and its width.
        admin: Option<(gpui::AnyView, gpui::Pixels)>,
        search: Option<Entity<SearchPanel<ScenarioSession>>>,
        /// A cross-workspace view in place of the conversation.
        aggregate: Option<Entity<components::AggregateView<crate::source::AggregateFixture>>>,
        /// The rail and header, for scenarios that show them.
        chrome: Option<Box<Chrome>>,
        session: Entity<ScenarioSession>,
        open: ConversationId,
    },
}

enum StageCollection {
    Later(Entity<components::SavedView<ScenarioSession>>),
    Threads(Entity<components::ThreadsView<ScenarioSession>>),
}

/// Serves one small generated picture for every image, so the rail icon needs
/// no network.
struct IconFetcher;

impl MediaFetcher for IconFetcher {
    fn fetch(&self, src: &ImageRef, _: u64) -> BoxFuture<'static, Result<Vec<u8>, MediaError>> {
        // Avatars (`https://avatars.example.test/<name>.png`) get a flat colour
        // and a lighter disc per name; every other image keeps the icon.
        let avatar = match src {
            ImageRef::Web(u) => u.strip_prefix("https://avatars.example.test/"),
            ImageRef::Provider(u) => u.0.strip_prefix("https://avatars.example.test/"),
        };
        if let Some(name) = avatar {
            // `edge-*`: a flat picture with dark blue corners, so a square
            // photo leaking outside its rounded edge shows.
            let edge = name.starts_with("edge-");
            let hue = name
                .bytes()
                .fold(7u32, |h, b| h.wrapping_mul(31) ^ u32::from(b))
                % 3;
            let (bg, fg) = match hue {
                0 => ([200, 90, 60], [250, 220, 200]),
                1 => ([70, 90, 190], [210, 220, 250]),
                _ => ([60, 150, 90], [210, 245, 220]),
            };
            let wide = name.starts_with("edge-wide");
            let w = if wide { 108 } else { 72 };
            let img = RgbaImage::from_fn(w, 72, |x, y| {
                let x = if wide {
                    x.saturating_sub(18).min(71)
                } else {
                    x
                };
                let (dx, dy) = (x as f32 - 36.0, y as f32 - 30.0);
                let head = (dx * dx + dy * dy).sqrt() < 13.0;
                let body = y > 50 && (x as f32 - 36.0).abs() < 24.0;
                let corner = edge && (wide || !(14..58).contains(&x)) && !(14..58).contains(&y);
                let c = if corner {
                    [0, 0, 120]
                } else if head || body {
                    fg
                } else {
                    bg
                };
                image::Rgba([c[0], c[1], c[2], 255])
            });
            let mut out = std::io::Cursor::new(Vec::new());
            let r = img
                .write_to(&mut out, image::ImageFormat::Png)
                .map(|()| out.into_inner())
                .map_err(|_| MediaError::Decode);
            return Box::pin(std::future::ready(r));
        }
        let img = RgbaImage::from_fn(72, 72, |x, y| {
            let on_ring = {
                let (dx, dy) = (x as f32 - 36.0, y as f32 - 36.0);
                let d = (dx * dx + dy * dy).sqrt();
                (14.0..24.0).contains(&d)
            };
            if on_ring {
                image::Rgba([250, 250, 250, 255])
            } else {
                image::Rgba([30, 110, 90, 255])
            }
        });
        let mut out = std::io::Cursor::new(Vec::new());
        let r = img
            .write_to(&mut out, image::ImageFormat::Png)
            .map(|()| out.into_inner())
            .map_err(|_| MediaError::Decode);
        Box::pin(std::future::ready(r))
    }
}

/// An open notification menu and its position.
type StageMenu = (Entity<components::NotifyMenu<ScenarioSession>>, (f32, f32));

/// The root view of one scenario.
pub struct Stage {
    body: Body,
    banner: Option<BannerFn>,
    switcher: Option<Entity<components::QuickSwitcher>>,
    command_palette: Option<Entity<components::CommandPalette>>,
    menu: Option<StageMenu>,
    rail_scroll: components::rail::RailScroll,
}

impl Stage {
    pub fn new(
        s: &Scenario,
        mut opts: StageOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        if let Some(make) = opts.toasts.take() {
            for t in make(cx) {
                components::Toasts::push(cx, t);
            }
        }
        if s.sign_in {
            let code = s.sign_in_stage.map(|_| {
                Rc::new(ExampleCodeSignIn {
                    send: Default::default(),
                    confirm: Default::default(),
                })
            });
            let screen = cx.new(|cx| {
                SignInScreen::new(
                    Rc::new(ExampleSignIn {
                        long: s.sign_in_long,
                        code: code.clone(),
                    }),
                    cx,
                )
            });
            if let (Some(stage), Some(code)) = (s.sign_in_stage, &code) {
                stage_code(&screen, code, stage, window, cx);
            }
            if s.sign_in_form || s.sign_in_error {
                let option = if s.sign_in_form { "paste" } else { "window" };
                screen.update(cx, |screen, cx| {
                    screen.set_consent(true, cx);
                    screen.choose(option, window, cx);
                });
            }
            return Self {
                rail_scroll: Default::default(),
                body: Body::SignIn(screen),
                banner: None,
                switcher: None,
                command_palette: None,
                menu: None,
            };
        }
        if let Some(choices) = &s.workspace_picker {
            let adder: Rc<dyn WorkspaceAdder> = Rc::new(ExampleAdder);
            let refresh = choices
                .iter()
                .filter(|c| c.updating)
                .map(|c| c.id.clone())
                .collect();
            let picker = cx.new(|cx| {
                WorkspacePicker::new(
                    WorkspacePickerModel::new(choices.clone()),
                    adder,
                    Vec::new(),
                    refresh,
                    cx,
                )
            });
            return Self {
                rail_scroll: Default::default(),
                body: Body::WorkspacePicker(picker),
                banner: None,
                switcher: None,
                command_palette: None,
                menu: None,
            };
        }
        let session = cx.new(|_| ScenarioSession::new(s));
        let open = s
            .open
            .clone()
            .expect("a workspace scenario opens a conversation");
        let sidebar = cx.new(|cx| Sidebar::new(session.clone(), cx));
        sidebar.update(cx, |v, cx| {
            if s.sidebar_organise {
                v.set_organising(true, window, cx);
            }
            // Under the workspace name; the section menu under its header.
            if s.sidebar_settings {
                v.open_settings(gpui::point(px(176.0), px(48.0)), window, cx);
            }
            if let Some(id) = s.section_menu {
                v.open_section_menu(id, None, window, cx);
            }
        });
        let timeline = {
            let (ss, o) = (session.clone(), open.clone());
            cx.new(|cx| Timeline::new(ss, o, Clock::H24, cx))
        };
        let composer = {
            let (ss, o) = (session.clone(), open.clone());
            cx.new(|cx| Composer::new(ss, o, cx))
        };
        if let Some(d) = &s.draft {
            let last = session
                .read(cx)
                .state()
                .windows
                .get(&open)
                .and_then(|w| w.messages().last().map(|m| m.id.clone()));
            composer.update(cx, |c, cx| match (d.editing, last) {
                (true, Some(id)) => c.begin_edit(id, d.text.clone(), cx),
                _ if d.typed => c
                    .input()
                    .clone()
                    .update(cx, |i, cx| i.replace_selection(&d.text, cx)),
                _ => c.set_text(d.text.clone(), cx),
            });
            let input = composer.read(cx).input().clone();
            if let Some(r) = d.select.clone() {
                input.update(cx, |t, cx| t.select(r, cx));
            }
            if let Some(m) = &d.composing {
                use gpui::EntityInputHandler as _;
                input.update(cx, |t, cx| {
                    t.replace_and_mark_text_in_range(None, m, None, window, cx)
                });
            }
            if d.link_prompt {
                composer.update(cx, |c, cx| c.begin_link(window, cx));
            }
            if d.emoji_picker {
                composer.update(cx, |c, cx| c.toggle_emoji_picker(window, cx));
                if d.emoji_picker_custom
                    && let Some(p) = composer.read(cx).emoji_picker().cloned()
                {
                    p.update(cx, |p, cx| p.select_tab(components::PickerTab::Custom, cx));
                }
            }
            if let Some(text) = &d.snippet_paste {
                composer.update(cx, |c, cx| c.offer_snippet(text.clone(), window, cx));
            }
            if let Some(stage) = d.send_later {
                composer.update(cx, |c, cx| {
                    c.open_send_later(cx);
                    if stage == crate::scenario::SendLaterStage::CustomPast {
                        c.choose_send_later(usize::MAX, window, cx);
                        // The last row is "Custom..."; any index past the presets opens it.
                        c.set_custom("2023-11-16", "08:00", cx);
                    }
                });
            }
            if d.select.is_some() || d.composing.is_some() {
                let handle = input.read(cx).focus_handle_for_test();
                window.focus(&handle, cx);
            }
        }
        if let Some(c) = &s.clip {
            stage_clip(&timeline, &session, &open, c, window, cx);
        }
        for id in &s.expanded_snippets {
            let id = domain::FileId(std::sync::Arc::from(*id));
            timeline.update(cx, |t, cx| t.toggle_snippet(id, cx));
        }
        if let Some(ts) = &s.selected {
            let id = MessageId {
                conversation: open.clone(),
                ts: ts.clone(),
            };
            timeline.update(cx, |t, cx| t.select(Some(id), cx));
        }
        let scheduled = s.scheduled.as_ref().filter(|x| x.view).map(|x| {
            let ss = session.clone();
            let view = cx.new(|cx| components::ScheduledView::new(ss, cx));
            if let Some(id) = x.edit {
                view.update(cx, |v, cx| {
                    v.begin_edit(domain::ScheduleId(std::sync::Arc::from(id)), window, cx)
                });
            }
            view
        });
        let reminders = s.reminders.as_ref().filter(|x| x.view).map(|x| {
            let ss = session.clone();
            let view = cx.new(|cx| components::RemindersView::new(ss, cx));
            let id = |s: &str| domain::ReminderId(std::sync::Arc::from(s));
            if let Some(r) = x.edit {
                view.update(cx, |v, cx| {
                    v.begin_edit(id(r), window, cx);
                    let text = v.edit_text(cx);
                    v.set_edit(&text, "tomorrow at 09:00", cx);
                    v.set_edit_repeat(domain::ReminderRepeat::Daily, cx);
                });
            }
            if let Some(r) = x.snooze {
                view.update(cx, |v, cx| v.toggle_snooze(id(r), cx));
            }
            view
        });
        let drafts = s.saved_drafts.as_ref().filter(|x| x.view).map(|_| {
            let ss = session.clone();
            cx.new(|cx| components::DraftsView::new(ss, cx))
        });
        let people = s.people.as_ref().map(|st| {
            let ss = session.clone();
            let q = st.query;
            cx.new(|cx| {
                let mut v = components::PeopleDirectory::new(ss, cx);
                v.set_query(q, cx);
                v
            })
        });
        let files = s.files_tab.then(|| {
            let (ss, c) = (session.clone(), open.clone());
            cx.new(|cx| components::FilesTab::new(ss, c, cx))
        });
        let (collection, pins, bookmarks) = match &s.collections {
            Some(c) => {
                if let Some(ts) = &c.menu {
                    let id = MessageId {
                        conversation: open.clone(),
                        ts: ts.clone(),
                    };
                    timeline.update(cx, |t, cx| t.open_menu(id, cx));
                }
                let collection = match c.show {
                    crate::scenario::CollectionsShow::Later => {
                        let ss = session.clone();
                        Some(StageCollection::Later(
                            cx.new(|cx| components::SavedView::new(ss, cx)),
                        ))
                    }
                    crate::scenario::CollectionsShow::Threads => {
                        let ss = session.clone();
                        Some(StageCollection::Threads(
                            cx.new(|cx| components::ThreadsView::new(ss, cx)),
                        ))
                    }
                    _ => None,
                };
                let pins = (c.show == crate::scenario::CollectionsShow::Pins).then(|| {
                    let ss = session.clone();
                    let o = open.clone();
                    cx.new(|cx| components::PinsPopover::new(ss, o, cx))
                });
                let bookmarks = c.bookmarks.as_ref().map(|_| {
                    let ss = session.clone();
                    let o = open.clone();
                    cx.new(|cx| components::BookmarksBar::new(ss, o, cx))
                });
                (collection, pins, bookmarks)
            }
            None => (None, None, None),
        };
        let call_bar = s
            .call
            .clone()
            .map(|m| cx.new(|cx| components::CallBar::new(m, cx)));
        let huddle = (!s.huddles.is_empty() || s.start_huddle).then(|| {
            let ss = session.clone();
            let o = open.clone();
            cx.new(|cx| components::HuddleBar::new(ss, o, cx))
        });
        let profile = s.profile.as_ref().map(|_| {
            let ss = session.clone();
            cx.new(|cx| components::ProfilePanel::new(ss, cx))
        });
        let details = s.details.map(|tab| {
            let ss = session.clone();
            cx.new(|cx| {
                let mut p = components::ChannelDetailsPanel::new(ss, cx);
                p.set_tab(tab, cx);
                p
            })
        });
        let dialog = s.profile.as_ref().filter(|p| p.dialog).map(|p| {
            let custom = p.dialog_custom;
            let preset = p.dialog_preset;
            cx.new(|cx| {
                let mut d = components::StatusDialog::new(None, cx);
                if custom {
                    d.choose(components::ExpiryChoice::Custom, cx);
                }
                if let Some(ix) = preset {
                    d.apply_preset(ix, cx);
                }
                d
            })
        });
        let admin = s.admin.clone().map(|a| admin_dialog(a, cx));
        let aggregate = s.aggregate.as_ref().map(|a| {
            let src = cx.new(|_| crate::source::AggregateFixture { data: a.clone() });
            let kind = a.kind;
            let view = cx.new(|cx| components::AggregateView::new(kind, src, cx));
            if a.confirm {
                view.update(cx, |v, cx| v.request_mark_all_read(cx));
            }
            view
        });
        let thread = s.thread.as_ref().map(|root| {
            let id = MessageId {
                conversation: open.clone(),
                ts: root.clone(),
            };
            let ss = session.clone();
            cx.new(|cx| ThreadPanel::new(ss, id, Clock::H24, cx))
        });
        if opts.popout
            && let Some(panel) = thread.clone()
        {
            panel.update(cx, |p, _| p.set_show_header(false));
            return Self {
                rail_scroll: Default::default(),
                body: Body::Popout(panel),
                banner: opts.banner,
                switcher: None,
                command_palette: None,
                menu: None,
            };
        }
        let search = s.search.as_ref().map(|st| {
            let ss = session.clone();
            let panel = cx.new(|cx| SearchPanel::new(ss, cx));
            panel.update(cx, |p, cx| p.set_query(st.query.clone(), cx));
            panel
        });
        if opts.picker {
            let last = session
                .read(cx)
                .state()
                .windows
                .get(&open)
                .and_then(|w| w.messages().last().map(|m| m.id.clone()))
                .expect("picker scenario has a message");
            timeline.update(cx, |t, cx| {
                t.open_picker(last, window, cx);
                // A short result list, all of it drawable by the bundled emoji font.
                if let Some(p) = t.picker() {
                    p.update(cx, |p, cx| p.set_query("kissing".into(), cx));
                }
            });
        }
        if s.chrome.is_some() || s.people.is_some() || s.custom_emoji_capability {
            let dir = std::env::temp_dir().join(format!("parla-ui-testing-{}", std::process::id()));
            cx.set_global(ImageCache::new(Arc::new(IconFetcher), dir, 8 << 20));
        }
        let switcher = opts.switcher.map(|(items, query)| {
            cx.new(|cx| {
                let mut p = components::QuickSwitcher::new(
                    components::SwitchDelegate::new(items, Vec::new()),
                    cx,
                );
                p.set_query(query, cx);
                p
            })
        });
        let command_palette = opts.command_palette.map(|(items, query)| {
            cx.new(|cx| {
                let mut p =
                    components::CommandPalette::new(components::CommandDelegate::new(items), cx);
                p.set_query(query, cx);
                p
            })
        });
        let menu = opts.menu.map(|(kind, at)| {
            let ss = session.clone();
            let extra = vec![components::MenuExtra {
                danger: false,
                divider_before: false,
                icon: None,
                after: false,
                shortcut: None,
                label: "Remove workspace".into(),
                action: Rc::new(|_, _| {}),
            }];
            // The remove entry belongs to the rail's workspace menu only.
            let extra = match &kind {
                components::MenuKind::Workspace => extra,
                // The sidebar row menu, as the app builds it (English labels:
                // the app's own strings live in its catalog).
                components::MenuKind::Row(c) => {
                    let starred = ss.read(cx).state().starred.contains(c);
                    let noop = || -> components::ExtraActionFn { Rc::new(|_, _| {}) };
                    vec![
                        components::MenuExtra::new("View details", noop())
                            .icon(components::IconName::Info),
                        components::MenuExtra::new(if starred { "Unstar" } else { "Star" }, noop())
                            .icon(components::IconName::Star),
                        components::MenuExtra::new("Mark as read", noop())
                            .icon(components::IconName::Check)
                            .after_notifications(),
                        components::MenuExtra::new("Copy link", noop())
                            .icon(components::IconName::Copy)
                            .after_notifications(),
                        components::MenuExtra::new("Open in new window", noop())
                            .icon(components::IconName::External)
                            .shortcut(Some("cmd-shift-o".into()))
                            .after_notifications(),
                    ]
                }
                // The header bell's menu: the app adds the settings link.
                components::MenuKind::Conversation(_) => vec![
                    components::MenuExtra::new("All notification settings…", Rc::new(|_, _| {}))
                        .icon(components::IconName::Sliders),
                ],
                _ => Vec::new(),
            };
            (
                cx.new(|cx| components::NotifyMenu::new(ss, kind, extra, window, cx)),
                at,
            )
        });
        Self {
            rail_scroll: Default::default(),
            switcher,
            command_palette,
            menu,
            body: Body::Workspace {
                sidebar,
                timeline,
                composer,
                thread,
                scheduled,
                reminders,
                drafts,
                people,
                files,
                collection,
                pins,
                bookmarks,
                huddle,
                call_bar,
                profile,
                details,
                dialog,
                admin,
                search,
                aggregate,
                chrome: s.chrome.clone().map(Box::new),
                session: session.clone(),
                open,
            },
            banner: opts.banner,
        }
    }
}

impl Render for Stage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let root = div()
            .flex()
            .size_full()
            .bg(colors.background)
            .text_color(colors.text);
        match &self.body {
            Body::SignIn(screen) => root.child(div().size_full().child(screen.clone())),
            Body::WorkspacePicker(picker) => root.child(div().size_full().child(picker.clone())),
            Body::Popout(panel) => {
                let header = self.banner.as_ref().map(|b| b(window, cx));
                root.flex_col()
                    .children(header)
                    .child(div().flex_1().min_h_0().child(panel.clone()))
            }
            Body::Workspace {
                sidebar,
                timeline,
                composer,
                thread,
                scheduled,
                reminders,
                drafts,
                people,
                files,
                collection,
                pins,
                bookmarks,
                huddle,
                call_bar,
                profile,
                details,
                dialog,
                admin,
                search,
                aggregate,
                chrome,
                session,
                open,
            } => {
                let tokens = parla_tokens(&theme);
                let banner = self.banner.as_ref().map(|b| b(window, cx));
                let rail = chrome.as_ref().map(|c| {
                    let items = c.rail.clone().unwrap_or_else(|| {
                        std::sync::Arc::from([components::rail::RailItem::Workspace(
                            components::rail::RailWorkspace {
                                key: settings::WorkspaceKey::new("fake", "w1"),
                                name: "Example Workspace".into(),
                                icon: c.icon.clone(),
                                initials: "EW".into(),
                                mentions: 0,
                                unread: false,
                                active: true,
                                expired: false,
                                lean: false,
                                paused: false,
                                accent: None,
                            },
                        )])
                    });
                    let actions = components::rail::RailActions {
                        scroll: self.rail_scroll.clone(),
                        activate: std::rc::Rc::new(|_, _, _| {}),
                        context: std::rc::Rc::new(|_, _, _, _| {}),
                        add: std::rc::Rc::new(|_, _| {}),
                        aggregate: std::rc::Rc::new(|_, _, _| {}),
                        aggregate_open: None,
                        settings: c
                            .me
                            .then(|| -> components::rail::AddFn { std::rc::Rc::new(|_, _| {}) }),
                        me: c
                            .me
                            .then(|| {
                                let st = session.read(cx).state();
                                let me = st.self_user.as_ref()?;
                                let name = st
                                    .members
                                    .get(me)
                                    .and_then(components::sidebar::best_name)?;
                                Some(components::rail::RailSelf::new(
                                    name.to_string().into(),
                                    st.presence.get(me).copied(),
                                    std::rc::Rc::new(|_, _| {}),
                                ))
                            })
                            .flatten(),
                    };
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .w(themes::metrics::LAYOUT_RAIL)
                        .h_full()
                        .pt(themes::metrics::SPACE_5)
                        .pb(themes::metrics::SPACE_6)
                        .gap(themes::metrics::SPACE_5)
                        .bg(tokens.sidebar_rail)
                        .border_r_1()
                        .border_color(tokens.sidebar_border)
                        .children(components::rail::render_rail(&items, &actions, cx))
                });
                if self.rail_scroll.pending() {
                    let me = cx.entity();
                    window.on_next_frame(move |_, cx| me.update(cx, |_, cx| cx.notify()));
                }
                let show_details = details.is_some();
                let header = chrome.as_ref().and_then(|_| {
                    let s = session.read(cx);
                    let c = s.state().conversations.get(open)?;
                    let direct = c.kind == domain::ConversationKind::Direct;
                    Some(components::conversation_header(
                        s,
                        s.state(),
                        c,
                        &theme,
                        &HeaderActions {
                            pins: (components::CollectionsSource::pins(s, open).is_some())
                                .then(|| -> components::PinsFn { std::rc::Rc::new(|_, _, _| {}) }),
                            notify_menu: s.state().local_notify_prefs.contains_key(open).then(
                                || -> components::NotifyMenuFn {
                                    std::rc::Rc::new(|_, _, _, _| {})
                                },
                            ),
                            details: show_details.then(noop_toggle),
                            details_open: show_details,
                            huddle: (show_details || direct).then(noop_toggle),
                            user: direct.then(|| -> components::rich_text::OpenUser {
                                std::rc::Rc::new(|_, _, _| {})
                            }),
                            more_menu: show_details.then(|| -> components::NotifyMenuFn {
                                std::rc::Rc::new(|_, _, _, _| {})
                            }),
                            tab: if files.is_some() {
                                components::ConversationTab::Files
                            } else {
                                components::ConversationTab::Messages
                            },
                            switch_tab: files
                                .is_some()
                                .then(|| -> components::TabFn { std::rc::Rc::new(|_, _, _| {}) }),
                            ..Default::default()
                        },
                        cx,
                    ))
                });
                // A host that shows the first-run tour measures the parts it points at.
                let targets = cx.try_global::<components::tour::TourTargets>().cloned();
                let track = |part, el: AnyElement| match &targets {
                    Some(t) => t.track(part, el).into_any_element(),
                    None => el,
                };
                let mut root = root
                    .children(
                        rail.map(|r| track(components::tour::TourPart::Rail, r.into_any_element())),
                    )
                    .child(track(
                        components::tour::TourPart::Sidebar,
                        div()
                            .w(themes::metrics::LAYOUT_SIDEBAR)
                            .h_full()
                            .bg(tokens.sidebar_background)
                            .text_color(tokens.sidebar_text)
                            .border_r_1()
                            .border_color(tokens.sidebar_border)
                            .child(div().size_full().child(sidebar.clone()))
                            .into_any_element(),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .bg(colors.editor_background)
                            .children(banner)
                            .children(call_bar.clone())
                            .relative()
                            .children(match (aggregate, collection) {
                                (Some(v), _) => {
                                    vec![div().flex_1().child(v.clone()).into_any_element()]
                                }
                                (None, Some(c)) => vec![
                                    div()
                                        .flex_1()
                                        .min_h_0()
                                        .child(match c {
                                            StageCollection::Later(v) => {
                                                v.clone().into_any_element()
                                            }
                                            StageCollection::Threads(v) => {
                                                v.clone().into_any_element()
                                            }
                                        })
                                        .into_any_element(),
                                ],
                                (None, None) => header
                                    .into_iter()
                                    .chain(huddle.clone().map(|b| b.into_any_element()))
                                    .chain(bookmarks.clone().map(|b| b.into_any_element()))
                                    .chain(match files {
                                        Some(f) => vec![
                                            div()
                                                .flex_1()
                                                .min_h_0()
                                                .child(f.clone())
                                                .into_any_element(),
                                        ],
                                        None => vec![
                                            div()
                                                .flex_1()
                                                .child(timeline.clone())
                                                .into_any_element(),
                                            composer.clone().into_any_element(),
                                        ],
                                    })
                                    .chain(pins.clone().map(|p| {
                                        div()
                                            .absolute()
                                            .top(
                                                components::header::TOTAL_HEIGHT
                                                    + themes::metrics::SPACE_2,
                                            )
                                            .right(px(12.0))
                                            .child(p)
                                            .into_any_element()
                                    }))
                                    .collect(),
                            })
                            // Last, so it paints over the composer.
                            .child(components::Toasts::overlay(window, cx)),
                    );
                if let Some(v) = scheduled {
                    root = root.child(
                        div()
                            .w(themes::metrics::LAYOUT_DETAIL_PANEL)
                            .h_full()
                            .bg(colors.panel_background)
                            .border_l_1()
                            .border_color(colors.border)
                            .child(div().size_full().child(v.clone())),
                    );
                }
                if let Some(v) = drafts {
                    root = root.child(
                        div()
                            .w(themes::metrics::LAYOUT_DETAIL_PANEL)
                            .h_full()
                            .bg(colors.panel_background)
                            .border_l_1()
                            .border_color(colors.border)
                            .child(div().size_full().child(v.clone())),
                    );
                }
                if let Some(v) = people {
                    root = root.child(
                        div()
                            .w(themes::metrics::LAYOUT_PEOPLE_PANEL)
                            .h_full()
                            .bg(colors.panel_background)
                            .border_l_1()
                            .border_color(colors.border)
                            .child(div().size_full().child(v.clone())),
                    );
                }
                if let Some(v) = reminders {
                    root = root.child(
                        div()
                            .w(themes::metrics::LAYOUT_DETAIL_PANEL)
                            .h_full()
                            .bg(colors.panel_background)
                            .border_l_1()
                            .border_color(colors.border)
                            .child(div().size_full().child(v.clone())),
                    );
                }
                if let Some(v) = profile {
                    root = root.child(
                        div()
                            .w(themes::metrics::LAYOUT_DETAIL_PANEL)
                            .h_full()
                            .bg(colors.panel_background)
                            .border_l_1()
                            .border_color(colors.border)
                            .child(div().size_full().child(v.clone())),
                    );
                }
                if let Some(v) = details {
                    root = root.child(
                        div()
                            .w(themes::metrics::LAYOUT_DETAIL_PANEL)
                            .h_full()
                            .bg(colors.panel_background)
                            .border_l_1()
                            .border_color(colors.border)
                            .child(div().size_full().child(v.clone())),
                    );
                }
                if let Some(d) = dialog {
                    root = root.relative().child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .justify_center()
                            .items_start()
                            .pt(px(96.0))
                            .child(div().w(px(420.0)).child(d.clone())),
                    );
                }
                if let Some((v, w)) = admin {
                    root = root.relative().child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .justify_center()
                            .items_start()
                            .pt(themes::metrics::LAYOUT_DIALOG_TOP)
                            .bg(themes::parla_tokens(cx.theme()).bg_overlay)
                            .child(div().w(*w).child(v.clone())),
                    );
                }
                if let Some(t) = search {
                    // The panel sizes itself: the hits, and the hit in
                    // context beside them when the window has room (the
                    // search stage has no rail).
                    let avail = window.viewport_size().width - themes::metrics::LAYOUT_SIDEBAR;
                    t.update(cx, |p, _| {
                        p.set_context_width(components::search_context_width(avail))
                    });
                    root = root.child(
                        div()
                            .flex_none()
                            .h_full()
                            .bg(colors.panel_background)
                            .border_l_1()
                            .border_color(colors.border)
                            .child(div().size_full().child(t.clone())),
                    );
                }
                if let Some(t) = thread {
                    root = root.child(
                        div()
                            .w(themes::metrics::LAYOUT_DETAIL_PANEL)
                            .h_full()
                            .bg(colors.panel_background)
                            .border_l_1()
                            .border_color(colors.border)
                            .child(div().size_full().child(t.clone())),
                    );
                }
                if let Some(p) = &self.switcher {
                    root = root.relative().child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .justify_center()
                            .items_start()
                            .px(themes::metrics::SPACE_6)
                            .pt(themes::metrics::LAYOUT_SWITCHER_TOP)
                            .child(
                                div()
                                    .w_full()
                                    .max_w(themes::metrics::LAYOUT_SWITCHER_WIDTH)
                                    .child(p.clone()),
                            ),
                    );
                }
                if let Some(p) = &self.command_palette {
                    root = root.relative().child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .justify_center()
                            .items_start()
                            .px(themes::metrics::SPACE_6)
                            .pt(themes::metrics::LAYOUT_SWITCHER_TOP)
                            .child(
                                div()
                                    .w_full()
                                    .max_w(themes::metrics::LAYOUT_SWITCHER_WIDTH)
                                    .child(p.clone()),
                            ),
                    );
                }
                if let Some((m, (x, y))) = &self.menu {
                    root = root
                        .relative()
                        .child(div().absolute().left(px(*x)).top(px(*y)).child(m.clone()));
                }
                root
            }
        }
    }
}

/// Renders `s` in `h` and captures the window.
pub fn capture(h: &mut UiHarness, s: Scenario, opts: StageOptions) -> Result<RgbaImage> {
    Ok(capture_frames(h, s, opts, 1)?.remove(0))
}

/// Captures `frames` successive frames of the stage; the stage is notified
/// between them, so a view that retains its last frame has to prove it still
/// matches (the retention oracle compares these with retention off).
pub fn capture_frames(
    h: &mut UiHarness,
    s: Scenario,
    opts: StageOptions,
    frames: usize,
) -> Result<Vec<RgbaImage>> {
    // The details scenarios keep the sidebar beside the 400 px panel, so they
    // use a wider stage than the 960 px default.
    let width = if s.details.is_some() {
        DETAILS_STAGE_WIDTH
    } else if s.search.is_some() {
        SEARCH_STAGE_WIDTH
    } else {
        crate::harness::LOGICAL_W
    };
    // The reminder editor is taller than the default stage: grow it so its
    // Delete, Cancel and Save show.
    let height = if s.reminders.as_ref().is_some_and(|r| r.edit.is_some()) {
        EDIT_STAGE_HEIGHT
    } else if s.search.is_some() {
        // Wide but short (the window's minimum height): the baselines stay small.
        SEARCH_STAGE_HEIGHT
    } else {
        crate::harness::LOGICAL_H
    };
    let scrolls_rail = s
        .chrome
        .as_ref()
        .and_then(|c| c.rail.as_ref())
        .is_some_and(|r| r.len() > 3);
    let w = h.open_sized(width, height, move |window, app| {
        app.new(|cx| Stage::new(&s, opts, window, cx))
    });
    // A first frame measures the rail's list; the next scrolls the selected
    // workspace into view.
    if scrolls_rail {
        h.settle();
    }
    let mut out = vec![h.capture(w.into())?];
    for _ in 1..frames {
        h.app().update(|app| {
            let _ = w.update(app, |_, _, cx| cx.notify());
        });
        out.push(h.capture(w.into())?);
    }
    Ok(out)
}

/// Logical height of the stage for the reminder editor scenario.
pub const EDIT_STAGE_HEIGHT: f32 = 800.0;

/// Logical width of the stage for the channel details scenarios.
pub const DETAILS_STAGE_WIDTH: f32 = 1280.0;

/// The search scenarios use a wide stage, so the hits and the hit in
/// context both fit beside the timeline.
pub const SEARCH_STAGE_WIDTH: f32 = 1280.0;
pub const SEARCH_STAGE_HEIGHT: f32 = 480.0;

/// A header action that does nothing: scenarios only draw the buttons.
fn noop_toggle() -> components::ToggleFn {
    std::rc::Rc::new(|_, _, _| {})
}

/// Builds the channel admin dialog a scenario asks for, in its state.
fn admin_dialog(a: crate::scenario::AdminStage, cx: &mut App) -> (gpui::AnyView, gpui::Pixels) {
    use crate::scenario::AdminStage;
    use themes::metrics::{LAYOUT_DIALOG_MD, LAYOUT_DIALOG_SM};
    let check: components::NameCheck = std::rc::Rc::new(domain::test_support::fake_channel_name);
    let browser = |cx: &mut App,
                   f: &dyn Fn(
        &mut components::ChannelBrowser,
        &mut gpui::Context<components::ChannelBrowser>,
    )| {
        let d = cx.new(|cx| {
            let mut d = components::ChannelBrowser::new(cx);
            f(&mut d, cx);
            d
        });
        (gpui::AnyView::from(d), LAYOUT_DIALOG_MD)
    };
    match a {
        AdminStage::Create {
            name,
            private,
            error,
        } => {
            let d = cx.new(|cx| {
                let mut d = components::CreateChannelDialog::new(check, cx);
                d.set_name(name, cx);
                d.set_private(private, cx);
                d.set_error(error.map(str::to_string), cx);
                d
            });
            (d.into(), LAYOUT_DIALOG_SM)
        }
        AdminStage::Browse(rows) => {
            browser(cx, &move |d, cx| d.set_page(rows.clone(), true, true, cx))
        }
        AdminStage::BrowseLoading => browser(cx, &|_, _| {}),
        AdminStage::BrowseEmpty => browser(cx, &|d, cx| d.set_page(Vec::new(), true, false, cx)),
        AdminStage::BrowseError(why) => {
            browser(cx, &move |d, cx| d.set_error(Some(why.to_string()), cx))
        }
        AdminStage::Edit {
            name,
            topic,
            purpose,
        } => {
            let d = cx.new(|cx| {
                components::EditChannelDialog::new(name, topic, purpose, true, check, cx)
            });
            (d.into(), LAYOUT_DIALOG_SM)
        }
        AdminStage::Invite {
            title,
            people,
            picked,
        } => {
            let ids: Vec<_> = picked.iter().map(|&i| people[i].id.clone()).collect();
            let d = cx.new(|cx| {
                let mut d = components::InviteDialog::new(title.to_string(), people, cx);
                for id in &ids {
                    d.toggle(id, cx);
                }
                d
            });
            (d.into(), LAYOUT_DIALOG_SM)
        }
        AdminStage::Confirm {
            title,
            body,
            ok,
            danger,
        } => {
            let d = cx.new(|cx| {
                components::ConfirmDialog::new(
                    title.to_string(),
                    body.to_string(),
                    ok.to_string(),
                    cx,
                )
                .danger(danger)
            });
            (d.into(), LAYOUT_DIALOG_SM)
        }
    }
}
