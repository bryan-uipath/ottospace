//! Transcripts: one app's audio heard and written down as lines, live (`docs/ai/transcripts.md`).
//! Not `transcript`: that is an agent conversation's. Only the user starts, pauses, resumes or
//! stops one; only the shell reports what was heard. The text is never in the state.
//!
//! A pane is the source. Each final line is kept in a Markdown file as it comes (`file`).

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::model::PaneId;

pub mod file;

/// Why a start, pause, resume or stop of anyone's but the user's is refused.
pub const USERS_ALONE: &str = "only the user starts or stops a transcript";
/// Why a client's report of what was heard, or of a stream's end, is refused.
pub const SHELLS_ALONE: &str = "only ottospace reports what a transcript heard";
/// Why a client's close of the pane being transcribed is refused.
pub const SOURCE_ON: &str = "this pane is being transcribed: only the user closes it";
/// v1 keeps one at a time.
pub const ONE_AT_A_TIME: &str = "a transcript is on: stop it first";
/// ⌘⌃T with focus on no pane that can be heard.
pub const FOCUS_APP: &str = "focus an app pane to transcribe it";
/// This Mac has no recogniser: `SpeechAnalyzer` is macOS 26's.
pub const NO_SOURCE: &str = "transcripts need macOS 26";

/// The most lines the drawer keeps of the transcript on.
pub const RING: usize = 2_000;
/// The most characters of one line.
pub const MAX_LINE: usize = 2_000;

/// Where the sound comes from: the app of a pane's window, or an app the user picked
/// ("Transcribe a window…"), by its pid, in a pane or not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Pane(PaneId),
    App { pid: i32 },
}

impl Source {
    pub fn pane(self) -> Option<PaneId> {
        match self {
            Source::Pane(pane) => Some(pane),
            Source::App { .. } => None,
        }
    }
}

/// Why a client's move or pick in the transcript's picker is refused.
pub const PICK_USERS: &str = "only the user picks what is transcribed";
/// A pick of an app that is not in the picker's list.
pub const NOT_LISTED: &str = "that app is not in the list";

/// An app that can play sound, as the shell lists it for the picker: its windows' titles (drawn
/// for the user, never in the state) and whether it plays sound now.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct SoundApp {
    pub pid: i32,
    pub name: String,
    #[serde(default)]
    pub bundle: Option<String>,
    #[serde(default)]
    pub windows: Vec<String>,
    #[serde(default)]
    pub playing: bool,
}

/// The picker's rows, in order: the apps playing sound now first, then by name; each window of
/// an app a row of its own, an app with none one row. Each app once.
pub fn sound_rows(all: &[SoundApp]) -> Vec<(i32, String, Option<String>, bool)> {
    let mut apps: Vec<&SoundApp> = Vec::new();
    for app in all {
        if !apps.iter().any(|seen| seen.pid == app.pid) {
            apps.push(app);
        }
    }
    apps.sort_by_key(|app| (!app.playing, app.name.to_lowercase(), app.pid));
    let rows = apps.into_iter().flat_map(|app| {
        let windows: Vec<Option<String>> = match app.windows.is_empty() {
            true => vec![None],
            false => app.windows.iter().map(|title| Some(crate::capture::plain(title, 200))).collect(),
        };
        windows.into_iter().map(|window| (app.pid, app.name.clone(), window, app.playing))
    });
    rows.collect()
}

/// Where the red dot of the pane being heard goes while its tab is not on show: on its tab in
/// the second row (its space selected; a tab's id), else on its space in the top row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dot {
    Space(crate::model::Owner),
    Tab(u32),
}

/// Whose voice a line is: the app's (`them`), or the microphone's (`me`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speaker {
    Them,
    Me,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Asked for: the streams are not running yet.
    Starting,
    On,
    /// Nothing is captured.
    Paused,
}

/// Why the shell ended one: an end is never the client's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ended {
    AppQuit,
    StreamFailed,
}

impl Ended {
    /// What the notice bar says of it.
    pub fn said(self) -> &'static str {
        match self {
            Ended::AppQuit => "Transcript stopped: the app quit",
            Ended::StreamFailed => "Transcript stopped: the sound stopped coming",
        }
    }
}

/// A line the shell heard, `start_ms` into this run of the streams. The shell hands them over in
/// batches on a fixed tick, so when a client's reply moves says nothing of when someone spoke.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Heard {
    pub from: Speaker,
    #[serde(default)]
    pub start_ms: u64,
    pub text: String,
    #[serde(rename = "final", default)]
    pub is_final: bool,
}

