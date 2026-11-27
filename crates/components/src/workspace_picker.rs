//! The native workspace list shown after a sign-in that found several
//! workspaces: one row per workspace, a tick per workspace to add, and the
//! per-row progress while they are added. Provider-neutral: a provider maps
//! its own workspace data into [`WorkspaceChoice`] and supplies a
//! [`WorkspaceAdder`] for the selected ids.

use std::collections::BTreeSet;
use std::rc::Rc;

use domain::ImageRef;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use futures::stream::StreamExt as _;
use gpui::{
    App, Context, EventEmitter, FocusHandle, IntoElement, KeyDownEvent, Render, Task, Window, div,
    prelude::*,
};
use settings::WorkspaceKey;
use themes::ActiveTheme;

use crate::badge::workspace_badge;
use crate::controls::{ButtonKind, button as ui_button, check_box, error_banner, on_activate};
use crate::sign_in::{Candidate, SignInError, SignInText};

/// One workspace the signed-in session can add.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceChoice {
    /// Provider workspace id (never a secret).
    pub id: String,
    pub name: String,
    /// Subdomain, shown under the name.
    pub domain: String,
    pub icon: Option<ImageRef>,
    pub sso: bool,
    pub two_factor: bool,
    pub signed_out: bool,
    /// Already in Parla: shown disabled with "Added".
    pub added: bool,
    /// Already in Parla and its credentials were just refreshed by this sign-in:
    /// shown as "Updated".
    pub updated: bool,
    /// This added workspace is being refreshed in the hidden sign-in session.
    pub updating: bool,
    /// Refreshing this added workspace failed.
    pub update_error: bool,
    /// The session needs the user to sign in to this workspace (SSO or two-factor):
    /// shown with the needs-user state and tickable, so the add runs its capture
    /// with the webview shown for that workspace.
    pub needs_sign_in: bool,
}

/// Where adding one workspace stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AddState {
    #[default]
    Waiting,
    SigningIn,
    Done,
    NeedsUser,
    Failed,
    /// The user chose to skip this workspace.
    Skipped,
}

/// What the picker is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Choosing,
    Adding,
}

/// Per-row progress reported by a [`WorkspaceAdder`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddStep {
    pub id: String,
    pub state: AddState,
}

/// The outcome of refreshing an added workspace in the background.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// The workspace's credentials were refreshed; restart it with this key.
    Updated(WorkspaceKey),
    /// The user must sign in to this workspace again.
    NeedsUser,
    /// Refreshing failed.
    Failed,
}

/// Progress reported while refreshing an added workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefreshStep {
    pub id: String,
    pub outcome: RefreshOutcome,
}

/// The picker's state: rows, ticks and progress. No GPUI here, so it can be
/// tested on its own.
pub struct WorkspacePickerModel {
    workspaces: Vec<WorkspaceChoice>,
    selected: Vec<bool>,
    states: Vec<AddState>,
    phase: Phase,
}

impl WorkspacePickerModel {
    /// Rows for `workspaces`; every workspace that is not already added and
    /// not signed out starts ticked.
    pub fn new(workspaces: Vec<WorkspaceChoice>) -> Self {
        let selected = workspaces
            .iter()
            .map(|w| !w.added && !w.signed_out)
            .collect();
        let states = vec![AddState::Waiting; workspaces.len()];
        Self {
            workspaces,
            selected,
            states,
            phase: Phase::Choosing,
        }
    }

