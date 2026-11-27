//! The People directory (board DirectMessages, "People" panel): every member
//! of the workspace's member index as a card with avatar, name, title,
//! presence and local time, searchable by name or title and filtered by time
//! zone and bots. A card's Message opens (or starts) the direct message; the
//! person button opens the profile panel.
//!
//! Built for 10k members: the index (sorted, lower-cased search keys, one
//! time zone per distinct zone) is built once and then patched one member at
//! a time when the source can say which members changed; the filtered rows
//! are cached per (query, filters, members revision); the local time lines
//! are formatted once per zone and minute; the grid is a virtualised list of
//! card rows, so a frame only builds the cards on screen.
//!
//! There is no "Active now" filter: presence is only known for the people
//! being watched, so it could not be an honest filter over everyone.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use domain::{Member, UserId, WorkspaceState};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render, SharedString,
    Subscription, UniformListScrollHandle, Window, div, prelude::*, uniform_list,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use themes::ActiveTheme;
use themes::metrics::{
    FONT_SIZE_LG, FONT_SIZE_SM, FONT_SIZE_XS, FONT_WEIGHT_BOLD, FONT_WEIGHT_SEMIBOLD,
    ICON_SIZE_BASE, LAYOUT_AVATAR_CARD, LAYOUT_HEADER, RADIUS_LG, SPACE_1, SPACE_3, SPACE_4,
    SPACE_5, SPACE_6,
};

use crate::controls::{ButtonKind, IconButtonSize};
use crate::icon::{Icon, IconName};
use crate::text_input::{TextInput, TextInputEvent};

/// Key context of the view.
pub const KEY_CONTEXT: &str = "PeopleDirectory";

/// Cards in one row of the grid (the board's three columns).
pub const CARDS_PER_ROW: usize = 3;

/// Changed members patched in place; more than this rebuilds the index.
const MAX_PATCH: usize = 64;

/// The filters the directory offers (each only when the data supports it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PeopleFilters {
    /// Only people whose clock reads the same as mine.
    pub my_time_zone: bool,
    /// Bots are listed too (hidden by default).
    pub bots: bool,
}

/// One filter chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeopleFilter {
    MyTimeZone,
    Bots,
}

/// One person, as the index keeps them.
#[derive(Clone, Debug)]
pub struct PersonEntry {
    pub id: UserId,
    pub name: SharedString,
    pub initials: SharedString,
    pub title: Option<SharedString>,
    pub is_bot: bool,
    /// The person's picture (initials while it loads and when there is none).
    pub avatar: Option<domain::ImageRef>,
    /// Index into the index's zones.
    zone: Option<u16>,
    /// Lower-cased name, the sort key.
    key: Box<str>,
    /// Lower-cased name, real name, handle and title, one per line.
    hay: Box<str>,
}

/// A distinct time zone of the directory, resolved once.
#[derive(Clone, Debug)]
struct Zone {
    tz: Option<TimeZone>,
    /// The city part of the IANA name ("Tokyo", "New York").
    place: SharedString,
}

/// The directory's people, sorted by name, with their search keys. Built
/// once, then patched per changed member; [`PeopleIndex::filter`] is a
/// linear scan with no allocation per person.
#[derive(Clone, Debug, Default)]
pub struct PeopleIndex {
    entries: Vec<PersonEntry>,
    zones: Vec<Zone>,
    zone_ix: HashMap<Arc<str>, u16>,
    bots: usize,
}

impl PeopleIndex {
    /// Everyone in `state.members` except deleted people and me.
    pub fn build(state: &WorkspaceState) -> Self {
        let mut idx = Self::default();
        idx.entries = state
            .members
            .values()
            .filter_map(|m| idx.entry_for(state, m))
            .collect();
        idx.entries
            .sort_by(|a, b| a.key.cmp(&b.key).then_with(|| a.id.cmp(&b.id)));
        idx.bots = idx.entries.iter().filter(|e| e.is_bot).count();
        idx
    }

    /// Re-reads `ids` from `state`: each is removed, then put back in its
    /// place when still listed. Costs a scan and a move per id, no rebuild.
    pub fn update(&mut self, state: &WorkspaceState, ids: &[UserId]) {
        for id in ids {
            if let Some(at) = self.entries.iter().position(|e| &e.id == id) {
                let old = self.entries.remove(at);
                self.bots -= usize::from(old.is_bot);
            }
            let Some(e) = state.members.get(id).and_then(|m| self.entry_for(state, m)) else {
                continue;
            };
            let at = self
                .entries
                .partition_point(|x| (&x.key, &x.id) < (&e.key, &e.id));
            self.bots += usize::from(e.is_bot);
            self.entries.insert(at, e);
        }
    }

