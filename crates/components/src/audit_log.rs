//! The MCP audit log: every
//! tool call an agent made, newest first, in a virtualised list that loads its
//! next page as the user scrolls. Filters narrow it by workspace, tool and
//! writes only; selecting a row shows its details, and the recorded text only
//! when the owner had `log.include_content` on at the time.

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Context, FocusHandle, Focusable, IntoElement, Render, SharedString, Task,
    UniformListScrollHandle, Window, actions, div, prelude::*, px, uniform_list,
};
use settings::WorkspaceKey;
use store::{AppStore, AuditOutcome, AuditQuery, AuditRow, WriteDecision};
use themes::{ActiveTheme, metrics, parla_tokens};

actions!(mcp, [ClearAuditLog]);

/// Key context of the view.
pub const KEY_CONTEXT: &str = "AuditLog";

/// Rows fetched per page.
pub const PAGE: usize = 50;
/// The next page loads when the list shows a row this close to the end.
const PREFETCH: usize = 8;

/// What the app knows about the names in the log.
pub trait AuditNames: 'static {
    /// The workspaces to filter by, with their display names.
    fn workspaces(&self, cx: &App) -> Vec<(WorkspaceKey, SharedString)>;
    /// The tool names to filter by.
    fn tools(&self) -> Vec<SharedString>;
    /// The conversation's name, looked up when the row is drawn.
    fn conversation(&self, ws: &WorkspaceKey, id: &str, cx: &App) -> Option<SharedString>;
}

/// What went wrong last; shown above the list until the user acts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditError {
    /// A page failed to load; "Retry" asks again.
    Load,
    /// Clearing the log failed; the rows are still there.
    Clear,
}

pub struct AuditLogView {
    store: Arc<dyn AppStore>,
    names: Rc<dyn AuditNames>,
    page: usize,
    rows: Vec<AuditRow>,
    exhausted: bool,
    loading: Option<Task<()>>,
    /// The running clear, kept apart so a filter change does not cancel it.
    clearing: Option<Task<()>>,
    error: Option<AuditError>,
    /// Bumped by every reload so a slow page of an old filter is dropped.
    generation: u64,
    workspace: Option<WorkspaceKey>,
    tool: Option<Arc<str>>,
    writes_only: bool,
    selected: Option<i64>,
    confirming_clear: bool,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

impl Focusable for AuditLogView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Date and time in the user's time zone and language.
fn local_time(l: &i18n::Localizer, ms: i64, cx: &App) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|t| l.date_time(t, &crate::time_zone(cx), i18n::HourCycle::H24))
        .unwrap_or_default()
}

fn outcome_label(l: &i18n::Localizer, o: AuditOutcome) -> String {
    match o {
        AuditOutcome::Ok => t!(l, "audit-outcome-ok"),
        AuditOutcome::Denied => t!(l, "audit-outcome-denied"),
        AuditOutcome::NotAvailable => t!(l, "audit-outcome-not-available"),
        AuditOutcome::RateLimited => t!(l, "audit-outcome-rate-limited"),
        AuditOutcome::Declined => t!(l, "audit-outcome-declined"),
        AuditOutcome::TimedOut => t!(l, "audit-outcome-timed-out"),
        AuditOutcome::Failed => t!(l, "audit-outcome-failed"),
    }
    .into_owned()
}

fn decision_label(l: &i18n::Localizer, d: Option<WriteDecision>) -> String {
    match d {
        None => String::new(),
        Some(WriteDecision::Confirmed) => t!(l, "audit-decision-confirmed").into_owned(),
        Some(WriteDecision::Auto) => t!(l, "audit-decision-auto").into_owned(),
        Some(WriteDecision::Declined) => t!(l, "audit-decision-declined").into_owned(),
        Some(WriteDecision::TimedOut) => t!(l, "audit-decision-timed-out").into_owned(),
        Some(WriteDecision::SessionAllowed) => t!(l, "audit-decision-session").into_owned(),
        Some(WriteDecision::ConfirmedForSession) => {
            t!(l, "audit-decision-confirmed-session").into_owned()
        }
    }
}

impl AuditLogView {
    pub fn new(
        store: Arc<dyn AppStore>,
        names: Rc<dyn AuditNames>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_page(store, names, PAGE, cx)
    }

