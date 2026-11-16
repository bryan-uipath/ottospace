//! The agent dial over a native conversation: what the core's `activity::Dial` says, drawn. The
//! 96-point dial and its lines under the header, or the 18-point marker beside the badge when
//! collapsed. Paths are built each paint (a few dozen segments); the app draws again on a change,
//! on a slow tick while a station is lit or the route fades, and frame by frame only while
//! something in the open dial moves (`motion::Motion`). See `docs/ai/agent-dial.md`.

use std::f32::consts::{PI, TAU};

use std::time::Instant;

use gpui::{
    AnyView, Bounds, Context, Div, Hsla, PathBuilder, Pixels, Point, Stateful, Transformation, Window, canvas, div,
    fill, point, prelude::*, px, rgb, size, svg,
};
use ottospace_core::activity::{self, Dial, Pulse, Station};
use ottospace_core::motion::{self, Motion};
use ottospace_core::talk::Subject;
use ottospace_core::{Action, theme};

use crate::{Tooltip, Workspace};

/// The dial's face with its stations' words around it, and the marker's side.
const FACE: (f32, f32) = (180., 120.);
const MARK: f32 = 18.;
/// A station's word: how far out it starts, the room it has, its text size.
const WORDS: f32 = 50.;
const WORD: (f32, f32) = (40., 11.);
const WORD_TEXT: f32 = 9.;
/// The agent chip: its height, and the most its line takes.
const CHIP: f32 = 24.;
const CHIP_LINE: f32 = 220.;
/// Between the chip and the row's end.
const CHIP_EDGE: f32 = 8.;
/// Radii in the dial: the centre, the context ring, where the spokes start and end, the stations.
const CENTRE: f32 = 12.;
const RING: f32 = 17.;
const SPOKE: (f32, f32) = (21., 34.);
const STATIONS: f32 = 38.;

/// A station's angle: clockwise from the top.
fn angle(station: Station) -> f32 {
    let at = Station::DIAL.iter().position(|s| *s == station).unwrap_or(0);
    at as f32 * TAU / Station::DIAL.len() as f32
}

fn at(centre: Point<Pixels>, r: f32, angle: f32) -> Point<Pixels> {
    centre + point(px(r * angle.sin()), px(-r * angle.cos()))
}

fn colour(rgb_: u32) -> Hsla {
    Hsla::from(rgb(rgb_))
}

/// An arc as a polyline, `sweep` radians clockwise from `from`.
fn arc(width: f32, centre: Point<Pixels>, r: f32, from: f32, sweep: f32) -> PathBuilder {
    let mut path = PathBuilder::stroke(px(width));
    let steps = ((sweep.abs() / (PI / 32.)).ceil() as usize).max(2);
    for i in 0..=steps {
        let p = at(centre, r, from + sweep * i as f32 / steps as f32);
        if i == 0 { path.move_to(p) } else { path.line_to(p) }
    }
    path
}

fn stroke(window: &mut Window, path: PathBuilder, colour: Hsla) {
    if let Ok(path) = path.build() {
        window.paint_path(path, colour);
    }
}

fn disc(window: &mut Window, centre: Point<Pixels>, r: f32, colour: Hsla) {
    let bounds = Bounds::centered_at(centre, size(px(r * 2.), px(r * 2.)));
    window.paint_quad(fill(bounds, colour).corner_radii(px(r)));
}

/// Where a station's word goes in the face, and how it is set: outside its dot, away from
/// the centre, so it meets neither the ring nor a neighbour's.
fn word(station: Station) -> (f32, f32, f32) {
    let (sin, cos) = angle(station).sin_cos();
    let r = WORDS + 4. * cos.abs();
    let (x, y) = (FACE.0 / 2. + r * sin, FACE.1 / 2. - r * cos);
    // Over or under the centre: centred. Else it starts, or ends, at its place.
    let (left, set) = match sin {
        _ if sin.abs() < 0.2 => (x - WORD.0 / 2., 0.5),
        _ if sin > 0. => (x, 0.),
        _ => (x - WORD.0, 1.),
    };
    (left, (y - WORD.1 / 2.).clamp(0., FACE.1 - WORD.1), set)
}