    /// The entry for `m`, or `None` when the directory leaves them out.
    fn entry_for(&mut self, state: &WorkspaceState, m: &Member) -> Option<PersonEntry> {
        if m.deleted || state.self_user.as_ref() == Some(&m.id) {
            return None;
        }
        let name = crate::sidebar::best_name(m)?;
        let title = m.title.as_ref().filter(|t| !t.trim().is_empty());
        let zone = m.tz.as_ref().map(|tz| self.zone_of(tz));
        let mut hay =
            String::with_capacity(m.display_name.len() + m.real_name.len() + m.handle.len() + 8);
        for part in [&m.display_name, &m.real_name, &m.handle] {
            hay.push_str(&part.to_lowercase());
            hay.push('\n');
        }
        if let Some(t) = title {
            hay.push_str(&t.to_lowercase());
        }
        Some(PersonEntry {
            id: m.id.clone(),
            initials: crate::badge::initials(&name).into(),
            key: name.to_lowercase().into_boxed_str(),
            name: SharedString::from(name.to_string()),
            title: title.map(|t| SharedString::from(t.to_string())),
            is_bot: m.is_bot,
            avatar: m.avatar_for(LAYOUT_AVATAR_CARD.as_f32()).cloned(),
            zone,
            hay: hay.into_boxed_str(),
        })
    }

    fn zone_of(&mut self, tz: &Arc<str>) -> u16 {
        if let Some(&z) = self.zone_ix.get(tz) {
            return z;
        }
        let place = tz.rsplit('/').next().unwrap_or(tz).replace('_', " ");
        self.zones.push(Zone {
            tz: TimeZone::get(tz).ok(),
            place: place.into(),
        });
        let z = (self.zones.len() - 1) as u16;
        self.zone_ix.insert(tz.clone(), z);
        z
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// People who are not bots (the header's count).
    pub fn humans(&self) -> usize {
        self.entries.len() - self.bots
    }

    pub fn entry(&self, i: usize) -> &PersonEntry {
        &self.entries[i]
    }

    /// Some member is a bot (the Bots chip is offered).
    pub fn has_bots(&self) -> bool {
        self.bots > 0
    }

    /// Some listed member has a known time zone (the time zone chip is offered).
    pub fn has_zones(&self) -> bool {
        self.entries.iter().any(|e| {
            e.zone
                .is_some_and(|z| self.zones[usize::from(z)].tz.is_some())
        })
    }

    /// Positions of the people matching `query` (every word in the name or
    /// title, ignoring case) and `f`, in name order.
    pub fn filter(
        &self,
        query: &str,
        f: &PeopleFilters,
        mine: &TimeZone,
        now: Timestamp,
    ) -> Vec<u32> {
        let query = query.to_lowercase();
        let terms: Vec<&str> = query.split_whitespace().collect();
        let my_offset = mine.to_offset(now).seconds();
        let same_zone: Vec<bool> = if f.my_time_zone {
            self.zones
                .iter()
                .map(|z| {
                    z.tz.as_ref()
                        .is_some_and(|z| z.to_offset(now).seconds() == my_offset)
                })
                .collect()
        } else {
            Vec::new()
        };
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| f.bots || !e.is_bot)
            .filter(|(_, e)| !f.my_time_zone || e.zone.is_some_and(|z| same_zone[usize::from(z)]))
            .filter(|(_, e)| terms.iter().all(|t| e.hay.contains(t)))
            .map(|(i, _)| i as u32)
            .collect()
    }
}

