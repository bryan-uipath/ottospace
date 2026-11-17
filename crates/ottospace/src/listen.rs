//! Transcripts in the shell (`ottospace_core::listen`): ⌘⌃T, the pane header's `Transcribe` and
//! its indicator, the top row's mark, the drawer's lines, the sound and the file
//! (`listen_file`). The decisions are the core's. One source so far: a test run's fixture
//! (`OTTOSPACE_LISTEN_FIXTURE`, read only under `OTTOSPACE_E2E`), replayed as what was heard.
//!
//! What was heard reaches the core in batches on a fixed tick (`BATCH`), empty or not, and the
//! level meter (`listen_meter`, draw-only) is drawn on a fixed tick (`FRAME`): a reply's
//! `frame.seq` and `frame.number` rise at the same rate whether someone speaks or not.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use gpui::{AnyElement, Context, Div, Stateful, Window, actions, div, prelude::*, px, rgb};
use ottospace_core::listen::{self, Heard, Record, Row, Speaker, Status};
use ottospace_core::{Action, Effect, Origin, PaneId, theme};
use serde::Deserialize;

use crate::listen_meter::{Hearing, Meter};
use crate::{Workspace, config, listen_file, now_ms, rect};

// ⌘⌃T: a transcript of the focused pane, or the one on stopped.
actions!(ottospace, [Transcribe]);

/// What the shell says when a test run names no fixture: it never hears a real app or the mic.
const NO_FIXTURE: &str = "no audio source in a test run";
/// A pane narrower than this, as last drawn, has no transcript control in its header: the title
/// and × need the room. The top row's mark and the drawer still show one that is on.
const HEADER_ROOM: f32 = 240.;
/// How often the meter, the label and the clock are drawn while one is on, sound or not.
const FRAME: Duration = Duration::from_millis(125);
/// Every this many frames what was heard goes to the core (250 ms), whether anything was or not.
const BATCH: u32 = 2;

/// A fixture line: `{"after_ms":300,"from":"them","text":"made-up words","final":true}`, heard
/// `after_ms` after the line before it (or the streams' start). `{"after_ms":100,"level":0.3}`:
/// the sound's level from then (RMS, 0 to 1), for the meter.
#[derive(Clone, Debug, Deserialize)]
struct Said {
    #[serde(default)]
    after_ms: u64,
    #[serde(default)]
    level: Option<f32>,
    #[serde(flatten)]
    heard: Option<Heard>,
}

/// The fixture being replayed for transcript `id`.
pub(crate) struct Feed {
    id: String,
    lines: VecDeque<Said>,
    /// When the next line is heard; none while paused.
    due: Option<Instant>,
    /// Ms into this run of the streams: a resume starts from 0, as new streams would.
    run_ms: u64,
    paused: bool,
    /// Heard since the last batch.
    pending: Vec<Heard>,
}

/// The file of transcript `id`. None kept in a test run with no folder of its own.
pub(crate) struct File {
    id: String,
    /// `Err`: it could not be made, and why, for the start to fail with.
    writer: Result<listen_file::Writer, String>,
}

/// `OTTOSPACE_LISTEN_FIXTURE`, in a test run only.
pub(crate) fn fixture_path() -> Option<String> {
    std::env::var("OTTOSPACE_LISTEN_FIXTURE").ok().filter(|_| std::env::var_os("OTTOSPACE_E2E").is_some())
}

fn read_fixture(path: &str) -> Result<VecDeque<Said>, String> {
    let text = std::fs::read_to_string(path).map_err(|why| format!("the fixture: {why}"))?;
    let lines = text.lines().filter(|line| !line.trim().is_empty());
    lines.map(|line| serde_json::from_str(line).map_err(|why| format!("the fixture: {why}"))).collect()
}

/// A transcript's id: its start time and a count, so two in one millisecond differ.
fn new_id(at: u64) -> String {
    static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!("t{at}-{}", COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

/// A small button of the indicator: `Pause`, `Stop`, `×`.
fn button(id: impl Into<gpui::ElementId>, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px_1()
        .rounded_sm()
        .cursor_pointer()
        .text_color(rgb(theme::MUTED))
        .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)))
        .child(label)
}

