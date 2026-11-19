//! Transcripts in the shell (`ottospace_core::listen`): ⌘⌃T, the pane header's `Transcribe` and
//! its dot, the dot on its tab or space while off show, the drawer's lines, the sound and the file
//! (`listen_file`). The decisions are the core's. The sound is the source pane's app's
//! (`listen_live`), or in a test run its fixture (`OTTOSPACE_LISTEN_FIXTURE`, read only under
//! `OTTOSPACE_E2E`), replayed as what was heard.
//!
//! What was heard reaches the core in batches on a fixed tick (`BATCH`), empty or not, and the
//! level meter (`listen_meter`, draw-only) is drawn on a fixed tick (`FRAME`): a reply's
//! `frame.seq` and `frame.number` rise at the same rate whether someone speaks or not.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use gpui::{AnyElement, App, Context, Div, Stateful, Window, actions, div, prelude::*, px, rgb};
use ottospace_core::listen::file::PastLine;
use ottospace_core::listen::{self, Heard, Record, Row, Speaker, Status};
use ottospace_core::row_menu::MenuOf;
use ottospace_core::{Action, Effect, Origin, PaneId, theme};
use serde::Deserialize;

use crate::listen_live::{self, Live};
use crate::listen_meter::{Hearing, Meter};
use crate::{Workspace, config, listen_file, now_ms, rect};

// ⌘⌃T: a transcript of the focused pane, or the one on stopped. The palette's Pause or Resume.
actions!(ottospace, [Transcribe, PauseTranscribing]);

/// What the shell says when a test run names no fixture: it never hears a real app or the mic.
const NO_FIXTURE: &str = "no audio source in a test run";
/// A pane narrower than this, as last drawn, has no `Transcribe` or lost mark in its header: the
/// title and × need the room. The red dot of one on stays down to `DOT_ROOM`.
const HEADER_ROOM: f32 = 240.;
/// Narrower still, the dot gives way too: × must stay in the header (measured on CI: 120 points).
const DOT_ROOM: f32 = 150.;
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
    /// The app's streams while they run (none while paused), and the app's pid: none for a fixture.
    live: Option<Live>,
    pid: Option<i32>,
}

/// Where a transcript's sound comes from.
enum Plan {
    Fixture(VecDeque<Said>),
    /// The app of this pid.
    App(i32),
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
    let said = |line: &str| -> Result<Said, String> {
        let said: Said = serde_json::from_str(line).map_err(|why| format!("the fixture: {why}"))?;
        // A flattened `Option` takes words it cannot read as none: `"from":"Them"` is a mistake, not a pause.
        match (&said.level, &said.heard) {
            (None, None) => Err(format!("the fixture: neither a level nor words: {line}")),
            _ => Ok(said),
        }
    };
    lines.map(said).collect()
}

/// `OTTOSPACE_LISTEN_APPS`, in a test run only: a JSON list of made-up apps the picker lists
/// (`{"pid":1,"name":"Zoom","windows":["Zoom Meeting"],"playing":true}`). None named: none.
fn fixture_apps() -> Vec<listen::SoundApp> {
    let path = std::env::var("OTTOSPACE_LISTEN_APPS").ok().filter(|_| std::env::var_os("OTTOSPACE_E2E").is_some());
    let text = path.and_then(|path| std::fs::read_to_string(path).ok()).unwrap_or_default();
    serde_json::from_str(&text).unwrap_or_default()
}

/// Today's files of `transcripts_dir` as last handed to the core (none since shown: the first
/// read goes, even if empty); the lines of the one open read-only, with its file's length and
/// change time when read; whether a read is under way.
#[derive(Default)]
pub(crate) struct Past {
    files: Option<Vec<(String, String)>>,
    open: Option<(String, listen_file::Stamp, Vec<PastLine>)>,
    reading: bool,
}

