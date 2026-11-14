//! The single-line text field, drawn: the new-task box's line and the picker's filter. The text,
//! cursor and selection are the core's (`ottospace_core::edit`); this draws them, turns keys, the
//! mouse and the input method into `Edit`s, and owns the clipboard.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, ContentMask, Context, CursorStyle, DispatchPhase, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, GlobalElementId, InspectorElementId, IntoElement, KeyDownEvent, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ShapedLine, SharedString, Style, TextRun,
    UTF16Selection, UnderlineStyle, Window, div, fill, point, prelude::*, px, relative, rgb, size,
};
use ottospace_core::clip::Verb;
use ottospace_core::edit::{self, Edit, Key};
use ottospace_core::{Action, theme};

use crate::Workspace;

/// The caret shows for this long, then hides for as long.
const BLINK: Duration = Duration::from_millis(530);

/// What the last paint drew, for the mouse and the input method to measure against.
#[derive(Default)]
pub struct Drawn {
    line: Option<ShapedLine>,
    bounds: Bounds<Pixels>,
    /// How far the text is shifted left, so the cursor stays in a box narrower than the text.
    scroll: Pixels,
    /// The mouse is down in the field: a move extends the selection.
    selecting: bool,
    /// The text and cursor last drawn, and since when: a change shows the caret at once.
    shown: (SharedString, usize),
    since: Option<Instant>,
    /// A redraw for the next blink is on its way.
    ticking: bool,
    /// The window's height when it was drawn: what hangs under the field knows its room.
    viewport: Pixels,
    /// Where the new-task box's content was last drawn, and its dropdown.
    panel: Bounds<Pixels>,
    pub menu: Bounds<Pixels>,
    /// A field of several lines (the conversation's composer): each line drawn, where it starts
    /// in the text and its top; and the pointer's last height, to pick the line it is over.
    rows: Vec<(usize, ShapedLine, Pixels)>,
    pointer_y: Pixels,
}

impl Drawn {
    pub fn set_panel(&mut self, bounds: Bounds<Pixels>) {
        self.panel = bounds;
    }

    /// True if `position` is over the new-task box's content, or the padding around it.
    pub fn over_panel(&self, position: gpui::Point<Pixels>) -> bool {
        let pad = px(14.);
        let (from, to) = (self.panel.origin - point(pad, pad), self.panel.bottom_right() + point(pad, pad));
        self.panel.size.width > px(1.) && Bounds::from_corners(from, to).contains(&position)
    }
}

pub type Shared = Rc<RefCell<Drawn>>;

impl Workspace {
    fn edit(&mut self, edit: Edit, window: &Window, cx: &mut Context<Self>) {
        self.dispatch(Action::FieldEdit { edit }, self.input, window, cx);
        cx.notify();
    }