/// What the directory needs from the session, so `components` does not
/// depend on `parla`.
pub trait PeopleSource {
    /// The workspace: members, presence, capabilities, name, self.
    fn people_state(&self) -> &WorkspaceState;
    /// Changes whenever the members change.
    fn members_revision(&self) -> u64;
    /// The members changed after revision `since`, when the source still
    /// knows them one by one; `None` rebuilds the directory.
    fn members_changed_since(&self, _since: u64) -> Option<Vec<UserId>> {
        None
    }
    fn clock(&self, cx: &App) -> settings::Clock;
    /// The people whose cards are on screen, so their presence is watched.
    fn watch_people(&mut self, _people: Vec<UserId>, _cx: &mut App) {}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeopleEvent {
    Close,
    /// Open or start the direct message with this person.
    Message {
        user: UserId,
        name: SharedString,
    },
    /// Show this person's profile.
    ViewProfile(UserId),
}

#[derive(Clone, PartialEq, Eq)]
struct RowsKey {
    query: String,
    filters: PeopleFilters,
    members: u64,
}

/// The view's fixed text, built once per language.
struct Labels {
    message: SharedString,
    profile: SharedString,
    bot: SharedString,
    title: SharedString,
    close: SharedString,
    zone: SharedString,
    bots: SharedString,
    partial: SharedString,
}

impl Labels {
    fn new(cx: &App) -> Self {
        let s = |c: std::borrow::Cow<'_, str>| SharedString::from(c.into_owned());
        Self {
            message: s(t!(cx, "people-message")),
            profile: s(t!(cx, "people-view-profile")),
            bot: s(t!(cx, "people-bot")),
            title: s(t!(cx, "people-title")),
            close: s(t!(cx, "close-label")),
            zone: s(t!(cx, "people-filter-zone")),
            bots: s(t!(cx, "people-filter-bots")),
            partial: s(t!(cx, "people-partial")),
        }
    }
}

/// Local time lines per zone, valid for one minute, clock and language.
#[derive(Default)]
struct TimeLines {
    minute: i64,
    clock: Option<settings::Clock>,
    lines: Vec<Option<SharedString>>,
}

const BLANK: SharedString = SharedString::new_static("\u{a0}");

pub struct PeopleDirectory<S: PeopleSource + 'static> {
    session: Entity<S>,
    focus: FocusHandle,
    grid_focus: FocusHandle,
    input: Entity<TextInput>,
    query: String,
    filters: PeopleFilters,
    index: Option<(u64, Arc<PeopleIndex>)>,
    rows: Option<(RowsKey, Arc<[u32]>)>,
    labels: Labels,
    times: TimeLines,
    /// `(people, workspace)` and the header count built for them.
    count: Option<((usize, Arc<str>), SharedString)>,
    cursor: usize,
    scroll: UniformListScrollHandle,
    /// The people last asked to be watched.
    watched: Vec<UserId>,
    index_builds: usize,
    index_patches: usize,
    filter_runs: usize,
    time_formats: usize,
    _observe: Subscription,
    _locale: Subscription,
    _input_sub: Subscription,
}

impl<S: PeopleSource + 'static> EventEmitter<PeopleEvent> for PeopleDirectory<S> {}

