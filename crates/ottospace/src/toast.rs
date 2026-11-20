//! The toast: the app's one place for what is transient. Cards in the window's top right, under
//! the top row and the tabs, over the panes: it takes no room, so nothing moves for it. Each card
//! has a level's colours and glyph, an optional title, one line of text, up to three actions and ×.
//! Pointed at, a card shows its whole text, wrapped, and is as tall as that takes.
//! Generic: what a card says is its caller's (`notices` for the app's notices).
//! A card's line is `ui::notice_line`. Over a live app's window each card is drawn in a floating
//! panel of its own instead (`float_panel`), so a click between two goes to the app.

use gpui::{AnyElement, App, BoxShadow, Div, ElementId, MouseButton, SharedString, Stateful, Window};
use gpui::{black, div, point, prelude::*, px, rgb};

use ottospace_core::notice;

/// How wide a card is, and the gap to the window's edge and between cards, in points.
const WIDTH: f32 = 380.;
const GAP: f32 = 8.;
/// How far a card slides in from the right as it comes.
const SLIDE: f32 = 24.;
/// A card's height, and a title's line on top of it; "+N more"'s.
const CARD: f32 = 28.;
const TITLE: f32 = 16.;
pub(crate) const MORE: f32 = 20.;
const MORE_WIDTH: f32 = 72.;
/// At most this many lines of a card's text while it is pointed at, and the room over and under them.
const LINES: usize = 8;
const OPEN_PAD: f32 = 6.;
/// The stack's pieces at most: the cards and "+N more". Over an app each has its own panel.
pub(crate) const PIECES: usize = notice::CARDS + 1;
/// At most this many actions on a card.
pub(crate) const ACTIONS: usize = 3;

/// One card as drawn.
pub(crate) struct Card {
    /// Its colours and glyph (`ui::notice_line`).
    pub(crate) level: notice::Level,
    pub(crate) glyph: &'static str,
    pub(crate) title: Option<SharedString>,
    /// One line, cut with "…"; all of it, wrapped, while `open`.
    pub(crate) text: SharedString,
    /// The pointer is over it.
    pub(crate) open: bool,
    /// The pointer came over it (true) or left.
    pub(crate) on_hover: Box<dyn Fn(&bool, &mut Window, &mut App)>,
    /// Up to `ACTIONS` buttons, made with `button`.
    pub(crate) actions: Vec<Stateful<Div>>,
    /// Its ×, made with `button`.
    pub(crate) close: Stateful<Div>,
    /// Invisible children that note where it is drawn.
    pub(crate) marks: Vec<AnyElement>,
}