/// The shown dial's motion, kept between frames: view state, never the model's.
#[derive(Default)]
pub(crate) struct DialMotion {
    /// Whose dial, its parts, and when they were last stepped.
    shown: Option<(Subject, Motion, Instant)>,
    /// The frame being drawn asked for another: something moves.
    pub(crate) moving: bool,
    /// How many frames asked for another (the state's `counts.dial_frames`, test runs).
    pub(crate) frames: u64,
}

impl DialMotion {
    /// The parts of `subject`'s dial now. Another conversation's starts at rest.
    fn step(&mut self, subject: &Subject, dial: &Dial, calm: bool) -> Motion {
        let now = Instant::now();
        let (mut parts, then) = match self.shown.take() {
            Some((shown, parts, then)) if shown == *subject => (parts, then),
            _ => (Motion::rest(dial), now),
        };
        self.moving = parts.step(dial, now.duration_since(then).as_secs_f32() * 1000., calm);
        self.frames += u64::from(self.moving);
        self.shown = Some((subject.clone(), parts, now));
        parts
    }
}

/// No motion: the system's Reduce Motion, or a test run that asks for it.
fn calm() -> bool {
    let asked = std::env::var_os("OTTOSPACE_E2E").and(std::env::var_os("OTTOSPACE_REDUCE_MOTION"));
    asked.is_some() || ottospace_mac::meeting::reduce_motion()
}

/// Paints the dial in `bounds`, its dots as `parts` has them. `calm`: no fading or pulse,
/// states only (Reduce motion).
fn paint_dial(dial: &Dial, parts: &Motion, bounds: Bounds<Pixels>, calm: bool, window: &mut Window) {
    let c = bounds.center();
    let at_rest = !dial.lit && dial.pulse != Pulse::NeedsYou;
    // The route of the last calls, under everything: dashes along the rim, the short way from
    // station to station, fading over a minute. Never across the centre.
    for pair in dial.route.windows(2) {
        let (from, to) = (angle(pair[0].station), angle(pair[1].station));
        let sweep = (to - from + PI).rem_euclid(TAU) - PI;
        let fade = 1. - pair[1].age_ms as f32 / activity::ROUTE_MS as f32;
        let alpha = if calm { 0.45 } else { 0.6 * fade.clamp(0., 1.) };
        let dashes = ((sweep.abs() * STATIONS / 2.5) as usize).max(1);
        let mut path = PathBuilder::stroke(px(1.));
        for i in (0..dashes).step_by(2) {
            let step = |i: usize| from + sweep * i as f32 / dashes as f32;
            path.move_to(at(c, STATIONS, step(i)));
            path.line_to(at(c, STATIONS, step(i + 1)));
        }
        stroke(window, path, colour(theme::station(pair[1].station)).opacity(alpha));
    }
    // Spokes: heavier for more use, decayed. Quieter at rest.
    for spot in dial.stations.iter().filter(|spot| spot.calls > 0) {
        let a = angle(spot.station);
        let mut path = PathBuilder::stroke(px(0.75 + 2.25 * spot.weight));
        path.move_to(at(c, SPOKE.0, a));
        path.line_to(at(c, SPOKE.1, a));
        let alpha = (0.15 + 0.55 * spot.weight) * if at_rest { 0.6 } else { 1. };
        stroke(window, path, colour(theme::station(spot.station)).opacity(alpha));
    }
    // Stations: a faint dot if unused; lit ones bright, with their time as arcs.
    // Each dot's radius is its spring's: larger for more calls at once, settling on a change.
    for (spot, dot) in dial.stations.iter().zip(&parts.dots) {
        let (a, hue) = (angle(spot.station), colour(theme::station(spot.station)));
        let (p, r) = (at(c, STATIONS, a), dot.at.max(0.5));
        match (spot.lit, spot.calls) {
            (true, _) => {
                // A call in progress blinks and swells, with a halo that widens as it fades.
                let beat = (!calm && motion::pulsing(spot, dial.timed)).then(|| parts.pulse());
                if let Some(beat) = beat {
                    disc(window, p, r + 1.5 + 3. * beat, hue.opacity(0.28 * (1. - beat)));
                }
                let beat = beat.unwrap_or(0.);
                disc(window, p, r + motion::PULSE_GROW * beat, hue.opacity(1. - 0.3 * beat));
                if let Some(ms) = spot.for_ms {
                    let inner = (ms as f32 / activity::INNER_MS as f32).min(1.);
                    let outer = (ms as f32 / activity::OUTER_MS as f32).min(1.);
                    stroke(window, arc(1.2, p, r + 2., 0., TAU * inner.max(0.02)), hue.opacity(0.9));
                    stroke(window, arc(1., p, r + 4., 0., TAU * outer.max(0.02)), hue.opacity(0.5));
                }
            }
            (false, 0) => disc(window, p, r, colour(theme::BORDER_STRONG)),
            (false, _) => disc(window, p, r, hue.opacity(if at_rest { 0.55 } else { 0.8 })),
        }
        if spot.failing > 0 {
            disc(window, at(c, STATIONS + 6.5, a), 1.8, colour(theme::ERROR));
        }
    }
    // The context ring: a track, then the part used, from the top.
    stroke(window, arc(2.5, c, RING, 0., TAU), colour(theme::BORDER));
    if let Some(used) = dial.context.filter(|used| *used > 0.) {
        stroke(window, arc(2.5, c, RING, 0., TAU * used), colour(theme::level(dial.level)));
    }
    // The centre: the agent, in its state's colour.
    disc(window, c, CENTRE, colour(theme::BACKGROUND));
    disc(window, c, CENTRE, colour(theme::pulse(dial.pulse)).opacity(0.16));
}

