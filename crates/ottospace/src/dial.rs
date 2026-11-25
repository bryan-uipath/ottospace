//! The agent's lights over a native conversation: what the core's `activity::Dial` says, drawn
//! in the drawer's header (canvas J6): the picture, one status sentence, a light per tool
//! station with a bar for its use, the context's number; and the context bar on the header's
//! bottom edge. Collapsed (⌘⇧A), an 18-point marker beside the badge instead. Nothing moves: the
//! app draws again on a change and on a slow tick while a station is lit.

use std::f32::consts::{PI, TAU};

use gpui::{
    AnyView, Bounds, Context, Div, Hsla, PathBuilder, Pixels, Point, Stateful, Window, canvas, div, fill, point,
    prelude::*, px, relative, rgb, size,
};
use ottospace_core::activity::{Dial, Level, Pulse, Station};
use ottospace_core::talk::Subject;
use ottospace_core::transcript::Face;
use ottospace_core::{Action, theme};

use crate::{Tooltip, Workspace};

/// The marker's side.
const MARK: f32 = 18.;
/// The agent chip: its height, and the most its line takes.
const CHIP: f32 = 24.;
const CHIP_LINE: f32 = 220.;
/// Between the chip and the row's end.
const CHIP_EDGE: f32 = 8.;
/// The picture's side: as tall as the status line and the lights beside it.
const PICTURE: f32 = 42.;
/// A light's side, and its use bar's height.
const LIGHT: f32 = 20.;
const BAR: f32 = 2.;
/// The faces under the picture: how many, and a face's side; the list of them all, its width.
const FACES: usize = 3;
const FACE: f32 = 16.;
const SWITCHER: f32 = 300.;
/// The context bar's height, on the header's bottom edge.
const CONTEXT_BAR: f32 = 3.;
/// The tool stations, in the lights' order, each with its glyph.
const LIGHTS: [(Station, &str); 9] = [
    (Station::Files, "▤"),
    (Station::Shell, "❯"),
    (Station::Network, "◍"),
    (Station::Mcp, "⌁"),
    (Station::Apps, "▣"),
    (Station::Agents, "⁂"),
    (Station::Skills, "✦"),
    (Station::Memory, "◈"),
    (Station::Hooks, "↪"),
];

fn at(centre: Point<Pixels>, r: f32, angle: f32) -> Point<Pixels> {
    centre + point(px(r * angle.sin()), px(-r * angle.cos()))
}

pub(crate) fn colour(rgb_: u32) -> Hsla {
    Hsla::from(rgb(rgb_))
}

/// An arc as a polyline, `sweep` radians clockwise from `from`.
pub(crate) fn arc(width: f32, centre: Point<Pixels>, r: f32, from: f32, sweep: f32) -> PathBuilder {
    let mut path = PathBuilder::stroke(px(width));
    let steps = ((sweep.abs() / (PI / 32.)).ceil() as usize).max(2);
    for i in 0..=steps {
        let p = at(centre, r, from + sweep * i as f32 / steps as f32);
        if i == 0 { path.move_to(p) } else { path.line_to(p) }
    }
    path
}

pub(crate) fn stroke(window: &mut Window, path: PathBuilder, colour: Hsla) {
    if let Ok(path) = path.build() {
        window.paint_path(path, colour);
    }
}

pub(crate) fn disc(window: &mut Window, centre: Point<Pixels>, r: f32, colour: Hsla) {
    let bounds = Bounds::centered_at(centre, size(px(r * 2.), px(r * 2.)));
    window.paint_quad(fill(bounds, colour).corner_radii(px(r)));
}

/// No motion: the system's Reduce Motion. A test run's is `OTTOSPACE_REDUCE_MOTION`, never the machine's.
pub(crate) fn calm() -> bool {
    match std::env::var_os("OTTOSPACE_E2E") {
        Some(_) => std::env::var_os("OTTOSPACE_REDUCE_MOTION").is_some(),
        None => ottospace_mac::meeting::reduce_motion(),
    }
}