/// What goes into a transcript's file, in order (`file` writes each).
#[derive(Clone, Debug, PartialEq)]
pub enum Record {
    /// The file is made: its header says `Ended: still transcribing`.
    Begin(file::Head),
    /// A final line, heard at `at` (ms since 1970).
    Line { from: Speaker, at: u64, text: String },
    /// A pause of `ms` that began at `at`.
    Paused { at: u64, ms: u64 },
    /// It ended at `at`: the header is written again with the end.
    End { at: u64 },
    /// It never began: the streams did not start. The file, with no line, goes.
    Failed,
}

/// A row of the drawer's view.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Row {
    /// A final line, `at` ms into the transcript (pauses not counted).
    Heard { from: Speaker, at: u64, text: String },
    /// A pause of `ms`, between two runs of the streams.
    Paused { ms: u64 },
}

/// The transcript that is on.
#[derive(Clone, Debug, PartialEq)]
pub struct Listen {
    pub id: String,
    pub source: Source,
    pub mic: bool,
    pub status: Status,
    /// When the user started it.
    pub started: u64,
    /// The pauses that ended, in ms.
    pub paused_total: u64,
    /// When the pause on began.
    pub paused_at: Option<u64>,
    /// Where the streams' clock starts in the transcript's: each run counts from 0.
    run_from: u64,
    /// The final lines, the last `RING`.
    rows: VecDeque<Row>,
    /// The line still being heard, per speaker: replaced until it is final.
    volatile: Vec<(Speaker, String)>,
}

impl Listen {
    /// Ms of it so far at `now`, pauses not counted.
    pub fn elapsed(&self, now: u64) -> u64 {
        let now = self.paused_at.unwrap_or(now);
        now.saturating_sub(self.started).saturating_sub(self.paused_total)
    }

    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter()
    }

    /// The lines still being heard, in the order they began.
    pub fn volatile(&self) -> impl Iterator<Item = &(Speaker, String)> {
        self.volatile.iter()
    }

    /// The lines still being heard kept as they stand, at `at`: a pause or an end cuts them off,
    /// and the recogniser's best guess is worth more than nothing. Their records for the file.
    pub fn settle(&mut self, at: u64) -> Vec<Record> {
        let into = self.elapsed(at);
        let records = std::mem::take(&mut self.volatile).into_iter().map(|(from, text)| {
            self.rows.push_back(Row::Heard { from, at: into, text: text.clone() });
            Record::Line { from, at, text }
        });
        let records: Vec<Record> = records.collect();
        while self.rows.len() > RING {
            self.rows.pop_front();
        }
        records
    }
}

/// What the core holds of transcripts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listening {
    pub on: Option<Listen>,
    /// The drawer shows the transcript on.
    pub shown: bool,
    /// A test run's fixture is the source (`ListenFixture`): a terminal pane may be heard.
    pub fixture: bool,
    /// The recogniser is here (`ListenAvailable`): an app pane may be heard.
    pub available: bool,
    /// The pane whose transcript stopped because its sound stopped coming: its header shows a grey
    /// dot until the user's Stop (its ×), a new start or the pane's close. Not in the state.
    pub lost: Option<PaneId>,
    /// The apps the picker last listed (`ListenPicker`): a pick names one by its pid. Not in the state.
    pub apps: Vec<SoundApp>,
}

/// `transcripts` in the state: today's finished ones (`file::today`), by file name, start and
/// length, never their text; and the one open read-only in the drawer, by name.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Today {
    pub today: Vec<file::Past>,
    pub open: Option<String>,
}

/// A transcript's source in the state: the pane, or `{"app": true}` (which app is not said).
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum SourceShown {
    Pane(PaneId),
    App(bool),
}

/// `listen` in the state: status, source, mic and times. Never the text, a line count or when
/// something was last heard: polled, either would say when someone speaks.
impl Serialize for Listening {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Shown {
            status: Status,
            source: SourceShown,
            mic: bool,
            auto: bool,
            started_ms: u64,
            paused_ms: u64,
            paused_at_ms: Option<u64>,
        }
        let shown = self.on.as_ref().map(|on| Shown {
            status: on.status,
            source: match on.source {
                Source::Pane(pane) => SourceShown::Pane(pane),
                Source::App { .. } => SourceShown::App(true),
            },
            mic: on.mic,
            auto: false,
            started_ms: on.started,
            paused_ms: on.paused_total,
            paused_at_ms: on.paused_at,
        });
        shown.serialize(serializer)
    }
}

