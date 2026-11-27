//! Shapes, paints and hit-tests the text of a [`TextInput`]: selection, caret, underlined
//! marked text, placeholder, and the mouse handlers.

use std::ops::Range;

use gpui::{
    App, Bounds, ContentMask, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, Size, Style,
    TextAlign, TextRun, UnderlineStyle, Window, WrappedLine, fill, point, px, relative, size,
};
use themes::ActiveTheme;
use unicode_segmentation::UnicodeSegmentation;

use super::TextInput;

/// The shaped hard lines and their shared geometry. Byte offsets stay relative to `text`.
struct Shaped {
    text: SharedString,
    lines: Vec<WrappedLine>,
    line_height: Pixels,
}

struct Row<'a> {
    line: &'a WrappedLine,
    line_start: usize,
    row: usize,
    start: usize,
    end: usize,
    index: usize,
}

impl Shaped {
    fn shape(
        window: &Window,
        text: SharedString,
        font_size: Pixels,
        runs: &[TextRun],
        wrap_width: Option<Pixels>,
        line_height: Pixels,
    ) -> gpui::Result<Self> {
        let lines =
            window
                .text_system()
                .shape_text(text.clone(), font_size, runs, wrap_width, None)?;
        Ok(Self {
            text,
            lines: lines.into_iter().collect(),
            line_height,
        })
    }

    fn size(&self) -> Size<Pixels> {
        let mut width = px(0.0);
        let mut height = px(0.0);
        for line in &self.lines {
            let size = line.size(self.line_height);
            width = width.max(size.width);
            height += size.height;
        }
        size(width, height)
    }

    fn line_count(&self) -> usize {
        self.lines
            .iter()
            .map(|line| line.wrap_boundaries().len() + 1)
            .sum()
    }

    fn wrap_byte(line: &WrappedLine, wrap: usize) -> usize {
        let boundary = line.wrap_boundaries()[wrap];
        line.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index
    }