    pub fn workspaces(&self) -> &[WorkspaceChoice] {
        &self.workspaces
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    fn index(&self, id: &str) -> Option<usize> {
        self.workspaces.iter().position(|w| w.id == id)
    }

    /// Whether the row can be ticked: a workspace already in Parla cannot, unless
    /// it needs the user to sign in, so the user can finish it through the add.
    pub fn selectable(&self, id: &str) -> bool {
        self.index(id)
            .is_some_and(|i| !self.workspaces[i].added || self.workspaces[i].needs_sign_in)
    }

    pub fn is_selected(&self, id: &str) -> bool {
        self.index(id).is_some_and(|i| self.selected[i])
    }

    pub fn set_selected(&mut self, id: &str, on: bool) {
        if let Some(i) = self.index(id).filter(|_| self.selectable(id)) {
            self.selected[i] = on;
        }
    }

    pub fn toggle(&mut self, id: &str) {
        let on = !self.is_selected(id);
        self.set_selected(id, on);
    }

    pub fn selected_ids(&self) -> Vec<String> {
        self.workspaces
            .iter()
            .zip(&self.selected)
            .filter(|(_, on)| **on)
            .map(|(w, _)| w.id.clone())
            .collect()
    }

    /// Starts adding the ticked workspaces and returns their ids. While
    /// choosing, every ticked row goes to [`AddState::Waiting`].
    pub fn begin_add(&mut self) -> Vec<String> {
        if self.phase != Phase::Choosing {
            return Vec::new();
        }
        let ids = self.selected_ids();
        if ids.is_empty() {
            return ids;
        }
        for (w, on) in self.workspaces.iter().zip(&self.selected) {
            if *on && let Some(i) = self.index(&w.id) {
                self.states[i] = AddState::Waiting;
            }
        }
        self.phase = Phase::Adding;
        ids
    }

    /// Records one row's progress. Ignored unless adding and the row is ticked.
    pub fn progress(&mut self, id: &str, state: AddState) {
        if self.phase != Phase::Adding {
            return;
        }
        if let Some(i) = self.index(id).filter(|i| self.selected[*i]) {
            self.states[i] = state;
        }
    }

    /// Name of the workspace that waits for the user to sign in, if any.
    pub fn needs_user(&self) -> Option<&str> {
        self.workspaces
            .iter()
            .zip(&self.states)
            .find(|(_, s)| **s == AddState::NeedsUser)
            .map(|(w, _)| w.name.as_str())
    }

    /// Marks the workspace waiting for the user as skipped. Returns whether
    /// there was one.
    pub fn skip_waiting(&mut self) -> bool {
        if self.phase != Phase::Adding {
            return false;
        }
        match self.states.iter_mut().find(|s| **s == AddState::NeedsUser) {
            Some(s) => {
                *s = AddState::Skipped;
                true
            }
            None => false,
        }
    }

    /// Whether any ticked row ended without being added.
    pub fn has_unfinished(&self) -> bool {
        self.selected
            .iter()
            .zip(&self.states)
            .any(|(on, s)| *on && *s != AddState::Done)
    }

    pub fn state(&self, id: &str) -> Option<AddState> {
        self.index(id).map(|i| self.states[i])
    }

    /// Marks an added workspace as being refreshed in the background.
    pub fn set_updating(&mut self, id: &str, updating: bool) {
        if let Some(i) = self.index(id) {
            self.workspaces[i].updating = updating;
        }
    }

    /// The background refresh captured this added workspace.
    pub fn refresh_updated(&mut self, id: &str) {
        if let Some(i) = self.index(id) {
            self.workspaces[i].updating = false;
            self.workspaces[i].update_error = false;
            self.workspaces[i].needs_sign_in = false;
            self.workspaces[i].updated = true;
        }
    }

    /// The background refresh needs the user to sign in to this workspace.
    pub fn refresh_needs_user(&mut self, id: &str) {
        if let Some(i) = self.index(id) {
            self.workspaces[i].updating = false;
            self.workspaces[i].update_error = false;
            self.workspaces[i].updated = false;
            self.workspaces[i].needs_sign_in = true;
        }
    }

    /// The background refresh failed for this added workspace.
    pub fn refresh_failed(&mut self, id: &str) {
        if let Some(i) = self.index(id) {
            self.workspaces[i].updating = false;
            self.workspaces[i].needs_sign_in = false;
            self.workspaces[i].update_error = true;
        }
    }

    /// Ends a run: `added` are done, and any other ticked row that did not
    /// finish (the run stopped early) is failed. Back to choosing.
    pub fn finish_add(&mut self, added: &[String]) {
        if self.phase != Phase::Adding {
            return;
        }
        let added: BTreeSet<&String> = added.iter().collect();
        for (i, on) in self.selected.iter().enumerate() {
            if !*on {
                continue;
            }
            if added.contains(&self.workspaces[i].id) {
                self.states[i] = AddState::Done;
                self.workspaces[i].needs_sign_in = false;
                self.workspaces[i].update_error = false;
                self.workspaces[i].updated = true;
            } else if !matches!(
                self.states[i],
                AddState::Done | AddState::Failed | AddState::Skipped
            ) {
                self.states[i] = AddState::Failed;
            }
        }
        self.phase = Phase::Choosing;
    }
}

/// Adds the ticked workspaces, reporting each row's progress. The provider
/// keeps whatever session it needs (for example a webview) alive and shows its
/// own UI when a workspace needs the user.
pub trait WorkspaceAdder: 'static {
    /// The user chose to skip the workspace that waits for them: stop waiting
    /// and go on with the next one.
    fn skip(&self) {}