    /// A key for the open field. True: it was an editing key, or one that must not type (⌃X).
    /// False: it types text (the input method inserts it) or is a ⌘ shortcut for a binding.
    pub fn field_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) -> bool {
        let (key, m) = (&event.keystroke.key, event.keystroke.modifiers);
        let mods = crate::mods(m);
        match edit::key(key, mods) {
            _ if self.model.field().is_none() => return false,
            Some(Key::Edit(edit)) => self.edit(edit, window, cx),
            Some(Key::Copy) => drop(self.field_copy(false, window, cx)),
            Some(Key::Cut) => drop(self.field_copy(true, window, cx)),
            Some(Key::Paste) => drop(self.field_paste(window, cx)),
            None => return m.control,
        }
        true
    }

    /// ⌘C or ⌘X in a field. False: no field has the keyboard.
    pub fn field_copy(&mut self, cut: bool, window: &Window, cx: &mut Context<Self>) -> bool {
        let Some((_, field)) = self.model.field() else { return false };
        let text = field.selected().to_string();
        if !text.is_empty() {
            let verb = if cut { Verb::Cut } else { Verb::Copied };
            self.copy_text(text, None, verb, cx);
            if cut {
                self.edit(Edit::Cut, window, cx);
            }
        }
        true
    }

    /// The field that has the keyboard has text selected: ⌘C has something to copy.
    pub fn field_selected(&self) -> bool {
        self.model.field().is_some_and(|(_, field)| !field.selected().is_empty())
    }

    /// ⌘V in a field. False: no field has the keyboard.
    pub fn field_paste(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.model.field().is_none() {
            return false;
        }
        // Files copied in Finder go into the new-task line as paths.
        if self.paste_files(window, cx) {
            return true;
        }
        if let Some(text) = self.clipboard.text(cx) {
            self.edit(Edit::Paste(text), window, cx);
        }
        true
    }

    /// What the last paint drew of the field `of`. Each has its own: two can show at once (the
    /// draft's title in the bar, and a note or the picker's filter).
    fn drawn_of(&self, of: &str) -> Shared {
        match of {
            "note" => self.note_field.clone(),
            // The picker and the palette are never open together.
            "picker" | "palette" => self.picker_field.clone(),
            "apps" => self.apps_field.clone(),
            "mark" | "workbook" => self.mark_field.clone(),
            "inspector" => self.inspector_field.clone(),
            _ => self.field.clone(),
        }
    }

    /// That of the field that has the keyboard.
    fn drawn(&self) -> Shared {
        self.drawn_of(self.model.field().map_or("compose", |(of, _)| of))
    }

    /// The byte of the text nearest window position `x`.
    fn field_index(&self, x: Pixels) -> usize {
        let drawn = self.drawn();
        let drawn = drawn.borrow();
        let empty = self.model.field().is_none_or(|(_, field)| field.text().is_empty());
        // Several lines: the one the pointer is over, else the last above it.
        let row = drawn.rows.iter().rev().find(|(_, _, top)| *top <= drawn.pointer_y).or(drawn.rows.first());
        if let Some((start, line, _)) = row.filter(|_| !empty) {
            return start + line.closest_index_for_x(x - drawn.bounds.left() + drawn.scroll);
        }
        match &drawn.line {
            Some(line) if !empty => line.closest_index_for_x(x - drawn.bounds.left() + drawn.scroll),
            _ => 0,
        }
    }

    /// A field opened, closed or lost the keyboard: a drag begun before is over.
    pub fn field_changed_hands(&mut self) {
        for of in ["compose", "note", "picker", "apps", "mark", "workbook", "inspector"] {
            self.drawn_of(of).borrow_mut().selecting = false;
        }
        // The new-task box closed: its panel is nowhere, and nothing is pointed at.
        if self.model.compose.is_none() {
            (self.field.borrow_mut().panel, self.pointing) = (Bounds::default(), None);
        }
    }

    /// True if a mark's name is being typed and `position` is in its field: a click there
    /// places the cursor, and does not end the typing.
    pub fn in_mark_name(&self, position: gpui::Point<Pixels>) -> bool {
        let renaming = self.model.field().is_some_and(|(of, _)| matches!(of, "mark" | "workbook"));
        renaming && self.drawn_of("mark").borrow().bounds.contains(&position)
    }

    /// The Edit menu's Cut, Select All, Undo and Redo, and their keys when no handler took them.
    /// Nothing without a field that has the keyboard.
    pub fn field_menu(&mut self, key: Key, window: &Window, cx: &mut Context<Self>) {
        match key {
            Key::Edit(edit) if self.model.field().is_some() => self.edit(edit, window, cx),
            Key::Cut => drop(self.field_copy(true, window, cx)),
            _ => {}
        }
    }

    /// Where byte `at` of the open field's text was last drawn: its window x, the field's
    /// bounds, and the window's height then. `None`: not drawn yet.
    pub fn field_place(&self, at: usize) -> Option<(Pixels, Bounds<Pixels>, Pixels)> {
        let drawn = self.field.borrow();
        let empty = self.model.open_field().is_none_or(|(_, field)| field.text().is_empty());
        let x = match &drawn.line {
            Some(line) if !empty => line.x_for_index(at),
            _ => px(0.),
        };
        (drawn.bounds.size.width > px(1.))
            .then(|| (drawn.bounds.left() + x - drawn.scroll, drawn.bounds, drawn.viewport))
    }

    /// The mouse went down in the field at window `x`.
    pub fn field_press(&mut self, x: Pixels, count: usize, shift: bool, window: &Window, cx: &mut Context<Self>) {
        if self.model.field().is_none() {
            return;
        }
        self.drawn().borrow_mut().selecting = true;
        self.edit(Edit::Click { at: self.field_index(x), count, shift }, window, cx);
    }

    /// The pressed mouse moved to window `x`, inside the field or past its edge.
    pub fn field_drag(&mut self, x: Pixels, window: &Window, cx: &mut Context<Self>) {
        if self.drawn().borrow().selecting && self.model.field().is_some() {
            self.edit(Edit::Drag { to: self.field_index(x) }, window, cx);
        }
    }

    /// The open field for the state: whose it is, the text, and the cursor and selection in
    /// UTF-16 units (JavaScript string indexes). `bounds` and `cursor_x` are as last drawn, in
    /// window points.
    pub fn field_snapshot(&self) -> serde_json::Value {
        let Some((of, field)) = self.model.field() else { return serde_json::Value::Null };
        let drawn = self.drawn();
        let drawn = drawn.borrow();
        let (b, selection) = (drawn.bounds, field.selection());
        let x = match &drawn.line {
            Some(line) if !field.text().is_empty() => line.x_for_index(field.cursor()),
            _ => px(0.),
        };
        serde_json::json!({
            "of": of,
            "text": field.text(),
            "cursor": field.utf16(field.cursor()),
            "selection": [field.utf16(selection.start), field.utf16(selection.end)],
            "selected": field.selected(),
            "marked": field.marked().map(|m| [field.utf16(m.start), field.utf16(m.end)]),
            "bounds": [f32::from(b.origin.x), f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height)],
            "cursor_x": f32::from(b.origin.x + x - drawn.scroll),
        })
    }

    /// The open field as an element: as wide as its parent lets it be, one line tall.
    pub fn render_field(&self, placeholder: &'static str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let of = self.model.open_field().map_or("compose", |(of, _)| of);
        self.render_field_of(of, placeholder, cx)
    }

    /// The field `of` (`compose`, `note`, `picker`, `apps` or `mark`), whether or not it has the keyboard.
    pub fn render_field_of(
        &self,
        of: &'static str,
        placeholder: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let field = self.model.field_of(of).cloned().unwrap_or_default();
        // The new-task line shows which of its tags are understood.
        let spans = if of == "compose" { self.model.compose_spans() } else { Vec::new() };
        let spans = spans.into_iter().map(|span| (span.range, crate::new_task::tag_color(span.mark))).collect();
        div()
            .id(("field", of.len()))
            .flex_1()
            .min_w_0()
            .cursor(CursorStyle::IBeam)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    // The draft's title in the bar takes the keyboard by a click on it.
                    if of == "compose" && this.model.capture.is_some() && !this.model.compose_keys() {
                        this.dispatch(Action::CaptureTitle, this.input, window, cx);
                    }
                    this.drawn_of(of).borrow_mut().pointer_y = event.position.y;
                    this.field_press(event.position.x, event.click_count, event.modifiers.shift, window, cx);
                    // A mark's name sits in its row: the row's click would go to the mark, and
                    // the section's take the keyboard back. Either ends the rename.
                    if this.in_mark_name(event.position) {
                        cx.stop_propagation();
                    }
                }),
            )
            .child(FieldElement {
                workspace: cx.entity(),
                spans,
                active: self.model.field().is_some_and(|(has, _)| has == of),
                focus: self.focus.clone(),
                drawn: self.drawn_of(of),
                text: field.text().to_string().into(),
                selection: field.selection(),
                cursor: field.cursor(),
                marked: field.marked(),
                placeholder: placeholder.into(),
                accent: theme::accent(self.model.mode()),
                rows: if of == "talk" { ROWS } else { 1 },
            })
    }
}

