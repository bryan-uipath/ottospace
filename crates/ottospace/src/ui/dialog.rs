//! A dialog's parts (`docs/ai/ui.md`, Dialogs): one frame (title, purpose, ×, body, actions) and
//! the controls a dialog holds. Each draws its look only: the caller adds the mark and the click.

use gpui::{
    AnyElement, Div, ElementId, FontWeight, IntoElement, SharedString, Stateful, black, div, prelude::*, px, rgb, svg,
};
use ottospace_core::theme;

use super::{Button, State, states};

/// Widths: a question, a form.
pub(crate) const QUESTION: f32 = 360.;
pub(crate) const FORM: f32 = 520.;
/// The one dim behind every centred dialog, and the lighter one behind a list anchored to its place (⌘E).
pub(crate) const DIM: f32 = 0.45;
pub(crate) const DIM_ANCHORED: f32 = 0.2;
/// A control's height in a dialog (22 elsewhere).
const CONTROL: f32 = 26.;
/// A form's label column.
const LABELS: f32 = 96.;
/// The summary line's size: the rule or question in one sentence, first in the body.
const HEADLINE: f32 = 15.;

/// The frame: `title` (a question or a verb phrase) and one muted sentence of `purpose` under it,
/// `close` (×) top right, then `body` and the action row. `width`: `QUESTION` or `FORM`.
pub(crate) fn frame(
    width: f32,
    title: impl Into<SharedString>,
    purpose: Option<SharedString>,
    close: impl IntoElement,
    body: Option<Div>,
    actions: Div,
) -> Div {
    let words = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(div().text_size(px(theme::TEXT_SM)).font_weight(FontWeight::SEMIBOLD).child(title.into()))
        .children(purpose.map(|text| div().text_size(px(theme::TEXT_XS)).text_color(rgb(theme::MUTED)).child(text)));
    let header = div()
        .flex()
        .items_start()
        .gap(px(theme::SPACE_3))
        .pt(px(14.))
        .pr(px(theme::SPACE_3))
        .pl(px(theme::SPACE_4))
        .child(words)
        .child(close);
    let body = body.map(|body| body.px(px(theme::SPACE_4)).pt(px(14.)).flex().flex_col().gap(px(theme::SPACE_3)));
    crate::dialog::floating(width)
        .text_color(rgb(theme::TEXT))
        .text_size(px(theme::TEXT_ROW))
        .child(header)
        .children(body)
        .child(actions.px(px(theme::SPACE_4)).pt(px(14.)).pb(px(theme::SPACE_4)))
}

/// × at the top right: the same as Esc and Cancel. Never drawn with a key.
pub(crate) fn close(id: impl Into<ElementId>, state: State) -> Stateful<Div> {
    // The glyph paints in its own colour: it brightens with its group's hover.
    let group = SharedString::from("dialog-close");
    let ink = if state == State::Hover { theme::TEXT } else { theme::MUTED };
    let glyph = svg().path("icons/dismiss.svg").size(px(theme::ICON_SM)).text_color(rgb(ink));
    let glyph = match state {
        State::Rest | State::Focus => glyph.group_hover(group.clone(), |s| s.text_color(rgb(theme::TEXT))),
        _ => glyph,
    };
    let d = div()
        .id(id)
        .group(group)
        .relative()
        .flex_none()
        .size(px(CONTROL - 4.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme::RADIUS_MD))
        .cursor_pointer()
        .child(glyph);
    states(d, state, |s| s.bg(rgb(theme::SELECTED)), None)
        .when(state == State::Focus, |d| d.child(super::ring(0., theme::RADIUS_MD)))
}

/// The action row: `left` (a destructive act, alone), the `status` line (what ↵ will do, or what
/// matches), then the buttons, the primary right-most.
pub(crate) fn actions(left: Option<AnyElement>, status: Option<SharedString>, right: Vec<AnyElement>) -> Div {
    let status = status
        .map(|text| div().min_w_0().truncate().text_size(px(theme::TEXT_XS)).text_color(rgb(theme::MUTED)).child(text));
    div()
        .flex()
        .items_center()
        .gap(px(theme::SPACE_2))
        .children(left)
        .child(div().flex_1())
        .children(status)
        .children(right)
}

