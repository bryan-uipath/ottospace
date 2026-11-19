//! Every component in every state, for a test run only (`ui_gallery`, `control::test_refusal`):
//! `scripts/shot ui-gallery` pictures it, so `shots.yml` shows any change to one.

use gpui::{AnyElement, Div, ElementId, SharedString, div, prelude::*, px, rgb};
use ottospace_core::{Mode, notice, theme};

use super::{Button, Face, Pill, Row, Side, State, Tip};

/// A component's name over its row of cases.
fn group(name: &'static str, cases: impl IntoIterator<Item = AnyElement>) -> Div {
    let title = div().text_size(px(theme::TEXT_XS)).text_color(rgb(theme::MUTED)).child(name);
    let cases = div().flex().flex_wrap().items_center().gap(px(theme::SPACE_3)).children(cases);
    div().flex().flex_col().gap(px(theme::SPACE_1_5)).child(title).child(cases)
}

/// One case and its caption below it.
fn case(caption: impl Into<SharedString>, element: impl IntoElement) -> AnyElement {
    let caption = div().text_size(px(theme::TEXT_2XS)).text_color(rgb(theme::MUTED)).child(caption.into());
    div().flex().flex_col().items_start().gap(px(theme::SPACE_1)).child(element).child(caption).into_any_element()
}

fn id(name: String) -> ElementId {
    ElementId::Name(name.into())
}

fn buttons() -> Div {
    let kinds = [
        ("primary", Button::Primary("Transcribe".into())),
        ("secondary", Button::Secondary("Decline".into())),
        ("icon", Button::Icon(crate::slide::LEFT)),
        ("ghost", Button::Ghost(Face::Label("Stop".into()))),
        ("ghost icon", Button::Ghost(Face::Icon("icons/plus.svg"))),
    ];
    let rows = kinds.into_iter().map(|(name, kind)| {
        let cases = State::ALL.map(|state| {
            case(format!("{state:?}"), super::button(id(format!("{name}{state:?}")), kind.clone(), state))
        });
        group(name, cases)
    });
    div().flex().flex_col().gap(px(theme::SPACE_2)).children(rows)
}

fn pills() -> Div {
    let keys =
        [(false, false, "key"), (true, false, "key lit"), (false, true, "key small"), (true, true, "key small lit")]
            .map(|(lit, small, name)| case(name, super::pill("⌘", Pill::Key { lit, small })));
    let modes = [("LIST", Mode::List), ("SNEAK", Mode::Sneak), ("APP", Mode::App)]
        .map(|(text, mode)| case("mode", super::pill(text, Pill::Mode(theme::accent(mode)))));
    group("pill", keys.into_iter().chain(modes))
}

fn rows() -> Div {
    let width = |d: gpui::Stateful<Div>| div().w(px(220.)).child(d);
    let rows = [("normal", Row::Normal), ("dim", Row::Dim)].map(|(name, row)| {
        let states = [State::Rest, State::Hover, State::Focus];
        let cases = states.map(|state| {
            let r = super::list_row(id(format!("row-{name}{state:?}")), row, state).child("fix the login loop");
            case(format!("{state:?}"), width(r))
        });
        group(name, cases)
    });
    let headings =
        [(true, State::Rest), (false, State::Rest), (true, State::Hover), (true, State::Focus)].map(|(open, state)| {
            let h = super::section_heading(id(format!("heading{open}{state:?}")), open, "app-api", 3, state);
            case(format!("{} {state:?}", if open { "open" } else { "folded" }), width(h))
        });
    div().flex().flex_col().gap(px(theme::SPACE_2)).children(rows).child(group("section heading", headings))
}

fn header() -> Div {
    let header = super::panel_header("Tasks")
        .child(div().flex_1())
        .child(super::button("gallery-add", Button::Ghost(Face::Icon("icons/plus.svg")), State::Rest))
        .child(super::button("gallery-collapse", Button::Icon(crate::slide::LEFT), State::Rest));
    group("panel header", [div().w(px(260.)).bg(rgb(theme::RAISED)).child(header).into_any_element()])
}

fn rails() -> Div {
    let icon = || gpui::svg().path("icons/tasks.svg").size(px(theme::ICON)).text_color(rgb(theme::MUTED));
    let cells = [Side::Left, Side::Right].map(|side| {
        let looks = [("rest", None, State::Rest), ("hover", None, State::Hover), ("focus", None, State::Focus)]
            .into_iter()
            .chain([("open", Some(theme::PRIMARY), State::Rest), ("open needs you", Some(theme::SNEAK), State::Rest)])
            .chain([("open focus", Some(theme::PRIMARY), State::Focus)]);
        let name = match side {
            Side::Left => "left",
            Side::Right => "right",
        };
        // As wide as its rail: the right one is narrower.
        let width = match side {
            Side::Left => crate::RAIL,
            Side::Right => crate::drawer_rail::WIDTH,
        };
        let cases = looks.map(|(look, open, state)| {
            let cell = super::rail_cell(id(format!("rail-{name}-{look}")), side, open, state).child(icon());
            case(look, div().w(px(width)).bg(rgb(theme::BACKGROUND)).child(cell))
        });
        group(if matches!(side, Side::Left) { "rail cell, left" } else { "rail cell, right" }, cases)
    });
    div().flex().flex_col().gap(px(theme::SPACE_2)).children(cells)
}

fn tips() -> Div {
    let tips = [("hint", Tip::Hint, "Collapse into the rail · ⌘⇧L"), ("drag", Tip::Drag, "app-api · cargo test")]
        .map(|(name, tip, text)| case(name, super::tooltip(text.into(), tip)));
    group("tooltip", tips)
}

fn notices() -> Div {
    let levels = [
        (notice::Level::Error, "!", "error"),
        (notice::Level::Warning, "▲", "warning"),
        (notice::Level::Info, "ⓘ", "info"),
        (notice::Level::Meeting, "◷", "meeting"),
    ];
    let lines = levels.map(|(level, glyph, name)| {
        let text = format!("A made-up {name} line");
        div()
            .w(px(420.))
            .child(super::notice_line(level, glyph, div().flex_1().child(text)).child("×"))
            .into_any_element()
    });
    group("notice line", lines)
}

/// The gallery, over the whole window.
pub(crate) fn view() -> Div {
    let left = div().flex().flex_col().gap(px(theme::SPACE_3)).child(buttons()).child(pills()).child(tips());
    let right =
        div().flex().flex_col().gap(px(theme::SPACE_3)).child(rows()).child(header()).child(rails()).child(notices());
    div()
        .absolute()
        // Takes the pointer: nothing under it is hovered or clicked.
        .occlude()
        .inset_0()
        .bg(rgb(theme::BACKGROUND))
        .text_color(rgb(theme::TEXT))
        .p(px(theme::SPACE_3))
        // Clear of the window's buttons.
        .pt(px(crate::TOP_BAR))
        .flex()
        .gap(px(24.))
        .child(left)
        .child(right)
}
