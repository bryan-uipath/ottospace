//! The shell's components, one builder each with its variants (`docs/ai/ui.md`). Each draws its
//! look only: the caller adds its control mark (its sneak label), tooltip and click.

pub(crate) mod gallery;

use gpui::{
    BoxShadow, CursorStyle, Div, ElementId, FontWeight, Refineable, SharedString, Stateful, StyleRefinement, Svg,
    black, div, point, prelude::*, px, rgb, svg, transparent_black,
};
use ottospace_core::{Axis, notice, theme};

use crate::{RAIL_CELL, ROW, cut_text};

/// A control's state. `Rest` is live: hover and press show as the pointer makes them. `Hover`
/// and `Pressed` draw that look still (the gallery). `Focus`: the keys are on it (list mode, a
/// rail's keys), in list mode's colour. `Disabled`: dimmed, no hover.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum State {
    Rest,
    Hover,
    Pressed,
    Focus,
    Disabled,
}

impl State {
    pub(crate) const ALL: [State; 5] = [State::Rest, State::Hover, State::Pressed, State::Focus, State::Disabled];

    /// List mode's keys on it, or not.
    pub(crate) fn focus(on: bool) -> Self {
        if on { State::Focus } else { State::Rest }
    }
}

type Look = fn(StyleRefinement) -> StyleRefinement;

/// `hover` and `press` (none: as hover) on `d` as `state` says: live, or drawn still.
fn states(d: Stateful<Div>, state: State, hover: Look, press: Option<Look>) -> Stateful<Div> {
    match state {
        State::Rest | State::Focus => d.hover(hover).when_some(press, |d, press| d.active(press)),
        State::Hover => still(d, hover(StyleRefinement::default())),
        State::Pressed => still(d, press.unwrap_or(|s| s)(hover(StyleRefinement::default()))),
        State::Disabled => d.opacity(0.4).cursor_default(),
    }
}

/// `look` laid over `d`'s own style, its text merged: a colour keeps the size.
fn still(mut d: Stateful<Div>, look: StyleRefinement) -> Stateful<Div> {
    let style = d.style();
    let mut text = style.text.take();
    match (&mut text, &look.text) {
        (Some(text), Some(over)) => text.refine(over),
        (None, over) => text = over.clone(),
        _ => {}
    }
    style.refine(&look);
    style.text = text;
    d
}

/// The keys' ring, `inset` points in from the edge: list mode's colour, apart from "open".
fn ring(inset: f32, radius: f32) -> Div {
    div().absolute().inset(px(inset)).rounded(px(radius)).border_2().border_color(rgb(theme::LIST))
}