    /// Refreshes added workspaces in the background, reporting each row's outcome.
    /// The default does nothing.
    fn refresh(
        &self,
        _ids: Vec<String>,
        _progress: UnboundedSender<RefreshStep>,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn add(
        &self,
        ids: Vec<String>,
        progress: UnboundedSender<AddStep>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Vec<Candidate>, SignInError>>;
}

/// What the picker reports to its host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspacePickerEvent {
    /// These workspaces were added (at least one).
    Added(Vec<WorkspaceKey>),
    Cancelled,
}

/// The list view. It owns the model and the add task, so the host only has to
/// place it and listen for [`WorkspacePickerEvent`].
pub struct WorkspacePicker {
    model: WorkspacePickerModel,
    adder: Rc<dyn WorkspaceAdder>,
    running: Option<Task<()>>,
    refresh_running: Option<Task<()>>,
    pending_refresh: Option<Vec<String>>,
    error: Option<SignInText>,
    /// Added workspaces held back while failed rows stay visible; reported on
    /// Close.
    pending: Option<Vec<WorkspaceKey>>,
    /// Already-added workspaces refreshed by this sign-in; always reported.
    refreshed: Vec<WorkspaceKey>,
    /// Receives Escape; focused on the first render.
    focus: FocusHandle,
    focus_pending: bool,
}

impl EventEmitter<WorkspacePickerEvent> for WorkspacePicker {}

impl WorkspacePicker {
    pub fn new(
        model: WorkspacePickerModel,
        adder: Rc<dyn WorkspaceAdder>,
        refreshed: Vec<WorkspaceKey>,
        refresh_ids: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            model,
            adder,
            running: None,
            refresh_running: None,
            pending_refresh: (!refresh_ids.is_empty()).then_some(refresh_ids),
            error: None,
            pending: None,
            refreshed,
            focus: cx.focus_handle(),
            focus_pending: true,
        }
    }