    fn row(line: &WrappedLine, line_start: usize, row: usize, index: usize) -> Row<'_> {
        let row_start = if row == 0 {
            0
        } else {
            Self::wrap_byte(line, row - 1)
        };
        let row_end = if row < line.wrap_boundaries().len() {
            Self::wrap_byte(line, row)
        } else {
            line.len()
        };
        Row {
            line,
            line_start,
            row,
            start: line_start + row_start,
            end: line_start + row_end,
            index,
        }
    }

    fn row_at(&self, target: usize) -> Option<Row<'_>> {
        let mut line_start = 0;
        let mut first_row = 0;
        for line in &self.lines {
            let rows = line.wrap_boundaries().len() + 1;
            if target < first_row + rows {
                return Some(Self::row(line, line_start, target - first_row, target));
            }
            first_row += rows;
            line_start += line.len() + 1;
        }
        None
    }

    /// The row holding `index`. A byte at a soft-wrap boundary belongs to the
    /// row after it, unless the caret has `upstream` affinity (End on the
    /// row before), which keeps it at that row's end.
    fn row_for_index(&self, index: usize, upstream: bool) -> Option<Row<'_>> {
        let mut line_start = 0;
        let mut first_row = 0;
        for line in &self.lines {
            let line_end = line_start + line.len();
            if index <= line_end {
                let within = index.saturating_sub(line_start);
                let row = line
                    .wrap_boundaries()
                    .iter()
                    .enumerate()
                    .take_while(|(wrap, _)| {
                        let at = Self::wrap_byte(line, *wrap);
                        at < within || (at == within && !upstream)
                    })
                    .count();
                return Some(Self::row(line, line_start, row, first_row + row));
            }
            first_row += line.wrap_boundaries().len() + 1;
            line_start = line_end + 1;
        }
        None
    }

    fn closest_byte_index_for_pixel_point(&self, p: Point<Pixels>) -> Result<usize, usize> {
        if p.y < px(0.0) {
            return Err(0);
        }
        let mut line_y = px(0.0);
        let mut line_start = 0;
        for line in &self.lines {
            let line_height = line.size(self.line_height).height;
            if p.y < line_y + line_height {
                let within = point(p.x, p.y - line_y);
                return match line.closest_index_for_position(within, self.line_height) {
                    Ok(i) => Ok(line_start + i),
                    Err(i) => Err(line_start + i),
                };
            }
            line_y += line_height;
            line_start += line.len() + 1;
        }
        Err(self.text.len())
    }

    fn selection_from_pixel_point(&self, p: Point<Pixels>) -> Range<usize> {
        let index = match self.closest_byte_index_for_pixel_point(p) {
            Ok(i) | Err(i) => i,
        };
        self.text
            .split_word_bound_indices()
            .find(|(start, word)| {
                let end = *start + word.len();
                index >= *start && index < end || index == self.text.len() && end == self.text.len()
            })
            .map_or(index..index, |(start, word)| start..start + word.len())
    }

    fn visual_position_for_byte_index(
        &self,
        index: usize,
        upstream: bool,
    ) -> Option<Point<Pixels>> {
        if index > self.text.len() {
            return None;
        }
        let row = self.row_for_index(index, upstream)?;
        let within = index - row.line_start;
        let row_start = row.start - row.line_start;
        let x = row.line.unwrapped_layout.x_for_index(within)
            - row.line.unwrapped_layout.x_for_index(row_start);
        Some(point(x, self.row_y(row.index)))
    }

    fn row_y(&self, row: usize) -> Pixels {
        self.line_height * row as f32
    }

    /// The start or end of the row holding `at`, and whether the caret there
    /// has upstream affinity: the end of a row that soft-wraps into the next.
    fn row_edge(&self, at: usize, end: bool, upstream: bool) -> (usize, bool) {
        self.row_for_index(at, upstream).map_or((at, false), |row| {
            if end {
                (row.end, row.row < row.line.wrap_boundaries().len())
            } else {
                (row.start, false)
            }
        })
    }

    fn vertical(
        &self,
        at: usize,
        down: bool,
        x: Option<Pixels>,
        upstream: bool,
    ) -> (usize, Option<Pixels>, bool) {
        let Some(current) = self.row_for_index(at, upstream) else {
            return (at, x, upstream);
        };
        let target = if down {
            current.index.checked_add(1)
        } else {
            current.index.checked_sub(1)
        };
        let Some(target) = target.and_then(|index| self.row_at(index)) else {
            return (at, x, upstream);
        };
        let x = x.or_else(|| {
            self.visual_position_for_byte_index(at, upstream)
                .map(|p| p.x)
        });
        let Some(x) = x else {
            return (at, None, upstream);
        };
        let within = point(x, self.line_height * (target.row as f32 + 0.5));
        let index = match target
            .line
            .closest_index_for_position(within, self.line_height)
        {
            Ok(i) | Err(i) => target.line_start + i,
        };
        // Past the right edge of a row that wraps on, the caret stays on it.
        let upstream = index == target.end && target.row < target.line.wrap_boundaries().len();
        (index, Some(x), upstream)
    }

    fn selection_bounds(&self, range: Range<usize>) -> Vec<Bounds<Pixels>> {
        if range.is_empty() {
            return Vec::new();
        }
        let mut result = Vec::new();
        let mut line_start = 0;
        let mut first_row = 0;
        for line in &self.lines {
            let mut row_start = 0;
            for row in 0..=line.wrap_boundaries().len() {
                let row_end = if row < line.wrap_boundaries().len() {
                    Self::wrap_byte(line, row)
                } else {
                    line.len()
                };
                let start = range.start.max(line_start + row_start);
                let end = range.end.min(line_start + row_end);
                if start < end {
                    let base_x = line.unwrapped_layout.x_for_index(row_start);
                    let x0 = line.unwrapped_layout.x_for_index(start - line_start) - base_x;
                    let x1 = line.unwrapped_layout.x_for_index(end - line_start) - base_x;
                    let (left, right) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
                    result.push(Bounds::new(
                        point(left, self.row_y(first_row)),
                        size(right - left, self.line_height),
                    ));
                }
                first_row += 1;
                row_start = row_end;
            }
            line_start += line.len() + 1;
        }
        result
    }

    fn paint(
        &self,
        origin: Point<Pixels>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> gpui::Result<()> {
        let mut origin = origin;
        for line in &self.lines {
            line.paint(
                origin,
                self.line_height,
                TextAlign::Left,
                Some(bounds),
                window,
                cx,
            )?;
            origin.y += line.size(self.line_height).height;
        }
        Ok(())
    }
}