/// What a ghost button shows.
#[derive(Clone)]
pub(crate) enum Face {
    Label(SharedString),
    Icon(&'static str),
}

/// A button's kind. One `Primary` per pane: what the pane is for. `Secondary`: outlined and
/// filled, beside a primary. `Icon`: an outlined arrow or glyph that reads as a button (the
/// rails' collapse arrows). `Ghost`: no outline at rest, a header's small words and icons.
#[derive(Clone)]
pub(crate) enum Button {
    Primary(SharedString),
    Secondary(SharedString),
    Icon(&'static str),
    Ghost(Face),
}

/// A button in `state`. A label is `CONTROL` tall; an icon `ICON_SM`, in a 28-point square when ghost.
pub(crate) fn button(id: impl Into<ElementId>, kind: Button, state: State) -> Stateful<Div> {
    let d = div().id(id).relative().flex_none().flex().items_center().cursor_pointer();
    let label = |d: Stateful<Div>, text: SharedString| {
        d.h(px(theme::CONTROL))
            .px(px(theme::SPACE_1_5))
            .rounded(px(theme::RADIUS_SM))
            .text_size(px(theme::TEXT_XS))
            .child(text)
    };
    let icon = |path: &'static str| svg().path(path).size(px(theme::ICON_SM)).text_color(rgb(theme::MUTED));
    let outlined = |d: Stateful<Div>| d.border_1().border_color(rgb(theme::BORDER_STRONG)).bg(rgb(theme::RAISED));
    let fill: Look = |s| s.bg(rgb(theme::SELECTED));
    let fill_lit: Look = |s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT));
    // Pressed: a lit key cap's fill (`Pill::Key`).
    let held: Look = |s| s.bg(rgb(theme::BORDER_STRONG));
    let radius = match kind {
        Button::Icon(_) | Button::Ghost(Face::Icon(_)) => theme::RADIUS_MD,
        _ => theme::RADIUS_SM,
    };
    let d = match kind {
        Button::Primary(text) => {
            let d = label(d, text).bg(rgb(theme::PRIMARY)).text_color(rgb(theme::BACKGROUND));
            states(d.font_weight(FontWeight::SEMIBOLD), state, |s| s.opacity(0.9), Some(|s| s.opacity(0.8)))
        }
        Button::Secondary(text) => {
            states(outlined(label(d, text)).text_color(rgb(theme::TEXT)), state, fill, Some(held))
        }
        Button::Icon(path) => {
            let d = d.p(px(theme::SPACE_1)).rounded(px(theme::RADIUS_MD)).child(icon(path));
            states(outlined(d), state, fill, Some(held))
        }
        Button::Ghost(Face::Label(text)) => {
            states(label(d, text).text_color(rgb(theme::MUTED)), state, fill_lit, Some(held))
        }
        Button::Ghost(Face::Icon(path)) => {
            let d = d.size(px(ROW)).justify_center().rounded(px(theme::RADIUS_MD)).child(icon(path));
            states(d.text_color(rgb(theme::MUTED)), state, fill_lit, Some(held))
        }
    };
    d.when(state == State::Focus, |d| d.child(ring(0., radius)))
}

/// A pill's kind: a key cap, lit while held (`small`: beside text of its size), or the status
/// bar's mode in the mode's accent.
pub(crate) enum Pill {
    Key { lit: bool, small: bool },
    Mode(u32),
}

pub(crate) fn pill(text: impl Into<SharedString>, kind: Pill) -> Div {
    match kind {
        Pill::Key { lit, small } => {
            // Lit: a lighter fill, never the primary colour.
            let (fill, edge, ink) = match lit {
                true => (theme::BORDER_STRONG, theme::SCROLL_THUMB_HELD, theme::TEXT),
                false => (theme::RAISED, theme::BORDER_STRONG, theme::MUTED),
            };
            let (side, round) = if small { (16., 4.) } else { (22., 5.) };
            div()
                .min_w(px(side))
                .h(px(side))
                .px(px(theme::SPACE_1))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(round))
                .border_1()
                .border_color(rgb(edge))
                .bg(rgb(fill))
                .text_size(px(if small { theme::TEXT_2XS } else { theme::TEXT_XS }))
                .text_color(rgb(ink))
                .child(text.into())
        }
        Pill::Mode(accent) => div()
            .flex_none()
            .text_size(px(theme::TEXT_XS))
            .px(px(theme::SPACE_2))
            .rounded(px(theme::RADIUS_SM))
            .bg(rgb(accent))
            .text_color(rgb(theme::BACKGROUND))
            .child(text.into()),
    }
}

/// A list row's kind. `Normal`: a sidebar section's row, `ROW` tall, its title in full colour.
/// `Dim`: a row under a heading (a checkout), muted until the pointer or the keys are on it.
#[derive(Clone, Copy)]
pub(crate) enum Row {
    Normal,
    Dim,
}

/// A list row: no outline at rest, so a long list stays quiet; hover fills it, and list mode's
/// selection (`Focus`) fills and outlines it.
pub(crate) fn list_row(id: impl Into<ElementId>, row: Row, state: State) -> Stateful<Div> {
    let d = div()
        .id(id)
        .flex_none()
        .relative()
        .rounded(px(theme::RADIUS_MD))
        // Always there, so a row does not shift by a point when it is selected.
        .border_1()
        .border_color(transparent_black())
        .text_size(px(theme::TEXT_ROW))
        .cursor_pointer();
    let focused = state == State::Focus;
    match row {
        Row::Normal => {
            let d = d.min_h(px(ROW)).px(px(theme::SPACE_2)).flex().items_center().gap(px(theme::SPACE_2));
            let d = d.text_color(rgb(theme::TEXT));
            states(d, state, |s| s.bg(rgb(theme::SELECTED)), None)
                .when(focused, |row| row.bg(rgb(theme::SELECTED)).border_color(rgb(theme::LIST)))
        }
        Row::Dim => {
            // Room on the left for the row's icon.
            let d = d.pl(px(26.)).pr(px(theme::SPACE_2)).py(px(theme::SPACE_0_5)).line_height(px(17.));
            states(
                d.text_color(rgb(theme::MUTED)),
                state,
                |s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)),
                None,
            )
            .when(focused, |row| {
                row.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)).border_color(rgb(theme::LIST))
            })
        }
    }
}

