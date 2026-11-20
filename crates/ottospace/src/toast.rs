//! The toast: the app's one place for what is transient. Cards in the window's bottom right, over
//! the panes, just above the status bar (and the timeline's band): it takes no room, so nothing moves for it.
//! The first card is nearest the status bar; the others stack upward. Each card
//! has a level's colours and glyph, an optional title, one line of text, up to three actions and ×.
//! Pointed at, a card shows its whole text, wrapped, and grows upward as tall as that takes.
//! Generic: what a card says is its caller's (`notices` for the app's notices).
//! A card's line is `ui::notice_line`. Over a live app's window each card is drawn in a floating
//! panel of its own instead (`float_panel`), so a click between two goes to the app.

use std::rc::Rc;

use gpui::{AnyElement, App, BoxShadow, DispatchPhase, Div, ElementId, MouseButton, MouseExitEvent, MouseMoveEvent};
use gpui::{SharedString, Stateful, Window, black, canvas, div, point, prelude::*, px, rgb};

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
    /// Whether the pointer is over it, told at each move and when it leaves the window.
    pub(crate) on_hover: Rc<dyn Fn(&bool, &mut Window, &mut App)>,
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
fn card(card: Card) -> Div {
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
        .relative()
        .occlude()
        .w_full()
        // Open, it grows upward: its glyph and buttons stay at the foot, where they are at rest.
        .map(|this| if card.open { this.min_h(height).items_end().py(px(OPEN_PAD)) } else { this.h(height) })
        .child(pointed(card.on_hover))
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

/// An invisible child that tells `over` whether the pointer is over its parent. Not `on_hover`:
/// that hears no leave when the pointer goes out of the window, as it does from a panel.
fn pointed(over: Rc<dyn Fn(&bool, &mut Window, &mut App)>) -> impl IntoElement {
    let listen = move |bounds: gpui::Bounds<gpui::Pixels>, _: (), window: &mut Window, _: &mut App| {
        let moved = over.clone();
        // Measured from the window's foot, where the card sits: a panel grown since this paint (to
        // fit its card, opened upward) moved the card down by as much. Else it reads as left, and closes.
        let painted = window.bounds().size.height;
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble {
                let at = event.position - point(px(0.), window.bounds().size.height - painted);
                moved(&bounds.contains(&at), window, cx);
            }
        });
        window.on_mouse_event(move |_: &MouseExitEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble {
                over(&false, window, cx);
            }
        });
    };
    canvas(|_, _, _| {}, listen).absolute().top_0().left_0().size_full()
}

/// How wide the cards are in `room` (the panes' width), less a gap each side.
pub(crate) fn width(room: f32) -> f32 {
    WIDTH.min(room - 2. * GAP).max(0.)
}

/// A card's height at rest: taller with a title.
pub(crate) fn rest(titled: bool) -> f32 {
    CARD + if titled { TITLE } else { 0. }
}

/// Where each piece of the stack goes in a window `[width, height]`: its foot `bottom` up from the
/// window's foot, `right` in from its right edge, in `room`, a gap between two. `heights`: each
/// piece's, the first (lowest) first; `more`: the last (highest) is "+N more". `[x, y, w, h]` from
/// the window's top left, as `stack` draws them.
pub(crate) fn places(
    [bottom, right, room]: [f32; 3],
    [width_of, height_of]: [f32; 2],
    heights: &[f32],
    more: bool,
) -> Vec<[f32; 4]> {
    let (w, mut foot) = (width(room), height_of - bottom);
    let x = width_of - right - GAP - w;
    let place = |(at, h): (usize, &f32)| {
        let label = more && at + 1 == heights.len();
        let y = foot - h;
        let place = if label { [x + w - MORE_WIDTH.min(w), y, MORE_WIDTH.min(w), *h] } else { [x, y, w, *h] };
        foot = y - GAP;
        place
    };
    heights.iter().enumerate().map(place).collect()
}

/// "+N more", over the cards.
fn more_label(more: usize) -> Div {
    let label = div().h(px(MORE)).w(px(MORE_WIDTH)).max_w_full().flex().items_center().justify_center().rounded_md();
    let label = label.bg(rgb(ottospace_core::theme::RAISED)).text_xs();
    label.text_color(rgb(ottospace_core::theme::MUTED)).child(format!("+{more} more"))
}

/// The cards, first at the foot, and "+N more" over them: the stack's inside, `mark` noting where.
fn column(cards: Vec<Card>, more: usize, mark: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col_reverse()
        .items_end()
        .gap(px(GAP))
        .children(cards.into_iter().map(card))
        .children((more > 0).then(|| more_label(more)))
        .child(mark)
}

/// The stack in the window: its foot `bottom` points up from the window's foot and `right` in from
/// its right edge, at most `room` (the panes' width) less its gaps. `shown`: how far in it is (0 to
/// 1); it slides in from the right and fades.
pub(crate) fn stack(
    [bottom, right, room]: [f32; 3],
    cards: Vec<Card>,
    more: usize,
    shown: f32,
    mark: impl IntoElement,
) -> Div {
    column(cards, more, mark)
        .absolute()
        .bottom(px(bottom))
        .right(px(right + GAP - SLIDE * (1. - shown)))
        .w(px(width(room)))
        .opacity(shown)
}

/// One card in a floating panel of its own, as wide as the panel and as tall as the card takes.
pub(crate) fn piece(of: Card) -> Div {
    // A column, its card not shrunk and at the foot: taller than the panel, upward, until the panel follows.
    div().size_full().flex().flex_col().justify_end().child(card(of).flex_none())
}

/// "+N more" in a floating panel of its own, `mark` noting where.
pub(crate) fn more_piece(more: usize, mark: impl IntoElement) -> Div {
    more_label(more).relative().child(mark)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pieces_lie_one_over_another_a_gap_between_at_the_panes_bottom_right() {
        assert_eq!(places([44., 300., 600.], [1000., 800.], &[], false), Vec::<[f32; 4]>::new());
        // A 1000 by 800 window, 300 of drawer: the panes end at 700, less a gap; the foot 44 up.
        let (x, foot) = (1000. - 300. - GAP - WIDTH, 800. - 44.);
        // A card pointed at is taller, upward: the next ends a gap over it.
        let (tall, card) = (rest(false) + 30., rest(false));
        let second = foot - tall - GAP - card;
        assert_eq!(
            places([44., 300., 600.], [1000., 800.], &[tall, card, MORE], true),
            [
                [x, foot - tall, WIDTH, tall],
                [x, second, WIDTH, card],
                [x + WIDTH - MORE_WIDTH, second - GAP - MORE, MORE_WIDTH, MORE]
            ]
        );
        assert_eq!(rest(true), CARD + TITLE);
        // Narrow panes: the cards narrow, never past the panes' left.
        let narrow = places([44., 300., 200.], [500., 400.], &[28.], false);
        assert_eq!(narrow, [[500. - 300. - GAP - 184., 400. - 44. - 28., 184., 28.]]);
    }
}