/// What the last paint laid out, kept for hit testing and caret moves.
pub struct Laid {
    shaped: Shaped,
    /// The real text when `shaped` shows bullets for it (a masked input).
    pub real: Option<SharedString>,
    /// Where the text's top-left sits in the window (scroll applied).
    pub origin: Point<Pixels>,
    pub line_height: Pixels,
}

/// What a masked input shows for each character.
const BULLET: char = '\u{2022}';

/// The text a masked input paints for `real`: one bullet per character.
fn mask_text(real: &str) -> String {
    real.chars().map(|_| BULLET).collect()
}

impl Laid {
    /// Whether this layout was made for `text`.
    pub fn matches(&self, text: &str) -> bool {
        match &self.real {
            Some(r) => r.as_ref() == text,
            None => self.shaped.text.as_ref() == text,
        }
    }

    /// Real byte offset to the shaped text's offset.
    fn disp(&self, i: usize) -> usize {
        match &self.real {
            Some(r) => {
                let mut i = i.min(r.len());
                while !r.is_char_boundary(i) {
                    i -= 1;
                }
                r[..i].chars().count() * BULLET.len_utf8()
            }
            None => i,
        }
    }

    /// The shaped text's offset to a real byte offset.
    fn real_at(&self, d: usize) -> usize {
        match &self.real {
            Some(r) => r
                .char_indices()
                .nth(d / BULLET.len_utf8())
                .map_or(r.len(), |(b, _)| b),
            None => d,
        }
    }

    fn local(&self, p: Point<Pixels>) -> Point<Pixels> {
        p - self.origin
    }

    pub fn index_at(&self, p: Point<Pixels>) -> usize {
        match self
            .shaped
            .closest_byte_index_for_pixel_point(self.local(p))
        {
            Ok(i) | Err(i) => self.real_at(i),
        }
    }

    pub fn word_at(&self, p: Point<Pixels>) -> Range<usize> {
        if let Some(r) = &self.real {
            // A masked value gives no word boundaries away.
            return 0..r.len();
        }
        self.shaped.selection_from_pixel_point(self.local(p))
    }

    /// Visual rows in the layout.
    pub fn rows(&self) -> usize {
        self.shaped.line_count()
    }

    /// Start or end of the visual row holding `at`.
    pub fn row_edge(&self, at: usize, end: bool, upstream: bool) -> (usize, bool) {
        let (index, upstream) = self.shaped.row_edge(self.disp(at), end, upstream);
        (self.real_at(index), upstream)
    }

    /// The visual row holding the caret at `at`.
    pub fn row_of(&self, at: usize, upstream: bool) -> Option<usize> {
        self.shaped
            .row_for_index(self.disp(at), upstream)
            .map(|row| row.index)
    }

    /// One visual row up or down, keeping the horizontal target.
    pub fn vertical(
        &self,
        at: usize,
        down: bool,
        x: Option<Pixels>,
        upstream: bool,
    ) -> (usize, Option<Pixels>, bool) {
        let (index, x, upstream) = self.shaped.vertical(self.disp(at), down, x, upstream);
        (self.real_at(index), x, upstream)
    }