/// A heading's fold mark, ▾ open or ▸ folded: drawn, not a glyph, so it sits on the words' middle
/// (the font's ▾ sat a point below it). Coloured by the caller's `text_color`.
pub(crate) fn fold_mark(open: bool) -> Svg {
    svg().path(if open { "icons/fold-open.svg" } else { "icons/fold-closed.svg" }).flex_none().size(px(8.))
}

/// A group's heading in a list: a chevron (`open`: ▾), its name and how many it holds, then room
/// for its actions. A dim row's hover and selection, a row's height; a click anywhere folds it.
pub(crate) fn section_heading(
    id: impl Into<ElementId>,
    open: bool,
    name: impl Into<SharedString>,
    count: usize,
    state: State,
) -> Stateful<Div> {
    let muted = |d: Div| d.flex_none().text_size(px(theme::TEXT_XS)).text_color(rgb(theme::MUTED));
    list_row(id, Row::Dim, state)
        .h(px(ROW))
        .px(px(theme::SPACE_2))
        .py_0()
        .flex()
        .items_center()
        .gap(px(theme::SPACE_1_5))
        .child(div().flex_none().w(px(10.)).child(fold_mark(open).text_color(rgb(theme::MUTED))))
        .child(cut_text(name).flex_shrink().text_size(px(theme::TEXT_XS)))
        .child(muted(div()).child(format!("· {count}")))
        .child(div().flex_1())
}

/// A panel's header, as tall as a rail cell: its title in capitals; the caller adds its actions
/// and, last, its collapse arrow (`Button::Icon`).
pub(crate) fn panel_header(title: &str) -> Div {
    div()
        .relative()
        .flex_none()
        .h(px(RAIL_CELL))
        .pl(px(theme::SPACE_3))
        .pr(px(theme::SPACE_1))
        .flex()
        .items_center()
        .justify_between()
        .border_b_1()
        .border_color(rgb(theme::BORDER))
        .text_size(px(theme::TEXT_XS))
        .text_color(rgb(theme::MUTED))
        .child(title.to_uppercase())
}

/// Which rail: the left one's cells are joined to the sidebar on their right, the right one's to
/// the drawer on their left.
#[derive(Clone, Copy)]
pub(crate) enum Side {
    Left,
    Right,
}

/// A rail cell, as wide as its rail. `open`: it shows in the panel beside it, raised in the
/// panel's background with an accent bar (this colour) at the window's edge and no line toward
/// the panel: one piece with it. Not open: the line toward the panel, and hover fills it.
pub(crate) fn rail_cell(id: impl Into<ElementId>, side: Side, open: Option<u32>, state: State) -> Stateful<Div> {
    let d = div().id(id).relative().flex_none().w_full().h(px(RAIL_CELL)).flex().items_center().justify_center();
    let d = d.cursor_pointer();
    let d = match open {
        Some(accent) => raised(d, accent, side),
        None => {
            let d = match side {
                Side::Left => d.border_r_1(),
                Side::Right => d.border_l_1(),
            };
            states(d.border_color(rgb(theme::BORDER)), state, |s| s.bg(rgb(theme::SELECTED)), None)
        }
    };
    d.when(state == State::Focus, |d| d.child(ring(4., 8.)))
}