/// The red dot: full while on, hollow while paused.
fn dot(paused: bool) -> Div {
    let dot = div().flex_none().size(px(8.)).rounded_full().border_1().border_color(rgb(theme::RECORDING));
    if paused { dot } else { dot.bg(rgb(theme::RECORDING)) }
}

/// The red dot pulsing with the sound: a halo as wide as the level.
fn pulse(level: f32) -> Div {
    let halo = 8. + 6. * level.clamp(0., 1.);
    let colour = gpui::Rgba { a: 0.45, ..gpui::rgb(theme::RECORDING) };
    let ring = div().flex_none().size(px(halo)).rounded_full().bg(colour).flex().items_center().justify_center();
    div().flex_none().size(px(14.)).flex().items_center().justify_center().child(ring.child(dot(false)))
}

/// The grey dot of a transcript whose sound stopped coming.
fn grey_dot() -> Div {
    div().flex_none().size(px(8.)).rounded_full().bg(rgb(theme::MUTED))
}

/// Five bars, lit up to the level.
fn bars(level: f32) -> Div {
    let lit = (level.clamp(0., 1.) * 5.).ceil() as usize;
    div().flex_none().h(px(12.)).flex().items_end().gap(px(1.5)).children((0..5).map(|i| {
        let colour = if i < lit { theme::RECORDING } else { theme::BORDER };
        div().w(px(3.)).h(px(4. + 2. * i as f32)).rounded_sm().bg(rgb(colour))
    }))
}

impl Workspace {
    /// ⌘⌃T, from the key, the menu or the palette.
    pub(crate) fn transcribe_key(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.listen_flush(window, cx);
        let at = now_ms();
        self.dispatch(Action::ListenToggle { id: new_id(at), at }, self.input, window, cx);
    }

    /// `Transcribe` in a pane's header.
    fn listen_start(&mut self, pane: PaneId, window: &Window, cx: &mut Context<Self>) {
        let (source, at) = (listen::Source::Pane(pane), now_ms());
        self.dispatch(Action::ListenStart { source, mic: false, id: new_id(at), at }, self.input, window, cx);
    }

    /// A click of the indicator's (or its sneak label): `start`, `pause`, `resume`, `stop`, `top`
    /// (the top row's mark) or `close` (the drawer's ×). False: no such control.
    pub(crate) fn click_listen(&mut self, kind: &str, name: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        let at = now_ms();
        let action = match kind {
            "listen-start" => {
                if let Ok(pane) = name.parse() {
                    self.listen_start(pane, window, cx);
                }
                return true;
            }
            "listen-pause" | "listen-stop" => {
                self.listen_flush(window, cx);
                if kind == "listen-pause" { Action::ListenPause { at } } else { Action::ListenStop { at } }
            }
            "listen-resume" => Action::ListenResume { at },
            "listen-top" => Action::ListenShow { on: true },
            "listen-close" => Action::ListenShow { on: false },
            _ => return false,
        };
        self.dispatch(action, self.input, window, cx);
        true
    }

    /// Carries out a transcript's effect: its streams open, pause, resume or close; its file is written.
    pub(crate) fn listen_effect(&mut self, effect: &Effect, window: &Window, cx: &mut Context<Self>) {
        match effect {
            Effect::ListenWrite { id, records } => self.listen_write(id, records),
            Effect::ListenOpen { id, .. } => self.listen_open(id.clone(), window, cx),
            Effect::ListenPauseStreams { id } => {
                if let Some(feed) = self.listen_feed.as_mut().filter(|feed| feed.id == *id) {
                    (feed.paused, feed.due) = (true, None);
                }
            }
            Effect::ListenResumeStreams { id } => {
                if let Some(feed) = self.listen_feed.as_mut().filter(|feed| feed.id == *id) {
                    (feed.paused, feed.run_ms) = (false, 0);
                    if let Some(meter) = &mut self.listen_meter {
                        meter.restart(Instant::now());
                    }
                }
            }
            Effect::ListenClose { id } => {
                if self.listen_feed.as_ref().is_some_and(|feed| feed.id == *id) {
                    (self.listen_feed, self.listen_meter) = (None, None);
                }
            }
            _ => {}
        }
    }