/// The input method (IME, dead keys, the accent menu, dictation) talks to the open field.
/// Ranges are UTF-16; the core converts.
impl EntityInputHandler for Workspace {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let (_, field) = self.model.field()?;
        let bytes = field.byte(range.start)..field.byte(range.end).max(field.byte(range.start));
        adjusted.replace(field.utf16(bytes.start)..field.utf16(bytes.end));
        Some(field.text()[bytes].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let (_, field) = self.model.field()?;
        let selection = field.selection();
        Some(UTF16Selection {
            range: field.utf16(selection.start)..field.utf16(selection.end),
            reversed: !selection.is_empty() && field.cursor() == selection.start,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let (_, field) = self.model.field()?;
        field.marked().map(|m| field.utf16(m.start)..field.utf16(m.end))
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.edit(Edit::Unmark, window, cx);
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(Edit::Replace { range, text: text.into() }, window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(Edit::Mark { range, text: text.into(), selected }, window, cx);
    }

    /// Where the candidate window goes: under the marked text.
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (_, field) = self.model.field()?;
        let drawn = self.drawn();
        let drawn = drawn.borrow();
        let line = drawn.line.as_ref().filter(|_| !field.text().is_empty())?;
        let x = |utf16| bounds.left() + line.x_for_index(field.byte(utf16)) - drawn.scroll;
        Some(Bounds::from_corners(point(x(range.start), bounds.top()), point(x(range.end), bounds.bottom())))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let (_, field) = self.model.field()?;
        Some(field.utf16(self.field_index(point.x)))
    }
}

/// Paints the line, the selection and the caret, and registers the input handler.
struct FieldElement {
    workspace: Entity<Workspace>,
    /// Tags to draw in their colour, by their bytes in the text.
    spans: Vec<(Range<usize>, u32)>,
    /// The field has the keyboard: not under the key reference or the quit question.
    active: bool,
    focus: FocusHandle,
    drawn: Shared,
    text: SharedString,
    selection: Range<usize>,
    cursor: usize,
    marked: Option<Range<usize>>,
    placeholder: SharedString,
    accent: u32,
    /// The most lines it shows: 1, one line that scrolls sideways; more, a line per `\n`.
    rows: usize,
}

/// The most lines the conversation's composer shows; past them, those around the cursor.
const ROWS: usize = 8;

impl IntoElement for FieldElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for FieldElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let lines = self.text.split('\n').count().clamp(1, self.rows);
        style.size.height = (window.line_height() * lines as f32).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        // The workspace's root has the focus, so the field gets text without taking it.
        if self.active {
            window.handle_input(&self.focus, ElementInputHandler::new(bounds, self.workspace.clone()), cx);
        }
        if self.rows > 1 {
            self.paint_rows(bounds, window, cx);
            return self.listen(window);
        }

        let style = window.text_style();
        let empty = self.text.is_empty();
        let (shown, color) = match empty {
            true => (self.placeholder.clone(), rgb(theme::MUTED).into()),
            false => (self.text.clone(), style.color),
        };
        let run = |len: usize, color: gpui::Hsla, underline: bool| TextRun {
            len,
            font: style.font(),
            color,
            background_color: None,
            // A composition in progress is underlined.
            underline: underline.then_some(UnderlineStyle { color: Some(color), thickness: px(1.), wavy: false }),
            strikethrough: None,
        };
        // One run between each two places where the look changes: a tag's ends, the composition's.
        let (marked, spans) = (self.marked.clone().filter(|_| !empty), self.spans.iter().filter(|_| !empty));
        let mut cuts: Vec<usize> = spans.clone().flat_map(|(range, _)| [range.start, range.end]).collect();
        cuts.extend(marked.iter().flat_map(|m| [m.start, m.end]));
        cuts.extend([0, shown.len()]);
        cuts.retain(|cut| *cut <= shown.len() && shown.is_char_boundary(*cut));
        cuts.sort_unstable();
        cuts.dedup();
        let part = |ends: &[usize]| {
            let (from, to) = (ends[0], ends[1]);
            let tag = spans.clone().find(|(range, _)| range.start <= from && to <= range.end);
            let composing = marked.as_ref().is_some_and(|m| m.start <= from && to <= m.end);
            run(to - from, tag.map_or(color, |(_, tint)| rgb(*tint).into()), composing)
        };
        let runs: Vec<TextRun> = cuts.windows(2).map(part).collect();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window.text_system().shape_line(shown, font_size, &runs, None);
        let x = |index: usize| if empty { px(0.) } else { line.x_for_index(index) };

        let mut drawn = self.drawn.borrow_mut();
        // Keep the cursor in view; never scroll past the text's end.
        let (cursor_x, width) = (x(self.cursor), bounds.size.width - px(2.));
        let furthest = if empty { px(0.) } else { (line.width - width).max(px(0.)) };
        drawn.scroll = drawn.scroll.max(cursor_x - width).min(cursor_x).min(furthest).max(px(0.));
        let origin = bounds.origin - point(drawn.scroll, px(0.));

        let caret_on = self.blink(&mut drawn, cx);

        let selection = self.selection.clone();
        let line_height = window.line_height();
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            // A tag that is understood sits on a faint chip of its own colour.
            for (range, tint) in self.spans.iter().filter(|(range, _)| !empty && range.end <= self.text.len()) {
                let (from, to) = (origin.x + x(range.start) - px(2.), origin.x + x(range.end) + px(2.));
                let chip =
                    Bounds::from_corners(point(from, bounds.top() + px(1.)), point(to, bounds.bottom() - px(1.)));
                window.paint_quad(fill(chip, gpui::Hsla::from(rgb(*tint)).opacity(0.14)).corner_radii(px(4.)));
            }
            if !selection.is_empty() {
                let (from, to) = (origin.x + x(selection.start), origin.x + x(selection.end));
                let quad = Bounds::from_corners(point(from, bounds.top()), point(to, bounds.bottom()));
                window.paint_quad(fill(quad, rgb(theme::SELECTION)));
            }
            let _ = line.paint(origin, line_height, window, cx);
            if selection.is_empty() && caret_on && self.active {
                let caret = Bounds::new(point(origin.x + cursor_x, bounds.top()), size(px(2.), bounds.size.height));
                window.paint_quad(fill(caret, rgb(self.accent)));
            }
        });
        drawn.line = Some(line);
        drawn.rows.clear();
        drawn.bounds = bounds;
        drawn.viewport = window.viewport_size().height;
        drop(drawn);
        self.listen(window);
    }
}