/// A dialog's button, its `keys` as caps after its label in its own ink: the caps are the only
/// place a dialog shows a key. One `Primary` (or `DangerPrimary`), right-most.
pub(crate) fn action(id: impl Into<ElementId>, kind: Button, keys: &[&'static str], state: State) -> Stateful<Div> {
    let filled = matches!(kind, Button::Primary(_) | Button::DangerPrimary(_));
    super::button(id, kind, state)
        .h(px(CONTROL))
        .px(px(9.))
        .gap(px(7.))
        .children(keys.iter().map(|key| cap(key, filled)))
}

/// A key cap on a control. On a filled button: dark, in the button's own ink, never the accent.
pub(crate) fn cap(key: &'static str, filled: bool) -> Div {
    match filled {
        false => super::pill(key, super::Pill::Key { lit: false, small: true }).font_weight(FontWeight::NORMAL),
        true => div()
            .min_w(px(16.))
            .h(px(16.))
            .px(px(theme::SPACE_1))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(theme::RADIUS_SM))
            .border_1()
            .border_color(black().opacity(0.28))
            .bg(black().opacity(0.12))
            .text_size(px(theme::TEXT_2XS))
            .font_weight(FontWeight::NORMAL)
            .child(key),
    }
}

/// A form's row: a muted label, right-aligned in its column, and the value on one left edge.
pub(crate) fn row(label: &'static str, value: impl IntoElement) -> Div {
    let label = div()
        .flex_none()
        .w(px(LABELS))
        .flex()
        .justify_end()
        .text_size(px(theme::TEXT_XS))
        .text_color(rgb(theme::MUTED))
        .child(label);
    let value = div().flex_1().min_w_0().flex().flex_wrap().items_center().gap(px(theme::SPACE_1_5)).child(value);
    div().flex().items_center().gap(px(theme::SPACE_3)).child(label).child(value)
}

/// A line under a row's value, in line with it: what the chosen option does.
pub(crate) fn consequence(text: impl Into<SharedString>) -> Div {
    let line = div().text_size(px(theme::TEXT_XS)).text_color(rgb(theme::MUTED)).child(text.into());
    div().flex().gap(px(theme::SPACE_3)).child(div().flex_none().w(px(LABELS))).child(line)
}

/// Two to four options, one chosen: `segment`s in a well.
pub(crate) fn segments(options: impl IntoIterator<Item = Stateful<Div>>, focus: bool) -> Div {
    div()
        .relative()
        .flex()
        .flex_none()
        .p(px(theme::SPACE_0_5))
        .gap(px(theme::SPACE_0_5))
        .rounded(px(7.))
        .bg(rgb(theme::WELL))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .children(options)
        .when(focus, |d| d.child(super::ring(-1., 7.)))
}

/// An option of `segments`: its result in a word or two, `key` its letter. Chosen: a lifted
/// fill, white text. Never the accent: cyan in a dialog is the primary's alone.
pub(crate) fn segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    key: Option<&'static str>,
    on: bool,
    state: State,
) -> Stateful<Div> {
    let d = div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(theme::SPACE_1_5))
        .h(px(CONTROL - 4.))
        .px(px(9.))
        .rounded(px(5.))
        .text_size(px(theme::TEXT_XS))
        .text_color(rgb(if on { theme::TEXT } else { theme::MUTED }))
        .cursor_pointer()
        // Every option has its edge, so choosing one moves nothing.
        .border_1()
        .border_color(gpui::transparent_black())
        .when(on, |d| d.bg(rgb(theme::SELECTED)).border_color(rgb(theme::BORDER_STRONG)))
        .child(label.into())
        .children(key.map(|key| cap(key, false)));
    states(d, state, |s| s.text_color(rgb(theme::TEXT)), None)
}

/// A box: ticked, a filled box with a check.
fn checkbox(on: bool) -> Div {
    let d = div()
        .flex_none()
        .size(px(theme::ICON_SM))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme::RADIUS_SM))
        .border_1()
        .border_color(rgb(theme::SCROLL_THUMB_HOVER));
    d.when(on, |d| {
        d.bg(rgb(theme::TEXT))
            .border_color(rgb(theme::TEXT))
            .text_color(rgb(theme::BACKGROUND))
            .text_size(px(theme::TEXT_2XS))
            .font_weight(FontWeight::BOLD)
            .child("✓")
    })
}