/// The context's number at the lights' end: shown from 70%, "compacts soon" from 90%; below,
/// on the bar's hover only.
/// A sub-agent's face: its type's first letter in a ring of the Agents colour, dimmer once done.
pub(crate) fn face_mark(face: &Face, side: f32) -> Div {
    let hue = colour(theme::station(Station::Agents));
    let running = face.ended.is_none();
    let letter = face.kind.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    div()
        .relative()
        .flex_none()
        .size(px(side))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(hue.opacity(if running { 1. } else { 0.4 }))
        .text_size(px((side * 0.56).max(9.)))
        .text_color(hue.opacity(if running { 1. } else { 0.5 }))
        .child(letter)
}

fn context_words(dial: &Dial) -> Option<String> {
    let used = (dial.context? * 100.).round() as u32;
    match dial.level {
        Level::Red => Some(format!("{used}% · compacts soon")),
        Level::Amber => Some(format!("{used}%")),
        Level::Calm => None,
    }
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
        let tip: gpui::SharedString = format!("{}\nOpen the conversation (⌘⇧R)", chip.line).into();
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
        if self.model.space_tabs(self.model.shown_space()).len() > 1 {
            return 0.;
        }
        self.chip_badge()
    }

    /// The room the chip's badge covers at a row's end, 0 with no chip: a row that scrolls ends
    /// before it, so its last item, fade and bar are not under it.
    pub(crate) fn chip_badge(&self) -> f32 {
        let Some(chip) = self.model.agent_chip() else { return 0. };
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
        let tip: gpui::SharedString =
            [dial.lines.now.clone(), dial.ring.clone()].into_iter().flatten().collect::<Vec<_>>().join("\n").into();
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

    /// The header's third row while the lights are open: the picture (the task's badge, else the
    /// runner's glyph) and, beside it, the status sentence over the lights, which stretch to the
    /// right edge with the context's number at their end. A light is its station's colour once
    /// used (dimmer when not lit), filled while a call runs there, with a bar under it for its
    /// use; a red dot for a failure since the user's last message.
    pub(crate) fn talk_lights(&self, subject: &Subject, cx: &mut Context<Self>) -> Option<Div> {
        let dial = self.dial_of(subject).filter(|_| !self.model.dial_collapsed(subject))?;
        let picture = match subject.task().and_then(|id| self.task_badge(id, PICTURE)) {
            Some(badge) => badge,
            None => div()
                .relative()
                .size(px(PICTURE))
                .rounded(px(PICTURE / 4.))
                .bg(rgb(theme::SELECTED))
                .child(self.glyph(subject, &dial, 18.)),
        };
        let picture = picture
            .id("talk-picture")
            .flex_none()
            .cursor_pointer()
            .child(self.controls.mark("talk:dial".to_string()))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(
                cx.listener(|this, _, window, cx| drop(this.dispatch(Action::DialToggle, this.input, window, cx))),
            );
        let tip: gpui::SharedString = [dial.lines.now.clone(), dial.lines.turns.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n")
            .into();
        let now = dial.lines.now.clone().unwrap_or_default();
        let timer = dial.lines.timer.clone().map(|timer| format!(" · {timer}"));
        let status = div()
            .id("talk-status")
            .flex()
            .min_w_0()
            .text_xs()
            .line_height(px(17.))
            .child(div().min_w_0().truncate().text_color(rgb(theme::TEXT)).child(now))
            .children(timer.map(|timer| div().flex_none().text_color(rgb(theme::MUTED)).child(timer)))
            .when(!tip.is_empty(), |d| d.tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone())))));
        let lights = LIGHTS.iter().filter_map(|(station, glyph)| {
            let spot = dial.spot(*station)?;
            let hue = theme::station(*station);
            let used = spot.calls > 0;
            let ink =
                if used { colour(hue).opacity(if spot.lit { 1. } else { 0.6 }) } else { colour(theme::BORDER_STRONG) };
            let bar = div()
                .absolute()
                .bottom_0()
                .left(px(3.))
                .h(px(BAR))
                .w(px((LIGHT - 6.) * spot.weight.clamp(0., 1.)))
                .rounded_full()
                .bg(colour(hue).opacity(0.7));
            let failed = (spot.failing > 0)
                .then(|| div().absolute().top_0().right_0().size(px(4.)).rounded_full().bg(rgb(theme::ERROR)));
            let tip: gpui::SharedString = spot.tip().into();
            let name = format!("light:{}", station.name().to_lowercase());
            Some(
                div()
                    .id(gpui::ElementId::Name(name.clone().into()))
                    .relative()
                    .flex_none()
                    .size(px(LIGHT))
                    .rounded(px(5.))
                    .flex()
                    .justify_center()
                    .pt(px(3.))
                    .text_size(px(11.))
                    .line_height(px(11.))
                    .text_color(ink)
                    .when(spot.lit, |d| d.bg(rgb(theme::SELECTED)))
                    .child(*glyph)
                    .when(used, |d| d.child(bar))
                    .children(failed)
                    .child(self.controls.mark(format!("talk:{name}")))
                    .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone())))),
            )
        });
        let number = context_words(&dial).map(|words| {
            div().min_w_0().truncate().pl_1().text_xs().text_color(rgb(theme::level(dial.level))).child(words)
        });
        let row = div().flex().items_center().justify_between().gap(px(2.)).min_w_0().children(lights).children(number);
        let column = div().flex_1().min_w_0().flex().flex_col().gap(px(5.)).child(status).child(row);
        let row = div().flex().items_center().gap_2().child(picture).child(column);
        Some(div().py_1().flex().flex_col().gap_1().child(row).children(self.faces_row(subject, None, cx)))
    }

    /// Under the picture while the sub-agents of `level` (`None`: the agent's own) run: a face
    /// each, at most `FACES` (running first, then the most recently active), then "+n". A face's
    /// hover: its type, task, state and time; its click shows its own calls. "+n" opens the list
    /// of them all, which floats under the row.
    pub(crate) fn faces_row(&self, subject: &Subject, level: Option<&str>, cx: &mut Context<Self>) -> Option<Div> {
        let chat = self.model.chat(subject)?;
        let faces = chat.transcript.faces(level);
        if faces.is_empty() {
            return None;
        }
        let now = crate::now_ms();
        let say = move |face: &Face| {
            let (state, from) = match face.ended {
                None => ("running", face.since),
                Some(at) => ("done", at),
            };
            let time = ottospace_core::progress::duration(now.saturating_sub(from));
            (format!("{} · {}", face.kind, face.about), format!("{state} · {time}"))
        };
        let shown = faces.iter().take(FACES).enumerate().map(|(at, face)| {
            let (what, when) = say(face);
            let tip: gpui::SharedString = format!("{what} · {when} · click to open").into();
            face_mark(face, FACE)
                .id(("talk-face", at))
                .cursor_pointer()
                .child(self.controls.mark(format!("talk:face:{at}")))
                .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone()))))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(Self::drill(face.id.clone(), cx))
        });
        let shown: Vec<_> = shown.collect();
        let rest = faces.len().saturating_sub(FACES);
        let more = (rest > 0).then(|| {
            let open = !chat.switcher;
            div()
                .id("talk-faces-more")
                .flex_none()
                .px_1()
                .rounded_sm()
                .cursor_pointer()
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .hover(|s| s.bg(rgb(theme::SELECTED)))
                .child(format!("+{rest}"))
                .child(self.controls.mark("talk:faces-more".to_string()))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| {
                    drop(this.dispatch(Action::TalkFaces { open }, this.input, window, cx))
                }))
        });
        // The list of them all, over what is under it: nothing in the header moves.
        let list = chat.switcher.then(|| {
            let rows = faces.iter().enumerate().map(|(at, face)| {
                let (what, when) = say(face);
                div()
                    .id(("talk-switch", at))
                    .px_2()
                    .py_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(theme::SELECTED)))
                    .child(face_mark(face, FACE))
                    .child(div().flex_1().min_w_0().truncate().text_xs().text_color(rgb(theme::TEXT)).child(what))
                    .child(div().flex_none().text_xs().text_color(rgb(theme::MUTED)).child(when))
                    .child(self.controls.mark(format!("talk:switch:{at}")))
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(Self::drill(face.id.clone(), cx))
            });
            let foot = div().px_2().py_1().text_xs().text_color(rgb(theme::MUTED)).child("Esc closes");
            let list = div()
                .absolute()
                .top_full()
                .left_0()
                .mt_1()
                .w(px(SWITCHER))
                .py_1()
                .rounded_md()
                .border_1()
                .border_color(rgb(theme::BORDER_STRONG))
                .bg(rgb(theme::RAISED))
                .children(rows)
                .child(foot);
            gpui::deferred(list)
        });
        Some(div().relative().flex().items_center().gap_1().children(shown).children(more).children(list))
    }

    /// A click that shows sub-agent `id`'s own calls, as whoever clicked.
    fn drill(id: String, cx: &mut Context<Self>) -> impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static {
        cx.listener(move |this: &mut Self, _: &gpui::ClickEvent, window, cx| {
            drop(this.dispatch(Action::TalkDrill { id: id.clone() }, this.input, window, cx))
        })
    }

    /// The context bar: 3 points on the header's bottom edge, the share of the context used in
    /// its level's colour; its hover says the tokens. None before the runner says a context.
    /// Tokens with no window yet (`8k tokens`): the track and its hover, no fill.
    pub(crate) fn context_bar(&self, subject: &Subject) -> Option<Stateful<Div>> {
        let dial = self.dial_of(subject)?;
        let tip: gpui::SharedString = match (&dial.ring, dial.context) {
            (Some(ring), _) => ring.clone().into(),
            (None, Some(used)) => format!("Context {}%", (used * 100.).round()).into(),
            (None, None) => return None,
        };
        let fill = dial.context.map(|used| {
            div().h_full().w(relative(used.clamp(0., 1.))).bg(rgb(match dial.level {
                Level::Calm => theme::BORDER_STRONG,
                level => theme::level(level),
            }))
        });
        Some(
            div()
                .id("talk-context")
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(-1.))
                .h(px(CONTEXT_BAR))
                .bg(rgb(theme::BORDER))
                .children(fill)
                .child(self.controls.mark("talk:context".to_string()))
                .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone())))),
        )
    }

    /// True while the open conversation's lights show a station lit: its timer needs a tick.
    pub(crate) fn lights_lit(&self) -> bool {
        let Some(subject) = self.model.talk.as_ref().filter(|talk| talk.open).map(|talk| &talk.task) else {
            return false;
        };
        self.dial_of(subject).filter(|_| !self.model.dial_collapsed(subject)).is_some_and(|dial| dial.lit)
    }

    /// What the state reply says of a conversation's dial: its structure and counts.
    pub(crate) fn dial_snapshot(&self, subject: &Subject) -> serde_json::Value {
        let Some(dial) = self.model.chat(subject).map(|chat| chat.transcript.dial(crate::now_ms())) else {
            return serde_json::Value::Null;
        };
        let mut value = serde_json::to_value(&dial).unwrap_or_default();
        value["collapsed"] = self.model.dial_collapsed(subject).into();
        // The faces of the level shown: how many shown and how many more; never their words. None
        // with the agent's lights collapsed; a sub-agent's view always has its own.
        let chat = self.model.chat(subject);
        let level = chat.and_then(|chat| chat.drilled()).map(|face| face.id.clone());
        let open = level.is_some() || !self.model.dial_collapsed(subject);
        let faces = chat.filter(|_| open).map(|chat| chat.transcript.faces(level.as_deref()).len()).unwrap_or(0);
        value["faces"] = faces.min(FACES).into();
        value["faces_more"] = faces.saturating_sub(FACES).into();
        // How deep the drawer shows (0: the agent; 1: a sub-agent's own calls), and its list open.
        value["drilled"] = chat.map_or(0, |chat| chat.depth()).into();
        value["switcher"] = chat.is_some_and(|chat| chat.switcher).into();
        // To three places: an f32 widened would say 0.9200000166893004.
        value["context"] = dial.context.map(|used| (f64::from(used) * 1000.).round() / 1000.).into();
        value
    }
}
