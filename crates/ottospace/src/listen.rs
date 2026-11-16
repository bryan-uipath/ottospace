//! Transcripts in the shell (`ottospace_core::listen`): ⌘⌃T, the pane header's `Transcribe` and
//! its indicator, the top row's mark, the drawer's lines, and the sound. The decisions are the
//! core's. Slice 1 has one source: a test run's fixture (`OTTOSPACE_LISTEN_FIXTURE`, read only
//! under `OTTOSPACE_E2E`), replayed as what was heard. No audio, no file.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use gpui::{AnyElement, Context, Div, Stateful, Window, actions, div, prelude::*, px, rgb};
use ottospace_core::listen::{self, Row, Speaker, Status};
use ottospace_core::{Action, Effect, Origin, PaneId, theme};
use serde::Deserialize;

use crate::{Workspace, now_ms, rect};

// ⌘⌃T: a transcript of the focused pane, or the one on stopped.
actions!(ottospace, [Transcribe]);

/// What the shell says when a test run names no fixture: it never hears a real app or the mic.
const NO_FIXTURE: &str = "no audio source in a test run";
/// The longest the clock goes undrawn while one is on.
const TICK: Duration = Duration::from_secs(1);

/// A fixture line: `{"after_ms":300,"from":"them","text":"made-up words","final":true}`, heard
/// `after_ms` after the line before it (or the streams' start).
#[derive(Clone, Debug, Deserialize)]
struct Said {
    #[serde(default)]
    after_ms: u64,
    from: Speaker,
    text: String,
    #[serde(rename = "final", default)]
    is_final: bool,
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

impl Workspace {
    /// ⌘⌃T, from the key, the menu or the palette.
    pub(crate) fn transcribe_key(&mut self, window: &Window, cx: &mut Context<Self>) {
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
            "listen-pause" => Action::ListenPause { at },
            "listen-resume" => Action::ListenResume { at },
            "listen-stop" => Action::ListenStop { at },
            "listen-top" => Action::ListenShow { on: true },
            "listen-close" => Action::ListenShow { on: false },
            _ => return false,
        };
        self.dispatch(action, self.input, window, cx);
        true
    }

    /// Carries out a transcript's effect: its streams open, pause, resume or close.
    pub(crate) fn listen_effect(&mut self, effect: &Effect, window: &Window, cx: &mut Context<Self>) {
        match effect {
            Effect::ListenOpen { id, .. } => self.listen_open(id.clone(), window, cx),
            Effect::ListenPauseStreams { id } => {
                if let Some(feed) = self.listen_feed.as_mut().filter(|feed| feed.id == *id) {
                    (feed.paused, feed.due) = (true, None);
                }
            }
            Effect::ListenResumeStreams { id } => {
                if let Some(feed) = self.listen_feed.as_mut().filter(|feed| feed.id == *id) {
                    (feed.paused, feed.run_ms) = (false, 0);
                }
            }
            Effect::ListenClose { id } => {
                if self.listen_feed.as_ref().is_some_and(|feed| feed.id == *id) {
                    self.listen_feed = None;
                }
            }
            _ => {}
        }
    }

    /// The streams of `id`: in a test run, its fixture's lines, one at a time; else none yet.
    /// A task of its own, as the streams would be: what it hears comes back as `System` news.
    fn listen_open(&mut self, id: String, window: &Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let began = this.update_in(cx, |this, window, cx| this.listen_begin(&id, window, cx));
            if !began.unwrap_or(false) {
                return;
            }
            loop {
                let Ok(Some(wait)) = this.update(cx, |this, _| this.listen_wait(&id)) else { return };
                cx.background_executor().timer(wait).await;
                let going = this.update_in(cx, |this, window, cx| this.listen_step(&id, window, cx));
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
        let lines = match fixture_path() {
            Some(path) => read_fixture(&path),
            None if std::env::var_os("OTTOSPACE_E2E").is_some() => Err(NO_FIXTURE.to_string()),
            None => Err(listen::NO_SOURCE.to_string()),
        };
        match lines {
            Ok(lines) => {
                self.listen_feed = Some(Feed { id: id.into(), lines, due: None, run_ms: 0, paused: false });
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

    /// How long until the next line or the clock's next second. None: it ended.
    fn listen_wait(&mut self, id: &str) -> Option<Duration> {
        let on = self.model.listen.on.as_ref().is_some_and(|on| on.id == id);
        let feed = self.listen_feed.as_mut().filter(|feed| on && feed.id == id)?;
        let now = Instant::now();
        let next = feed.lines.front().filter(|_| !feed.paused);
        if let (None, Some(next)) = (feed.due, next) {
            feed.due = Some(now + Duration::from_millis(next.after_ms));
        }
        Some(feed.due.map_or(TICK, |due| due.saturating_duration_since(now).min(TICK)))
    }

    /// A line heard if it is due, and the clock drawn again. False: it ended.
    fn listen_step(&mut self, id: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.model.listen.on.as_ref().is_none_or(|on| on.id != id) {
            return false;
        }
        cx.notify();
        let Some(feed) = self.listen_feed.as_mut().filter(|feed| feed.id == id) else { return false };
        if feed.paused || feed.due.is_none_or(|due| due > Instant::now()) {
            return true;
        }
        let Some(said) = feed.lines.pop_front() else { return true };
        feed.due = None;
        feed.run_ms = feed.run_ms.saturating_add(said.after_ms);
        let (start_ms, from, text, is_final) = (feed.run_ms, said.from, said.text, said.is_final);
        let heard = Action::ListenHeard { id: id.into(), from, start_ms, text, is_final };
        self.dispatch(heard, Origin::System, window, cx);
        true
    }

    /// The pane header's part: `Transcribe` where one could start, the indicator on the pane being
    /// heard. None elsewhere, and outside a test run (no source yet).
    pub(crate) fn listen_header(&self, pane: PaneId, cx: &mut Context<Self>) -> Option<AnyElement> {
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
            .child(dot(paused))
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
            .child(dot(paused))
            .child(div().flex_1().min_w_0().truncate().child(format!(
                "Transcript · {} · {}",
                source.filter(|s| !s.is_empty()).unwrap_or_else(|| "pane".into()),
                self.listen_clock()
            )))
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