/// A round: chosen, a thick white ring.
fn radio(on: bool) -> Div {
    let d = div().flex_none().size(px(theme::ICON_SM)).rounded_full();
    match on {
        true => d.border(px(4.)).border_color(rgb(theme::TEXT)),
        false => d.border_1().border_color(rgb(theme::SCROLL_THUMB_HOVER)),
    }
}

/// A checkbox row (`radio`: one of a group): no border round the row, the box is the control.
/// The label in white, the `value` it acts on muted beside it.
pub(crate) fn choice_row(
    id: impl Into<ElementId>,
    radio: bool,
    on: bool,
    label: impl Into<SharedString>,
    value: Option<SharedString>,
    state: State,
) -> Stateful<Div> {
    let mark = if radio { self::radio(on) } else { checkbox(on) };
    let d = div()
        .id(id)
        .relative()
        .flex()
        .items_center()
        .gap(px(theme::SPACE_2))
        .py(px(3.))
        .px(px(theme::SPACE_1_5))
        .mx(px(-theme::SPACE_1_5))
        .rounded(px(theme::RADIUS_MD))
        .text_size(px(theme::TEXT_ROW))
        .cursor_pointer()
        .child(mark)
        .child(label.into())
        .children(value.map(|value| div().min_w_0().truncate().text_color(rgb(theme::MUTED)).child(value)));
    states(d, state, |s| s.bg(rgb(theme::SELECTED)), None)
        .when(state == State::Focus, |d| d.child(super::ring(0., theme::RADIUS_MD)))
}

/// A text box in a dialog: the caller puts the field in it. `focus`: it has the keys.
pub(crate) fn text_box(focus: bool) -> Div {
    div()
        .relative()
        .flex_1()
        .min_w_0()
        .h(px(CONTROL))
        .px(px(theme::SPACE_2))
        .flex()
        .items_center()
        .rounded(px(theme::RADIUS_MD))
        .bg(rgb(theme::BACKGROUND))
        .border_1()
        .border_color(rgb(theme::BORDER_STRONG))
        .text_size(px(theme::TEXT_XS))
        .when(focus, |d| d.child(super::ring(-1., theme::RADIUS_MD)))
}

/// The rule or the question in one sentence, first in the body: the most important line is the
/// most prominent.
pub(crate) fn summary(text: impl IntoElement) -> Div {
    div().text_size(px(HEADLINE)).line_height(px(22.)).font_weight(FontWeight::MEDIUM).child(text)
}

/// A window the list shows: whether the rule takes it, its title (drawn, never in the state), and
/// what tells it apart (size, display, kind).
pub(crate) struct Listed {
    pub(crate) taken: bool,
    pub(crate) title: SharedString,
    pub(crate) detail: SharedString,
}

/// The windows open now, those the rule takes ticked.
pub(crate) fn window_list(rows: Vec<Listed>) -> Div {
    let line = |at: usize, row: Listed| {
        let ink = if row.taken { theme::TEXT } else { theme::SCROLL_THUMB_HELD };
        div()
            .flex()
            .items_center()
            .gap(px(theme::SPACE_2))
            .px(px(9.))
            .py(px(5.))
            .when(at > 0, |d| d.border_t_1().border_color(rgb(theme::BORDER)))
            .text_size(px(theme::TEXT_XS))
            .text_color(rgb(ink))
            .child(div().flex_none().w(px(theme::ICON_SM)).text_color(rgb(theme::TEXT)).child(if row.taken {
                "✓"
            } else {
                ""
            }))
            .child(div().flex_1().min_w_0().truncate().child(row.title))
            .child(div().flex_none().text_color(rgb(theme::MUTED)).child(row.detail))
    };
    div()
        .flex()
        .flex_col()
        .rounded(px(crate::dialog::INNER_RADIUS))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .bg(rgb(theme::WELL))
        .overflow_hidden()
        .children(rows.into_iter().enumerate().map(|(at, row)| line(at, row)))
}
