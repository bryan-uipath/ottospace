//! The toast: the app's one place for what is transient. Cards in the window's top right, under
//! the top row and the tabs, over the panes: it takes no room, so nothing moves for it. Each card
//! has a level's colours and glyph, an optional title, one line of text, up to three actions and ×.
//! Generic: what a card says is its caller's (`notices` for the app's notices).
//! A card's line is `ui::notice_line`. Over a live app's window the same cards are drawn in a
//! floating panel instead (`float_panel`): cards are of fixed heights, so its size is known.

use gpui::{AnyElement, AnyView, App, BoxShadow, Div, ElementId, MouseButton, SharedString, Stateful, Window};
use gpui::{black, div, point, prelude::*, px, rgb};

use ottospace_core::notice;

use crate::Tooltip;

/// How wide a card is, and the gap to the window's edge and between cards, in points.
const WIDTH: f32 = 380.;
const GAP: f32 = 8.;
/// How far a card slides in from the right as it comes.
const SLIDE: f32 = 24.;
/// A card's height, and a title's line on top of it; "+N more"'s.
const CARD: f32 = 28.;
const TITLE: f32 = 16.;
const MORE: f32 = 20.;
/// At most this many actions on a card.
pub(crate) const ACTIONS: usize = 3;

/// One card as drawn.
pub(crate) struct Card {
    /// Its colours and glyph (`ui::notice_line`).
    pub(crate) level: notice::Level,
    pub(crate) glyph: &'static str,
    pub(crate) title: Option<SharedString>,
    /// One line, cut with "…"; the hover shows all of it.
    pub(crate) text: SharedString,
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
fn card(at: usize, card: Card) -> Div {
    let (_, text) = ottospace_core::theme::notice(card.level);
    let titled = card.title.is_some();
    let full = card.text.clone();
    let tip = move |_: &mut Window, cx: &mut App| AnyView::from(cx.new(|_| Tooltip(full.clone())));
    let line = crate::cut_text(card.text).id(("toast-text", at)).tooltip(tip);
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
        .h(px(CARD + if titled { TITLE } else { 0. }))
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

/// How tall the stack is: cards (each `true` if titled) and "+N more" (`more` > 0), gaps between.
pub(crate) fn height(titled: impl IntoIterator<Item = bool>, more: usize) -> f32 {
    let heights = titled.into_iter().map(|t| CARD + if t { TITLE } else { 0. });
    let heights: Vec<f32> = heights.chain((more > 0).then_some(MORE)).collect();
    heights.iter().sum::<f32>() + GAP * heights.len().saturating_sub(1) as f32
}

/// Where the stack goes in a window `width` wide: `top` down, `right` in from its right edge,
/// in `room`. `[x, y, w, h]` from the window's top left, as `stack` draws it at rest.
pub(crate) fn place([top, right, room]: [f32; 3], width_of: f32, height: f32) -> [f32; 4] {
    let w = width(room);
    [width_of - right - GAP - w, top, w, height]
}

/// The cards, first on top, and "+N more" under them: the stack's inside, `mark` noting where.
fn column(cards: Vec<Card>, more: usize, mark: impl IntoElement) -> Div {
    let more = (more > 0).then(|| {
        let label = div().h(px(MORE)).flex().items_center().px_2().rounded_md();
        let label = label.bg(rgb(ottospace_core::theme::RAISED)).text_xs();
        label.text_color(rgb(ottospace_core::theme::MUTED)).child(format!("+{more} more"))
    });
    div()
        .flex()
        .flex_col()
        .items_end()
        .gap(px(GAP))
        .children(cards.into_iter().enumerate().map(|(at, c)| card(at, c)))
        .children(more)
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

/// The stack filling a floating panel sized to it (`place`): at rest, nothing around it.
pub(crate) fn panel(cards: Vec<Card>, more: usize, mark: impl IntoElement) -> Div {
    column(cards, more, mark).size_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stack_is_as_tall_as_its_cards_and_sits_at_the_panes_top_right() {
        assert_eq!(height([], 0), 0.);
        assert_eq!(height([false, false], 0), CARD * 2. + GAP);
        assert_eq!(height([true], 2), CARD + TITLE + GAP + MORE);
        // A 1000-point window, 300 of drawer: the panes end at 700, less a gap.
        assert_eq!(place([60., 300., 600.], 1000., 28.), [1000. - 300. - GAP - WIDTH, 60., WIDTH, 28.]);
        // Narrow panes: the cards narrow, never past the panes' left.
        assert_eq!(place([60., 300., 200.], 500., 28.), [500. - 300. - GAP - 184., 60., 184., 28.]);
    }
}
