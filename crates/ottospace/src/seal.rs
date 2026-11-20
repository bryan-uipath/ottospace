//! The seal, fold and land in the shell: drawn over everything, where the drawer and the task's
//! rail item were when the user's Done came (`Effect::Seal`). The drawer's own header and
//! conversation stay drawn and fade out under the ring (GPUI cannot scale an element's picture). Where each part is at a moment is the
//! core's (`ottospace_core::seal::frame`); this keeps the clock, draws, and asks for frames only
//! while it plays.

use std::time::Instant;

use gpui::{
    AnyElement, Bounds, Context, Div, Hsla, PathBuilder, Pixels, Window, canvas, div, point, prelude::*, px, rgb,
};
use ottospace_core::model::rail::RailItem;
use ottospace_core::seal::{self, Frame};
use ottospace_core::talk::Subject;
use ottospace_core::tasks::TaskId;
use ottospace_core::theme;

use crate::Workspace;
use crate::dial::{arc, colour, disc, stroke};
use crate::finish::DONE;

/// The ring's radius at rest, in points, and the ✓ on the item's corner.
const RING: f32 = 32.;
const CORNER: f32 = 14.;

/// One playing: whose, since when, and where its drawer and rail item were drawn.
pub(crate) struct Sealing {
    task: TaskId,
    since: Instant,
    panel: Option<Bounds<Pixels>>,
    item: Option<Bounds<Pixels>>,
    /// Its place among the rail's tasks: kept empty until it lands, so the next does not move up under it.
    slot: Option<usize>,
}

/// The seal playing, if one is, and the frames it asked for (`counts`, test runs).
#[derive(Default)]
pub(crate) struct Seals {
    now: Option<Sealing>,
    pub(crate) frames: u64,
    /// Of those, the frames with sparks: none under Reduce Motion.
    pub(crate) spark_frames: u64,
    /// A Decisions card answered: since when, and where its pane was. The next card already takes keys.
    card: Option<(Instant, Bounds<Pixels>)>,
}

impl Workspace {
    /// The core said the user's Done plays: the drawer and the rail item where they were last drawn.
    pub(crate) fn seal_start(&mut self, task: TaskId) {
        // The whole drawer as last drawn: its header, dial, body or brief, and foot.
        let panel = self.controls.bounds("talk");
        let control = |id: TaskId| format!("drawer-rail:{id}");
        let item = self.controls.bounds(&control(task));
        let above = |at: &Bounds<Pixels>| {
            let ids = self.model.rail().into_iter().filter_map(|item| match item {
                RailItem::Task { id } => Some(id),
                RailItem::Transcript => None,
            });
            ids.filter_map(|id| self.controls.bounds(&control(id))).filter(|b| b.origin.y < at.origin.y).count()
        };
        let slot = item.as_ref().map(above);
        self.seals.now = Some(Sealing { task, since: Instant::now(), panel, item, slot });
    }

    /// A Decisions card was answered: its seal plays over the card pane, the next card under it.
    /// None under Reduce Motion: the next card is there at once.
    pub(crate) fn card_seal_start(&mut self, pane: ottospace_core::model::PaneId) {
        let at = self.controls.bounds(&format!("pane:{pane}")).filter(|_| !crate::dial::calm());
        self.seals.card = at.map(|at| (Instant::now(), at));
    }

    /// The card's seal now: the ring, tick and sparks at the pane's middle, fading as the
    /// drawer's fold would. Draws nothing that takes a click.
    pub(crate) fn render_card_seal(&mut self, window: &mut Window) -> Option<AnyElement> {
        let (since, at) = self.seals.card?;
        let t = since.elapsed().as_secs_f32() * 1000.;
        // Decisions closed or off show: it plays on no other workspace.
        let shown = *self.model.shown_space() == ottospace_core::model::Owner::Decisions;
        let Some(mut frame) = seal::frame(t, false).filter(|_| shown && t < seal::FOLD_MS) else {
            self.seals.card = None;
            return None;
        };
        frame.ring_alpha *= 1. - frame.fold;
        self.seals.frames += 1;
        self.seals.spark_frames += u64::from(!frame.sparks.is_empty());
        window.request_animation_frame();
        let paint = move |bounds: Bounds<Pixels>, _: (), window: &mut Window, _: &mut gpui::App| {
            paint_seal(&frame, bounds, window)
        };
        let layer = div().absolute().left(at.origin.x).top(at.origin.y).w(at.size.width).h(at.size.height);
        Some(layer.child(canvas(|_, _, _| {}, paint).absolute().size_full()).into_any_element())
    }

    /// Where the rail keeps an empty place for the task landing, while it does.
    pub(crate) fn seal_slot(&self) -> Option<usize> {
        self.seals.now.as_ref().and_then(|sealing| sealing.slot)
    }