impl<S: PeopleSource + 'static> PeopleDirectory<S> {
    pub fn new(session: Entity<S>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&session, |_, _, cx| cx.notify());
        let locale = cx.observe_global::<i18n::gpui::I18n>(|this, cx| {
            this.relabel(cx);
            cx.notify();
        });
        let placeholder = t!(cx, "people-search").into_owned();
        let input = cx.new(|cx| {
            let mut i = TextInput::new(cx);
            i.set_single_line(true);
            // Down moves to the cards.
            i.set_intercept_down(true);
            i.set_placeholder(placeholder);
            i
        });
        let input_sub = cx.subscribe(&input, |this, input, _: &TextInputEvent, cx| {
            let text = input.read(cx).text().to_string();
            if text != this.query {
                this.query = text;
                this.cursor = 0;
                cx.notify();
            }
        });
        Self {
            session,
            focus: cx.focus_handle(),
            grid_focus: cx.focus_handle().tab_stop(true),
            input,
            query: String::new(),
            filters: PeopleFilters::default(),
            index: None,
            rows: None,
            labels: Labels::new(cx),
            times: TimeLines::default(),
            count: None,
            cursor: 0,
            scroll: UniformListScrollHandle::new(),
            watched: Vec::new(),
            index_builds: 0,
            index_patches: 0,
            filter_runs: 0,
            time_formats: 0,
            _observe: observe,
            _locale: locale,
            _input_sub: input_sub,
        }
    }

    fn relabel(&mut self, cx: &mut Context<Self>) {
        self.labels = Labels::new(cx);
        self.times = TimeLines::default();
        self.count = None;
        let placeholder = t!(cx, "people-search").into_owned();
        self.input.update(cx, |i, _| i.set_placeholder(placeholder));
    }

    /// The search field's focus (the host focuses it on open).
    pub fn search_focus(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }

    /// The card grid's focus (arrow keys move between cards).
    pub fn grid_focus(&self) -> FocusHandle {
        self.grid_focus.clone()
    }

    /// The card the keyboard is on.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn filters(&self) -> PeopleFilters {
        self.filters
    }

    /// How often the index was built (a test hook).
    pub fn index_builds(&self) -> usize {
        self.index_builds
    }

    /// How often the index was patched for single members (a test hook).
    pub fn index_patches(&self) -> usize {
        self.index_patches
    }

    /// How often the rows were filtered (a test hook: once per key).
    pub fn filter_runs(&self) -> usize {
        self.filter_runs
    }

    /// How many local time lines were formatted (a test hook: redraws add none).
    pub fn time_formats(&self) -> usize {
        self.time_formats
    }

    /// Scrolls card row `row` to the top (a test hook).
    pub fn scroll_to_row(&mut self, row: usize) {
        self.scroll.scroll_to_item(row, gpui::ScrollStrategy::Top);
    }

    /// Asks again for the cards on screen to be watched (the directory is
    /// back on screen after the profile it opened closed).
    pub fn rewatch(&mut self, cx: &mut Context<Self>) {
        self.watched.clear();
        cx.notify();
    }

    pub fn set_query(&mut self, q: &str, cx: &mut Context<Self>) {
        self.input.update(cx, |i, cx| i.set_text(q, cx));
        self.query = q.to_string();
        self.cursor = 0;
        cx.notify();
    }

    /// Turns a filter on or off; one the data does not support stays off.
    pub fn toggle_filter(&mut self, f: PeopleFilter, cx: &mut Context<Self>) {
        let (index, _) = self.refresh(cx);
        match f {
            PeopleFilter::MyTimeZone if index.has_zones() => {
                self.filters.my_time_zone = !self.filters.my_time_zone
            }
            PeopleFilter::Bots if index.has_bots() => self.filters.bots = !self.filters.bots,
            _ => return,
        }
        self.cursor = 0;
        cx.notify();
    }

    /// The people listed, in order.
    pub fn results(&mut self, cx: &App) -> Vec<UserId> {
        let (index, rows) = self.refresh(cx);
        rows.iter()
            .map(|&i| index.entry(i as usize).id.clone())
            .collect()
    }

    /// The header's count: "42 in Northwind Studio".
    pub fn header_count(&mut self, cx: &App) -> SharedString {
        let (index, _) = self.refresh(cx);
        let name = self.session.read(cx).people_state().name.clone();
        let key = (index.humans(), name);
        if let Some((k, text)) = &self.count
            && *k == key
        {
            return text.clone();
        }
        let l = i18n::Translate::localizer(cx);
        let text: SharedString = if key.1.is_empty() {
            t!(l, "people-count-plain", count = key.0).into_owned()
        } else {
            t!(l, "people-count", count = key.0, workspace = &*key.1).into_owned()
        }
        .into();
        self.count = Some((key, text.clone()));
        text
    }

    /// Card `n`'s name, title and local time line, as drawn.
    pub fn card_text(&mut self, n: usize, cx: &App) -> Option<(String, String, String)> {
        let (index, rows) = self.refresh(cx);
        let e = index.entry(*rows.get(n)? as usize);
        Some((
            e.name.to_string(),
            e.title.as_ref().map(|t| t.to_string()).unwrap_or_default(),
            self.local_time(&index, e, cx)
                .map(|t| t.to_string())
                .unwrap_or_default(),
        ))
    }

    /// The index and rows for the current key, rebuilt only when it changed.
    fn refresh(&mut self, cx: &App) -> (Arc<PeopleIndex>, Arc<[u32]>) {
        let src = self.session.read(cx);
        let members = src.members_revision();
        let state = src.people_state();
        let index = match &mut self.index {
            Some((rev, idx)) if *rev == members => idx.clone(),
            Some((rev, idx))
                if let Some(ids) = src
                    .members_changed_since(*rev)
                    .filter(|ids| ids.len() <= MAX_PATCH) =>
            {
                Arc::make_mut(idx).update(state, &ids);
                *rev = members;
                self.index_patches += 1;
                idx.clone()
            }
            _ => {
                let idx = Arc::new(PeopleIndex::build(state));
                self.index_builds += 1;
                self.index = Some((members, idx.clone()));
                idx
            }
        };
        // A filter the data no longer supports goes off.
        self.filters.my_time_zone &= index.has_zones();
        self.filters.bots &= index.has_bots();
        let key = RowsKey {
            query: self.query.trim().to_string(),
            filters: self.filters,
            members,
        };
        let rows = match &self.rows {
            Some((k, rows)) if *k == key => rows.clone(),
            _ => {
                let rows: Arc<[u32]> = index
                    .filter(
                        &key.query,
                        &key.filters,
                        &crate::time_zone(cx),
                        crate::now(cx),
                    )
                    .into();
                self.filter_runs += 1;
                self.rows = Some((key, rows.clone()));
                rows
            }
        };
        if self.cursor >= rows.len() {
            self.cursor = rows.len().saturating_sub(1);
        }
        (index, rows)
    }

    /// "18:42 · Tokyo" for a person with a known zone, formatted once per
    /// zone and minute.
    fn local_time(
        &mut self,
        index: &PeopleIndex,
        e: &PersonEntry,
        cx: &App,
    ) -> Option<SharedString> {
        let z = usize::from(e.zone?);
        let zone = &index.zones[z];
        let tz = zone.tz.as_ref()?;
        let now = crate::now(cx);
        let clock = self.session.read(cx).clock(cx);
        let minute = now.as_second().div_euclid(60);
        if self.times.minute != minute || self.times.clock != Some(clock) {
            self.times = TimeLines {
                minute,
                clock: Some(clock),
                lines: Vec::new(),
            };
        }
        if self.times.lines.len() < index.zones.len() {
            self.times.lines.resize(index.zones.len(), None);
        }
        if let Some(line) = &self.times.lines[z] {
            return Some(line.clone());
        }
        let l = i18n::Translate::localizer(cx);
        let time = l.time(now, tz, crate::message_row::hour_cycle(clock));
        let line: SharedString = t!(
            l,
            "people-local-time",
            time = time,
            place = zone.place.as_ref()
        )
        .into_owned()
        .into();
        self.time_formats += 1;
        self.times.lines[z] = Some(line.clone());
        Some(line)
    }

    fn message(&mut self, n: usize, cx: &mut Context<Self>) {
        let (index, rows) = self.refresh(cx);
        if let Some(&i) = rows.get(n) {
            let e = index.entry(i as usize);
            cx.emit(PeopleEvent::Message {
                user: e.id.clone(),
                name: e.name.clone(),
            });
        }
    }

    fn view_profile(&mut self, n: usize, cx: &mut Context<Self>) {
        let (index, rows) = self.refresh(cx);
        if let Some(&i) = rows.get(n) {
            cx.emit(PeopleEvent::ViewProfile(index.entry(i as usize).id.clone()));
        }
    }

    fn reveal(&mut self, n: usize) {
        self.scroll
            .scroll_to_item(n / CARDS_PER_ROW, gpui::ScrollStrategy::Nearest);
    }

    fn on_grid_key(&mut self, ev: &gpui::KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        if k.modifiers.platform || k.modifiers.control || k.modifiers.alt {
            return;
        }
        let (_, rows) = self.refresh(cx);
        if rows.is_empty() {
            return;
        }
        let last = rows.len() - 1;
        let at = self.cursor;
        let next = match k.key.as_str() {
            "left" => at.saturating_sub(1),
            "right" => (at + 1).min(last),
            "up" => at.saturating_sub(CARDS_PER_ROW),
            "down" => (at + CARDS_PER_ROW).min(last),
            "home" => 0,
            "end" => last,
            "enter" | "space" => {
                cx.stop_propagation();
                if !k.modifiers.shift && !ev.is_held {
                    // The card acted on is the one in view.
                    self.reveal(at);
                    cx.notify();
                    self.message(at, cx);
                }
                return;
            }
            _ => return,
        };
        cx.stop_propagation();
        self.reveal(next);
        if next != at {
            self.cursor = next;
        }
        cx.notify();
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let (index, rows) = self.refresh(cx);
        let theme = cx.theme().clone();
        let colors = *theme.colors();
        let tokens = themes::parla_tokens(&theme);
        let presence_on = self.session.read(cx).people_state().caps.presence_watch;
        let ring_at = self.grid_focus.is_focused(window).then_some(self.cursor);
        let mut on_screen = Vec::with_capacity(range.len() * CARDS_PER_ROW);
        let mut out = Vec::with_capacity(range.len());
        for r in range {
            let mut row = div().flex().w_full().gap(SPACE_4).pb(SPACE_4);
            for n in r * CARDS_PER_ROW..((r + 1) * CARDS_PER_ROW).min(rows.len()) {
                let e = index.entry(rows[n] as usize);
                on_screen.push(e.id.clone());
                let presence = presence_on
                    .then(|| {
                        self.session
                            .read(cx)
                            .people_state()
                            .presence
                            .get(&e.id)
                            .copied()
                    })
                    .flatten();
                let time = self.local_time(&index, e, cx);
                let sub = |text: Option<SharedString>| {
                    div()
                        .w_full()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_center()
                        .text_size(FONT_SIZE_XS)
                        .text_color(tokens.text_subtle)
                        // A blank line keeps every card the same height.
                        .child(text.unwrap_or(BLANK))
                };
                let title = e
                    .title
                    .clone()
                    .or_else(|| e.is_bot.then(|| self.labels.bot.clone()));
                let this = cx.entity().downgrade();
                let card = div()
                    .id(("people-card", n))
                    .debug_selector(move || format!("people-card-{n}"))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(SPACE_3)
                    .px(SPACE_5)
                    .py(SPACE_6)
                    .rounded(RADIUS_LG)
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.surface_background)
                    .when(ring_at == Some(n), |d| {
                        d.border_color(gpui::transparent_black())
                            .shadow(crate::controls::ring(&colors))
                    })
                    .child(crate::message_row::avatar_with_presence(
                        &e.name,
                        e.initials.clone(),
                        &theme,
                        LAYOUT_AVATAR_CARD,
                        presence,
                        ("people-dot", colors.surface_background),
                        e.avatar.as_ref(),
                    ))
                    .child(
                        div()
                            .w_full()
                            .mt(SPACE_1)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_center()
                            .text_size(FONT_SIZE_SM)
                            .font_weight(FONT_WEIGHT_BOLD)
                            .text_color(colors.text)
                            .child(e.name.clone()),
                    )
                    .child(sub(title))
                    .child(sub(time))
                    .child(
                        div()
                            .flex()
                            .w_full()
                            .items_center()
                            .gap(SPACE_3)
                            .mt(SPACE_1)
                            .child(crate::controls::on_activate(
                                crate::controls::button_small(
                                    ("people-message", n),
                                    self.labels.message.clone(),
                                    ButtonKind::Secondary,
                                    &colors,
                                )
                                .debug_selector(move || format!("people-message-{n}"))
                                .flex_1()
                                .min_w_0(),
                                cx,
                                move |this, _, cx| this.message(n, cx),
                            ))
                            .child(
                                crate::controls::icon_button_sized(
                                    ("people-profile", n),
                                    IconName::User,
                                    self.labels.profile.clone(),
                                    IconButtonSize::Large,
                                    &colors,
                                    move |_, cx| {
                                        this.update(cx, |v, cx| v.view_profile(n, cx)).ok();
                                    },
                                )
                                .debug_selector(move || format!("people-profile-{n}")),
                            ),
                    );
                row = row.child(card);
            }
            // A short last row keeps the cards their width.
            let filled = rows
                .len()
                .saturating_sub(r * CARDS_PER_ROW)
                .min(CARDS_PER_ROW);
            for _ in filled..CARDS_PER_ROW {
                row = row.child(div().flex_1().min_w_0());
            }
            out.push(row.into_any_element());
        }
        self.watch(on_screen, cx);
        out
    }

    /// Tells the source who is on screen when that changed.
    fn watch(&mut self, people: Vec<UserId>, cx: &mut Context<Self>) {
        if people != self.watched {
            self.watched = people.clone();
            let session = self.session.clone();
            cx.defer(move |cx| session.update(cx, |s, cx| s.watch_people(people, cx)));
        }
    }

    fn chip(
        &self,
        id: &'static str,
        label: SharedString,
        on: bool,
        filter: PeopleFilter,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = *theme.colors();
        let accent = themes::parla_tokens(&theme).accent_soft_text;
        crate::controls::on_activate(
            crate::settings_controls::chip(id, label, on, &colors, accent),
            cx,
            move |this, _, cx| this.toggle_filter(filter, cx),
        )
    }
}