/// An open rail cell, raised: the panel's background, a soft shadow below it, and an accent bar
/// at the window's edge. No line or shadow above: every cell reads as the first one does.
fn raised(cell: Stateful<Div>, accent: u32, side: Side) -> Stateful<Div> {
    let shadow = BoxShadow {
        color: black().opacity(0.85),
        offset: point(px(0.), px(7.)),
        blur_radius: px(9.),
        spread_radius: px(-6.),
    };
    let bar = div().absolute().top_0().w(px(2.)).h_full().bg(rgb(accent));
    let bar = match side {
        Side::Left => bar.left_0(),
        Side::Right => bar.right_0(),
    };
    cell.bg(rgb(theme::RAISED)).shadow(vec![shadow]).child(bar)
}

/// A floating label's kind: a tooltip under the pointer, or the chip a dragged pane or tab carries.
#[derive(Clone, Copy)]
pub(crate) enum Tip {
    Hint,
    Drag,
}

/// A tooltip: one short line. A key appears here, never on the control.
pub(crate) fn tooltip(text: SharedString, tip: Tip) -> Div {
    let (fill, edge) = match tip {
        Tip::Hint => (theme::SELECTED, theme::BORDER_STRONG),
        Tip::Drag => (theme::RAISED, theme::PRIMARY),
    };
    div()
        .px(px(theme::SPACE_2))
        .py(px(theme::SPACE_1))
        .rounded(px(theme::RADIUS_MD))
        .bg(rgb(fill))
        .border_1()
        .border_color(rgb(edge))
        .text_size(px(theme::TEXT_XS))
        .text_color(rgb(theme::TEXT))
        .child(text)
}

/// The notice bar's line, `notice::HEIGHT` tall in its level's colours: the level's glyph, then
/// `text`; the caller adds what it offers and ×. An error's `!` sits in a filled circle: no cross
/// beside the text that could read as the ×.
pub(crate) fn notice_line(level: notice::Level, glyph: &'static str, text: impl IntoElement) -> Div {
    let (background, ink) = theme::notice(level);
    let mark = div().flex_none().child(glyph);
    let mark = match level {
        notice::Level::Error => {
            let round = mark.size(px(13.)).rounded_full().flex().items_center().justify_center();
            round.bg(rgb(ink)).text_color(rgb(background)).font_weight(FontWeight::BOLD)
        }
        _ => mark,
    };
    div()
        .h(px(notice::HEIGHT))
        .px(px(theme::SPACE_3))
        .flex()
        .items_center()
        .gap(px(theme::SPACE_2))
        .bg(rgb(background))
        .text_size(px(theme::TEXT_XS))
        .text_color(rgb(ink))
        .child(mark)
        .child(text)
}

/// How far across a divider its grab reaches: a pane's border and padding on each side (3 points),
/// so it takes nothing of the panes' content, nor of a hosted window laid over it.
pub(crate) const DIVIDER_GRAB: f32 = theme::SPACE_1_5;

/// The grab of the divider between two panes, `DIVIDER_GRAB` across, with the resize pointer.
/// Hover lights a line down its middle, `Pressed` (held, dragging) brighter; at rest it draws
/// nothing over the panes' borders. The caller places it and adds the drag.
pub(crate) fn divider(id: impl Into<ElementId>, axis: Axis, state: State) -> Stateful<Div> {
    let group = SharedString::from("divider");
    let inset = px((DIVIDER_GRAB - theme::SPACE_0_5) / 2.);
    let (d, line) = match axis {
        Axis::Right => {
            let line = div().absolute().top_0().bottom_0().left(inset).w(px(theme::SPACE_0_5));
            (div().id(id).w(px(DIVIDER_GRAB)).h_full().cursor(CursorStyle::ResizeLeftRight), line)
        }
        Axis::Down => {
            let line = div().absolute().left_0().right_0().top(inset).h(px(theme::SPACE_0_5));
            (div().id(id).h(px(DIVIDER_GRAB)).w_full().cursor(CursorStyle::ResizeUpDown), line)
        }
    };
    let line = match state {
        State::Pressed => line.bg(rgb(theme::SCROLL_THUMB_HELD)),
        State::Hover => line.bg(rgb(theme::SCROLL_THUMB_HOVER)),
        _ => line.group_hover(group.clone(), |s| s.bg(rgb(theme::SCROLL_THUMB_HOVER))),
    };
    d.group(group).relative().child(line)
}