    /// The seal's frame now, over everything; none at rest. Asks for the next while it plays.
    pub(crate) fn render_seal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sealing = self.seals.now.as_ref()?;
        let t = sealing.since.elapsed().as_secs_f32() * 1000.;
        // Undone, or opened again: it stops at once (`Model::seal_plays`).
        let plays = self.model.seal_plays(sealing.task);
        let Some(frame) = seal::frame(t, crate::dial::calm()).filter(|_| plays) else {
            self.seals.now = None;
            return None;
        };
        self.seals.frames += 1;
        self.seals.spark_frames += u64::from(!frame.sparks.is_empty());
        window.request_animation_frame();
        let sealing = self.seals.now.as_ref()?;
        // The drawer's own header and conversation, drawn as they were, fading out under the ring.
        let subject = Subject::Task(sealing.task);
        let width = sealing.panel.map(|panel| panel.size.width);
        let content = (frame.content > 0.).then(|| width).flatten().map(|width| {
            let header = self.talk_header(&subject, cx);
            let dial = self.dial_section(&subject, f32::from(width), cx);
            let body = self.chat_body(&subject, cx);
            div()
                .absolute()
                .top_0()
                .left_0()
                .w(width)
                .h_full()
                .flex()
                .flex_col()
                .overflow_hidden()
                .children(header)
                .children(dial)
                .child(body)
        });
        let sealing = self.seals.now.as_ref()?;
        let panel = sealing.panel.map(|panel| ghost_panel(panel, frame.clone(), content));
        let landed = t >= seal::FOLD_MS;
        let item = sealing.item.filter(|_| landed).and_then(|item| self.ghost_item(sealing.task, item, &frame));
        let layer = div().absolute().top_0().left_0().size_full().children(panel).children(item);
        Some(layer.into_any_element())
    }

    /// The task's rail item, popping where it was, with the ✓ on its corner.
    fn ghost_item(&self, task: TaskId, item: Bounds<Pixels>, frame: &Frame) -> Option<AnyElement> {
        let side = 22. * frame.pop;
        let badge = self.task_badge(task, side)?;
        let c = item.center();
        let tick = div()
            .absolute()
            .top(px(-CORNER / 2.))
            .right(px(-CORNER / 2.))
            .size(px(CORNER))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(DONE))
            .text_size(px(9.))
            .text_color(rgb(theme::BACKGROUND))
            .opacity(frame.badge)
            .child("✓");
        let at = div().absolute().left(c.x - px(side / 2.)).top(c.y - px(side / 2.)).size(px(side));
        Some(at.child(badge).child(tick).into_any_element())
    }
}

/// The drawer as it folds into the rail: a panel sliding right and fading, the ring, tick and
/// sparks at its middle.
fn ghost_panel(panel: Bounds<Pixels>, frame: Frame, content: Option<Div>) -> AnyElement {
    let width = f32::from(panel.size.width);
    let shrink = 1. - 0.8 * frame.fold;
    let left = panel.origin.x + px(width * frame.fold);
    let (shows, fading) = (frame.panel, frame.content);
    let paint =
        move |bounds: Bounds<Pixels>, _: (), window: &mut Window, _: &mut gpui::App| paint_seal(&frame, bounds, window);
    div()
        .absolute()
        .left(left)
        .top(panel.origin.y)
        .w(px(width * shrink))
        .h(panel.size.height)
        .overflow_hidden()
        .bg(rgb(theme::BACKGROUND))
        .border_l_1()
        .border_color(rgb(theme::BORDER))
        .opacity(shows)
        .children(content.map(|content| content.opacity(fading)))
        .child(canvas(|_, _, _| {}, paint).absolute().size_full())
        .into_any_element()
}

/// The ring, the tick drawn as far as it has got, and the sparks.
fn paint_seal(frame: &Frame, bounds: Bounds<Pixels>, window: &mut Window) {
    let c = bounds.center();
    let green = colour(DONE);
    let alpha = frame.ring_alpha;
    let r = RING * frame.ring;
    let ring = arc(3., c, r, 0., std::f32::consts::TAU);
    stroke(window, ring, green.opacity(alpha));
    // The tick: two strokes, drawn along their length.
    let scale = frame.ring * 1.4;
    let pts = [(-10., 1.), (-3., 8.), (10., -7.)].map(|(x, y): (f32, f32)| c + point(px(x * scale), px(y * scale)));
    let lengths = [dist(pts[0], pts[1]), dist(pts[1], pts[2])];
    let mut left = frame.tick * (lengths[0] + lengths[1]);
    let mut tick = PathBuilder::stroke(px(4.));
    tick.move_to(pts[0]);
    for (i, len) in lengths.iter().enumerate() {
        if left <= 0. {
            break;
        }
        let along = (left / len).min(1.);
        let (a, b) = (pts[i], pts[i + 1]);
        tick.line_to(a + point((b.x - a.x) * along, (b.y - a.y) * along));
        left -= len;
    }
    if frame.tick > 0. {
        stroke(window, tick, green.opacity(alpha));
    }
    let hues: [Hsla; 4] = [colour(theme::PRIMARY), colour(theme::SNEAK), green, colour(theme::PRIMARY)];
    for (spark, hue) in frame.sparks.iter().zip(hues) {
        disc(window, c + point(px(spark.dx), px(spark.dy)), spark.r, hue.opacity(spark.alpha));
    }
}

fn dist(a: gpui::Point<Pixels>, b: gpui::Point<Pixels>) -> f32 {
    let (dx, dy) = (f32::from(b.x - a.x), f32::from(b.y - a.y));
    (dx * dx + dy * dy).sqrt()
}