/// A card's button: `id` unique per card, e.g. `("toast-close", 1)`. The click is the card's: it
/// does not also leave a mode, as one on the workspace would.
pub(crate) fn button(id: impl Into<ElementId>) -> Stateful<Div> {
    let button = div().id(id).relative().flex_none().px_1p5().rounded_sm().cursor_pointer();
    button
        .hover(|s| s.bg(gpui::white().opacity(0.12)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// A card: glyph, title over text, actions, ×; the level's colours and a soft shadow, so it floats.
fn card(at: usize, card: Card) -> Stateful<Div> {
    let (_, text) = ottospace_core::theme::notice(card.level);
    let height = px(rest(card.title.is_some()));
    let line = if card.open { div().line_clamp(LINES).child(card.text) } else { crate::cut_text(card.text) };
    let words = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .children(card.title.map(|title| crate::cut_text(title).font_weight(gpui::FontWeight::SEMIBOLD)));
    let mut actions = card.actions;
    actions.truncate(ACTIONS);
    // Occluded: a click on its body does not reach the pane under it.
    crate::ui::notice_line(card.level, card.glyph, words.child(line))
        .id(("toast-card", at))
        .relative()
        .occlude()
        .w_full()
        // Open, its glyph and buttons stay at the top, where they are at rest.
        .map(|this| if card.open { this.min_h(height).items_start().py(px(OPEN_PAD)) } else { this.h(height) })
        .on_hover(card.on_hover)
        .rounded_md()
        .border_1()
        .border_color(gpui::Hsla::from(rgb(text)).opacity(0.25))
        .shadow(vec![BoxShadow {
            color: black().opacity(0.55),
            offset: point(px(0.), px(6.)),
            blur_radius: px(18.),
            spread_radius: px(-4.),
        }])
        .children(actions.into_iter().map(|action| action.underline()))
        .child(card.close)
        .children(card.marks)
}

/// How wide the cards are in `room` (the panes' width), less a gap each side.
pub(crate) fn width(room: f32) -> f32 {
    WIDTH.min(room - 2. * GAP).max(0.)
}

/// A card's height at rest: taller with a title.
pub(crate) fn rest(titled: bool) -> f32 {
    CARD + if titled { TITLE } else { 0. }
}

/// Where each piece of the stack goes in a window `width_of` wide: `top` down, `right` in from
/// its right edge, in `room`, a gap between two. `heights`: each piece's, the top one first;
/// `more`: the last is "+N more". `[x, y, w, h]` from the window's top left, as `stack` draws them.
pub(crate) fn places([top, right, room]: [f32; 3], width_of: f32, heights: &[f32], more: bool) -> Vec<[f32; 4]> {
    let (w, mut y) = (width(room), top);
    let x = width_of - right - GAP - w;
    let place = |(at, h): (usize, &f32)| {
        let label = more && at + 1 == heights.len();
        let place = if label { [x + w - MORE_WIDTH.min(w), y, MORE_WIDTH.min(w), *h] } else { [x, y, w, *h] };
        y += h + GAP;
        place
    };
    heights.iter().enumerate().map(place).collect()
}

/// "+N more", under the cards.
fn more_label(more: usize) -> Div {
    let label = div().h(px(MORE)).w(px(MORE_WIDTH)).max_w_full().flex().items_center().justify_center().rounded_md();
    let label = label.bg(rgb(ottospace_core::theme::RAISED)).text_xs();
    label.text_color(rgb(ottospace_core::theme::MUTED)).child(format!("+{more} more"))
}

/// The cards, first on top, and "+N more" under them: the stack's inside, `mark` noting where.
fn column(cards: Vec<Card>, more: usize, mark: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .items_end()
        .gap(px(GAP))
        .children(cards.into_iter().enumerate().map(|(at, c)| card(at, c)))
        .children((more > 0).then(|| more_label(more)))
        .child(mark)
}

/// The stack in the window: `top` points down the window and `right` in from its right edge, at
/// most `room` (the panes' width) less its gaps. `shown`: how far in it is (0 to 1); it slides in
/// from the right and fades.
pub(crate) fn stack(
    [top, right, room]: [f32; 3],
    cards: Vec<Card>,
    more: usize,
    shown: f32,
    mark: impl IntoElement,
) -> Div {
    column(cards, more, mark)
        .absolute()
        .top(px(top))
        .right(px(right + GAP - SLIDE * (1. - shown)))
        .w(px(width(room)))
        .opacity(shown)
}

/// One card in a floating panel of its own, as wide as the panel and as tall as the card takes.
pub(crate) fn piece(at: usize, of: Card) -> Div {
    // A column, its card not shrunk: taller than the panel until the panel follows.
    div().size_full().flex().flex_col().child(card(at, of).flex_none())
}

/// "+N more" in a floating panel of its own, `mark` noting where.
pub(crate) fn more_piece(more: usize, mark: impl IntoElement) -> Div {
    more_label(more).relative().child(mark)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pieces_lie_one_under_another_a_gap_between_at_the_panes_top_right() {
        assert_eq!(places([60., 300., 600.], 1000., &[], false), Vec::<[f32; 4]>::new());
        // A 1000-point window, 300 of drawer: the panes end at 700, less a gap.
        let x = 1000. - 300. - GAP - WIDTH;
        // A card pointed at is taller: the next starts a gap under it.
        let (tall, card) = (rest(false) + 30., rest(false));
        assert_eq!(
            places([60., 300., 600.], 1000., &[tall, card, MORE], true),
            [
                [x, 60., WIDTH, tall],
                [x, 60. + tall + GAP, WIDTH, card],
                [x + WIDTH - MORE_WIDTH, 60. + tall + GAP + card + GAP, MORE_WIDTH, MORE]
            ]
        );
        assert_eq!(rest(true), CARD + TITLE);
        // Narrow panes: the cards narrow, never past the panes' left.
        assert_eq!(places([60., 300., 200.], 500., &[28.], false), [[500. - 300. - GAP - 184., 60., 184., 28.]]);
    }
}