impl Listening {
    /// Starts one on `source`, if none is on: `Err` says why not.
    pub fn start(&mut self, source: Source, mic: bool, id: String, at: u64) -> Result<(), &'static str> {
        if self.on.is_some() {
            return Err(ONE_AT_A_TIME);
        }
        let (paused_total, paused_at, run_from) = (0, None, 0);
        let (rows, volatile) = (VecDeque::new(), Vec::new());
        let status = Status::Starting;
        self.lost = None;
        self.on =
            Some(Listen { id, source, mic, status, started: at, paused_total, paused_at, run_from, rows, volatile });
        self.shown = true;
        Ok(())
    }

    /// The one on, if it is `id`.
    fn of(&mut self, id: &str) -> Option<&mut Listen> {
        self.on.as_mut().filter(|on| on.id == id)
    }

    /// Its streams run from `at`.
    pub fn began(&mut self, id: &str, at: u64) {
        if let Some(on) = self.of(id).filter(|on| on.status == Status::Starting) {
            on.status = Status::On;
            on.run_from = on.elapsed(at);
        }
    }

    /// A line heard, `start_ms` into this run. A volatile one replaces its speaker's last; a
    /// final one is kept and clears it, and is the file's next line. Nothing is heard while paused.
    pub fn heard(&mut self, id: &str, from: Speaker, start_ms: u64, text: &str, is_final: bool) -> Option<Record> {
        let on = self.of(id).filter(|on| on.status != Status::Paused)?;
        let text = crate::capture::plain(text, MAX_LINE);
        on.volatile.retain(|(speaker, _)| *speaker != from);
        if !is_final {
            if !text.is_empty() {
                on.volatile.push((from, text));
            }
            return None;
        }
        if text.is_empty() {
            return None;
        }
        let at = on.run_from.saturating_add(start_ms);
        // On the wall clock: the start, the pauses before this run, and the time into it.
        let wall = on.started.saturating_add(on.paused_total).saturating_add(at);
        on.rows.push_back(Row::Heard { from, at, text: text.clone() });
        while on.rows.len() > RING {
            on.rows.pop_front();
        }
        Some(Record::Line { from, at: wall, text })
    }

    /// The lines it cut off, if it paused: the streams stop.
    pub fn pause(&mut self, at: u64) -> Option<Vec<Record>> {
        let on = self.on.as_mut().filter(|on| on.status == Status::On)?;
        let at = at.max(on.started);
        let cut = on.settle(at);
        (on.status, on.paused_at) = (Status::Paused, Some(at));
        Some(cut)
    }

    /// The pause, if it resumed: new streams, counting from 0, start where the transcript is.
    pub fn resume(&mut self, at: u64) -> Option<Record> {
        let on = self.on.as_mut().filter(|on| on.status == Status::Paused)?;
        let since = on.paused_at.take().unwrap_or(at);
        let ms = at.saturating_sub(since);
        on.paused_total = on.paused_total.saturating_add(ms);
        on.status = Status::On;
        on.run_from = on.elapsed(at);
        on.rows.push_back(Row::Paused { ms });
        while on.rows.len() > RING {
            on.rows.pop_front();
        }
        Some(Record::Paused { at: since, ms })
    }

    /// Ends the one on (of `id`, if given): the one that ended, for the shell to close its
    /// streams and its file. A drawer on it stays, on the transcript idle.
    pub fn end(&mut self, id: Option<&str>) -> Option<Listen> {
        let ends = self.on.as_ref().is_some_and(|on| id.is_none_or(|id| on.id == id));
        self.on.take_if(|_| ends)
    }

    /// The pane being heard, if one is.
    pub fn pane(&self) -> Option<PaneId> {
        self.on.as_ref().and_then(|on| on.source.pane())
    }
}