    /// Like [`AuditLogView::new`] with a page size (tests).
    pub fn with_page(
        store: Arc<dyn AppStore>,
        names: Rc<dyn AuditNames>,
        page: usize,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut v = Self {
            store,
            names,
            page,
            rows: Vec::new(),
            exhausted: false,
            loading: None,
            clearing: None,
            error: None,
            generation: 0,
            workspace: None,
            tool: None,
            writes_only: false,
            selected: None,
            confirming_clear: false,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
        };
        v.load_more(cx);
        v
    }

    /// The loaded rows, newest first.
    pub fn rows(&self) -> &[AuditRow] {
        &self.rows
    }

    /// Whether the log has no older rows to load.
    pub fn is_exhausted(&self) -> bool {
        self.exhausted
    }

    pub fn workspace_filter(&self) -> Option<&WorkspaceKey> {
        self.workspace.as_ref()
    }

    pub fn tool_filter(&self) -> Option<&str> {
        self.tool.as_deref()
    }

    pub fn writes_only(&self) -> bool {
        self.writes_only
    }

    /// The last failure, if any.
    pub fn error(&self) -> Option<AuditError> {
        self.error
    }

    pub fn is_confirming_clear(&self) -> bool {
        self.confirming_clear
    }

    fn query(&self) -> AuditQuery {
        AuditQuery {
            before_id: self.rows.last().map(|r| r.id),
            limit: self.page,
            workspace: self.workspace.clone(),
            tool: self.tool.clone(),
            writes_only: self.writes_only,
        }
    }