    fn ensure_refresh_started(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ids) = self.pending_refresh.take() else {
            return;
        };
        let (tx, rx) = unbounded();
        self.adder.refresh(ids, tx, window, cx);
        self.refresh_running = Some(cx.spawn(async move |this, cx| {
            let mut rx = rx.fuse();
            while let Some(step) = rx.next().await {
                this.update(cx, |this, cx| this.refresh_progress(step, cx))
                    .ok();
            }
        }));
    }

    fn refresh_progress(&mut self, step: RefreshStep, cx: &mut Context<Self>) {
        match step.outcome {
            RefreshOutcome::Updated(key) => {
                self.model.refresh_updated(&step.id);
                if !self.refreshed.iter().any(|k| k == &key) {
                    self.refreshed.push(key);
                }
            }
            RefreshOutcome::NeedsUser => self.model.refresh_needs_user(&step.id),
            RefreshOutcome::Failed => self.model.refresh_failed(&step.id),
        }
        cx.notify();
    }

    pub fn model(&self) -> &WorkspacePickerModel {
        &self.model
    }

    pub fn error(&self) -> Option<&SignInText> {
        self.error.as_ref()
    }

    /// Ticks or unticks a row while choosing. Added workspaces do nothing.
    pub fn choose(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.model.phase() != Phase::Choosing {
            return;
        }
        self.model.toggle(id);
        cx.notify();
    }

    /// Adds the ticked workspaces. Does nothing with none ticked or while a
    /// run is in flight.
    pub fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.running.is_some() {
            return;
        }
        let ids = self.model.begin_add();
        if ids.is_empty() {
            cx.notify();
            return;
        }
        self.error = None;
        let (tx, rx) = unbounded();
        let task = self.adder.add(ids, tx, window, cx);
        self.running = Some(crate::pump::pump(
            task,
            rx,
            cx,
            |this, step, cx| this.progress(step, cx),
            |this, result, cx| this.finished(result, cx),
        ));
        cx.notify();
    }

    /// Skips the workspace that waits for the user and goes on.
    pub fn skip(&mut self, cx: &mut Context<Self>) {
        if self.model.skip_waiting() {
            self.adder.skip();
        }
        cx.notify();
    }

    /// Closes the list after a partial success and reports what was added.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if let Some(keys) = self.pending.take() {
            cx.emit(WorkspacePickerEvent::Added(self.with_refreshed(keys)));
        }
        cx.notify();
    }

    /// Refreshed workspaces first, so the newly added one opens last.
    fn with_refreshed(&self, keys: Vec<WorkspaceKey>) -> Vec<WorkspaceKey> {
        let mut all = self.refreshed.clone();
        all.extend(keys);
        all
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.model.skip_waiting() {
            self.adder.skip();
        }
        self.running = None;
        self.refresh_running = None;
        if self.refreshed.is_empty() {
            cx.emit(WorkspacePickerEvent::Cancelled);
        } else {
            // The sign-in ends, but the refreshed workspaces still restart.
            cx.emit(WorkspacePickerEvent::Added(self.refreshed.clone()));
        }
        cx.notify();
    }

    fn progress(&mut self, step: AddStep, cx: &mut Context<Self>) {
        self.model.progress(&step.id, step.state);
        cx.notify();
    }

    fn finished(&mut self, result: Result<Vec<Candidate>, SignInError>, cx: &mut Context<Self>) {
        self.running = None;
        match result {
            Ok(candidates) => {
                let keys: Vec<WorkspaceKey> = candidates
                    .into_iter()
                    .filter(|c| c.ready)
                    .map(|c| c.key)
                    .collect();
                let ids: Vec<String> = keys.iter().map(|k| k.id.0.to_string()).collect();
                self.model.finish_add(&ids);
                if keys.is_empty() {
                    self.error = Some(t!(cx, "workspace-picker-nothing-added").into_owned().into());
                } else if self.model.has_unfinished() {
                    // Keep the failed rows in view; Close reports the rest.
                    self.pending = Some(keys);
                } else {
                    cx.emit(WorkspacePickerEvent::Added(self.with_refreshed(keys)));
                }
            }
            Err(e) => {
                self.model.finish_add(&[]);
                self.error = Some(match e {
                    SignInError::Failed(text) => text,
                    SignInError::SwitchTo { reason, .. } => reason,
                });
            }
        }
        cx.notify();
    }
}

fn chip(label: String, colors: &themes::ThemeColors) -> impl IntoElement {
    div()
        .px(themes::metrics::SPACE_4)
        .py(themes::metrics::SPACE_1)
        .rounded_full()
        .border_1()
        .border_color(colors.border)
        .text_color(colors.text_muted)
        .text_size(themes::metrics::FONT_SIZE_XS)
        .child(label)
}