/// `m:ss`, or `h:mm:ss` from an hour: how long a transcript has run. e.g. `12:04`.
pub fn clock(ms: u64) -> String {
    let s = ms / 1000;
    match (s / 3600, s / 60 % 60, s % 60) {
        (0, m, s) => format!("{m}:{s:02}"),
        (h, m, s) => format!("{h}:{m:02}:{s:02}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> Listening {
        let mut l = Listening::default();
        l.start(Source::Pane(1), false, "t1".into(), 1_000).unwrap();
        l.began("t1", 1_500);
        l
    }

    fn texts(l: &Listening) -> Vec<String> {
        let on = l.on.as_ref().unwrap();
        on.rows()
            .map(|row| match row {
                Row::Heard { text, at, .. } => format!("{at} {text}"),
                Row::Paused { ms } => format!("paused {ms}"),
            })
            .collect()
    }

    #[test]
    fn one_at_a_time() {
        let mut l = on();
        assert_eq!(l.start(Source::Pane(2), false, "t2".into(), 2_000), Err(ONE_AT_A_TIME));
        assert_eq!(l.pane(), Some(1));
        assert_eq!(l.end(Some("t2")), None, "another's end ends nothing");
        assert_eq!(l.end(None).map(|on| on.id).as_deref(), Some("t1"));
        assert!(l.on.is_none() && l.shown, "the drawer stays, on the transcript idle");
    }

    #[test]
    fn volatile_lines_are_replaced_and_finals_kept() {
        let mut l = on();
        l.heard("t1", Speaker::Them, 0, "hel", false);
        l.heard("t1", Speaker::Me, 0, "yes", false);
        l.heard("t1", Speaker::Them, 0, "hello th", false);
        let volatile: Vec<_> = l.on.as_ref().unwrap().volatile().cloned().collect();
        assert_eq!(volatile, [(Speaker::Me, "yes".into()), (Speaker::Them, "hello th".into())]);
        let record = l.heard("t1", Speaker::Them, 200, "hello there", true);
        assert_eq!(l.on.as_ref().unwrap().volatile().count(), 1, "the final clears only its speaker's");
        // The file's line: on the wall clock, 700 ms after the start at 1 s.
        assert_eq!(record, Some(Record::Line { from: Speaker::Them, at: 1_700, text: "hello there".into() }));
        // Into the transcript: the streams began 500 ms after the start.
        assert_eq!(texts(&l), ["700 hello there"]);
        // Another's id, or a line with nothing that shows, keeps nothing.
        l.heard("t9", Speaker::Them, 0, "other", true);
        l.heard("t1", Speaker::Them, 0, "\u{202e}\u{7}", true);
        assert_eq!(texts(&l).len(), 1);
    }

    #[test]
    fn a_line_is_one_plain_line() {
        let mut l = on();
        l.heard("t1", Speaker::Them, 0, "two\nlines\u{202e} \u{1b}[31m", true);
        let text = texts(&l).remove(0);
        assert!(!text.contains('\n') && !text.contains('\u{202e}') && !text.contains('\u{1b}'), "{text:?}");
    }

    #[test]
    fn a_pause_hears_nothing_and_the_next_run_counts_on() {
        let mut l = on();
        l.heard("t1", Speaker::Them, 1_000, "before", true);
        l.heard("t1", Speaker::Them, 1_200, "cut o", false);
        // The line being heard is kept as it stands: on the wall clock at the pause.
        let cut = Record::Line { from: Speaker::Them, at: 5_000, text: "cut o".into() };
        assert_eq!((l.pause(5_000), l.pause(6_000)), (Some(vec![cut]), None));
        assert_eq!(l.on.as_ref().unwrap().elapsed(9_000), 4_000, "the clock stops while paused");
        l.heard("t1", Speaker::Them, 0, "while paused", true);
        assert_eq!(l.resume(65_000), Some(Record::Paused { at: 5_000, ms: 60_000 }));
        assert_eq!(l.resume(66_000), None);
        // The new run counts from 0, at 4 s into the transcript; on the wall clock, 1 s after the resume.
        let record = l.heard("t1", Speaker::Them, 1_000, "after", true);
        assert_eq!(record, Some(Record::Line { from: Speaker::Them, at: 66_000, text: "after".into() }));
        assert_eq!(texts(&l), ["1500 before", "4000 cut o", "paused 60000", "5000 after"]);
        let on = l.on.as_ref().unwrap();
        assert_eq!((on.paused_total, on.elapsed(70_000)), (60_000, 9_000));
    }

    #[test]
    fn the_rows_are_bounded() {
        let mut l = on();
        for n in 0..RING + 5 {
            l.heard("t1", Speaker::Them, n as u64, &format!("line {n}"), true);
        }
        assert_eq!(l.on.as_ref().unwrap().rows().count(), RING);
        assert!(texts(&l)[0].ends_with("line 5"));
    }

    #[test]
    fn the_state_has_times_and_never_the_text() {
        let mut l = on();
        l.heard("t1", Speaker::Them, 0, "SECRET words", true);
        l.heard("t1", Speaker::Me, 0, "SECRET too", false);
        let state = serde_json::to_value(&l).unwrap();
        assert_eq!(
            state,
            serde_json::json!({ "status": "on", "source": {"pane": 1}, "mic": false, "auto": false,
                "started_ms": 1000, "paused_ms": 0, "paused_at_ms": null })
        );
        assert_eq!(serde_json::to_value(Listening::default()).unwrap(), serde_json::Value::Null);
    }

    #[test]
    fn clock_reads_minutes_then_hours() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(724_000), "12:04");
        assert_eq!(clock(3_725_000), "1:02:05");
    }
}