    /// The file's records: `Begin` makes it, an end or a failure is its last.
    fn listen_write(&mut self, id: &str, records: &[Record]) {
        for record in records {
            if let Record::Begin(head) = record {
                let writer = config::transcripts_dir()
                    .map(|dir| listen_file::begin(&dir, head.clone()).map_err(|why| format!("transcripts_dir: {why}")));
                self.listen_file = writer.map(|writer| File { id: id.into(), writer });
            }
        }
        let Some(file) = self.listen_file.as_ref().filter(|file| file.id == id) else { return };
        if let Ok(writer) = &file.writer {
            writer.send(records.iter().filter(|record| !matches!(record, Record::Begin(_))).cloned().collect());
        }
        if records.iter().any(|record| matches!(record, Record::End { .. } | Record::Failed)) {
            self.listen_file = None;
        }
    }

    /// Before the user's Pause or Stop: what was heard since the last batch goes to the core now,
    /// so none of it is lost. At the user's act, not when someone spoke.
    pub(crate) fn listen_flush(&mut self, window: &Window, cx: &mut Context<Self>) {
        if let Some(action) = self.listen_rest() {
            self.dispatch(action, Origin::System, window, cx);
        }
    }

    /// The batch of what was heard and not yet handed over, if one is on.
    fn listen_rest(&mut self) -> Option<Action> {
        let feed = self.listen_feed.as_mut()?;
        let lines = std::mem::take(&mut feed.pending);
        Some(Action::ListenHeard { id: feed.id.clone(), lines })
    }

    /// ottospace is quitting: the one on ends now, as when its app quits, with what was heard and
    /// what was being heard; its file is written to the end before the app goes.
    pub(crate) fn listen_quit(&mut self) {
        let Some(id) = self.model.listen.on.as_ref().map(|on| on.id.clone()) else { return };
        let mut effects: Vec<Effect> =
            self.listen_rest().map(|rest| self.model.update(rest.sent_by(Origin::System))).unwrap_or_default();
        let ended = Action::ListenEnded { id, why: listen::Ended::AppQuit, at: now_ms() };
        effects.extend(self.model.update(ended.sent_by(Origin::System)));
        let writer = self.listen_file.as_ref().map(|file| file.id.clone());
        for effect in &effects {
            if let Effect::ListenWrite { id, records } = effect {
                // The last write takes the file: then it is finished, not dropped.
                let last = records.iter().any(|record| matches!(record, Record::End { .. } | Record::Failed));
                if last && writer.as_deref() == Some(id.as_str()) {
                    if let Some(File { writer: Ok(writer), .. }) = self.listen_file.take() {
                        writer.send(records.clone());
                        writer.finish();
                    }
                } else {
                    self.listen_write(id, records);
                }
            }
        }
    }

