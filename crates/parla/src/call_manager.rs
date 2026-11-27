//! `CallManager`: the one place that joins a huddle. It builds the fallback
//! ladder (`calls.join_with`, the platform, the provider's capabilities and
//! URLs), starts native audio, opens the call window through the
//! `CallController` or hands the URL to the OS, and takes the next rung once
//! when native audio (before media is up) or the window cannot join.
//!
//! Native audio runs in this process (`calls::native::NativeCall`) behind the
//! [`NativeEngine`] seam, so tests need no devices or network. Its events map
//! onto the call bar (connecting, live, muted, who is in it and speaking, a
//! screen share Parla cannot show); a failure after media came up is an
//! error, never a fallback.
//!
//! Devices: a native call opens `calls.input_device` and `calls.output_device`
//! (the system default when unset) and reopens a side when its stream fails,
//! when the default it follows changes (a CoreAudio listener, only while a
//! call runs), when a device is plugged in or out while a named one is
//! chosen, and when the setting changes (the call bar's picker or the file).
//!
//! One call at a time across workspaces. Event driven: it owns no timer (the
//! controller's 30 s load timer and 2 s leave grace are the only ones, and
//! exist only around a call).

use std::sync::Arc;

use calls::controller::{CallController, CallControllerEvent, HelperSpawner};
use calls::native::{
    CallEvent, CallEvents, CpalSink, CpalSource, DeviceChange, DeviceWatcher, Devices, NativeCall,
    OnDeviceChange, Participant,
};
use calls::{CallKey, CallPhase, CaptureState, FailReason, JoinRoute, ladder};
use components::{CallBarModel, CallBarPhase, CallPerson, DeviceMenu, MediaState};
use domain::{ChatProvider, ConversationId, NativeCallJoin, UserId, WorkspaceState};
use futures::StreamExt as _;
use gpui::SharedString;
use gpui::{App, AppContext as _, Context, Entity, Global, PromptLevel, Subscription, Task};
use i18n::LocalizedText;
use settings::{CallSettings, SettingsStore, WorkspaceKey};

use crate::huddles::HuddleOpen;
use crate::notifications::Notifications;

/// Reopens of one device after stream errors, between device changes; a
/// device that keeps failing is then left closed and a notice shown.
pub const MAX_REOPENS: usize = 3;

/// The audio devices a native call uses, by name; `None` is the system
/// default (`calls.input_device`, `calls.output_device`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallDevices {
    pub input: Option<String>,
    pub output: Option<String>,
}

impl CallDevices {
    pub fn from_settings(c: &CallSettings) -> Self {
        Self {
            input: c.input_device.clone(),
            output: c.output_device.clone(),
        }
    }
}

fn bar_phase(p: CallPhase) -> CallBarPhase {
    match p {
        CallPhase::Live => CallBarPhase::Live,
        CallPhase::Leaving => CallBarPhase::Leaving,
        CallPhase::Idle | CallPhase::Fetching | CallPhase::Starting | CallPhase::Loading => {
            CallBarPhase::Loading
        }
    }
}

fn media(s: CaptureState) -> MediaState {
    match s {
        CaptureState::Unsupported => MediaState::Unsupported,
        CaptureState::Inactive => MediaState::Inactive,
        CaptureState::Active => MediaState::Active,
        CaptureState::Muted => MediaState::Muted,
    }
}

/// Lists the connected microphones and speakers, by name, for the Settings
/// page's selects. The app uses cpal; tests install a fake as a global.
pub struct DeviceLister(pub Arc<dyn Fn() -> (Vec<String>, Vec<String>) + Send + Sync>);

impl Global for DeviceLister {}

/// The device lister: the installed one, else the system's (cpal).
pub fn device_lister(cx: &App) -> Arc<dyn Fn() -> (Vec<String>, Vec<String>) + Send + Sync> {
    match cx.try_global::<DeviceLister>() {
        Some(l) => l.0.clone(),
        None => Arc::new(|| {
            (
                calls::native::input_device_names(),
                calls::native::output_device_names(),
            )
        }),
    }
}

/// A running native call as the manager drives it (mute, devices, leave).
pub trait NativeHandle: Send {
    fn mute(&self, muted: bool);
    /// Reopens the microphone (`input`), the speaker (`output`) or both on
    /// `devices`. Returns at once.
    fn reopen(&self, devices: &CallDevices, input: bool, output: bool);
    /// Leaves and stops the media thread; may block for about two seconds,
    /// so the manager calls it off the main thread.
    fn leave(self: Box<Self>);
}

/// Starts native calls. The app uses [`CpalEngine`]; tests use a fake.
pub trait NativeEngine: Send + Sync {
    /// Starts joining on `devices` and returns at once; progress arrives as
    /// events.
    fn join(
        &self,
        join: NativeCallJoin,
        devices: &CallDevices,
    ) -> (Box<dyn NativeHandle>, CallEvents);
    /// Watches the system's audio devices while a call runs; dropping the
    /// result stops it. `None` where nothing can be watched (tests).
    fn watch_devices(&self, _on_change: OnDeviceChange) -> Option<Box<dyn Send>> {
        None
    }
    /// The connected microphones and speakers, by name, for the picker. May
    /// take a moment: the manager calls it off the main thread.
    fn device_names(&self) -> (Vec<String>, Vec<String>) {
        (Vec::new(), Vec::new())
    }
}