/// Paints the marker: the context ring, a dot for a station in use, a red one for a failure.
fn paint_marker(dial: &Dial, bounds: Bounds<Pixels>, window: &mut Window) {
    let c = bounds.center();
    stroke(window, arc(1.5, c, 7.5, 0., TAU), colour(theme::BORDER_STRONG));
    if let Some(used) = dial.context.filter(|used| *used > 0.) {
        stroke(window, arc(1.5, c, 7.5, 0., TAU * used), colour(theme::level(dial.level)));
    }
    if dial.lit {
        disc(window, at(c, 7.5, TAU / 8.), 2.2, colour(theme::TEXT));
    }
    if dial.failed {
        disc(window, at(c, 7.5, TAU * 3. / 8.), 2.2, colour(theme::ERROR));
    }
}

impl Workspace {
    /// The shown conversation's dial, if it has one: a native transcript of its own.
    fn dial_of(&self, subject: &Subject) -> Option<Dial> {
        if !self.is_native(subject) {
            return None;
        }
        Some(self.model.chat(subject)?.transcript.dial(crate::now_ms()))
    }

    fn glyph(&self, subject: &Subject, dial: &Dial, text: f32) -> Div {
        let glyph = self.model.runner_of(subject).map_or("•", |runner| runner.spec().glyph);
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(text))
            .text_color(rgb(theme::pulse(dial.pulse)))
            .child(glyph)
    }

    /// The centre of the open dial: the runner's picture in the state's colour, turned by
    /// `angle` about its upright axis (flat: narrower, mirrored and dimmer from behind).
    fn icon(&self, subject: &Subject, dial: &Dial, angle: f32) -> Div {
        let Some(runner) = self.model.runner_of(subject) else { return self.glyph(subject, dial, 13.) };
        let (width, light) = motion::facing(angle);
        // Never quite no width: a line, edge on.
        let width = width.abs().max(0.12).copysign(width);
        let tone = colour(theme::pulse(dial.pulse)).opacity(light);
        let icon = svg().path(runner.spec().icon).size(px(14.)).text_color(tone);
        let icon = icon.when(angle != 0., |icon| icon.with_transformation(Transformation::scale(size(width, 1.))));
        div().absolute().inset_0().flex().items_center().justify_center().child(icon)
    }

    /// The marker as drawn: the ring, the dots, the runner's glyph in the state's colour.
    fn marker_face(&self, subject: &Subject, dial: &Dial) -> Div {
        let drawn = dial.clone();
        let rings = canvas(|_, _, _| {}, move |bounds, _, window, _| paint_marker(&drawn, bounds, window));
        let face = div().relative().flex_none().size(px(MARK));
        face.child(rings.absolute().size_full()).child(self.glyph(subject, dial, 9.))
    }

    /// The agent chip (`Model::agent_chip`) for the end of a `relative` row `height` tall, as
    /// (the badge alone, over the row's end; the whole chip, in the room the row has left).
    /// Neither takes room of its own: nothing in the row moves. The badge goes first.
    pub(crate) fn agent_chip(&self, height: f32, cx: &Context<Self>) -> Option<(Div, Div)> {
        let chip = self.model.agent_chip()?;
        let needs = chip.state == Pulse::NeedsYou;
        let colour = if needs || chip.state == Pulse::Error { theme::pulse(chip.state) } else { theme::MUTED };
        let tip: gpui::SharedString = format!("{}\nOpen the conversation (⌘⇧L)", chip.line).into();
        let pill = |name: &'static str| {
            let marker = self.dial_of(&chip.task).map(|dial| self.marker_face(&chip.task, &dial));
            // A task's badge; a session's, or one drawn in a terminal, its state's dot.
            let badge = chip.task.task().and_then(|id| self.task_badge(id, MARK));
            let dot = (badge.is_none() && marker.is_none()).then(|| crate::home::dot(theme::pulse(chip.state)));
            let (tip, space) = (tip.clone(), chip.space);
            div()
                .id(name)
                .relative()
                .flex_none()
                .h(px(CHIP))
                .px_1()
                .flex()
                .items_center()
                .gap_1()
                .rounded_md()
                .border_1()
                .border_color(rgb(if needs { colour } else { theme::BORDER }))
                .bg(rgb(theme::BACKGROUND))
                .text_xs()
                .text_color(rgb(colour))
                .cursor_pointer()
                .hover(|s| s.bg(rgb(theme::SELECTED)))
                .children(badge)
                .children(marker)
                .children(dot)
                .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone()))))
                .on_click(cx.listener(move |this, _, window, cx| {
                    drop(this.dispatch(Action::SpaceClick { index: space }, this.input, window, cx))
                }))
        };
        // The badge alone, always at the row's end, under the whole chip when that fits.
        let short_pill = pill("agent-chip").child(self.controls.mark("agent-chip"));
        // Opaque to the row's end: a tab under it does not show beside it.
        let short = div().absolute().top_0().bottom_0().right_0().pl_1().pr(px(CHIP_EDGE)).bg(rgb(theme::BACKGROUND));
        let short = short.flex().items_center().child(short_pill);
        // The whole chip is second in a row that wraps: with no room, it goes to a line not drawn.
        let seen = self.chip_line.clone();
        let note = move |bounds: Bounds<Pixels>, window: &mut Window, _: &mut gpui::App| {
            let shown = bounds.intersect(&window.content_mask().bounds);
            seen.set((shown.size.height > px(1.) && shown.size.width > px(1.)).then_some(shown));
        };
        let line = div().max_w(px(CHIP_LINE)).pr_1().truncate().child(chip.line.clone());
        let full = pill("agent-chip-line").child(line).child(canvas(note, |_, _, _, _| {}).absolute().size_full());
        let full = div().flex_none().h(px(height)).pl_1().pr(px(CHIP_EDGE)).flex().items_center().child(full);
        let room = div().flex_1().min_w_0().h(px(height)).flex().flex_wrap().content_start().justify_end();
        Some((short, room.overflow_hidden().child(div().w_0().h(px(height))).child(full)))
    }

    /// The room the top row keeps for the chip's badge while the chip is there (a space of one
    /// tab has no second row): the badge, the marker of a native one, and its edges.
    pub(crate) fn chip_reserve(&self) -> f32 {
        let Some(chip) = self.model.agent_chip() else { return 0. };
        if self.model.space_tabs(self.model.space()).len() > 1 {
            return 0.;
        }
        let both = chip.task.task().is_some() && self.dial_of(&chip.task).is_some();
        10. + MARK + if both { MARK + 4. } else { 0. } + 2. * CHIP_EDGE
    }

    /// What the state reply says of the agent chip: the core's, and where it was drawn (`rect`,
    /// the whole chip or the badge alone: `short`).
    pub(crate) fn chip_snapshot(&self) -> serde_json::Value {
        let Some(chip) = self.model.agent_chip() else { return serde_json::Value::Null };
        let (line, badge) = (self.chip_line.get(), self.controls.bounds("agent-chip"));
        let mut value = serde_json::to_value(&chip).unwrap_or_default();
        value["rect"] = line.or(badge).map(crate::rect).into();
        value["short"] = line.is_none().into();
        value
    }

    /// The marker beside the header's badge while the dial is collapsed; a click opens it.
    pub(crate) fn dial_marker(&self, subject: &Subject, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let dial = self.dial_of(subject).filter(|_| self.model.dial_collapsed(subject))?;
        let tip: gpui::SharedString = [dial.lines.now.clone(), dial.lines.context.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n")
            .into();
        Some(
            self.marker_face(subject, &dial)
                .id("dial-marker")
                .cursor_pointer()
                .child(self.controls.mark("talk:dial-marker".to_string()))
                .when(!tip.is_empty(), |d| d.tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone())))))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(
                    cx.listener(|this, _, window, cx| drop(this.dispatch(Action::DialToggle, this.input, window, cx))),
                ),
        )
    }

    /// The dial and its lines, between the header and the transcript, `width` points wide. None
    /// while collapsed: it costs no height then.
    pub(crate) fn dial_section(&self, subject: &Subject, width: f32, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let dial = self.dial_of(subject).filter(|_| !self.model.dial_collapsed(subject))?;
        let calm = calm();
        // Stepped once a frame drawn, here: a dial not on screen is not stepped and asks for none.
        let (parts, moving) = {
            let mut kept = self.dial_motion.borrow_mut();
            (kept.step(subject, &dial, calm), kept.moving)
        };
        let drawn = dial.clone();
        let paint = move |bounds: Bounds<Pixels>, _: (), window: &mut Window, _: &mut gpui::App| {
            paint_dial(&drawn, &parts, bounds, calm, window);
            // The next frame, only while something moves.
            if moving {
                window.request_animation_frame();
            }
        };
        let paint = canvas(|_, _, _| {}, paint);
        let half = (FACE.0 / 2., FACE.1 / 2.);
        // Each station's word, lit with it; fainter for one not used yet.
        let words = dial.stations.iter().map(|spot| {
            let (left, top, set) = word(spot.station);
            let tone = colour(if spot.lit { theme::TEXT } else { theme::MUTED });
            let word = div().absolute().left(px(left)).top(px(top)).w(px(WORD.0)).h(px(WORD.1)).flex().items_center();
            let word = match set {
                0. => word.justify_start(),
                1. => word.justify_end(),
                _ => word.justify_center(),
            };
            word.text_size(px(WORD_TEXT))
                .line_height(px(WORD.1))
                .text_color(tone.opacity(if spot.lit || spot.calls > 0 { 1. } else { 0.6 }))
                .child(spot.station.name())
        });
        // Hovers: each station names itself and its counts; the centre, the tokens.
        let spots = dial.stations.iter().map(|spot| {
            let p = (half.0 + STATIONS * angle(spot.station).sin(), half.1 - STATIONS * angle(spot.station).cos());
            let tip: gpui::SharedString = spot.tip().into();
            let name = format!("dial-station:{}", spot.station.name().to_lowercase());
            div()
                .id(gpui::ElementId::Name(name.into()))
                .absolute()
                .left(px(p.0 - 7.))
                .top(px(p.1 - 7.))
                .size(px(14.))
                .rounded_full()
                .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone()))))
        });
        let ring = dial.ring.clone().map(|tip| {
            let tip: gpui::SharedString = tip.into();
            div()
                .id("dial-ring")
                .absolute()
                .left(px(half.0 - RING - 3.))
                .top(px(half.1 - RING - 3.))
                .size(px((RING + 3.) * 2.))
                .rounded_full()
                .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone()))))
        });
        let face = div()
            .id("dial")
            .relative()
            .flex_none()
            .w(px(FACE.0))
            .h(px(FACE.1))
            .cursor_pointer()
            .child(paint.absolute().size_full())
            .child(self.icon(subject, &dial, parts.angle))
            .children(words)
            .child(self.controls.mark("talk:dial".to_string()))
            .children(ring)
            .children(spots)
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(
                cx.listener(|this, _, window, cx| drop(this.dispatch(Action::DialToggle, this.input, window, cx))),
            );
        // Narrow: the dial alone.
        let lines = (width >= activity::LINES_MIN_WIDTH).then(|| {
            let line = |text: Option<String>, colour: u32| {
                text.map(|text| div().truncate().text_xs().text_color(rgb(colour)).child(text))
            };
            div()
                .id("dial-lines")
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_0p5()
                .children(line(dial.lines.now.clone(), theme::TEXT))
                .children(
                    line(dial.lines.context.clone(), theme::level(dial.level))
                        .map(|d| d.when(dial.level == activity::Level::Calm, |d| d.text_color(rgb(theme::MUTED)))),
                )
                .children(line(dial.lines.turns.clone(), theme::MUTED))
        });
        Some(
            div()
                .id("dial-section")
                .flex_none()
                .px_3()
                .py_1()
                .flex()
                .items_center()
                .gap_3()
                .border_b_1()
                .border_color(rgb(theme::BORDER))
                .child(face)
                .children(lines),
        )
    }

    /// True if the shown dial changes with time alone: a station lit or the route fading. The
    /// app draws it again on a slow tick then, and never at rest.
    pub(crate) fn dial_moves(&self) -> bool {
        let Some(subject) = self.model.talk.as_ref().filter(|talk| talk.open).map(|talk| &talk.task) else {
            return false;
        };
        self.dial_of(subject)
            .filter(|_| !self.model.dial_collapsed(subject))
            .is_some_and(|dial| dial.lit || dial.route.len() > 1)
    }

    /// What the state reply says of a conversation's dial: its structure and counts.
    pub(crate) fn dial_snapshot(&self, subject: &Subject) -> serde_json::Value {
        let Some(dial) = self.model.chat(subject).map(|chat| chat.transcript.dial(crate::now_ms())) else {
            return serde_json::Value::Null;
        };
        let mut value = serde_json::to_value(&dial).unwrap_or_default();
        value["collapsed"] = self.model.dial_collapsed(subject).into();
        // Its last frame drawn asked for another: a dot pulses or settles, the glyph turns.
        let kept = self.dial_motion.borrow();
        value["animating"] = (kept.moving && kept.shown.as_ref().is_some_and(|shown| shown.0 == *subject)).into();
        // To three places: an f32 widened would say 0.9200000166893004.
        value["context"] = dial.context.map(|used| (f64::from(used) * 1000.).round() / 1000.).into();
        value
    }
}