    /// Window bounds of the text in `r` (a caret-wide box for an empty range).
    pub fn bounds_of(&self, r: Range<usize>) -> Option<Bounds<Pixels>> {
        if r.is_empty() {
            return self.caret(r.start, false);
        }
        let rects = self
            .shaped
            .selection_bounds(self.disp(r.start)..self.disp(r.end));
        let (first, rest) = rects.split_first().map(|(f, r)| (*f, r))?;
        let mut b = first;
        for q in rest {
            b = b.union(q);
        }
        Some(Bounds::new(self.origin + b.origin, b.size))
    }

    fn caret(&self, at: usize, upstream: bool) -> Option<Bounds<Pixels>> {
        let p = self
            .shaped
            .visual_position_for_byte_index(self.disp(at), upstream)?;
        Some(Bounds::new(
            self.origin + p,
            size(themes::metrics::LAYOUT_CARET, self.line_height),
        ))
    }
}

pub struct TextElement {
    input: Entity<TextInput>,
    max_rows: f32,
}

impl TextElement {
    pub fn new(input: Entity<TextInput>, max_rows: f32) -> Self {
        Self { input, max_rows }
    }

    fn runs(
        window: &Window,
        len: usize,
        marked: Option<Range<usize>>,
        color: gpui::Hsla,
    ) -> Vec<TextRun> {
        let font = window.text_style().font();
        let plain = |len| TextRun {
            len,
            font: font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        match marked {
            Some(m) if m.end <= len && !m.is_empty() => {
                let mut v = Vec::new();
                if m.start > 0 {
                    v.push(plain(m.start));
                }
                v.push(TextRun {
                    underline: Some(UnderlineStyle {
                        thickness: px(1.0),
                        color: Some(color),
                        wavy: false,
                    }),
                    ..plain(m.end - m.start)
                });
                if m.end < len {
                    v.push(plain(len - m.end));
                }
                v
            }
            _ => vec![plain(len)],
        }
    }
}

pub struct Prepaint {
    laid: Option<Laid>,
    scroll_y: Pixels,
    hitbox: Hitbox,
    selection: Vec<Bounds<Pixels>>,
    caret: Option<Bounds<Pixels>>,
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let (text, placeholder) = {
            let i = self.input.read(cx);
            let text = if i.masked {
                mask_text(i.text())
            } else {
                i.text().to_string()
            };
            (SharedString::from(text), i.placeholder.clone())
        };
        let shown = if text.is_empty() { placeholder } else { text };
        let font_size = window.text_style().font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let max_h = line_height * self.max_rows;
        let color = window.text_style().color;
        let runs = Self::runs(window, shown.len(), None, color);
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        let id = window.request_measured_layout(style, move |known, available, window, _| {
            let width = known.width.or(match available.width {
                gpui::AvailableSpace::Definite(w) => Some(w),
                _ => None,
            });
            let h = if shown.is_empty() {
                line_height
            } else {
                Shaped::shape(window, shown.clone(), font_size, &runs, width, line_height)
                    .map(|s| s.size().height.max(line_height))
                    .unwrap_or(line_height)
            };
            Size {
                width: width.unwrap_or_default(),
                height: h.min(max_h),
            }
        });
        (id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let colors = cx.theme().colors();
        let (text, real, placeholder, placeholder_color, marked, sel, scroll, upstream) = {
            let i = self.input.read(cx);
            let real = i.masked.then(|| SharedString::from(i.text().to_string()));
            let text = match &real {
                Some(r) => SharedString::from(mask_text(r)),
                None => SharedString::from(i.text().to_string()),
            };
            (
                text,
                real,
                i.placeholder.clone(),
                i.placeholder_color,
                i.buf.marked(),
                i.buf.selection(),
                i.scroll_y,
                i.caret_upstream(),
            )
        };
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let empty = text.is_empty();
        let (shown, color) = if empty {
            (placeholder, placeholder_color.unwrap_or(colors.text_muted))
        } else {
            (text, style.color)
        };
        let to_disp = |i: usize| match &real {
            Some(r) => {
                let mut i = i.min(r.len());
                while !r.is_char_boundary(i) {
                    i -= 1;
                }
                r[..i].chars().count() * BULLET.len_utf8()
            }
            None => i,
        };
        let marked = marked.map(|m| to_disp(m.start)..to_disp(m.end));
        let runs = Self::runs(window, shown.len(), marked, color);
        // A shaping failure (for example a font load error) paints nothing
        // rather than panicking the app.
        let Ok(shaped) = Shaped::shape(
            window,
            shown,
            font_size,
            &runs,
            Some(bounds.size.width),
            line_height,
        ) else {
            return Prepaint {
                laid: None,
                scroll_y: scroll,
                hitbox: window.insert_hitbox(bounds, HitboxBehavior::Normal),
                selection: Vec::new(),
                caret: None,
            };
        };
        // Keep the caret in view when the text is taller than the box.
        let content_h = shaped.size().height;
        let mut scroll_y = scroll.min((content_h - bounds.size.height).max(px(0.0)));
        let mut laid = Laid {
            shaped,
            real: if empty { None } else { real },
            origin: bounds.origin,
            line_height,
        };
        let caret_row = if empty {
            None
        } else {
            laid.shaped
                .visual_position_for_byte_index(laid.disp(sel.head), upstream)
        };
        if let Some(p) = caret_row {
            if p.y < scroll_y {
                scroll_y = p.y;
            } else if p.y + line_height > scroll_y + bounds.size.height {
                scroll_y = p.y + line_height - bounds.size.height;
            }
        }
        scroll_y = scroll_y.max(px(0.0));
        laid.origin = bounds.origin - point(px(0.0), scroll_y);
        let (selection, caret) = if empty {
            (
                Vec::new(),
                Some(Bounds::new(
                    bounds.origin,
                    size(themes::metrics::LAYOUT_CARET, line_height),
                )),
            )
        } else {
            let sel_rects = laid
                .shaped
                .selection_bounds(laid.disp(sel.range().start)..laid.disp(sel.range().end))
                .into_iter()
                .map(|r| Bounds::new(laid.origin + r.origin, r.size))
                .collect();
            (
                sel_rects,
                sel.is_empty()
                    .then(|| laid.caret(sel.head, upstream))
                    .flatten(),
            )
        };
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        Prepaint {
            laid: Some(laid),
            scroll_y,
            hitbox,
            selection,
            caret,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        pre: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus.clone();
        let focused = focus.is_focused(window);
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        window.set_cursor_style(CursorStyle::IBeam, &pre.hitbox);
        let colors = *cx.theme().colors();
        let text_color = window.text_style().color;
        let Some(laid) = pre.laid.take() else {
            return;
        };
        let origin = laid.origin;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if focused {
                for r in &pre.selection {
                    window.paint_quad(fill(*r, colors.text_accent.opacity(0.3)));
                }
            }
            let _ = laid.shaped.paint(origin, bounds, window, cx);
            if focused && let Some(c) = pre.caret {
                window.paint_quad(fill(c, text_color));
            }
        });
        let scroll_y = pre.scroll_y;
        self.input.update(cx, |i, _| i.set_laid(laid, scroll_y));

        let hit = pre.hitbox.clone();
        let input = self.input.clone();
        window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
            if phase.bubble() && e.button == MouseButton::Left && hit.is_hovered(window) {
                window.focus(&input.read(cx).focus.clone(), cx);
                input.update(cx, |i, cx| {
                    i.mouse_down(e.position, e.click_count, e.modifiers.shift, cx)
                });
                window.refresh();
            }
        });
        let input = self.input.clone();
        window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
            if phase.bubble()
                && e.pressed_button == Some(MouseButton::Left)
                && input.read(cx).selecting
            {
                input.update(cx, |i, cx| i.mouse_drag(e.position, cx));
                window.refresh();
            }
        });
        let input = self.input.clone();
        window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
            if phase.bubble() && e.button == MouseButton::Left {
                input.update(cx, |i, _| i.mouse_up());
            }
        });
    }
}