/// The real engine: `NativeCall` over cpal devices.
pub struct CpalEngine;

impl NativeHandle for NativeCall {
    fn mute(&self, muted: bool) {
        NativeCall::mute(self, muted);
    }
    fn reopen(&self, devices: &CallDevices, input: bool, output: bool) {
        self.replace_devices(
            input.then(|| CpalSource::factory_for(devices.input.clone())),
            output.then(|| CpalSink::factory_for(devices.output.clone())),
        );
    }
    fn leave(self: Box<Self>) {
        NativeCall::leave(*self);
    }
}

impl NativeEngine for CpalEngine {
    fn join(
        &self,
        join: NativeCallJoin,
        devices: &CallDevices,
    ) -> (Box<dyn NativeHandle>, CallEvents) {
        let (call, events) = NativeCall::join(
            join,
            Devices {
                source: CpalSource::factory_for(devices.input.clone()),
                sink: CpalSink::factory_for(devices.output.clone()),
            },
        );
        (Box::new(call), events)
    }
    fn watch_devices(&self, on_change: OnDeviceChange) -> Option<Box<dyn Send>> {
        Some(Box::new(DeviceWatcher::start(on_change)))
    }
    fn device_names(&self) -> (Vec<String>, Vec<String>) {
        (
            calls::native::input_device_names(),
            calls::native::output_device_names(),
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NativePhase {
    Connecting,
    Live,
}

/// The native call being set up or running.
struct Native {
    key: CallKey,
    provider: Arc<dyn ChatProvider>,
    calls: CallSettings,
    /// The routes left if native cannot join before media is up.
    rest: Vec<JoinRoute>,
    phase: NativePhase,
    muted: bool,
    /// The attendees present (not dropped), as the call lists them.
    people: Vec<Participant>,
    /// Attendee ids speaking now.
    speaking: Vec<String>,
    /// Someone shares a screen (Parla cannot show it).
    sharing: bool,
    /// The devices the call uses now.
    devices: CallDevices,
    /// The microphone and speaker: reopen budget, notice, presence.
    input: Side,
    output: Side,
    /// The picker is open, with the devices listed when it opened.
    menu_open: bool,
    device_names: (Vec<String>, Vec<String>),
    /// Watches the audio devices while the call runs.
    _watch: Option<Box<dyn Send>>,
    handle: Option<Box<dyn NativeHandle>>,
    /// Distinguishes this call from one started after it.
    id: u64,
    /// Fetches the join, then reads the call's events.
    _task: Task<()>,
}

/// One audio side (microphone or speaker) of a native call.
#[derive(Default)]
struct Side {
    /// Reopens after stream errors since the device last really changed.
    reopens: usize,
    /// The "unavailable" notice was shown for the spent budget.
    noticed: bool,
    /// Whether the chosen named device was connected at the last listing;
    /// `None` before the first listing or when following the default.
    present: Option<bool>,
}

impl Side {
    /// The device really changed: a fresh budget and notice.
    fn reset(&mut self) {
        self.reopens = 0;
        self.noticed = false;
    }

    /// Takes one reopen from the budget; `Err(true)` when spent and the
    /// notice is due, `Err(false)` when spent and already said.
    fn take(&mut self) -> Result<(), bool> {
        if self.reopens < MAX_REOPENS {
            self.reopens += 1;
            Ok(())
        } else {
            Err(!std::mem::replace(&mut self.noticed, true))
        }
    }
}

/// The call being set up or running, with the routes left if the window fails.
struct Ctx {
    key: CallKey,
    provider: Arc<dyn ChatProvider>,
    calls: CallSettings,
    rest: Vec<JoinRoute>,
    /// This user showed up in the huddle's participants (so leaving it from
    /// another device ends the call, but a stale list before joining does not).
    seen_self: bool,
}

pub struct CallManager {
    controller: Option<Entity<CallController>>,
    window_ok: bool,
    current: Option<Ctx>,
    engine: Option<Arc<dyn NativeEngine>>,
    rt: Option<tokio::runtime::Handle>,
    native: Option<Native>,
    next_id: u64,
    /// A join asked for while in a call, waiting for the answer.
    incoming: Option<Ctx>,
    /// The answer was "Leave and join": starts when the current call ends.
    then: Option<Ctx>,
    _sub: Subscription,
    _settings: Option<Subscription>,
}

/// The manager entity in a GPUI global, so sessions, the ringer and actions reach it.
pub struct CallManagerHandle(pub Entity<CallManager>);

impl Global for CallManagerHandle {}

impl CallManager {
    /// Creates the manager over `spawner`, installs it as a global and ends the
    /// call when the app quits.
    pub fn install(spawner: Box<dyn HelperSpawner>, cx: &mut App) -> Entity<Self> {
        let rt = crate::runtime::handle(cx);
        let controller = cx.new(|cx| {
            let c = CallController::new(spawner, cx).with_title(window_title);
            match rt {
                Some(h) => c.with_runtime(h),
                None => c,
            }
        });
        let mgr = cx.new(|cx| {
            let sub = cx.subscribe(
                &controller,
                |this: &mut Self, _, e: &CallControllerEvent, cx| this.on_event(e.clone(), cx),
            );
            // A changed device setting applies to the running call at once.
            let settings = cx.try_global::<SettingsStore>().is_some().then(|| {
                cx.observe_global::<SettingsStore>(|m: &mut Self, cx| m.settings_changed(cx))
            });
            Self {
                _settings: settings,
                controller: Some(controller),
                window_ok: calls::platform::window_supported(),
                current: None,
                engine: Some(Arc::new(CpalEngine)),
                rt: crate::runtime::handle(cx),
                native: None,
                next_id: 0,
                incoming: None,
                then: None,
                _sub: sub,
            }
        });
        cx.set_global(CallManagerHandle(mgr.clone()));
        let quit = mgr.clone();
        cx.on_app_quit(move |cx| {
            quit.update(cx, |m, _| m.shutdown());
            async {}
        })
        .detach();
        mgr
    }

    /// Overrides the platform check (tests).
    pub fn set_window_supported(&mut self, ok: bool) {
        self.window_ok = ok;
    }

    /// Replaces the native engine; `None` turns the native rung off (tests).
    pub fn set_native_engine(&mut self, engine: Option<Arc<dyn NativeEngine>>) {
        self.engine = engine;
    }

    /// App quit: drops the controller, which closes the helper's stdin and
    /// kills it, so no helper outlives Parla.
    pub fn shutdown(&mut self) {
        self.current = None;
        self.incoming = None;
        self.then = None;
        self.controller = None;
        // Leaving blocks for up to two seconds: never on the main thread.
        if let Some(h) = self.native.take().and_then(|n| n.handle) {
            std::thread::spawn(move || h.leave());
        }
    }

    /// A call is being set up or running.
    pub fn in_call(&self, cx: &App) -> bool {
        self.native.is_some()
            || self
                .controller
                .as_ref()
                .is_some_and(|c| c.read(cx).state().phase != CallPhase::Idle)
    }

    /// Opens the huddle of `c` through the ladder. A window join starts the
    /// call (it may still fall back later); other rungs open at once.
    pub fn join(
        &mut self,
        ws: &WorkspaceKey,
        c: ConversationId,
        provider: Arc<dyn ChatProvider>,
        calls: &CallSettings,
        cx: &mut Context<Self>,
    ) -> HuddleOpen {
        let caps = provider.capabilities();
        let window_ok = self.window_ok && caps.call_surface;
        let key = CallKey {
            workspace: ws.clone(),
            conversation: c.clone(),
        };
        if let Some(n) = &self.native {
            if n.key == key {
                return HuddleOpen::Native;
            }
            // Joining another huddle leaves this one first.
            self.end_native(cx);
        }
        let mut native_ok = self.engine.is_some() && caps.native_calls;
        if self.in_call(cx) {
            // A window call is running: its own "Leave and join?" prompt
            // decides, so native stays out of it.
            native_ok = false;
        }
        let mut routes = ladder(
            calls.join_with,
            native_ok,
            window_ok,
            provider.huddle_url(&c),
            provider.app_url(&c),
        );
        if routes.is_empty() {
            return HuddleOpen::Toast;
        }
        match routes.remove(0) {
            JoinRoute::Native => {
                self.begin_native(key, provider, calls.clone(), routes, cx);
                HuddleOpen::Native
            }
            JoinRoute::Window => {
                self.begin(
                    Ctx {
                        key: CallKey {
                            workspace: ws.clone(),
                            conversation: c,
                        },
                        provider,
                        calls: calls.clone(),
                        rest: routes,
                        seen_self: false,
                    },
                    cx,
                );
                HuddleOpen::Window
            }
            JoinRoute::Browser(u) => {
                cx.open_url(&u);
                HuddleOpen::Browser(u)
            }
            JoinRoute::ProviderApp(u) => {
                cx.open_url(&u);
                HuddleOpen::App(u)
            }
        }
    }

    /// Starts a native join: fetch the join credentials, then run the call.
    fn begin_native(
        &mut self,
        key: CallKey,
        provider: Arc<dyn ChatProvider>,
        calls: CallSettings,
        rest: Vec<JoinRoute>,
        cx: &mut Context<Self>,
    ) {
        self.next_id += 1;
        let id = self.next_id;
        let (p, conv, rt) = (provider.clone(), key.conversation.clone(), self.rt.clone());
        let task = cx.spawn(async move |this, cx| {
            let fut = async move { p.native_call_join(&conv).await };
            let res = match rt {
                Some(rt) => rt
                    .spawn(fut)
                    .await
                    .map_err(|_| ())
                    .and_then(|r| r.map_err(|_| ())),
                None => fut.await.map_err(|_| ()),
            };
            let events = match this.update(cx, |m, cx| m.native_joined(id, res, cx)) {
                Ok(Some(e)) => e,
                _ => return,
            };
            let mut events = events;
            while let Some(e) = events.next().await {
                if this.update(cx, |m, cx| m.native_event(id, e, cx)).is_err() {
                    return;
                }
            }
        });
        let devices = CallDevices::from_settings(&calls);
        self.native = Some(Native {
            key,
            provider,
            calls,
            rest,
            phase: NativePhase::Connecting,
            muted: false,
            people: Vec::new(),
            speaking: Vec::new(),
            sharing: false,
            devices,
            input: Side::default(),
            output: Side::default(),
            menu_open: false,
            device_names: (Vec::new(), Vec::new()),
            _watch: None,
            handle: None,
            id,
            _task: task,
        });
        cx.notify();
    }

    /// The join credentials arrived (or not): start the engine, or fall back.
    fn native_joined(
        &mut self,
        id: u64,
        res: Result<NativeCallJoin, ()>,
        cx: &mut Context<Self>,
    ) -> Option<CallEvents> {
        if self.native.as_ref().is_none_or(|n| n.id != id) {
            return None;
        }
        let engine = self.engine.clone();
        let (Ok(join), Some(engine)) = (res, engine) else {
            self.native_fall_back("join-unavailable", cx);
            return None;
        };
        let devices = self.native.as_ref()?.devices.clone();
        let (handle, events) = engine.join(join, &devices);
        let watch = self.watch_devices(id, engine.as_ref(), cx);
        let n = self.native.as_mut()?;
        handle.mute(n.calls.mute_on_join);
        n.muted = n.calls.mute_on_join;
        n.handle = Some(handle);
        n._watch = watch;
        // The first listing is the baseline for a chosen device's presence.
        if n.devices.input.is_some() || n.devices.output.is_some() {
            self.list_devices(cx);
        }
        cx.notify();
        Some(events)
    }

    fn native_event(&mut self, id: u64, e: CallEvent, cx: &mut Context<Self>) {
        let Some(n) = self.native.as_mut().filter(|n| n.id == id) else {
            return;
        };
        let ws = n.key.workspace.clone();
        match e {
            CallEvent::Connecting => {}
            CallEvent::Live => {
                n.phase = NativePhase::Live;
            }
            CallEvent::Muted(m) => n.muted = m,
            CallEvent::Participants(list) => {
                n.people = list.into_iter().filter(|p| !p.dropped).collect();
                let present = &n.people;
                n.speaking
                    .retain(|a| present.iter().any(|p| &p.attendee_id == a));
            }
            CallEvent::Speaking(who) => n.speaking = who,
            CallEvent::ScreenShare(on) => n.sharing = on,
            CallEvent::CaptureUnavailable(_) => {
                // Non-blocking: the call goes on, receive-only.
                notice(&ws, LocalizedText::new("parla", "call-native-no-mic"), cx);
            }
            CallEvent::CaptureError(reason) => {
                // Reopen the microphone (unplugged, or the device changed
                // under the stream); one that keeps failing is left closed.
                crate::log::warn(&format!("native call capture error: {reason}"));
                match n.input.take() {
                    Ok(()) => {
                        if let Some(h) = &n.handle {
                            h.reopen(&n.devices, true, false);
                        }
                    }
                    Err(true) => notice(&ws, LocalizedText::new("parla", "call-native-no-mic"), cx),
                    Err(false) => {}
                }
            }
            CallEvent::PlaybackError(reason) => {
                crate::log::warn(&format!("native call playback error: {reason}"));
                match n.output.take() {
                    Ok(()) => {
                        if let Some(h) = &n.handle {
                            h.reopen(&n.devices, false, true);
                        }
                    }
                    Err(true) => notice(
                        &ws,
                        LocalizedText::new("parla", "call-native-no-speaker"),
                        cx,
                    ),
                    Err(false) => {}
                }
            }
            CallEvent::Failed {
                before_media,
                reason,
            } => {
                crate::log::warn(&format!("native call failed ({before_media}): {reason}"));
                if before_media {
                    self.native_fall_back("media-failed", cx);
                } else {
                    // Media was up: an error, no fallback.
                    self.drop_native(cx);
                    notice(&ws, LocalizedText::new("parla", "call-native-failed"), cx);
                }
                cx.notify();
                return;
            }
            CallEvent::Left => self.drop_native(cx),
        }
        cx.notify();
    }

    /// Native could not join before media: take the next rung, once.
    fn native_fall_back(&mut self, why: &str, cx: &mut Context<Self>) {
        let Some(n) = self.native.take() else {
            return;
        };
        crate::log::warn(&format!("native call ({why}); taking the next rung"));
        if let Some(h) = n.handle {
            Self::leave_off_thread(h, cx);
        }
        let Native {
            key,
            provider,
            calls,
            mut rest,
            ..
        } = n;
        if rest.is_empty() {
            notice(
                &key.workspace,
                LocalizedText::new("parla", "huddle-open-in"),
                cx,
            );
            return;
        }
        match rest.remove(0) {
            JoinRoute::Window => self.begin(
                Ctx {
                    key,
                    provider,
                    calls,
                    rest,
                    seen_self: false,
                },
                cx,
            ),
            JoinRoute::Browser(u) => {
                cx.open_url(&u);
                notice(
                    &key.workspace,
                    LocalizedText::new("parla", "call-native-fallback-browser"),
                    cx,
                );
            }
            JoinRoute::ProviderApp(u) => {
                cx.open_url(&u);
                notice(
                    &key.workspace,
                    LocalizedText::new("parla", "call-native-fallback-app"),
                    cx,
                );
            }
            JoinRoute::Native => {}
        }
    }

    /// Leaves the native call now (the call bar's Leave, or a switch).
    fn end_native(&mut self, cx: &mut Context<Self>) {
        let Some(n) = self.native.take() else {
            return;
        };
        if let Some(h) = n.handle {
            Self::leave_off_thread(h, cx);
        }
        let (p, conv) = (n.provider, n.key.conversation);
        let rt = self.rt.clone();
        cx.spawn(async move |_, _| {
            let fut = async move { p.leave_huddle(&conv).await };
            match rt {
                Some(rt) => {
                    rt.spawn(fut).await.ok();
                }
                None => {
                    fut.await.ok();
                }
            }
        })
        .detach();
        cx.notify();
    }

    /// The call is over on its own: drop it, stopping the engine off the
    /// main thread.
    fn drop_native(&mut self, cx: &mut Context<Self>) {
        if let Some(h) = self.native.take().and_then(|n| n.handle) {
            Self::leave_off_thread(h, cx);
        }
    }

    fn leave_off_thread(h: Box<dyn NativeHandle>, cx: &mut Context<Self>) {
        cx.background_spawn(async move { h.leave() }).detach();
    }

    fn begin(&mut self, ctx: Ctx, cx: &mut Context<Self>) {
        let Some(ctl) = self.controller.clone() else {
            return;
        };
        let (key, provider, calls) = (ctx.key.clone(), ctx.provider.clone(), ctx.calls.clone());
        let st = ctl.read(cx).state();
        let idle = st.phase == CallPhase::Idle;
        let same = st.key.as_ref() == Some(&key);
        if idle {
            self.current = Some(ctx);
        } else if !same {
            self.incoming = Some(ctx);
        }
        ctl.update(cx, |c, cx| {
            if idle {
                c.set_settings(calls);
            }
            c.open(key, provider, cx)
        });
    }

    /// The workspace's huddles changed: a call whose huddle ended, or that this
    /// user left from another device, closes its window.
    pub fn huddles_changed(
        &mut self,
        ws: &WorkspaceKey,
        state: &WorkspaceState,
        cx: &mut Context<Self>,
    ) {
        if let Some(n) = self.native.as_mut()
            && n.key.workspace == *ws
        {
            cx.notify();
            // The huddle ended while live.
            if n.phase == NativePhase::Live && !state.huddles.contains_key(&n.key.conversation) {
                self.end_native(cx);
            }
            return;
        }
        let (Some(ctl), Some(ctx)) = (self.controller.clone(), self.current.as_mut()) else {
            return;
        };
        if ctx.key.workspace != *ws {
            return;
        }
        let h = state.huddles.get(&ctx.key.conversation);
        let me_in = h
            .zip(state.self_user.as_ref())
            .is_some_and(|(h, me)| h.participants.contains(me));
        ctx.seen_self |= me_in;
        // The participant count of the call bar.
        cx.notify();
        let live = ctl.read(cx).state().phase == CallPhase::Live;
        if live && (h.is_none() || (ctx.seen_self && !me_in)) {
            let key = ctx.key.clone();
            ctl.update(cx, |c, cx| c.huddle_ended(key, cx));
        }
    }

    /// The model for the call bar; `None` when there is no call.
    pub fn bar_model(&self, cx: &App) -> Option<CallBarModel> {
        if let Some(n) = &self.native {
            return Some(CallBarModel {
                title: self.title(&n.key, cx).into(),
                phase: match n.phase {
                    NativePhase::Connecting => CallBarPhase::Loading,
                    NativePhase::Live => CallBarPhase::Live,
                },
                mic: if n.muted {
                    MediaState::Muted
                } else {
                    MediaState::Active
                },
                camera: MediaState::Unsupported,
                participants: n.people.len() as u32,
                load_slow: false,
                screen_share_unavailable: n.sharing,
                show_window: false,
                people: self.people(n, cx),
                devices: Some(DeviceMenu {
                    open: n.menu_open,
                    inputs: n.device_names.0.iter().map(|d| d.clone().into()).collect(),
                    outputs: n.device_names.1.iter().map(|d| d.clone().into()).collect(),
                    input: n.devices.input.clone().map(Into::into),
                    output: n.devices.output.clone().map(Into::into),
                }),
            });
        }
        let st = self.controller.as_ref()?.read(cx).state();
        if st.phase == CallPhase::Idle {
            return None;
        }
        let key = st.key.as_ref()?;
        let session = cx
            .try_global::<Notifications>()
            .and_then(|h| h.session(&key.workspace));
        let (title, participants) = match session {
            Some(s) => {
                let s = s.read(cx);
                let state = s.state();
                (
                    state
                        .conversations
                        .get(&key.conversation)
                        .map(|c| c.name.to_string())
                        .unwrap_or_default(),
                    state
                        .huddles
                        .get(&key.conversation)
                        .map_or(0, |h| h.participants.len() as u32),
                )
            }
            None => (String::new(), 0),
        };
        Some(CallBarModel {
            title: title.into(),
            phase: bar_phase(st.phase),
            mic: media(st.mic),
            camera: media(st.camera),
            participants,
            load_slow: st.load_slow,
            // The call window shows a share itself.
            screen_share_unavailable: false,
            show_window: true,
            people: Vec::new(),
            devices: None,
        })
    }

    /// The call's people for the bar: everyone but this user, named from the
    /// workspace's members (an attendee's external user id is the provider's
    /// user id, see `domain::NativeCallJoin`).
    fn people(&self, n: &Native, cx: &App) -> Vec<CallPerson> {
        let session = cx
            .try_global::<Notifications>()
            .and_then(|h| h.session(&n.key.workspace));
        let state = session.as_ref().map(|s| s.read(cx).state());
        let me = state.and_then(|s| s.self_user.as_ref());
        let l = i18n::gpui::localizer(cx);
        let someone = || t!(l, "call-person-unknown").into_owned();
        n.people
            .iter()
            .filter(|p| me.is_none_or(|me| *me.0 != *p.external_user_id))
            .map(|p| {
                let user = UserId(p.external_user_id.as_str().into());
                let name = state
                    .and_then(|s| s.members.get(&user))
                    .and_then(components::sidebar::best_name)
                    .map(|n| n.to_string())
                    .unwrap_or_else(someone);
                CallPerson {
                    name: SharedString::from(name),
                    speaking: n.speaking.contains(&p.attendee_id),
                    muted: p.muted,
                }
            })
            .collect()
    }

    /// Starts watching the audio devices for call `id`: CoreAudio calls back
    /// on its own thread, the change is folded into one pending value and
    /// one wake-up, and the manager applies it on the main thread.
    fn watch_devices(
        &self,
        id: u64,
        engine: &dyn NativeEngine,
        cx: &mut Context<Self>,
    ) -> Option<Box<dyn Send>> {
        let pending = Arc::new(std::sync::Mutex::new(DeviceChange::default()));
        // One wake-up in flight at most: the flag is set by the callback that
        // sends it and cleared by the receiver before it reads the changes.
        let queued = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<()>();
        let (p, q) = (pending.clone(), queued.clone());
        let watch = engine.watch_devices(Arc::new(move |c: DeviceChange| {
            if let Ok(mut acc) = p.lock() {
                acc.default_input |= c.default_input;
                acc.default_output |= c.default_output;
                acc.list |= c.list;
            }
            if !q.swap(true, std::sync::atomic::Ordering::AcqRel) {
                let _ = tx.unbounded_send(());
            }
        }))?;
        cx.spawn(async move |this, cx| {
            while rx.next().await.is_some() {
                queued.store(false, std::sync::atomic::Ordering::Release);
                let change = pending
                    .lock()
                    .map(|mut c| std::mem::take(&mut *c))
                    .unwrap_or_default();
                let alive = this.update(cx, |m, cx| {
                    if m.native.as_ref().is_some_and(|n| n.id == id) {
                        m.audio_devices_changed(change, cx);
                        true
                    } else {
                        false
                    }
                });
                if !matches!(alive, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
        Some(watch)
    }

    /// The system's audio devices changed. A side that follows the default
    /// reopens, with a fresh budget, when the default moved. A device plugged
    /// in or out lists the devices (off the main thread); a side with a chosen
    /// device reopens, with a fresh budget, only when that device came or
    /// went. Anything else keeps the budget, so a device that keeps failing
    /// is not retried forever because something unrelated was plugged in.
    pub fn audio_devices_changed(&mut self, change: DeviceChange, cx: &mut Context<Self>) {
        let Some(n) = self.native.as_mut() else {
            return;
        };
        let input = n.devices.input.is_none() && change.default_input;
        let output = n.devices.output.is_none() && change.default_output;
        if input {
            n.input.reset();
        }
        if output {
            n.output.reset();
        }
        if (input || output)
            && let Some(h) = &n.handle
        {
            h.reopen(&n.devices, input, output);
        }
        let named = n.devices.input.is_some() || n.devices.output.is_some();
        if change.list && (named || n.menu_open) {
            self.list_devices(cx);
        }
    }

    /// A fresh device listing: shows it in the picker and reopens each side
    /// whose chosen device came or went since the last listing.
    fn devices_listed(&mut self, names: (Vec<String>, Vec<String>), cx: &mut Context<Self>) {
        let Some(n) = self.native.as_mut() else {
            return;
        };
        let moved = |side: &mut Side, chosen: &Option<String>, list: &[String]| {
            let now = chosen.as_ref().map(|c| list.contains(c));
            let changed = matches!((side.present, now), (Some(a), Some(b)) if a != b);
            side.present = now;
            if changed {
                side.reset();
            }
            changed
        };
        let input = moved(&mut n.input, &n.devices.input, &names.0);
        let output = moved(&mut n.output, &n.devices.output, &names.1);
        if (input || output)
            && let Some(h) = &n.handle
        {
            h.reopen(&n.devices, input, output);
        }
        if n.device_names != names {
            n.device_names = names;
            cx.notify();
        }
    }

    /// The settings changed: a new device choice applies to the running call.
    fn settings_changed(&mut self, cx: &mut Context<Self>) {
        let Some(store) = cx.try_global::<SettingsStore>() else {
            return;
        };
        let devices = CallDevices::from_settings(&store.current().calls);
        self.use_devices(devices, cx);
    }

    /// Switches the running call to `devices`, reopening what changed.
    fn use_devices(&mut self, devices: CallDevices, cx: &mut Context<Self>) {
        let Some(n) = self.native.as_mut() else {
            return;
        };
        let input = n.devices.input != devices.input;
        let output = n.devices.output != devices.output;
        if !input && !output {
            return;
        }
        n.devices = devices;
        // A new choice: a fresh budget, and its presence is learned anew.
        if input {
            n.input = Side::default();
        }
        if output {
            n.output = Side::default();
        }
        if let Some(h) = &n.handle {
            h.reopen(&n.devices, input, output);
        }
        if n.devices.input.is_some() || n.devices.output.is_some() {
            self.list_devices(cx);
        }
        cx.notify();
    }

    /// Lists the connected devices off the main thread and shows them in the
    /// picker.
    fn list_devices(&mut self, cx: &mut Context<Self>) {
        let (Some(engine), Some(id)) = (self.engine.clone(), self.native.as_ref().map(|n| n.id))
        else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let names = cx
                .background_spawn(async move { engine.device_names() })
                .await;
            this.update(cx, |m, cx| {
                if m.native.as_ref().is_some_and(|n| n.id == id) {
                    m.devices_listed(names, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Opens or closes the call bar's device picker (`CallBarEvent::ToggleDevices`).
    pub fn toggle_devices(cx: &mut App) {
        Self::native_action(cx, |m, cx| {
            let open = m.native.as_ref().is_some_and(|n| !n.menu_open);
            if let Some(n) = m.native.as_mut() {
                n.menu_open = open;
            }
            if open {
                m.list_devices(cx);
            }
        });
    }

    /// Closes the device picker without a pick (`CallBarEvent::CloseDevices`).
    pub fn close_devices(cx: &mut App) {
        Self::native_action(cx, |m, _| {
            if let Some(n) = m.native.as_mut() {
                n.menu_open = false;
            }
        });
    }

    /// The picker chose a microphone (`None`: the system default).
    pub fn pick_input(device: Option<SharedString>, cx: &mut App) {
        Self::pick(cx, "input_device", device, |d, v| d.input = v);
    }

    /// The picker chose a speaker (`None`: the system default).
    pub fn pick_output(device: Option<SharedString>, cx: &mut App) {
        Self::pick(cx, "output_device", device, |d, v| d.output = v);
    }

    /// Saves the choice as `calls.<key>`, closes the picker and applies it
    /// at once (the store's reload then finds nothing new to do).
    fn pick(
        cx: &mut App,
        key: &str,
        device: Option<SharedString>,
        set: impl FnOnce(&mut CallDevices, Option<String>),
    ) {
        let value = device.map(|d| d.to_string());
        let json = serde_json::to_string(&value).unwrap_or_else(|_| "null".into());
        let path = crate::app_state::settings_path(cx);
        if let Err(e) = settings::edit_file(&path, &["calls", key], &json) {
            crate::log::error(&format!("settings: cannot write calls.{key}: {e}"));
        }
        Self::native_action(cx, |m, cx| {
            let Some(n) = m.native.as_mut() else {
                return;
            };
            n.menu_open = false;
            let mut devices = n.devices.clone();
            set(&mut devices, value);
            m.use_devices(devices, cx);
        });
    }

    /// The conversation's name for the call bar.
    fn title(&self, key: &CallKey, cx: &App) -> String {
        cx.try_global::<Notifications>()
            .and_then(|h| h.session(&key.workspace))
            .and_then(|s| {
                s.read(cx)
                    .state()
                    .conversations
                    .get(&key.conversation)
                    .map(|c| c.name.to_string())
            })
            .unwrap_or_default()
    }

    /// The running call is the huddle of `c` in `ws` (the call bar shows it,
    /// so the conversation offers no Start or Join).
    pub fn in_call_for(&self, ws: &WorkspaceKey, c: &ConversationId, cx: &App) -> bool {
        if let Some(n) = &self.native {
            return n.key.workspace == *ws && n.key.conversation == *c;
        }
        self.controller.as_ref().is_some_and(|ctl| {
            let st = ctl.read(cx).state();
            st.phase != CallPhase::Idle
                && st
                    .key
                    .as_ref()
                    .is_some_and(|k| k.workspace == *ws && k.conversation == *c)
        })
    }

    /// Opens the running call's huddle in the default browser
    /// (`CallBarEvent::OpenInBrowser`); nothing without a call or a URL.
    pub fn open_in_browser(cx: &mut App) {
        let Some(h) = cx.try_global::<CallManagerHandle>().map(|h| h.0.clone()) else {
            return;
        };
        let m = h.read(cx);
        let url = match &m.native {
            Some(n) => n.provider.huddle_url(&n.key.conversation),
            None => m
                .current
                .as_ref()
                .and_then(|c| c.provider.huddle_url(&c.key.conversation)),
        };
        if let Some(u) = url {
            cx.open_url(&u);
        }
    }

    /// Toggles the microphone of the running call (`call::ToggleMute`).
    pub fn toggle_mute(cx: &mut App) {
        if Self::native_action(cx, |m, _| {
            if let Some(n) = m.native.as_mut() {
                // The engine confirms with a `Muted` event; show it at once.
                n.muted = !n.muted;
                if let Some(h) = &n.handle {
                    h.mute(n.muted);
                }
            }
        }) {
            return;
        }
        Self::drive(cx, |c, cx| c.toggle_mute(cx));
    }

    /// Leaves the running call (`call::Leave`).
    pub fn leave(cx: &mut App) {
        if Self::native_action(cx, |m, cx| m.end_native(cx)) {
            return;
        }
        Self::drive(cx, |c, cx| c.leave(cx));
    }

    /// Runs `f` on the manager when a native call is running.
    fn native_action(cx: &mut App, f: impl FnOnce(&mut Self, &mut Context<Self>)) -> bool {
        let Some(h) = cx.try_global::<CallManagerHandle>().map(|h| h.0.clone()) else {
            return false;
        };
        h.update(cx, |m, cx| {
            if m.native.is_none() {
                return false;
            }
            f(m, cx);
            cx.notify();
            true
        })
    }

    /// Raises the call window (`call::ShowWindow`); nothing without a call.
    pub fn show_window(cx: &mut App) {
        Self::drive(cx, |c, cx| c.show(cx));
    }

    fn drive(cx: &mut App, f: impl FnOnce(&mut CallController, &mut Context<CallController>)) {
        let Some(h) = cx.try_global::<CallManagerHandle>().map(|h| h.0.clone()) else {
            return;
        };
        h.update(cx, |m, cx| {
            if let Some(ctl) = m.controller.clone()
                && ctl.read(cx).state().phase != CallPhase::Idle
            {
                ctl.update(cx, f);
            }
        });
    }

    fn on_event(&mut self, e: CallControllerEvent, cx: &mut Context<Self>) {
        match e {
            CallControllerEvent::Changed => {
                // The call bar repaints from the new state.
                cx.notify();
                let idle = self
                    .controller
                    .as_ref()
                    .is_some_and(|c| c.read(cx).state().phase == CallPhase::Idle);
                if idle {
                    self.current = None;
                    if let Some(next) = self.then.take() {
                        self.begin(next, cx);
                    }
                }
            }
            CallControllerEvent::Fallback { key, reason } => self.fall_back(key, reason, cx),
            CallControllerEvent::AskSwitch { to, .. } => self.ask_switch(to, cx),
            CallControllerEvent::Toast(id) => {
                let ws = self.current.as_ref().map(|c| c.key.workspace.clone());
                if let Some(ws) = ws {
                    notice(&ws, LocalizedText::new("parla", &id), cx);
                }
            }
        }
    }

    /// The window could not join: takes the next rung, once, and says so.
    fn fall_back(&mut self, key: CallKey, reason: FailReason, cx: &mut Context<Self>) {
        crate::log::warn(&format!(
            "call window could not join ({reason:?}); taking the next rung"
        ));
        let rest = match self.current.take() {
            Some(c) if c.key == key => c.rest,
            other => {
                self.current = other;
                Vec::new()
            }
        };
        let id = match rest.into_iter().next() {
            Some(JoinRoute::Browser(u)) => {
                cx.open_url(&u);
                "call-fallback-browser"
            }
            Some(JoinRoute::ProviderApp(u)) => {
                cx.open_url(&u);
                "call-fallback-app"
            }
            _ => "huddle-open-in",
        };
        notice(&key.workspace, LocalizedText::new("parla", id), cx);
    }

    fn ask_switch(&mut self, _to: CallKey, cx: &mut Context<Self>) {
        let l = i18n::gpui::localizer(cx);
        let title = t!(l, "call-switch-title").into_owned();
        let leave = t!(l, "call-switch-leave").into_owned();
        let stay = t!(l, "call-switch-stay").into_owned();
        let prompt_window = cx
            .active_window()
            .or_else(|| cx.windows().into_iter().next());
        let rx = prompt_window.and_then(|w| {
            w.update(cx, |_, window, cx| {
                window.prompt(
                    PromptLevel::Info,
                    &title,
                    None,
                    &[leave.as_str(), stay.as_str()],
                    cx,
                )
            })
            .ok()
        });
        let Some(rx) = rx else {
            self.answer_switch(false, cx);
            return;
        };
        cx.spawn(async move |this, cx| {
            let leave = matches!(rx.await, Ok(0));
            this.update(cx, |m, cx| m.answer_switch(leave, cx)).ok();
        })
        .detach();
    }

    /// "Leave and join" ends this call and starts the other once it is over;
    /// "Stay" drops the request.
    fn answer_switch(&mut self, leave: bool, cx: &mut Context<Self>) {
        let incoming = self.incoming.take();
        let Some(ctl) = self.controller.clone() else {
            return;
        };
        if leave && let Some(i) = incoming {
            self.then = Some(i);
            ctl.update(cx, |c, cx| c.leave(cx));
        }
    }
}

/// The call window title: the provider's conversation name in the app's
/// "Huddle in {name}" (or plain "Huddle" when it has no name).
fn window_title(cx: &App, name: &str) -> String {
    if cx.try_global::<i18n::gpui::I18n>().is_none() {
        return name.to_string();
    }
    let l = i18n::gpui::localizer(cx);
    if name.is_empty() {
        return l
            .format("parla", "call-window-title-empty", None)
            .into_owned();
    }
    let mut args = i18n::FluentArgs::new();
    args.set(
        "name".to_string(),
        i18n::Arg::Str(name.to_string().into()).to_value(),
    );
    l.format("parla", "call-window-title", Some(&args))
        .into_owned()
}

/// Shows `text` as the workspace's notice. Deferred: the session may be
/// updating right now (it is what asked to join).
fn notice(ws: &WorkspaceKey, text: LocalizedText, cx: &mut App) {
    let Some(s) = cx.try_global::<Notifications>().and_then(|h| h.session(ws)) else {
        return;
    };
    cx.defer(move |cx| s.update(cx, |s, cx| s.call_notice(text, cx)));
}