impl Past {
    /// The lines of `name`, if it is the one read.
    fn lines(&self, name: &str) -> &[PastLine] {
        self.open.as_ref().filter(|(open, ..)| open == name).map_or(&[], |(_, _, lines)| lines.as_slice())
    }
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

/// Stops an app's streams on another thread: ScreenCaptureKit's stop waits for the system.
fn stop_off_main(live: Option<Live>, cx: &App) {
    if let Some(live) = live {
        cx.background_executor().spawn(async move { drop(live) }).detach();
    }
}

/// A browser, whose every tab is heard with the one in the pane (`Transcribing (all tabs)`).
fn browser(bundle: &str) -> bool {
    const BROWSERS: [&str; 7] = [
        "com.google.Chrome",
        "com.apple.Safari",
        "org.mozilla.firefox",
        "com.microsoft.edgemac",
        "com.brave.Browser",
        "company.thebrowser.Browser",
        "com.vivaldi.Vivaldi",
    ];
    BROWSERS.contains(&bundle)
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

    /// The palette's and the menu bar's Pause or Resume: whichever the one on can do.
    pub(crate) fn pause_key(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.listen_pause(self.input, window, cx);
    }

    /// Pause or Resume as `origin`, as the core says fits (`Model::listen_pause_action`); what
    /// was heard before a pause goes to the core first.
    fn listen_pause(&mut self, origin: Origin, window: &Window, cx: &mut Context<Self>) {
        let Some(action) = self.model.listen_pause_action(now_ms()) else { return };
        if matches!(action, Action::ListenPause { .. }) {
            self.listen_flush(window, cx);
        }
        self.dispatch(action, origin, window, cx);
    }

    /// `Transcribe` in a pane's header.
    fn listen_start(&mut self, pane: PaneId, window: &Window, cx: &mut Context<Self>) {
        let (source, at) = (listen::Source::Pane(pane), now_ms());
        self.dispatch(Action::ListenStart { source, mic: false, id: new_id(at), at }, self.input, window, cx);
    }

    /// A click of the indicator's (or its sneak label): `start`, `resume`, `stop`, `menu` (the
    /// drawer's ⋯), `header` (the source pane's dot) or `dot` (its tab's or space's). False: no such control.
    pub(crate) fn click_listen(&mut self, kind: &str, name: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        let at = now_ms();
        let action = match kind {
            "listen-start" => {
                if let Ok(pane) = name.parse() {
                    self.listen_start(pane, window, cx);
                }
                return true;
            }
            "listen-stop" => {
                self.listen_flush(window, cx);
                Action::ListenStop { at }
            }
            "listen-resume" => Action::ListenResume { at },
            "listen-header" | "listen-dot" => Action::ListenShow { on: true },
            "listen-pick" => {
                self.listen_pick(window, cx);
                return true;
            }
            "listen-past" => {
                let at: Option<usize> = name.parse().ok();
                let past = at.and_then(|at| self.model.transcripts.today.get(at)).map(|past| past.name.clone());
                Action::ListenOpenPast { name: past }
            }
            "listen-back" => Action::ListenOpenPast { name: None },
            "listen-file" => {
                self.listen_open_file(window, cx);
                return true;
            }
            // The header's →, as ⌘⇧R.
            "listen-collapse" => Action::TalkToggle,
            // Only a label comes here (a click opens it itself): the keys on its first item.
            "listen-menu" => {
                self.open_row_menu(MenuOf::Transcript, true, window, cx);
                return true;
            }
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
            // Nothing is captured while paused: the app's streams stop.
            Effect::ListenPauseStreams { id } => {
                if let Some(feed) = self.listen_feed.as_mut().filter(|feed| feed.id == *id) {
                    (feed.paused, feed.due) = (true, None);
                    stop_off_main(feed.live.take(), cx);
                }
            }
            Effect::ListenResumeStreams { id } => {
                if let Some(feed) = self.listen_feed.as_mut().filter(|feed| feed.id == *id) {
                    (feed.paused, feed.run_ms) = (false, 0);
                    if let Some(meter) = &mut self.listen_meter {
                        meter.restart(Instant::now());
                    }
                    if let Some(pid) = feed.pid {
                        self.listen_resume_live(id.clone(), pid, window, cx);
                    }
                }
            }
            Effect::ListenClose { id } => self.listen_close(id, cx),
            _ => {}
        }
    }

    /// Its streams close for good.
    fn listen_close(&mut self, id: &str, cx: &App) {
        if self.listen_feed.as_ref().is_some_and(|feed| feed.id == id) {
            stop_off_main(self.listen_feed.take().and_then(|feed| feed.live), cx);
            self.listen_meter = None;
        }
    }

    /// A transcript a turned-off flag ended, outside a dispatch: its file's end, its streams' close.
    pub(crate) fn listen_ended(&mut self, effects: &[Effect], cx: &App) {
        for effect in effects {
            match effect {
                Effect::ListenWrite { id, records } => self.listen_write(id, records),
                Effect::ListenClose { id } => self.listen_close(id, cx),
                _ => {}
            }
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
            // Kept until written, for a quit right after to wait for.
            if let Some(File { writer: Ok(writer), .. }) = self.listen_file.take() {
                self.listen_ending.retain(|writer| !writer.done());
                self.listen_ending.push(writer);
            }
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
        let mut lines = std::mem::take(&mut feed.pending);
        // Its lines only: a failure waits for the next frame, which ends it.
        lines.extend(feed.live.as_ref().map(Live::take_lines).unwrap_or_default());
        Some(Action::ListenHeard { id: feed.id.clone(), lines })
    }

    /// ottospace is quitting: the one on ends now, as when its app quits, with what was heard and
    /// what was being heard; its file is written to the end before the app goes.
    pub(crate) fn listen_quit(&mut self) {
        std::mem::take(&mut self.listen_ending).into_iter().for_each(listen_file::Writer::finish);
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

    /// The streams of `id`: the source pane's app's sound, or in a test run its fixture's lines.
    /// A task of its own: what it hears goes to the core every `BATCH`, as `System` news.
    fn listen_open(&mut self, id: String, window: &Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let Ok(plan) = this.update(cx, |this, _| this.listen_plan(&id)) else { return };
            let began = match plan {
                Ok(Plan::Fixture(lines)) => Ok((lines, None, None)),
                Ok(Plan::App(pid)) => {
                    let ready = cx.background_executor().spawn(async { listen_live::ready() }).await;
                    let ready = match ready {
                        Ok(true) => Ok(()),
                        // The user's Start asks the system for its model: "Downloading speech model…".
                        Ok(false) => {
                            // Stopped while the system was asked: no download.
                            let still = this.update(cx, |this, cx| {
                                let on = this.model.listen.on.as_ref().is_some_and(|on| on.id == id);
                                if on {
                                    this.listen_downloading = Some(id.clone());
                                    cx.notify();
                                }
                                on
                            });
                            if still.unwrap_or(false) {
                                cx.background_executor().spawn(async { listen_live::install() }).await
                            } else {
                                Err("stopped".into())
                            }
                        }
                        Err(why) => Err(why),
                    };
                    // Stopped while the model came: nothing is captured.
                    let on = |this: &Workspace| this.model.listen.on.as_ref().is_some_and(|on| on.id == id);
                    let live = match ready {
                        Ok(()) if !this.update(cx, |this, _| on(this)).unwrap_or(false) => Err("stopped".into()),
                        Ok(()) => cx.background_executor().spawn(async move { Live::start(pid) }).await,
                        Err(why) => Err(why),
                    };
                    live.map(|live| (VecDeque::new(), Some(live), Some(pid)))
                }
                Err(why) => Err(why),
            };
            let began = this.update_in(cx, |this, window, cx| this.listen_begin(&id, began, window, cx));
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

    /// Where the sound of `id` comes from, or why there is none.
    fn listen_plan(&self, id: &str) -> Result<Plan, String> {
        let unwritable =
            self.listen_file.as_ref().filter(|file| file.id == id).and_then(|file| file.writer.as_ref().err());
        if let Some(why) = unwritable {
            return Err(why.clone());
        }
        if let Some(path) = fixture_path() {
            return read_fixture(&path).map(Plan::Fixture);
        }
        let source = self.model.listen.on.as_ref().filter(|on| on.id == id).map(|on| on.source);
        if let Some(listen::Source::App { pid }) = source {
            return match listen_live::refusal(pid) {
                Some(why) => Err(why.into()),
                None => Ok(Plan::App(pid)),
            };
        }
        match source.and_then(|source| source.pane()).and_then(|pane| self.hosted.get(&pane)) {
            Some(hosted) => match listen_live::refusal(hosted.pid) {
                Some(why) => Err(why.into()),
                None => Ok(Plan::App(hosted.pid)),
            },
            None if std::env::var_os("OTTOSPACE_E2E").is_some() => Err(NO_FIXTURE.to_string()),
            None => Err("its window is not in the pane".into()),
        }
    }

    /// The streams run (the fixture's lines, or the app's streams and its pid), or say why not.
    /// True: they run. One stopped meanwhile (a download is long) keeps nothing.
    fn listen_begin(
        &mut self,
        id: &str,
        began: Result<(VecDeque<Said>, Option<Live>, Option<i32>), String>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.listen_downloading.as_deref() == Some(id) {
            self.listen_downloading = None;
        }
        if self.model.listen.on.as_ref().is_none_or(|on| on.id != id) {
            stop_off_main(began.ok().and_then(|(_, live, _)| live), cx);
            return false;
        }
        match began {
            Ok((lines, live, pid)) => {
                let pending = Vec::new();
                let paused = false;
                self.listen_feed =
                    Some(Feed { id: id.into(), lines, due: None, run_ms: 0, paused, pending, live, pid });
                self.listen_meter = Some(Meter::new(Instant::now()));
                self.dispatch(Action::ListenBegan { id: id.into(), at: now_ms() }, Origin::System, window, cx);
                // The others in the call are not told by ottospace: the user is reminded to.
                let picked = |pid| self.model.listen.apps.iter().find(|app| app.pid == pid).map(|app| app.name.clone());
                let app = match self.model.listen.on.as_ref().map(|on| on.source) {
                    Some(listen::Source::App { pid }) => picked(pid),
                    _ => self.model.listen.pane().and_then(|pane| self.hosted.get(&pane)).map(|h| h.app.clone()),
                };
                if let Some(app) = app {
                    self.told = Some((format!("Transcribing {app}: let the others know."), Instant::now()));
                }
                true
            }
            Err(why) => {
                self.dispatch(Action::ListenFailed { id: id.into(), why: why.clone() }, Origin::System, window, cx);
                self.broke(format!("Transcript did not start: {why}"));
                false
            }
        }
    }

    /// New streams after a pause, counting from 0, once they run. One paused or stopped
    /// meanwhile keeps none.
    fn listen_resume_live(&mut self, id: String, pid: i32, window: &Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let wanted = |this: &Workspace| {
                this.listen_feed.as_ref().is_some_and(|feed| feed.id == id && !feed.paused && feed.live.is_none())
            };
            // Paused or stopped again meanwhile: nothing is captured.
            if !this.update(cx, |this, _| wanted(this)).unwrap_or(false) {
                return;
            }
            let live = cx.background_executor().spawn(async move { Live::start(pid) }).await;
            let _ = this.update_in(cx, |this, window, cx| match (wanted(this), live) {
                (true, Ok(live)) => {
                    if let Some(feed) = this.listen_feed.as_mut() {
                        feed.live = Some(live);
                    }
                }
                (true, Err(why)) => this.listen_stream_lost(&id, why, window, cx),
                (false, live) => stop_off_main(live.ok(), cx),
            });
        })
        .detach();
    }

    /// The sound stopped coming: the core ends it, and the bar says why.
    fn listen_stream_lost(&mut self, id: &str, why: String, window: &Window, cx: &mut Context<Self>) {
        // What was heard before it stopped is kept.
        self.listen_flush(window, cx);
        let ended = Action::ListenEnded { id: id.into(), why: listen::Ended::StreamFailed, at: now_ms() };
        self.dispatch(ended, Origin::System, window, cx);
        self.broke(format!("Transcript stopped: {why}"));
    }

    /// One frame: what the streams heard (or the fixture's lines that are due) joins the batch,
    /// the level goes to the meter; on a `batch` frame the batch goes to the core, empty or not;
    /// the meter and the clock are drawn again. False: it ended.
    fn listen_step(&mut self, id: &str, batch: bool, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.model.listen.on.as_ref().is_none_or(|on| on.id != id) {
            return false;
        }
        // An app heard in no pane: no pane closes when it quits, and paused it has no stream to
        // fail. Its stream's pid (none for a test run's fixture) is asked.
        let picked = self.model.listen.on.as_ref().is_some_and(|on| matches!(on.source, listen::Source::App { .. }));
        let pid = self.listen_feed.as_ref().filter(|feed| feed.id == id).and_then(|feed| feed.pid);
        if batch && picked && pid.is_some_and(|pid| !ottospace_mac::process::alive(pid)) {
            self.listen_flush(window, cx);
            let ended = Action::ListenEnded { id: id.into(), why: listen::Ended::AppQuit, at: now_ms() };
            self.dispatch(ended, Origin::System, window, cx);
            return false;
        }
        let Some(feed) = self.listen_feed.as_mut().filter(|feed| feed.id == id) else { return false };
        let now = Instant::now();
        if let Some(audio) = feed.live.as_mut().filter(|_| batch).and_then(|live| live.tick(now)) {
            cx.background_executor().spawn(async move { listen_live::refresh(&audio) }).detach();
        }
        if let Some(taken) = feed.live.as_ref().map(Live::take) {
            if let Some(meter) = self.listen_meter.as_mut() {
                if let Some(level) = taken.level {
                    meter.level(level, now);
                }
                if !taken.lines.is_empty() {
                    meter.words(now);
                }
            }
            feed.pending.extend(taken.lines);
            if let Some(why) = taken.failed {
                self.listen_stream_lost(id, why, window, cx);
                return false;
            }
        }
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
        let under = |room: f32| width.is_some_and(|width| width > 1. && width < room);
        let heard = self.model.listen.pane() == Some(pane);
        if under(DOT_ROOM) || (under(HEADER_ROOM) && !heard) {
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
        // A browser is heard whole: SCK filters by app, not by tab.
        let tabs = self.hosted.get(&pane).and_then(|hosted| hosted.bundle.as_deref()).is_some_and(browser);
        let tip = match (paused, tabs) {
            (true, _) => "Transcript paused",
            (false, true) => "Transcribing: every tab of this browser is heard",
            (false, false) => "Transcribing",
        };
        // The source's mark only: the red dot. The time and the controls are the drawer's; a
        // click shows it there.
        let row = div()
            .id("listen-header")
            .flex()
            .min_w_0()
            .overflow_hidden()
            .items_center()
            .px_1()
            .rounded_sm()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(theme::SELECTED)))
            .child(if paused { dot(true) } else { pulse(self.listen_level()) })
            .child(self.controls.mark("listen-header"))
            .tooltip(move |_, cx| gpui::AnyView::from(cx.new(|_| crate::Tooltip(tip.into()))))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| {
                this.click_listen("listen-header", "", window, cx);
            }));
        Some(row.into_any_element())
    }

    /// The red dot on the tab or space of the pane being heard, while off show (`Model::listen_dot`),
    /// at its corner (the caller places it). A click shows the pane and the drawer on it. Paused:
    /// hollow, as the header's.
    pub(crate) fn off_show_dot(&self, id: String, cx: &Context<Self>) -> Stateful<Div> {
        let paused = self.model.listen.on.as_ref().is_some_and(|on| on.status == Status::Paused);
        let tip = if paused { "Transcript paused here" } else { "Transcribing a pane here" };
        let dot = div().id(gpui::ElementId::Name(id.clone().into())).absolute().size(px(7.)).rounded_full().border_1();
        let dot = match paused {
            true => dot.border_color(rgb(theme::RECORDING)),
            false => dot.bg(rgb(theme::RECORDING)).border_color(rgb(theme::BACKGROUND)),
        };
        dot.cursor_pointer()
            .child(self.controls.mark(id))
            .tooltip(move |_, cx| gpui::AnyView::from(cx.new(|_| crate::Tooltip(tip.into()))))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                this.click_listen("listen-dot", "", window, cx);
            }))
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

    /// The menu bar's dot while one is on, and the user's choice in its menu: Stop, Pause or
    /// Resume, or Show. Not in a test run: it would sit in the user's own menu bar.
    pub(crate) fn listen_menu_bar(&mut self, window: &Window, cx: &mut Context<Self>) {
        if std::env::var_os("OTTOSPACE_E2E").is_some() {
            return;
        }
        // Pause or Resume as it fits, none while it starts.
        let pause = self.model.listen_pause_action(0).map(|action| matches!(action, Action::ListenResume { .. }));
        ottospace_mac::status_item::set(self.model.listen.on.as_ref().map(|_| "ottospace is transcribing"), pause);
        let at = now_ms();
        // Only the user's own mouse reaches a status item's menu.
        match ottospace_mac::status_item::take() {
            Some(ottospace_mac::status_item::Chosen::Stop) => {
                self.listen_flush(window, cx);
                self.dispatch(Action::ListenStop { at }, Origin::User, window, cx);
            }
            Some(ottospace_mac::status_item::Chosen::Pause) => self.listen_pause(Origin::User, window, cx),
            Some(ottospace_mac::status_item::Chosen::Show) => {
                cx.activate(true);
                self.dispatch(Action::ListenShow { on: true }, Origin::User, window, cx);
            }
            None => {}
        }
    }

    /// The drawer on the transcript: its lines as they come, the last at the foot. Plain text,
    /// each line one line (`capture::plain` in the core): no markdown, no links. None on: idle.
    pub(crate) fn render_listen_view(&self, width: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.model.listen.shown {
            return None;
        }
        let Some(on) = self.model.listen.on.as_ref() else { return Some(self.render_listen_idle(width, cx)) };
        // The app's name, as the file's header has it: a window's title is content.
        let source = Some(self.model.listen_head(on.source, false, 0).app);
        let paused = on.status == Status::Paused;
        // Paused: the word and Resume where the meter was.
        let paused_note = paused.then(|| {
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_1p5()
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .child("Paused")
                .child(self.listen_button("listen-resume", "Resume", "view", cx))
        });
        // ⋯: Pause or Resume (`MenuOf::Transcript`). None while it starts: nothing to pause yet.
        let open = self.model.row_menu().is_some_and(|(menu, _)| menu.row == MenuOf::Transcript);
        let dots = (!self.model.menu_items(&MenuOf::Transcript).is_empty()).then(|| {
            button("listen-menu", "⋯")
                .when(open, |d| d.bg(rgb(theme::BORDER_STRONG)))
                .child(self.controls.mark("listen-menu"))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_row_menu(MenuOf::Transcript, false, window, cx);
                }))
        });
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
            // The app's name gives way; the time always shows.
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .gap_1()
                    .child(div().min_w_0().truncate().child(format!(
                        "{}{}",
                        if paused { "" } else { "Transcribing " },
                        source.filter(|s| !s.is_empty()).unwrap_or_else(|| "pane".into())
                    )))
                    .child(div().flex_none().child(format!("· {}", self.listen_clock()))),
            )
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
                    // Named by what it hears only in a test run: every mark is a sneak target, and
                    // sneak mode's state goes to every client.
                    .when(std::env::var_os("OTTOSPACE_E2E").is_some(), |d| {
                        d.child(self.controls.mark(format!("listen-hearing:{}", hearing.id())))
                    })
            }))
            .children(paused_note)
            .child(self.listen_button("listen-stop", "Stop", "view", cx).text_xs())
            .children(dots);
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
            let downloading = self.listen_downloading.as_deref() == Some(on.id.as_str());
            let waiting = match on.status {
                Status::Starting if downloading => "Downloading speech model… (Apple's, on this Mac; once)",
                Status::Starting => "Starting…",
                _ => "Nothing heard yet",
            };
            div().text_xs().text_color(rgb(theme::MUTED)).child(waiting)
        });
        // The file it is written to, a click from read-only in a viewer: the path says it all.
        // A long folder is cut in its middle (`short_dir`), then clipped in a narrow drawer: the name stays whole.
        let file = self.listen_path().map(|path| {
            let home = config::user_home();
            let folder =
                path.parent().map(|dir| ottospace_core::talk::short_dir(&dir.to_string_lossy(), Some(&home), None));
            let name = path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
            div()
                .id("listen-file")
                .flex_none()
                .flex()
                .overflow_hidden()
                .whitespace_nowrap()
                .px_3()
                .py_1()
                .border_b_1()
                .border_color(rgb(theme::BORDER))
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .cursor_pointer()
                .hover(|s| s.text_color(rgb(theme::TEXT)))
                .child(div().min_w_0().overflow_hidden().child(format!("{}/", folder.unwrap_or_default())))
                .child(div().flex_none().child(name.into_owned()))
                .child(self.controls.mark("listen-file"))
                .on_click(cx.listener(|this, _, window, cx| this.listen_open_file(window, cx)))
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
        let body = div().flex_1().min_h_0().flex().flex_col().children(file).child(body);
        Some(self.listen_panel(width, header, body, cx))
    }

    /// The file the transcript on is written to; none if it could not be made.
    fn listen_path(&self) -> Option<&std::path::Path> {
        let on = self.model.listen.on.as_ref()?;
        let file = self.listen_file.as_ref().filter(|file| file.id == on.id)?;
        file.writer.as_ref().ok().map(|writer| writer.path.as_path())
    }

    /// The user's click on the file's path: it opens read-only in a viewer. Only the user's: the
    /// viewer's pane would put the path on the socket, which never has it (`docs/ai/transcripts.md`).
    fn listen_open_file(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.input != Origin::User {
            self.error = Some("only the user opens the transcript's file".into());
            return cx.notify();
        }
        // Absolute: a relative OTTOSPACE_HOME writes to a relative path, and a viewer takes none.
        let path = self.listen_path().and_then(|path| std::path::absolute(path).ok());
        let Some(path) = path.map(|path| path.to_string_lossy().into_owned()) else { return };
        if let Some(why) = crate::viewer::open_refusal(&path) {
            self.error = Some(why.into());
            return cx.notify();
        }
        let open = Action::OpenFile { path, axis: ottospace_core::Axis::Right, beside: None, reuse: false };
        self.dispatch(open, self.input, window, cx);
    }

    /// The drawer's transcript view, `width` wide: its header over its body.
    fn listen_panel(&self, width: f32, header: Div, body: impl IntoElement, cx: &mut Context<Self>) -> AnyElement {
        // At the header's right end: collapsed into the rail, the transcript goes on.
        let header = header.child(self.drawer_collapse("listen-collapse", cx).ml_auto());
        let header = header.id("drawer-head");
        div()
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
            .child(self.controls.mark("listen-view"))
            .into_any_element()
    }

    /// The transcript with none on: "Transcribe a window…" and today's finished ones, each a
    /// click from read-only in the drawer; or the one open.
    fn render_listen_idle(&self, width: f32, cx: &mut Context<Self>) -> AnyElement {
        if let Some(open) = self.render_listen_past(width, cx) {
            return open;
        }
        let header = div()
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .text_sm()
            .child("Transcript")
            .child(self.controls.mark("listen-idle"));
        // The primary fill (the chat's Send), with the rail's waveform.
        let pick = div()
            .id("listen-pick")
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .justify_center()
            .gap_2()
            .rounded_md()
            .bg(rgb(theme::PRIMARY))
            .text_color(rgb(theme::BACKGROUND))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .cursor_pointer()
            .hover(|s| s.opacity(0.9))
            .text_sm()
            .child(gpui::svg().path("icons/waveform.svg").size(px(16.)).text_color(rgb(theme::BACKGROUND)))
            .child("Transcribe a window…")
            .child(self.controls.mark("listen-pick"))
            .on_click(cx.listener(|this, _, window, cx| this.listen_pick(window, cx)));
        let rows = self.model.transcripts.today.iter().enumerate().map(|(at, past)| {
            let name = past.name.clone();
            let start = ottospace_core::calendar::clock(past.started_ms, crate::calendar::zone_offset(past.started_ms));
            let long = format!("{} min", past.length_ms / 60_000);
            div()
                .id(("listen-past", at))
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded_sm()
                .cursor_pointer()
                .hover(|s| s.bg(rgb(theme::SELECTED)).text_color(rgb(theme::TEXT)))
                .text_xs()
                // Finished, read-only: a document, dimmed, never a live control's look.
                .text_color(rgb(theme::MUTED))
                .child(gpui::svg().path("icons/workbook.svg").flex_none().size(px(12.)).text_color(rgb(theme::MUTED)))
                .child(div().flex_1().min_w_0().truncate().child(past.app.clone()))
                .child(div().flex_none().child(format!("{start} · {long}")))
                .child(self.controls.mark(format!("listen-past:{at}")))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::ListenOpenPast { name: Some(name.clone()) }, this.input, window, cx);
                }))
        });
        let today = (!self.model.transcripts.today.is_empty()).then(|| {
            let heading = div().pt_3().text_xs().text_color(rgb(theme::MUTED)).child("EARLIER TODAY");
            div().flex().flex_col().gap_0p5().child(heading).children(rows)
        });
        // Scrolls: a long day's rows stay in reach.
        let body = div()
            .id("listen-idle-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_3()
            .py_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(pick)
            .children(today);
        self.listen_panel(width, header, self.bars.area("listen-idle", body, theme::RAISED), cx)
    }

    /// The past transcript open, read-only: its lines as the file has them, plain; ‹ goes back.
    fn render_listen_past(&self, width: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let name = self.model.transcripts.open.as_ref()?;
        let past = self.model.transcripts.today.iter().find(|past| past.name == *name)?;
        let back = button("listen-back", "‹ Earlier today").child(self.controls.mark("listen-back")).on_click(
            cx.listener(|this, _, window, cx| {
                this.dispatch(Action::ListenOpenPast { name: None }, this.input, window, cx);
            }),
        );
        let when = format!(
            "{} · {} min",
            ottospace_core::calendar::clock(past.started_ms, crate::calendar::zone_offset(past.started_ms)),
            past.length_ms / 60_000
        );
        let header = div()
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .text_sm()
            .child(back)
            .child(div().flex_1().min_w_0().truncate().child(format!("{} · {when}", past.app)))
            .child(div().flex_none().text_xs().text_color(rgb(theme::MUTED)).child("read-only"))
            .child(self.controls.mark("listen-past-view"));
        let lines = self.listen_past.lines(name);
        let rows = lines.iter().map(|line| match line {
            PastLine::Heard { time, from, text } => {
                let (word, colour) = match from {
                    Speaker::Them => ("them", theme::PRIMARY),
                    Speaker::Me => ("me", theme::PICK),
                };
                div()
                    .flex()
                    .gap_2()
                    .text_xs()
                    .child(div().flex_none().w(px(60.)).text_color(rgb(theme::MUTED)).child(time.clone()))
                    .child(div().flex_none().w(px(36.)).text_color(rgb(colour)).child(word))
                    .child(div().flex_1().min_w_0().child(text.clone()))
            }
            PastLine::Paused { time, minutes } => {
                div().text_xs().text_color(rgb(theme::MUTED)).child(format!("{time} paused {minutes}"))
            }
        });
        let body = div()
            .id("listen-past-lines")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_3()
            .py_2()
            .flex()
            .flex_col()
            .gap_1()
            .children(rows);
        Some(self.listen_panel(width, header, self.bars.area("listen-past", body, theme::RAISED), cx))
    }

    /// "Transcribe a window…": the apps that can play sound, read off the main thread (in a test
    /// run its fixture's, `OTTOSPACE_LISTEN_APPS`, never the user's), then the picker as whoever
    /// clicked (a client's is refused).
    pub(crate) fn listen_pick(&mut self, window: &Window, cx: &mut Context<Self>) {
        let origin = self.input;
        let probe = Action::ListenPicker { apps: Vec::new(), id: String::new(), at: 0 };
        if let Some(why) = self.model.refusal(&probe, origin) {
            self.error = Some(why.into());
            return cx.notify();
        }
        if std::env::var_os("OTTOSPACE_E2E").is_some() {
            let at = now_ms();
            self.dispatch(Action::ListenPicker { apps: fixture_apps(), id: new_id(at), at }, origin, window, cx);
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let apps = cx.background_executor().spawn(async { ottospace_mac::app_audio::sound_apps() }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                let at = now_ms();
                this.dispatch(Action::ListenPicker { apps, id: new_id(at), at }, origin, window, cx);
            });
        })
        .detach();
    }

    /// About once a second while the idle view shows: today's files of `transcripts_dir`, to
    /// the core when they changed. The open one's lines are read again with them.
    pub(crate) fn listen_read_past(&mut self, window: &Window, cx: &mut Context<Self>) {
        let idle = self.model.listen.shown && self.model.listen.on.is_none();
        let Some(dir) = config::transcripts_dir().filter(|_| idle) else {
            // Shown again, it is all read and sent anew.
            self.listen_past = Past::default();
            return;
        };
        if self.listen_past.reading {
            return;
        }
        self.listen_past.reading = true;
        let now = now_ms();
        let (y, m, d) =
            ottospace_core::ics::civil(((now / 1_000) as i64 + crate::calendar::zone_offset(now)).div_euclid(86_400));
        let today = format!("{y:04}-{m:02}-{d:02}");
        let open = self.model.transcripts.open.clone();
        let known = self.listen_past.open.as_ref().filter(|(name, ..)| Some(name) == open.as_ref()).map(|o| o.1);
        // Off the main thread: a folder, a header each, and the open one only once it changed.
        cx.spawn_in(window, async move |this, cx| {
            let read = cx.background_executor().spawn(async move {
                let files = listen_file::today(&dir, &today);
                let lines = open.map(|name| {
                    let stamp = listen_file::stamp(&dir, &name);
                    let lines = (Some(stamp) != known).then(|| listen_file::read_lines(&dir, &name));
                    (name, stamp, lines)
                });
                (files, today, lines)
            });
            let (files, today, lines) = read.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.listen_past.reading = false;
                match lines {
                    Some((name, stamp, Some(lines))) => {
                        this.listen_past.open = Some((name, stamp, lines));
                        cx.notify();
                    }
                    Some(_) => {}
                    None => this.listen_past.open = None,
                }
                if this.listen_past.files.as_ref() != Some(&files) {
                    this.listen_past.files = Some(files.clone());
                    this.dispatch(Action::ListenPast { files, today }, Origin::System, window, cx);
                }
            });
        })
        .detach();
    }

    /// Test runs only: the drawer's rows (`from`, `final`, `text`; a pause as `paused_ms`), the
    /// volatile lines last. Null outside a test run: what was heard never reaches a client.
    pub(crate) fn listen_lines(&self) -> Option<serde_json::Value> {
        std::env::var_os("OTTOSPACE_E2E")?;
        let Some(on) = &self.model.listen.on else {
            let open =
                self.model.transcripts.open.as_deref().map(|name| self.listen_past.lines(name)).unwrap_or_default();
            let row = |line: &PastLine| match line {
                PastLine::Heard { from, text, .. } => serde_json::json!({ "from": from, "final": true, "text": text }),
                PastLine::Paused { .. } => serde_json::json!({ "paused_ms": 0 }),
            };
            return Some(open.iter().map(row).collect());
        };
        let rows = on.rows().map(|row| match row {
            Row::Heard { from, text, .. } => serde_json::json!({ "from": from, "final": true, "text": text }),
            Row::Paused { ms } => serde_json::json!({ "paused_ms": ms }),
        });
        let volatile =
            on.volatile().map(|(from, text)| serde_json::json!({ "from": from, "final": false, "text": text }));
        Some(rows.chain(volatile).collect())
    }

    /// Test runs only: where each of a transcript's controls was drawn, by id (`listen-start:3`,
    /// `listen-header`, `listen-stop:view`, `listen-resume:view`, `listen-menu`, `listen-file`, `listen-view`,
    /// `listen-idle`, the off-show dot `listen-dot:space:<place>` or `listen-dot:tab:<id>`).
    /// Null outside a test run.
    pub(crate) fn listen_rects(&self) -> Option<BTreeMap<String, [f32; 4]>> {
        std::env::var_os("OTTOSPACE_E2E")?;
        Some(self.controls.drawn_with("listen").into_iter().map(|(id, bounds)| (id, rect(bounds))).collect())
    }
}