impl Render for WorkspacePicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_refresh_started(window, cx);
        if std::mem::take(&mut self.focus_pending) {
            window.focus(&self.focus, cx);
        }
        let colors = *cx.theme().colors();
        let error_colour = cx.theme().status().error;
        let loc = i18n::gpui::localizer(cx);
        let adding = self.model.phase() == Phase::Adding;
        let selected = self.model.selected_ids().len();

        let mut rows = Vec::new();
        for w in self.model.workspaces() {
            let id = w.id.clone();
            let id2 = id.clone();
            let ticked = self.model.is_selected(&id);
            let state = self.model.state(&id).unwrap_or_default();
            let selectable = self.model.selectable(&id);
            let mut marks = Vec::new();
            if w.sso {
                marks.push(
                    chip(t!(cx, "workspace-picker-sso").into_owned(), &colors).into_any_element(),
                );
            }
            if w.two_factor {
                marks.push(
                    chip(t!(cx, "workspace-picker-2fa").into_owned(), &colors).into_any_element(),
                );
            }
            if w.signed_out {
                marks.push(
                    chip(t!(cx, "workspace-picker-signed-out").into_owned(), &colors)
                        .into_any_element(),
                );
            }
            let status = if adding {
                Some(
                    match state {
                        AddState::Waiting => t!(cx, "workspace-picker-state-waiting"),
                        AddState::SigningIn => t!(cx, "workspace-picker-state-signing-in"),
                        AddState::Done => t!(cx, "workspace-picker-state-done"),
                        AddState::NeedsUser => t!(cx, "workspace-picker-state-needs-user"),
                        AddState::Failed => t!(cx, "workspace-picker-state-failed"),
                        AddState::Skipped => t!(cx, "workspace-picker-state-skipped"),
                    }
                    .into_owned(),
                )
            } else if w.updating {
                Some(t!(cx, "workspace-picker-updating").into_owned())
            } else if w.needs_sign_in {
                // An already-added workspace whose refresh asked the user to sign
                // in: the row waits for them, and the user can tick it to run its
                // capture with the webview shown for that workspace.
                Some(t!(cx, "workspace-picker-state-needs-user").into_owned())
            } else if w.update_error {
                Some(t!(cx, "workspace-picker-state-failed").into_owned())
            } else if w.updated {
                Some(t!(cx, "workspace-picker-updated").into_owned())
            } else if w.added {
                Some(t!(cx, "workspace-picker-added").into_owned())
            } else {
                None
            };
            // An added workspace is dimmed, but its status text stays at full
            // contrast. Dim each colour, not the row: GPUI Fast multiplies
            // element opacity into every paint, so the badge's initials faded
            // over the already-faded fill (main faded the badge as one image
            // and kept the initials white on the pale fill).
            const DIM: f32 = 0.55;
            let dimmed = w.added && !w.needs_sign_in;
            let alpha = if dimmed { DIM } else { 1.0 };
            let control_border = themes::parla_tokens(cx.theme()).border_control;
            let mut row = div()
                .id(gpui::SharedString::from(format!("workspace-picker-{id}")))
                .debug_selector({
                    let id = id.clone();
                    move || format!("workspace-picker-row-{id}")
                })
                .flex()
                .items_center()
                .gap(themes::metrics::SPACE_5)
                .p(themes::metrics::SPACE_5)
                .rounded(themes::metrics::RADIUS_LG)
                .border_1()
                .border_color(if ticked {
                    colors.element_background
                } else {
                    colors.border
                })
                .bg(colors.surface_background)
                .when(ticked, |d| {
                    // The selected row's soft accent halo.
                    d.shadow(vec![gpui::BoxShadow {
                        color: colors.accent_soft,
                        offset: gpui::point(gpui::px(0.0), gpui::px(0.0)),
                        blur_radius: gpui::px(0.0),
                        spread_radius: themes::metrics::SPACE_1,
                        inset: false,
                    }])
                })
                .when(selectable && !adding, |d| d.cursor_pointer())
                .tab_stop(true)
                .focusable()
                .focus_visible(crate::controls::focus_style(&colors))
                .child(
                    check_box(ticked, &colors, control_border)
                        .when(dimmed, |d| {
                            let fill = if ticked {
                                colors.element_background
                            } else {
                                colors.surface_background
                            };
                            let ground = colors.surface_background;
                            // A ticked box's border is its fill: drop it, so
                            // the dimmed fill is not outlined. The tick keeps
                            // its own colour, close to main's (the row ground
                            // sits behind it in both themes).
                            d.bg(themes::color::faded(fill, ground, DIM))
                                .border_color(if ticked {
                                    gpui::transparent_black()
                                } else {
                                    themes::color::faded(control_border, ground, DIM)
                                })
                        })
                        .into_any_element(),
                )
                .child(div().flex_none().child(if dimmed {
                    crate::badge::workspace_badge_dimmed(
                        &w.name,
                        w.icon.clone(),
                        DIM,
                        colors.surface_background,
                    )
                } else {
                    workspace_badge(&w.name, w.icon.clone())
                }))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(themes::metrics::FONT_SIZE_LG)
                                .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                                .text_color(colors.text.opacity(alpha))
                                .child(w.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(themes::metrics::FONT_SIZE_SM)
                                .text_color(colors.text_muted.opacity(alpha))
                                .child(w.domain.clone()),
                        ),
                )
                .children(marks);
            if let Some(status) = status {
                let error = state == AddState::Failed || (!adding && w.update_error);
                row = row.child(
                    div()
                        .text_size(themes::metrics::FONT_SIZE_SM)
                        .text_color(if error {
                            error_colour
                        } else {
                            colors.text_muted
                        })
                        .child(status),
                );
            }
            rows.push(on_activate(row, cx, move |this, _, cx| {
                this.choose(&id2, cx)
            }));
        }

        let button = |id: &'static str,
                      label: String,
                      enabled: bool,
                      accent: bool|
         -> gpui::Stateful<gpui::Div> {
            ui_button(
                gpui::SharedString::from(format!("workspace-picker-{id}")),
                label,
                if accent {
                    ButtonKind::Primary
                } else {
                    ButtonKind::Secondary
                },
                enabled,
                &colors,
            )
            .debug_selector(move || format!("workspace-picker-{id}"))
        };

        if let Some(name) = self.model.needs_user() {
            // The sign-in page covers everything below this strip.
            return div()
                .debug_selector(|| "workspace-picker-needs-user".into())
                .flex()
                .flex_col()
                .size_full()
                .bg(colors.background)
                .text_color(colors.text)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .px_3()
                        .h(crate::sign_in::OVERLAY_TOP)
                        .child(
                            div().flex_1().child(
                                t!(cx, "workspace-picker-needs-you", name = name.to_string())
                                    .into_owned(),
                            ),
                        )
                        .child(on_activate(
                            button(
                                "skip",
                                t!(cx, "workspace-picker-skip").into_owned(),
                                true,
                                false,
                            ),
                            cx,
                            |this, _, cx| this.skip(cx),
                        ))
                        .child(on_activate(
                            button(
                                "cancel",
                                t!(cx, "workspace-picker-cancel").into_owned(),
                                true,
                                false,
                            ),
                            cx,
                            |this, _, cx| this.cancel(cx),
                        )),
                )
                .into_any_element();
        }
        let closing = self.pending.is_some();
        let add_label = t!(cx, "workspace-picker-add", count = selected).into_owned();
        let mut footer = div().flex().items_center().gap(themes::metrics::SPACE_5);
        if !closing {
            footer = footer.child(on_activate(
                button(
                    "cancel",
                    t!(cx, "workspace-picker-cancel").into_owned(),
                    true,
                    false,
                ),
                cx,
                |this, _, cx| this.cancel(cx),
            ));
        }
        footer = footer.child(div().flex_1());
        if closing {
            footer = footer.child(on_activate(
                button(
                    "close",
                    t!(cx, "workspace-picker-close").into_owned(),
                    true,
                    true,
                ),
                cx,
                |this, _, cx| this.close(cx),
            ));
        } else if selected > 0 {
            footer = footer.child(on_activate(
                button("add", add_label, !adding, true),
                cx,
                |this, window, cx| this.add(window, cx),
            ));
        }

        let mut col = div()
            .key_context("WorkspacePicker")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let k = &ev.keystroke;
                let m = &k.modifiers;
                if m.platform || m.control || m.alt {
                    return;
                }
                match k.key.as_str() {
                    // After a partial add Close is the only way out: Escape must
                    // not drop the workspaces that were added.
                    "escape" if this.pending.is_some() => this.close(cx),
                    "escape" => this.cancel(cx),
                    "tab" if m.shift => window.focus_prev(cx),
                    "tab" => window.focus_next(cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .flex()
            .flex_col()
            .w_full()
            .gap(themes::metrics::SPACE_5)
            .p(themes::metrics::SPACE_9)
            .max_w(themes::metrics::LAYOUT_SIGN_IN_CONTENT_MAX)
            .child(
                div()
                    .text_size(themes::metrics::FONT_SIZE_3XL)
                    .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                    .child(t!(cx, "workspace-picker-title").into_owned()),
            )
            .child(
                div()
                    .text_size(themes::metrics::FONT_SIZE_MD)
                    .text_color(colors.text_muted)
                    .child(t!(cx, "workspace-picker-subtitle").into_owned()),
            )
            .children(rows);
        if let Some(e) = &self.error {
            col = col.child(error_banner(e.resolve(&loc), error_colour, &colors));
        }
        col = col.child(footer);

        // Scrolls when many workspaces make the card taller than the window;
        // `my_auto` centres it while it fits.
        div()
            .id("workspace-picker-scroll")
            .debug_selector(|| "workspace-picker".into())
            .flex()
            .flex_col()
            .size_full()
            .items_center()
            .overflow_y_scroll()
            .bg(colors.background)
            .text_color(colors.text)
            .child(col.flex_none().my_auto())
            .into_any_element()
    }
}