    /// Fetches the next page (nothing when one is in flight or the end is reached).
    pub fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.exhausted || self.loading.is_some() {
            return;
        }
        let (q, generation, store) = (self.query(), self.generation, self.store.clone());
        let limit = q.limit;
        self.loading = Some(cx.spawn(async move |this, cx| {
            let rows = store.audit_page(q).await;
            this.update(cx, |v, cx| {
                if v.generation != generation {
                    return;
                }
                v.loading = None;
                match rows {
                    Ok(rows) => {
                        v.exhausted = rows.len() < limit;
                        v.rows.extend(rows);
                    }
                    Err(_) => v.error = Some(AuditError::Load),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// The list shows rows up to `end` (exclusive): loads more when near the end.
    pub fn on_visible(&mut self, end: usize, cx: &mut Context<Self>) {
        // A failed page waits for an explicit retry instead of looping.
        if self.error.is_none() && end + PREFETCH >= self.rows.len() {
            self.load_more(cx);
        }
    }

    /// Asks again after a failed page.
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        if self.error == Some(AuditError::Load) {
            self.error = None;
            self.load_more(cx);
            cx.notify();
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        self.generation += 1;
        self.rows.clear();
        self.selected = None;
        self.exhausted = false;
        self.loading = None;
        self.load_more(cx);
        cx.notify();
    }

    pub fn set_workspace_filter(&mut self, ws: Option<WorkspaceKey>, cx: &mut Context<Self>) {
        self.workspace = ws;
        self.reload(cx);
    }

    pub fn set_tool_filter(&mut self, tool: Option<Arc<str>>, cx: &mut Context<Self>) {
        self.tool = tool;
        self.reload(cx);
    }

    pub fn set_writes_only(&mut self, on: bool, cx: &mut Context<Self>) {
        self.writes_only = on;
        self.reload(cx);
    }

    /// Selects a row (a second click on it unselects).
    pub fn select(&mut self, id: i64, cx: &mut Context<Self>) {
        self.selected = (self.selected != Some(id)).then_some(id);
        cx.notify();
    }

    pub fn selected(&self) -> Option<&AuditRow> {
        let id = self.selected?;
        self.rows.iter().find(|r| r.id == id)
    }

    /// The recorded text of the selected row; only present when it was logged.
    pub fn selected_content(&self) -> Option<&str> {
        self.selected()?.content.as_deref()
    }

    /// `mcp::ClearAuditLog`: asks first, deletes nothing yet.
    pub fn request_clear(&mut self, cx: &mut Context<Self>) {
        self.confirming_clear = true;
        cx.notify();
    }

    pub fn cancel_clear(&mut self, cx: &mut Context<Self>) {
        self.confirming_clear = false;
        cx.notify();
    }

    /// Deletes every row (after [`AuditLogView::request_clear`]).
    pub fn confirm_clear(&mut self, cx: &mut Context<Self>) {
        if !self.confirming_clear {
            return;
        }
        self.confirming_clear = false;
        let store = self.store.clone();
        if self.clearing.is_some() {
            return;
        }
        self.error = None;
        self.clearing = Some(cx.spawn(async move |this, cx| {
            let done = store.audit_clear().await.is_ok();
            this.update(cx, |v, cx| {
                v.clearing = None;
                if done {
                    // Drops any page still loading from before the delete.
                    v.generation += 1;
                    v.loading = None;
                    v.rows.clear();
                    v.selected = None;
                    v.exhausted = true;
                } else {
                    v.error = Some(AuditError::Clear);
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// One workspace name for a row: the display name, or the key when signed out.
    pub fn workspace_label(&self, row: &AuditRow, cx: &App) -> String {
        Self::label_in(&row.workspace, &self.names.workspaces(cx))
    }

    fn label_in(ws: &Option<WorkspaceKey>, names: &[(WorkspaceKey, SharedString)]) -> String {
        match ws {
            None => String::new(),
            Some(k) => names
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, n)| n.to_string())
                .unwrap_or_else(|| k.to_string()),
        }
    }

    /// The conversation's name looked up now, else the stored id.
    pub fn target_label(&self, row: &AuditRow, cx: &App) -> String {
        let Some(t) = row.target.as_deref() else {
            return String::new();
        };
        row.workspace
            .as_ref()
            .and_then(|ws| self.names.conversation(ws, t, cx))
            .map(|n| format!("#{n}"))
            .unwrap_or_else(|| t.to_string())
    }

    /// Column widths, in cell order; `0` is the flexible target column.
    fn cols() -> [f32; 8] {
        [
            150.0,
            110.0,
            130.0,
            110.0,
            0.0,
            f32::from(metrics::LAYOUT_COLUMN_WIDE),
            80.0,
            60.0,
        ]
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        self.on_visible(range.end, cx);
        let colors = *cx.theme().colors();
        let status = *cx.theme().status();
        let tokens = parla_tokens(cx.theme());
        let l = i18n::gpui::localizer(cx);
        let workspaces = self.names.workspaces(cx);
        range
            .filter_map(|ix| self.rows.get(ix).map(|r| (ix, r)))
            .map(|(ix, row)| {
                let id = row.id;
                let selected = self.selected == Some(id);
                let cells = [
                    local_time(&l, row.at_ms, cx),
                    row.client.to_string(),
                    row.tool.to_string(),
                    Self::label_in(&row.workspace, &workspaces),
                    self.target_label(row, cx),
                    outcome_label(&l, row.outcome),
                    decision_label(&l, row.decision),
                    format!("{} ms", row.duration_ms),
                ];
                let mut line = div()
                    .id(("audit-row", ix))
                    .debug_selector(move || format!("audit-row-{ix}"))
                    .flex()
                    .items_center()
                    .gap(metrics::SPACE_4)
                    .w_full()
                    .h(metrics::LAYOUT_ROW_COMPACT)
                    .px(metrics::SPACE_6)
                    .text_size(metrics::FONT_SIZE_XS)
                    .border_b_1()
                    .border_color(tokens.border_subtle)
                    .cursor_pointer()
                    .when(!selected, |d| d.hover(|d| d.bg(colors.element_hover)))
                    .when(selected, |d| d.bg(colors.element_selected))
                    .on_click(cx.listener(move |v, _, _, cx| v.select(id, cx)));
                let (pill_fg, pill_bg) = match row.outcome {
                    AuditOutcome::Ok => (status.success, tokens.success_soft),
                    AuditOutcome::RateLimited | AuditOutcome::TimedOut => {
                        (status.warning, tokens.warning_soft)
                    }
                    _ => (status.error, tokens.danger_soft),
                };
                for (i, (w, text)) in Self::cols().into_iter().zip(cells).enumerate() {
                    let cell = match i {
                        0 => div()
                            .truncate()
                            .font_family("JetBrains Mono")
                            .text_color(colors.text_muted)
                            .child(text),
                        2 => div().truncate().font_family("JetBrains Mono").child(text),
                        // The soft fill sits on the app ground, so the text keeps
                        // its contrast on a hovered or selected row too.
                        5 => div().flex().child(
                            div()
                                .rounded(metrics::RADIUS_FULL)
                                .bg(colors.background)
                                .child(
                                    div()
                                        .truncate()
                                        .px(metrics::SPACE_4)
                                        .h(metrics::SPACE_7)
                                        .flex()
                                        .items_center()
                                        .rounded(metrics::RADIUS_FULL)
                                        .bg(pill_bg)
                                        .text_color(pill_fg)
                                        .text_size(metrics::FONT_SIZE_2XS)
                                        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                                        .child(text),
                                ),
                        ),
                        _ => div().truncate().child(text),
                    };
                    line = line.child(if w == 0.0 {
                        cell.flex_1().min_w_0()
                    } else {
                        cell.w(px(w)).flex_none()
                    });
                }
                line.into_any_element()
            })
            .collect()
    }

    fn pill(
        &self,
        id: &'static str,
        label: String,
        active: bool,
        cx: &mut Context<Self>,
        on: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        let colors = *cx.theme().colors();
        let tokens = parla_tokens(cx.theme());
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .px(metrics::SPACE_5)
            .py(metrics::SPACE_2)
            .rounded(metrics::RADIUS_MD)
            .border_1()
            .border_color(if active {
                colors.accent_soft
            } else {
                colors.border
            })
            .when(active, |d| {
                d.bg(colors.accent_soft).text_color(tokens.accent_soft_text)
            })
            .text_size(metrics::FONT_SIZE_XS)
            .font_weight(metrics::FONT_WEIGHT_MEDIUM)
            .cursor_pointer()
            .when(!active, |d| d.hover(|d| d.bg(colors.element_hover)))
            .child(label)
            .on_click(cx.listener(move |v, _, _, cx| on(v, cx)))
    }

    /// Next value of a filter that cycles through `options` and back to none.
    fn next<T: Clone + PartialEq>(current: Option<&T>, options: &[T]) -> Option<T> {
        match current {
            None => options.first().cloned(),
            Some(c) => {
                let i = options.iter().position(|o| o == c)?;
                options.get(i + 1).cloned()
            }
        }
    }
}

impl Render for AuditLogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.theme().colors();
        let l = i18n::gpui::localizer(cx);
        let workspaces = self.names.workspaces(cx);
        let tools = self.names.tools();

        let ws_label = match &self.workspace {
            None => t!(l, "audit-filter-workspace-all").into_owned(),
            Some(k) => workspaces
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, n)| n.to_string())
                .unwrap_or_else(|| k.to_string()),
        };
        let tool_label = match &self.tool {
            None => t!(l, "audit-filter-tool-all").into_owned(),
            Some(t) => t.to_string(),
        };
        let keys: Vec<WorkspaceKey> = workspaces.iter().map(|(k, _)| k.clone()).collect();
        let names: Vec<Arc<str>> = tools.iter().map(|t| Arc::from(t.as_ref())).collect();

        let toolbar = div()
            .flex()
            .items_center()
            .gap(metrics::SPACE_4)
            .p(metrics::SPACE_6)
            .child(
                div()
                    .flex_1()
                    .text_size(metrics::FONT_SIZE_XL)
                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                    .child(t!(l, "audit-title").into_owned()),
            )
            .child(self.pill(
                "audit-filter-workspace",
                ws_label,
                self.workspace.is_some(),
                cx,
                {
                    move |v, cx| {
                        let n = Self::next(v.workspace.as_ref(), &keys);
                        v.set_workspace_filter(n, cx)
                    }
                },
            ))
            .child(
                self.pill("audit-filter-tool", tool_label, self.tool.is_some(), cx, {
                    move |v, cx| {
                        let n = Self::next(v.tool.as_ref(), &names);
                        v.set_tool_filter(n, cx)
                    }
                }),
            )
            .child(self.pill(
                "audit-filter-writes",
                t!(l, "audit-filter-writes-only").into_owned(),
                self.writes_only,
                cx,
                |v, cx| {
                    let on = !v.writes_only;
                    v.set_writes_only(on, cx)
                },
            ))
            .child(self.pill(
                "audit-clear",
                t!(l, "audit-clear").into_owned(),
                false,
                cx,
                |v, cx| v.request_clear(cx),
            ));

        let confirm = self.confirming_clear.then(|| {
            div()
                .debug_selector(|| "audit-clear-confirm".to_string())
                .flex()
                .items_center()
                .gap(metrics::SPACE_4)
                .px(metrics::SPACE_5)
                .py(metrics::SPACE_2)
                .bg(colors.element_hover)
                .child(
                    div()
                        .flex_1()
                        .child(t!(l, "audit-clear-confirm").into_owned()),
                )
                .child(self.pill(
                    "audit-clear-no",
                    t!(l, "audit-clear-no").into_owned(),
                    true,
                    cx,
                    |v, cx| v.cancel_clear(cx),
                ))
                .child(self.pill(
                    "audit-clear-yes",
                    t!(l, "audit-clear-yes").into_owned(),
                    false,
                    cx,
                    |v, cx| v.confirm_clear(cx),
                ))
        });

        let heads = [
            t!(l, "audit-col-time"),
            t!(l, "audit-col-client"),
            t!(l, "audit-col-tool"),
            t!(l, "audit-col-workspace"),
            t!(l, "audit-col-target"),
            t!(l, "audit-col-outcome"),
            t!(l, "audit-col-decision"),
            t!(l, "audit-col-duration"),
        ];
        let mut header = div()
            .flex()
            .items_center()
            .gap(metrics::SPACE_4)
            .w_full()
            .h(metrics::LAYOUT_ROW_COMPACT)
            .px(metrics::SPACE_6)
            .text_size(metrics::FONT_SIZE_2XS)
            .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
            .text_color(parla_tokens(cx.theme()).text_subtle)
            .bg(colors.background)
            .border_b_1()
            .border_color(colors.border);
        for (w, text) in Self::cols().into_iter().zip(heads) {
            let cell = div().truncate().child(text.into_owned().to_uppercase());
            header = header.child(if w == 0.0 {
                cell.flex_1().min_w_0()
            } else {
                cell.w(px(w)).flex_none()
            });
        }

        let error = self.error.map(|e| {
            let (msg, retry) = match e {
                AuditError::Load => (t!(l, "audit-load-error"), true),
                AuditError::Clear => (t!(l, "audit-clear-error"), false),
            };
            div()
                .debug_selector(|| "audit-error".to_string())
                .flex()
                .items_center()
                .gap(metrics::SPACE_4)
                .px(metrics::SPACE_5)
                .py(metrics::SPACE_2)
                .bg(parla_tokens(cx.theme()).danger_soft)
                .text_color(cx.theme().status().error)
                .child(div().flex_1().child(msg.into_owned()))
                .when(retry, |d| {
                    d.child(self.pill(
                        "audit-retry",
                        t!(l, "audit-retry").into_owned(),
                        true,
                        cx,
                        |v, cx| v.retry(cx),
                    ))
                })
        });

        let body = if self.rows.is_empty() && self.exhausted {
            div()
                .flex_1()
                .p(metrics::SPACE_6)
                .text_size(metrics::FONT_SIZE_SM)
                .text_color(colors.text_muted)
                .child(t!(l, "audit-empty").into_owned())
                .into_any_element()
        } else {
            uniform_list(
                "audit-list",
                self.rows.len(),
                cx.processor(Self::render_rows),
            )
            .track_scroll(&self.scroll)
            .w_full()
            .flex_1()
            .min_h_0()
            .into_any_element()
        };

        let detail = self.selected().map(|row| {
            let mut d = div()
                .debug_selector(|| "audit-detail".to_string())
                .flex()
                .flex_col()
                .gap(metrics::SPACE_2)
                .p(metrics::SPACE_6)
                .border_t_1()
                .border_color(colors.border)
                .text_size(metrics::FONT_SIZE_SM);
            if let Some(why) = row.detail.as_deref() {
                d = d.child(t!(l, "audit-detail-reason", reason = why.to_string()).into_owned());
            }
            if let Some(pid) = row.peer_pid {
                d = d.child(t!(l, "audit-detail-pid", pid = pid.to_string()).into_owned());
            }
            if let Some(content) = row.content.as_deref() {
                d = d
                    .child(
                        div()
                            .text_color(colors.text_muted)
                            .child(t!(l, "audit-content-label").into_owned()),
                    )
                    .child(
                        div()
                            .debug_selector(|| "audit-content".to_string())
                            .p(metrics::SPACE_4)
                            .rounded(metrics::RADIUS_MD)
                            .bg(colors.background)
                            .border_1()
                            .border_color(colors.border)
                            .font_family("JetBrains Mono")
                            .child(crate::confirm_write::escape_text(content)),
                    );
            }
            d
        });

        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|v, _: &ClearAuditLog, _, cx| v.request_clear(cx)))
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .child(toolbar)
            .children(confirm)
            .children(error)
            .child(header)
            .child(body)
            .children(detail)
    }
}