    /// The streams of `id`: in a test run, its fixture's lines; else none yet. A task of its own,
    /// as the streams would be: what it hears goes to the core every `BATCH`, as `System` news.
    fn listen_open(&mut self, id: String, window: &Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let began = this.update_in(cx, |this, window, cx| this.listen_begin(&id, window, cx));
            if !began.unwrap_or(false) {
                return;
            }
            for frame in 1u32.. {
                cx.background_executor().timer(FRAME).await;
                let going =
                    this.update_in(cx, |this, window, cx| this.listen_step(&id, frame % BATCH == 0, window, cx));
                if !going.unwrap_or(false) {
                    return;
                }
            }
        })
        .detach();
    }

    /// The streams start, or say why not. True: they run.
    fn listen_begin(&mut self, id: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.model.listen.on.as_ref().is_none_or(|on| on.id != id) {
            return false;
        }
        let unwritable =
            self.listen_file.as_ref().filter(|file| file.id == id).and_then(|file| file.writer.as_ref().err());
        let lines = match (unwritable, fixture_path()) {
            (Some(why), _) => Err(why.clone()),
            (None, Some(path)) => read_fixture(&path),
            (None, None) if std::env::var_os("OTTOSPACE_E2E").is_some() => Err(NO_FIXTURE.to_string()),
            (None, None) => Err(listen::NO_SOURCE.to_string()),
        };
        match lines {
            Ok(lines) => {
                let pending = Vec::new();
                self.listen_feed = Some(Feed { id: id.into(), lines, due: None, run_ms: 0, paused: false, pending });
                self.listen_meter = Some(Meter::new(Instant::now()));
                self.dispatch(Action::ListenBegan { id: id.into(), at: now_ms() }, Origin::System, window, cx);
                true
            }
            Err(why) => {
                self.dispatch(Action::ListenFailed { id: id.into(), why: why.clone() }, Origin::System, window, cx);
                self.broke(format!("Transcript did not start: {why}"));
                false
            }
        }
    }

    /// One frame: the fixture's lines that are due join the batch (its levels go to the meter);
    /// on a `batch` frame the batch goes to the core, empty or not; the meter and the clock are
    /// drawn again. False: it ended.
    fn listen_step(&mut self, id: &str, batch: bool, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.model.listen.on.as_ref().is_none_or(|on| on.id != id) {
            return false;
        }
        let Some(feed) = self.listen_feed.as_mut().filter(|feed| feed.id == id) else { return false };
        let now = Instant::now();
        while !feed.paused
            && let Some(next) = feed.lines.front()
        {
            // The first after a start or a resume counts from now; each next from the one before.
            let due = *feed.due.get_or_insert(now + Duration::from_millis(next.after_ms));
            if due > now {
                break;
            }
            let Some(said) = feed.lines.pop_front() else { break };
            feed.run_ms = feed.run_ms.saturating_add(said.after_ms);
            if let (Some(level), Some(meter)) = (said.level, self.listen_meter.as_mut()) {
                meter.level(level, now);
            }
            if let Some(heard) = said.heard {
                feed.pending.push(Heard { start_ms: feed.run_ms, ..heard });
                if let Some(meter) = self.listen_meter.as_mut() {
                    meter.words(now);
                }
            }
            feed.due = feed.lines.front().map(|next| due + Duration::from_millis(next.after_ms));
        }
        if batch {
            let lines = std::mem::take(&mut feed.pending);
            self.dispatch(Action::ListenHeard { id: id.into(), lines }, Origin::System, window, cx);
        }
        cx.notify();
        true
    }

    /// The pane header's part: `Transcribe` where one could start, the indicator on the pane being
    /// heard. None elsewhere, and outside a test run (no source yet).
    pub(crate) fn listen_header(&self, pane: PaneId, cx: &mut Context<Self>) -> Option<AnyElement> {
        let width = self.frames.get(&pane).map(|frame| f32::from(frame.get().size.width));
        if width.is_some_and(|width| width > 1. && width < HEADER_ROOM) {
            return None;
        }
        if self.model.listen.lost == Some(pane) && self.model.listen.on.is_none() {
            return Some(self.listen_lost(cx));
        }
        let on = self.model.listen.on.as_ref().filter(|on| on.source.pane() == Some(pane));
        let Some(on) = on else {
            let start = button(("listen-start", pane as usize), "Transcribe")
                .child(self.controls.mark(format!("listen-start:{pane}")))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| this.listen_start(pane, window, cx)));
            return self.model.listen_offered(pane).then(|| start.into_any_element());
        };
        let paused = on.status == Status::Paused;
        let word = if paused { "Paused" } else { "Transcribing" };
        let (toggle, label) = if paused { ("listen-resume", "Resume") } else { ("listen-pause", "Pause") };
        // In a narrow header it gives way before ×: the words first, then Pause, then Stop. The top
        // row's mark and the drawer still show it.
        let row = div()
            .flex()
            .min_w_0()
            .overflow_hidden()
            .items_center()
            .gap_1()
            .child(if paused { dot(true) } else { pulse(self.listen_level()) })
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(rgb(theme::RECORDING))
                    .child(format!("{word} {}", self.listen_clock())),
            )
            .child(self.controls.mark("listen-header"))
            .child(self.listen_button("listen-stop", "Stop", "header", cx))
            // No pause before the streams run: there is nothing to stop yet.
            .when(on.status != Status::Starting, |d| d.child(self.listen_button(toggle, label, "header", cx)));
        Some(row.into_any_element())
    }

    /// A pane's header after its transcript's sound stopped coming: a grey dot, why, and × (the
    /// user's Stop) to put it away. The notice bar says more.
    fn listen_lost(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .min_w_0()
            .overflow_hidden()
            .items_center()
            .gap_1()
            .child(grey_dot())
            .child(div().min_w_0().truncate().text_color(rgb(theme::MUTED)).child("Transcript stopped: no sound"))
            .child(self.controls.mark("listen-lost"))
            .child(self.listen_button("listen-stop", "×", "lost", cx))
            .into_any_element()
    }

    /// The level the meter shows now, 0 to 1: none while paused or starting.
    fn listen_level(&self) -> f32 {
        let on = self.model.listen.on.as_ref().is_some_and(|on| on.status == Status::On);
        self.listen_meter.as_ref().filter(|_| on).map_or(0., |meter| meter.shown(Instant::now()))
    }

    /// What the label says now: none while paused or starting.
    fn listen_hearing(&self) -> Option<Hearing> {
        let on = self.model.listen.on.as_ref().is_some_and(|on| on.status == Status::On);
        self.listen_meter.as_ref().filter(|_| on).map(|meter| meter.hearing(Instant::now()))
    }

    /// One of the indicator's buttons, in the pane's header or the drawer (`place`).
    fn listen_button(
        &self,
        kind: &'static str,
        label: &'static str,
        place: &str,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        button(kind, label)
            .child(self.controls.mark(format!("{kind}:{place}")))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| {
                this.click_listen(kind, "", window, cx);
            }))
    }

    /// How long the one on has run: `12:04`.
    fn listen_clock(&self) -> String {
        listen::clock(self.model.listen.on.as_ref().map_or(0, |on| on.elapsed(now_ms())))
    }

    /// Points the top row keeps for its mark while one is on.
    pub(crate) fn listen_reserve(&self) -> f32 {
        if self.model.listen.on.is_some() { 72. } else { 0. }
    }

    /// The top row's mark, from every space: a red dot and the time. A click shows its pane and
    /// the drawer on it.
    pub(crate) fn render_listen_mark(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let on = self.model.listen.on.as_ref()?;
        let paused = on.status == Status::Paused;
        let mark = div()
            .id("listen-top")
            .flex_none()
            .h(px(22.))
            .px_2()
            .mx_1()
            .rounded_md()
            .flex()
            .items_center()
            .gap_1p5()
            .cursor_pointer()
            .text_xs()
            .text_color(rgb(theme::RECORDING))
            .hover(|s| s.bg(rgb(theme::SELECTED)))
            .child(dot(paused))
            .child(self.listen_clock())
            .child(self.controls.mark("listen-top:"))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| {
                this.click_listen("listen-top", "", window, cx);
            }));
        Some(mark.into_any_element())
    }

    /// The drawer on the transcript: its lines as they come, the last at the foot. Plain text,
    /// each line one line (`capture::plain` in the core): no markdown, no links.
    pub(crate) fn render_listen_view(&self, width: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let on = self.model.listen.on.as_ref().filter(|_| self.model.listen.shown)?;
        let source = on.source.pane().and_then(|pane| self.model.pane(pane)).map(|pane| self.pane_title(pane));
        let paused = on.status == Status::Paused;
        let (toggle, label) = if paused { ("listen-resume", "Resume") } else { ("listen-pause", "Pause") };
        let header = div()
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .text_sm()
            .child(if paused { dot(true) } else { pulse(self.listen_level()) })
            .child(div().flex_1().min_w_0().truncate().child(format!(
                "Transcript · {} · {}",
                source.filter(|s| !s.is_empty()).unwrap_or_else(|| "pane".into()),
                self.listen_clock()
            )))
            // What it hears now, for the eye alone: never in the state.
            .children(self.listen_hearing().map(|hearing| {
                let colour = if hearing == Hearing::Silent { theme::SNEAK } else { theme::MUTED };
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_xs()
                    .text_color(rgb(colour))
                    .child(bars(self.listen_level()))
                    .child(hearing.label())
                    .child(self.controls.mark(format!("listen-hearing:{}", hearing.id())))
            }))
            .when(on.status != Status::Starting, |d| d.child(self.listen_button(toggle, label, "view", cx).text_xs()))
            .child(self.listen_button("listen-stop", "Stop", "view", cx).text_xs())
            .child(self.listen_button("listen-close", "×", "view", cx));
        let speaker = |from: Speaker| match from {
            Speaker::Them => ("them", theme::PRIMARY),
            Speaker::Me => ("me", theme::PICK),
        };
        let line = |time: String, from: Speaker, text: String, volatile: bool| {
            let (word, colour) = speaker(from);
            div()
                .flex()
                .gap_2()
                .text_xs()
                .child(div().flex_none().w(px(44.)).text_color(rgb(theme::MUTED)).child(time))
                .child(div().flex_none().w(px(36.)).text_color(rgb(colour)).child(word))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(rgb(if volatile { theme::MUTED } else { theme::TEXT }))
                        .child(text),
                )
        };
        let rows = on.rows().map(|row| match row {
            Row::Heard { from, at, text } => line(listen::clock(*at), *from, text.clone(), false),
            Row::Paused { ms } => {
                div().text_xs().text_color(rgb(theme::MUTED)).child(format!("paused {}", listen::clock(*ms)))
            }
        });
        let volatile = on.volatile().map(|(from, text)| line(String::new(), *from, text.clone(), true));
        let empty = (on.rows().next().is_none() && on.volatile().next().is_none()).then(|| {
            let waiting = if on.status == Status::Starting { "Starting…" } else { "Nothing heard yet" };
            div().text_xs().text_color(rgb(theme::MUTED)).child(waiting)
        });
        // The last line at the foot: what does not fit goes off the top.
        let body = div()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .px_3()
            .py_2()
            .flex()
            .flex_col()
            .justify_end()
            .gap_1()
            .children(empty)
            .children(rows)
            .children(volatile);
        let panel = div()
            .id("listen")
            .w(px(width))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(theme::RAISED))
            .border_l_1()
            .border_color(rgb(theme::BORDER))
            .child(header)
            .child(body)
            .child(self.controls.mark("listen-view"));
        Some(panel.into_any_element())
    }

    /// Test runs only: the drawer's rows (`from`, `final`, `text`; a pause as `paused_ms`), the
    /// volatile lines last. Null outside a test run: what was heard never reaches a client.
    pub(crate) fn listen_lines(&self) -> Option<serde_json::Value> {
        std::env::var_os("OTTOSPACE_E2E")?;
        let Some(on) = &self.model.listen.on else { return Some(serde_json::json!([])) };
        let rows = on.rows().map(|row| match row {
            Row::Heard { from, text, .. } => serde_json::json!({ "from": from, "final": true, "text": text }),
            Row::Paused { ms } => serde_json::json!({ "paused_ms": ms }),
        });
        let volatile =
            on.volatile().map(|(from, text)| serde_json::json!({ "from": from, "final": false, "text": text }));
        Some(rows.chain(volatile).collect())
    }

    /// Test runs only: where each of a transcript's controls was drawn, by id (`listen-start:3`,
    /// `listen-header`, `listen-stop:header`, `listen-pause:view`, `listen-top:`, `listen-view`).
    /// Null outside a test run.
    pub(crate) fn listen_rects(&self) -> Option<BTreeMap<String, [f32; 4]>> {
        std::env::var_os("OTTOSPACE_E2E")?;
        Some(self.controls.drawn_with("listen").into_iter().map(|(id, bounds)| (id, rect(bounds))).collect())
    }
}