impl FieldElement {
    /// True while the caret shows. Asks for the redraw of the next blink, for the field with
    /// the keyboard only: one that only shows asks for none.
    fn blink(&self, drawn: &mut Drawn, cx: &mut App) -> bool {
        let now = Instant::now();
        if drawn.shown != (self.text.clone(), self.cursor) {
            drawn.shown = (self.text.clone(), self.cursor);
            drawn.since = Some(now);
        }
        let caret_on = ((now - drawn.since.unwrap_or(now)).as_millis() / BLINK.as_millis()).is_multiple_of(2);
        if self.active && !drawn.ticking {
            drawn.ticking = true;
            let (shared, workspace) = (self.drawn.clone(), self.workspace.downgrade());
            cx.spawn(async move |cx| {
                cx.background_executor().timer(BLINK).await;
                shared.borrow_mut().ticking = false;
                let _ = workspace.update(cx, |_, cx| cx.notify());
            })
            .detach();
        }
        caret_on
    }

    /// A field of several lines: a line per `\n`, at most `rows`, those around the cursor; the
    /// selection and the caret on the lines they are on. Long lines are cut at the edge.
    fn paint_rows(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let style = window.text_style();
        let empty = self.text.is_empty();
        let (shown, color) = match empty {
            true => (self.placeholder.clone(), rgb(theme::MUTED).into()),
            false => (self.text.clone(), style.color),
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let mut starts = vec![0];
        starts.extend(shown.match_indices('\n').map(|(at, _)| at + 1));
        let on = starts.iter().rposition(|start| *start <= self.cursor).unwrap_or(0);
        let first = (on + 1).saturating_sub(self.rows);
        let mut drawn = self.drawn.borrow_mut();
        let caret_on = self.blink(&mut drawn, cx);
        drawn.rows.clear();
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for (row, start) in starts.iter().copied().enumerate().skip(first).take(self.rows) {
                let end = starts.get(row + 1).map_or(shown.len(), |next| next - 1);
                let text: SharedString = shown[start..end].to_string().into();
                let marked = self.marked.clone().filter(|m| !empty && m.start >= start && m.end <= end);
                let run = |len: usize, underline: bool| TextRun {
                    len,
                    font: style.font(),
                    color,
                    background_color: None,
                    underline: underline.then_some(UnderlineStyle {
                        color: Some(color),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    strikethrough: None,
                };
                let runs = match marked {
                    Some(m) => {
                        let parts = [(m.start - start, false), (m.end - m.start, true), (end - m.end, false)];
                        parts.into_iter().filter(|(len, _)| *len > 0).map(|(len, under)| run(len, under)).collect()
                    }
                    None => vec![run(text.len(), false)],
                };
                let line = window.text_system().shape_line(text, font_size, &runs, None);
                let top = bounds.top() + line_height * (row - first) as f32;
                let x = |index: usize| bounds.left() + if empty { px(0.) } else { line.x_for_index(index - start) };
                let (from, to) = (self.selection.start.max(start), self.selection.end.min(end));
                if from < to || (self.selection.start < start && self.selection.end > end) {
                    let quad = Bounds::from_corners(point(x(from), top), point(x(to.max(from)), top + line_height));
                    window.paint_quad(fill(quad, rgb(theme::SELECTION)));
                }
                let _ = line.paint(point(bounds.left(), top), line_height, window, cx);
                if self.selection.is_empty() && caret_on && self.active && row == on {
                    let caret = Bounds::new(point(x(self.cursor), top), size(px(2.), line_height));
                    window.paint_quad(fill(caret, rgb(self.accent)));
                }
                drawn.rows.push((start, line, top));
            }
        });
        drawn.line = drawn.rows.iter().find(|(start, ..)| *start == starts[on]).map(|(_, line, _)| line.clone());
        drawn.scroll = px(0.);
        drawn.bounds = bounds;
        drawn.viewport = window.viewport_size().height;
    }

    /// The mouse and modifier handlers every field has.
    fn listen(&self, window: &mut Window) {
        // On the window, not the field: a drag goes on past its edges, and ends anywhere.
        let (workspace, shared) = (self.workspace.clone(), self.drawn.clone());
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            if event.pressed_button == Some(MouseButton::Left) {
                shared.borrow_mut().pointer_y = event.position.y;
            }
            workspace.update(cx, |this, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    this.field_drag(event.position.x, window, cx);
                }
                // With ⌥ held over the workspace, the new-task box shows what a click would reference.
                this.compose_hover(event.position, event.modifiers.alt, cx);
            });
        });
        let shared = self.drawn.clone();
        window.on_mouse_event(move |_: &MouseUpEvent, _, _, _| shared.borrow_mut().selecting = false);
        // ⌥ let go, or pressed, with the pointer still: the outline follows the key.
        let workspace = self.workspace.clone();
        window.on_modifiers_changed(move |event: &gpui::ModifiersChangedEvent, _, cx| {
            workspace.update(cx, |this, cx| this.compose_hover(this.hover_at, event.modifiers.alt, cx));
        });
    }
}