impl<S: PeopleSource + 'static> Focusable for PeopleDirectory<S> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<S: PeopleSource + 'static> Render for PeopleDirectory<S> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (index, rows) = self.refresh(cx);
        let count = self.header_count(cx);
        let theme = cx.theme().clone();
        let colors = *theme.colors();
        let tokens = themes::parla_tokens(&theme);
        let partial = self.session.read(cx).people_state().caps.members_partial;
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let mut chips = div().flex().flex_wrap().gap(SPACE_3);
        if index.has_zones() {
            chips = chips.child(self.chip(
                "people-filter-zone",
                self.labels.zone.clone(),
                self.filters.my_time_zone,
                PeopleFilter::MyTimeZone,
                cx,
            ));
        }
        if index.has_bots() {
            chips = chips.child(self.chip(
                "people-filter-bots",
                self.labels.bots.clone(),
                self.filters.bots,
                PeopleFilter::Bots,
                cx,
            ));
        }
        let has_rows = !rows.is_empty();
        let body = if !has_rows {
            // Nothing on screen: nobody is watched for the directory.
            self.watch(Vec::new(), cx);
            let (title, hint) = if index.is_empty() {
                (t!(cx, "people-none-title"), t!(cx, "people-none"))
            } else {
                (t!(cx, "people-empty-title"), t!(cx, "people-empty"))
            };
            div().flex_1().min_h_0().child(
                crate::controls::empty_state(title.into_owned(), hint.into_owned(), &colors)
                    .id("people-empty")
                    .debug_selector(|| "people-empty".to_string()),
            )
        } else {
            let row_count = rows.len().div_ceil(CARDS_PER_ROW);
            div().flex_1().min_h_0().flex().child(
                div()
                    .id("people-grid")
                    .debug_selector(|| "people-grid".to_string())
                    .track_focus(&self.grid_focus)
                    .on_key_down(cx.listener(Self::on_grid_key))
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        uniform_list("people-list", row_count, cx.processor(Self::render_rows))
                            .track_scroll(&self.scroll)
                            .w_full()
                            .px(SPACE_6)
                            .flex_1()
                            .min_h_0(),
                    ),
            )
        };
        let grid = self.grid_focus.clone();
        div()
            .id("people-directory")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(move |_, ev: &gpui::KeyDownEvent, window, cx| {
                match ev.keystroke.key.as_str() {
                    "escape" => {
                        cx.stop_propagation();
                        cx.emit(PeopleEvent::Close);
                    }
                    // Down from the search field moves to the cards, when any.
                    "down" if has_rows && !grid.is_focused(window) => {
                        cx.stop_propagation();
                        window.focus(&grid, cx);
                        cx.notify();
                    }
                    _ => {}
                }
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(SPACE_4)
                    .h(LAYOUT_HEADER)
                    .px(SPACE_6)
                    .border_b_1()
                    .border_color(tokens.border_subtle)
                    .bg(colors.surface_background)
                    .child(
                        div()
                            .flex_1()
                            .text_size(FONT_SIZE_LG)
                            .font_weight(FONT_WEIGHT_SEMIBOLD)
                            .child(self.labels.title.clone()),
                    )
                    .child(
                        div()
                            .debug_selector(|| "people-count".to_string())
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(FONT_SIZE_XS)
                            .text_color(tokens.text_subtle)
                            .child(count),
                    )
                    .child(crate::controls::icon_button(
                        "people-close",
                        IconName::X,
                        self.labels.close.clone(),
                        &colors,
                        {
                            let this = cx.entity();
                            move |_, cx| this.update(cx, |_, cx| cx.emit(PeopleEvent::Close))
                        },
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap(SPACE_4)
                    .px(SPACE_6)
                    .py(SPACE_5)
                    .child(
                        crate::controls::field_frame(focused, &colors, tokens.border_control)
                            .id("people-search")
                            .debug_selector(|| "people-search".to_string())
                            .gap(SPACE_4)
                            .child(
                                Icon::new(IconName::Search)
                                    .size(ICON_SIZE_BASE)
                                    .color(tokens.text_subtle),
                            )
                            .child(div().flex_1().min_w_0().child(self.input.clone())),
                    )
                    .child(chips),
            )
            .child(body)
            .when(partial, |d| {
                d.child(
                    div()
                        .debug_selector(|| "people-partial".to_string())
                        .flex_none()
                        .px(SPACE_6)
                        .py(SPACE_4)
                        .text_size(FONT_SIZE_XS)
                        .text_color(tokens.text_subtle)
                        .child(self.labels.partial.clone()),
                )
            })
    }
}
