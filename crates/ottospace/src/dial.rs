//! The agent dial over a native conversation: what the core's `activity::Dial` says, drawn. The
//! 96-point dial and its lines under the header, or the 18-point marker beside the badge when
//! collapsed. Paths are built each paint (a few dozen segments); the app draws again on a change,
//! and on a slow tick while a station is lit or the route fades. See `docs/ai/agent-dial.md`.

use std::f32::consts::{PI, TAU};

use gpui::{
    AnyView, Bounds, Context, Div, Hsla, PathBuilder, Pixels, Point, Stateful, Window, canvas, div, fill, point,
    prelude::*, px, rgb, size,
};
use ottospace_core::activity::{self, Dial, Pulse, Station};
use ottospace_core::talk::Subject;
use ottospace_core::{Action, theme};

use crate::{Tooltip, Workspace};

/// The dial's side, and the marker's.
const SIDE: f32 = 96.;
const MARK: f32 = 18.;
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

/// Paints the dial in `bounds`. `calm`: no fading, states only (Reduce motion).
fn paint_dial(dial: &Dial, bounds: Bounds<Pixels>, calm: bool, window: &mut Window) {
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
    for spot in &dial.stations {
        let (a, hue) = (angle(spot.station), colour(theme::station(spot.station)));
        let p = at(c, STATIONS, a);
        match (spot.lit, spot.calls) {
            (true, _) => {
                disc(window, p, 3.5, hue);
                if let Some(ms) = spot.for_ms {
                    let inner = (ms as f32 / activity::INNER_MS as f32).min(1.);
                    let outer = (ms as f32 / activity::OUTER_MS as f32).min(1.);
                    stroke(window, arc(1.2, p, 5.5, 0., TAU * inner.max(0.02)), hue.opacity(0.9));
                    stroke(window, arc(1., p, 7.5, 0., TAU * outer.max(0.02)), hue.opacity(0.5));
                }
            }
            (false, 0) => disc(window, p, 1.2, colour(theme::BORDER_STRONG)),
            (false, _) => disc(window, p, 2.2, hue.opacity(if at_rest { 0.55 } else { 0.8 })),
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

    /// The marker beside the header's badge while the dial is collapsed; a click opens it.
    pub(crate) fn dial_marker(&self, subject: &Subject, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let dial = self.dial_of(subject).filter(|_| self.model.dial_collapsed(subject))?;
        let tip: gpui::SharedString = [dial.lines.now.clone(), dial.lines.context.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n")
            .into();
        let drawn = dial.clone();
        let rings = canvas(|_, _, _| {}, move |bounds, _, window, _| paint_marker(&drawn, bounds, window));
        Some(
            div()
                .id("dial-marker")
                .relative()
                .flex_none()
                .size(px(MARK))
                .cursor_pointer()
                .child(rings.absolute().size_full())
                .child(self.glyph(subject, &dial, 9.))
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
        let calm = ottospace_mac::meeting::reduce_motion();
        let drawn = dial.clone();
        let paint = canvas(|_, _, _| {}, move |bounds, _, window, _| paint_dial(&drawn, bounds, calm, window));
        let half = SIDE / 2.;
        // Hovers: each station names itself and its counts; the centre, the tokens.
        let spots = dial.stations.iter().map(|spot| {
            let p = (half + STATIONS * angle(spot.station).sin(), half - STATIONS * angle(spot.station).cos());
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
                .left(px(half - RING - 3.))
                .top(px(half - RING - 3.))
                .size(px((RING + 3.) * 2.))
                .rounded_full()
                .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone()))))
        });
        let face = div()
            .id("dial")
            .relative()
            .flex_none()
            .size(px(SIDE))
            .cursor_pointer()
            .child(paint.absolute().size_full())
            .child(self.glyph(subject, &dial, 13.))
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
        // To three places: an f32 widened would say 0.9200000166893004.
        value["context"] = dial.context.map(|used| (f64::from(used) * 1000.).round() / 1000.).into();
        value
    }
}
