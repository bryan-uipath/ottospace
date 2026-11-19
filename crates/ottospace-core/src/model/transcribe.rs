//! The model's side of a transcript (`crate::listen`): which pane may be heard, and what a start,
//! pause, stop, or the shell's news of the streams does. Who may send each is `Model::refusal`.

use super::{Action, Effect, Field, Kind, MAX_FILTER, Model, PaneId, PickItem, Picker};
use crate::layout::Axis;
use crate::listen::{self, Record, Source, Status};

impl Model {
    /// Why `pane` cannot be heard now, if it cannot. An app pane, where the recogniser is here
    /// (`ListenAvailable`). A test run's fixture is a source too, and then a terminal pane stands
    /// in for an app's (a test run hosts none).
    pub fn listen_refusal(&self, pane: Option<PaneId>) -> Option<&'static str> {
        let kind = pane.and_then(|pane| self.pane(pane)).map(|pane| &pane.kind);
        let (fixture, available) = (self.listen.fixture, self.listen.available);
        match kind {
            None | Some(Kind::Empty) => Some(listen::FOCUS_APP),
            Some(Kind::Terminal) if !fixture => Some(listen::FOCUS_APP),
            Some(Kind::App { .. }) if !fixture && !available => Some(listen::NO_SOURCE),
            Some(_) => None,
        }
    }

    /// Whether the pane's header offers `Transcribe`: none is on, and it could start there.
    pub fn listen_offered(&self, pane: PaneId) -> bool {
        self.features.on(crate::features::Feature::Transcripts)
            && self.listen.on.is_none()
            && self.listen_refusal(Some(pane)).is_none()
    }

    /// What Pause or Resume (the palette, the menu bar's dot) does now: Pause while it runs,
    /// Resume while paused; nothing while it starts, or with none on.
    pub fn listen_pause_action(&self, at: u64) -> Option<Action> {
        match self.listen.on.as_ref()?.status {
            Status::On => Some(Action::ListenPause { at }),
            Status::Paused => Some(Action::ListenResume { at }),
            Status::Starting => None,
        }
    }

    /// Whether the rail holds the transcript, idle or on: the flag is on and something could be
    /// heard (the recogniser is here, or a test run's fixture), or one is on.
    pub fn listen_item(&self) -> bool {
        let here = self.listen.available || self.listen.fixture || self.listen.on.is_some();
        self.features.on(crate::features::Feature::Transcripts) && here
    }

    /// The panes whose header shows the red dot: the source pane; for an app picked
    /// (`Source::App`), each pane of that app's windows, since its whole sound is heard (SCK
    /// filters by app). By bundle when both have one, else by name. None with none on.
    pub fn listen_marked(&self) -> Vec<PaneId> {
        let Some(on) = &self.listen.on else { return Vec::new() };
        let pid = match on.source {
            Source::Pane(pane) => return vec![pane],
            Source::App { pid } => pid,
        };
        let Some(app) = self.listen.apps.iter().find(|app| app.pid == pid) else { return Vec::new() };
        let heard = (app.bundle.as_deref(), app.name.as_str());
        let of_app = |kind: &Kind| matches!(kind, Kind::App { bundle, name, .. } if crate::marks::same_app((bundle.as_deref(), name), heard));
        self.panes.iter().filter(|pane| of_app(&pane.kind)).map(|pane| pane.id).collect()
    }

    /// The red dot of the pane being heard, off show: none while its tab is on show (its header
    /// has the dot), none for a source in no pane, and none in a hidden space (off the row).
    pub fn listen_dot(&self) -> Option<listen::Dot> {
        let pane = self.listen.pane()?;
        let at = self.tab_panes().iter().position(|panes| panes.contains(&pane))?;
        let tab = self.tabs.get(at).filter(|_| at != self.active)?;
        if self.spaces().iter().any(|space| space.of == tab.space && space.hidden) {
            return None;
        }
        Some(if tab.space == *self.shown_space() {
            listen::Dot::Tab(tab.id)
        } else {
            listen::Dot::Space(tab.space.clone())
        })
    }

    /// The drawer shows the transcript only while nothing that takes keys is in it: the new-task
    /// box, the conversation with the keyboard and the editor take it back, so no key goes unseen.
    pub(crate) fn listen_hidden(&self) -> bool {
        self.compose.is_some() || self.talk.as_ref().is_some_and(|talk| talk.keys) || self.editing.is_some()
    }

    pub(super) fn settle_listen(&mut self) {
        if self.listen_hidden() || !self.listen_item() {
            self.listen.shown = false;
        }
        // A past one is open only on the idle view, and only while Today lists it.
        let idle = self.listen.shown && self.listen.on.is_none();
        let listed = |name: &String| self.transcripts.today.iter().any(|past| past.name == *name);
        if !idle || !self.transcripts.open.as_ref().is_some_and(listed) {
            self.transcripts.open = None;
        }
        if !self.features.on(crate::features::Feature::Transcripts) {
            self.transcripts.today.clear();
        }
    }

    /// Why `source` cannot be heard now, if it cannot: a pane as `listen_refusal` says; an app
    /// only where something can be heard, and only one the picker listed.
    fn listen_source_refusal(&self, source: Source) -> Option<&'static str> {
        match source {
            Source::Pane(pane) => self.listen_refusal(Some(pane)),
            Source::App { .. } if !self.listen.fixture && !self.listen.available => Some(listen::NO_SOURCE),
            Source::App { pid } if !self.listen.apps.iter().any(|app| app.pid == pid) => Some(listen::NOT_LISTED),
            Source::App { .. } => None,
        }
    }

    /// The user's "Transcribe a window…": the picker on the apps that can play sound, the one
    /// playing now first (`listen::sound_rows`). None while one is on.
    fn listen_picker(&mut self, apps: Vec<listen::SoundApp>, id: String, at: u64, effects: &mut Vec<Effect>) {
        if self.listen.on.is_some() {
            return effects.push(Effect::Notice { text: listen::ONE_AT_A_TIME.into() });
        }
        if !self.listen_item() {
            return;
        }
        self.leave(effects);
        let items = listen::sound_rows(&apps)
            .into_iter()
            .map(|(pid, name, window, playing)| PickItem::Sound { pid, name, window, playing })
            .collect();
        self.listen.apps = apps;
        let query = Field::new("", MAX_FILTER);
        let start = Some((id, at));
        let (axis, new_tab, installed) = (Axis::Right, false, Vec::new());
        let app = None;
        self.picker =
            Some(Picker { axis, new_tab, items, query, selected: 0, installed, transcribe: true, start, app });
    }

    /// Whether the close would take the pane being transcribed.
    pub(super) fn closes_listened(&self, action: &Action) -> bool {
        let Some(source) = self.listen.pane() else { return false };
        match action {
            Action::Close { pane } => pane.or(self.focused) == Some(source),
            Action::CloseTab { index } => {
                self.tab_panes().get(index.unwrap_or(self.active)).is_some_and(|panes| panes.contains(&source))
            }
            _ => false,
        }
    }

    pub(crate) fn listen_act(&mut self, action: Action, effects: &mut Vec<Effect>) {
        match action {
            Action::ListenStart { source, mic, id, at } => self.listen_start(source, mic, id, at, effects),
            // One key to stop, from any pane: worth more than symmetry.
            Action::ListenToggle { at, .. } if self.listen.on.is_some() => {
                self.listen_act(Action::ListenStop { at }, effects)
            }
            Action::ListenToggle { id, at } => match self.focused {
                Some(pane) => self.listen_start(Source::Pane(pane), false, id, at, effects),
                None => effects.push(Effect::Notice { text: listen::FOCUS_APP.into() }),
            },
            Action::ListenPause { at } => {
                if let Some(cut) = self.listen.pause(at)
                    && let Some(on) = &self.listen.on
                {
                    // Written even with none cut: the effects say nothing of whether someone was speaking.
                    effects.push(Effect::ListenWrite { id: on.id.clone(), records: cut });
                    effects.push(Effect::ListenPauseStreams { id: on.id.clone() });
                }
            }
            Action::ListenResume { at } => {
                if let Some(pause) = self.listen.resume(at)
                    && let Some(on) = &self.listen.on
                {
                    effects.push(Effect::ListenWrite { id: on.id.clone(), records: vec![pause] });
                    effects.push(Effect::ListenResumeStreams { id: on.id.clone() });
                }
            }
            // With none on, the user's Stop (the grey dot's ×) puts the lost mark away.
            Action::ListenStop { at } => {
                if !self.listen_end(None, Some(at), effects) {
                    self.listen.lost = None;
                }
            }
            // The drawer on the transcript, idle or on (the pane's dot, the menu bar's Show): in
            // its pane's tab while one is on.
            Action::ListenShow { on: true } => {
                if let Some(pane) = self.listen.pane() {
                    self.show_tab_of(pane, effects);
                }
                self.listen.shown = self.listen_item();
            }
            Action::ListenShow { on: false } => self.listen.shown = false,
            Action::ListenPicker { apps, id, at } => self.listen_picker(apps, id, at, effects),
            Action::ListenOpenPast { name } => self.transcripts.open = name,
            // The shell's alone: a key or click of the user's reports nothing.
            _ if self.by_user => {}
            Action::ListenPast { files, today } => self.transcripts.today = listen::file::today(&files, &today),
            Action::ListenFixture => self.listen.fixture = true,
            Action::ListenAvailable { on } => self.listen.available = on,
            Action::ListenBegan { id, at } => self.listen.began(&id, at),
            // A batch, on the shell's fixed tick, empty or not: the file's write is one effect either way,
            // so neither the reply nor the trace says when someone spoke.
            Action::ListenHeard { id, lines } => {
                if self.listen.on.as_ref().is_some_and(|on| on.id == id) {
                    let mut records: Vec<Record> = lines
                        .into_iter()
                        .filter_map(|line| self.listen.heard(&id, line.from, line.start_ms, &line.text, line.is_final))
                        .collect();
                    // After a session ended, the first words begin the next one's file, from them.
                    if let (Some(Record::Line { at, .. }), Some(on)) = (records.first(), &self.listen.on)
                        && on.session.armed.is_some()
                    {
                        let (source, mic, at) = (on.source, on.mic, *at);
                        let (head, meetings) = self.listen_file_head(source, mic, at);
                        if let Some(on) = self.listen.on.as_mut() {
                            (on.meetings, on.session.armed) = (meetings, None);
                            on.new_session(records.len());
                        }
                        records.insert(0, Record::Begin(head));
                    }
                    effects.push(Effect::ListenWrite { id, records });
                }
            }
            Action::ListenWindows { id, windows, at } => self.listen_windows(&id, &windows, at, effects),
            // The shell that reports the end says why on the notice bar (`Ended::said`).
            Action::ListenFailed { id, .. } => {
                self.listen_end(Some(&id), None, effects);
            }
            Action::ListenEnded { id, why, at } => {
                let pane = self.listen.pane().filter(|_| self.listen.on.as_ref().is_some_and(|on| on.id == id));
                if self.listen_end(Some(&id), Some(at), effects) && why == listen::Ended::StreamFailed {
                    self.listen.lost = pane;
                }
            }
            _ => {}
        }
    }

    pub(super) fn listen_start(&mut self, source: Source, mic: bool, id: String, at: u64, effects: &mut Vec<Effect>) {
        let refused = match self.listen.on {
            Some(_) => Some(listen::ONE_AT_A_TIME),
            None => self.listen_source_refusal(source),
        };
        if let Some(why) = refused {
            return effects.push(Effect::Notice { text: why.into() });
        }
        let (head, meetings) = self.listen_file_head(source, mic, at);
        // The source pane's window is its meeting's.
        let picked = match source.pane().and_then(|pane| self.pane(pane)).map(|pane| &pane.kind) {
            Some(Kind::App { window, .. }) => Some(*window),
            _ => None,
        };
        if self.listen.start(source, mic, id.clone(), at).is_ok() {
            if let Some(on) = self.listen.on.as_mut() {
                (on.meetings, on.session.picked) = (meetings, picked);
            }
            effects.push(Effect::ListenWrite { id: id.clone(), records: vec![Record::Begin(head)] });
            effects.push(Effect::ListenOpen { id, source, mic });
        }
    }

    /// A file's header for a transcript of `source` from `at`, with the calendar's meetings it is
    /// possibly of, and their ids.
    fn listen_file_head(&self, source: Source, mic: bool, at: u64) -> (listen::file::Head, Vec<u64>) {
        let possible = if self.calendar.on { crate::calendar::possible(&self.calendar.events, at) } else { Vec::new() };
        let meetings = possible.iter().map(|e| listen::file::Meeting::of(e)).collect();
        let head = listen::file::Head { meetings, ..self.listen_head(source, mic, at) };
        (head, possible.iter().map(|e| e.id).collect())
    }

    /// The app's windows, as the shell looked: a meeting whose windows went ends its session. What
    /// was being heard is kept, the file closes at the time they went, and the streams go on: the
    /// next words heard begin a new file (`ListenHeard`).
    fn listen_windows(&mut self, id: &str, windows: &[u32], at: u64, effects: &mut Vec<Effect>) {
        let Some(on) = self.listen.on.as_mut().filter(|on| on.id == id && on.status != Status::Starting) else {
            return;
        };
        let Some(went) = on.session.windows(windows, at) else { return };
        let went = went.max(on.started);
        let records = on.settle(went).into_iter().chain([Record::End { at: went }]).collect();
        effects.push(Effect::ListenWrite { id: id.into(), records });
    }

    /// What the file's header says of a transcript of `source`: its app's name as ottospace reads
    /// it, never a window title. A test run's terminal pane is `Terminal`.
    pub fn listen_head(&self, source: Source, mic: bool, started: u64) -> listen::file::Head {
        let kind = source.pane().and_then(|pane| self.pane(pane)).map(|pane| &pane.kind);
        let picked = match source {
            Source::App { pid } => self.listen.apps.iter().find(|app| app.pid == pid),
            Source::Pane(_) => None,
        };
        let (app, bundle) = match (kind, picked) {
            (_, Some(app)) => (app.name.clone(), app.bundle.clone()),
            (Some(Kind::App { name, bundle, .. }), _) => (name.clone(), bundle.clone()),
            _ => ("Terminal".into(), None),
        };
        listen::file::Head { app, bundle, mic, started, meetings: Vec::new() }
    }

    /// A meeting that began since the transcript on did, and is still on: a line at its start in
    /// the file, once. On the clock's tick, never when someone spoke. Not while paused: the pause's
    /// own line comes at its end, and the file stays in time order.
    pub(super) fn listen_meetings(&mut self, now: u64, effects: &mut Vec<Effect>) {
        // Nothing heard for long after its meeting ended: it ends.
        let armed = self.listen.on.as_ref().and_then(|on| on.session.armed);
        if armed.is_some_and(|since| now.saturating_sub(since) >= listen::ARMED_MAX) {
            self.listen_end(None, Some(now), effects);
            return effects.push(Effect::Notice { text: listen::ARMED_ENDED.into() });
        }
        let Some(on) =
            self.listen.on.as_mut().filter(|on| on.status == Status::On && armed.is_none() && self.calendar.on)
        else {
            return;
        };
        let began = |e: &&crate::calendar::Event| {
            crate::calendar::meeting(e) && e.start > on.started && e.start <= now && now < e.end
        };
        let new: Vec<&crate::calendar::Event> =
            self.calendar.events.iter().filter(began).filter(|e| !on.meetings.contains(&e.id)).collect();
        if new.is_empty() {
            return;
        }
        on.meetings.extend(new.iter().map(|e| e.id));
        let records = new.into_iter().map(|e| Record::Meeting(listen::file::Meeting::of(e))).collect();
        effects.push(Effect::ListenWrite { id: on.id.clone(), records });
    }

    /// Ends the transcript on (of `id`, if given), its streams and its file, at `at` (else the
    /// last tick). One that never began keeps no file. True if one ended.
    pub(super) fn listen_end(&mut self, id: Option<&str>, at: Option<u64>, effects: &mut Vec<Effect>) -> bool {
        let Some(mut on) = self.listen.end(id) else { return false };
        let at = at.unwrap_or(self.now).max(on.started);
        // What was being heard is kept as it stands, then the end.
        let records = match on.status {
            Status::Starting => vec![Record::Failed],
            Status::Paused => vec![Record::End { at }],
            Status::On => on.settle(at).into_iter().chain([Record::End { at }]).collect(),
        };
        effects.push(Effect::ListenWrite { id: on.id.clone(), records });
        effects.push(Effect::ListenClose { id: on.id });
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::features::{Feature, Features};
    use crate::listen::{self, Ended, Heard, Record, Source, Speaker, Status};
    use crate::model::{Action, Effect, Kind, Model, Origin, PickItem};
    use crate::{Axis, Envelope};

    /// A terminal pane (1) and an app pane (2), with the test fixture as the sound.
    fn model() -> Model {
        let mut m = Model::default();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        let app = Kind::App { window: 7, title: "Call".into(), bundle: None, name: "Zoom".into() };
        m.update(Action::Open { kind: app, axis: Axis::Right, beside: None });
        m.update(Action::ListenFixture.sent_by(Origin::System));
        m
    }

    fn start(pane: u32) -> Action {
        Action::ListenStart { source: Source::Pane(pane), mic: false, id: format!("t{pane}"), at: 1_000 }
    }

    fn heard(text: &str, is_final: bool) -> Envelope {
        let line = Heard { from: Speaker::Them, start_ms: 0, text: text.into(), is_final };
        Action::ListenHeard { id: "t2".into(), lines: vec![line] }.sent_by(Origin::System)
    }

    /// The effects of an end of `id`: its file closed (`End` at `at`, or gone if it never began),
    /// then its streams.
    fn ended(id: &str, record: Record) -> [Effect; 2] {
        [Effect::ListenWrite { id: id.into(), records: vec![record] }, Effect::ListenClose { id: id.into() }]
    }

    /// The effects of a start on pane 2 (Zoom's), as `id`: the file's header, then its streams.
    fn opened(id: &str, pane: u32, app: &str) -> [Effect; 2] {
        let head =
            listen::file::Head { app: app.into(), bundle: None, mic: false, started: 1_000, meetings: Vec::new() };
        let source = Source::Pane(pane);
        let begin = Effect::ListenWrite { id: id.into(), records: vec![Record::Begin(head)] };
        [begin, Effect::ListenOpen { id: id.into(), source, mic: false }]
    }

    fn status(m: &Model) -> Option<Status> {
        m.listen.on.as_ref().map(|on| on.status)
    }

    #[test]
    fn only_the_user_starts_pauses_resumes_or_stops_one() {
        let mut m = model();
        for origin in [Origin::Socket, Origin::System] {
            assert!(m.update(start(2).sent_by(origin)).is_empty());
            assert_eq!(m.refusal(&start(2), origin), Some(listen::USERS_ALONE));
            let toggle = Action::ListenToggle { id: "t".into(), at: 0 };
            assert_eq!(m.refusal(&toggle, origin), Some(listen::USERS_ALONE));
        }
        assert!(m.listen.on.is_none());
        assert_eq!(m.update(start(2)), opened("t2", 2, "Zoom"));
        m.update(Action::ListenBegan { id: "t2".into(), at: 1_200 }.sent_by(Origin::System));
        for origin in [Origin::Socket, Origin::System] {
            for action in [Action::ListenPause { at: 2 }, Action::ListenStop { at: 2 }] {
                assert!(m.update(action.sent_by(origin)).is_empty());
            }
        }
        assert_eq!(status(&m), Some(Status::On));
        m.update(heard("cut o", false));
        let cut = Effect::ListenWrite {
            id: "t2".into(),
            records: vec![Record::Line { from: Speaker::Them, at: 2_000, text: "cut o".into() }],
        };
        assert_eq!(m.update(Action::ListenPause { at: 2_000 }), [cut, Effect::ListenPauseStreams { id: "t2".into() }]);
        assert_eq!(m.update(Action::ListenPause { at: 2_500 }), [], "paused already");
        // A pause in silence makes the same effects, with nothing cut.
        let mut quiet = model();
        quiet.update(start(2));
        quiet.update(Action::ListenBegan { id: "t2".into(), at: 1_200 }.sent_by(Origin::System));
        let none = Effect::ListenWrite { id: "t2".into(), records: vec![] };
        assert_eq!(
            quiet.update(Action::ListenPause { at: 2_000 }),
            [none, Effect::ListenPauseStreams { id: "t2".into() }]
        );
        for origin in [Origin::Socket, Origin::System] {
            assert!(m.update(Action::ListenResume { at: 3 }.sent_by(origin)).is_empty());
        }
        let pause = Effect::ListenWrite { id: "t2".into(), records: vec![Record::Paused { at: 2_000, ms: 1_000 }] };
        assert_eq!(
            m.update(Action::ListenResume { at: 3_000 }),
            [pause, Effect::ListenResumeStreams { id: "t2".into() }]
        );
        assert_eq!(m.update(Action::ListenStop { at: 4_000 }), ended("t2", Record::End { at: 4_000 }));
        assert!(m.listen.on.is_none());
    }

    #[test]
    fn only_the_shell_reports_what_was_heard() {
        let mut m = model();
        m.update(start(2));
        let began = Action::ListenBegan { id: "t2".into(), at: 1_000 };
        assert_eq!(m.refusal(&began, Origin::Socket), Some(listen::SHELLS_ALONE));
        // The user's own key or click reports nothing either.
        for origin in [Origin::Socket, Origin::User] {
            m.update(began.clone().sent_by(origin));
            m.update(Action::ListenEnded { id: "t2".into(), why: Ended::AppQuit, at: 0 }.sent_by(origin));
        }
        assert_eq!(status(&m), Some(Status::Starting));
        m.update(began.sent_by(Origin::System));
        let mut said = heard("words", true);
        said.origin = Origin::Socket;
        m.update(said);
        assert_eq!(m.listen.on.as_ref().map(|on| on.rows().count()), Some(0));
        m.update(heard("words", true));
        assert_eq!(m.listen.on.as_ref().map(|on| on.rows().count()), Some(1));
        // A client cannot make a terminal pane a source.
        let mut m = Model::default();
        assert!(m.update(Action::ListenFixture.sent_by(Origin::Socket)).is_empty());
        assert!(!m.listen.fixture);
    }

    #[test]
    fn without_the_fixture_a_terminal_pane_is_refused() {
        let mut m = Model::default();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        assert_eq!(m.update(start(1)), [Effect::Notice { text: listen::FOCUS_APP.into() }]);
        let app = Kind::App { window: 7, title: "Call".into(), bundle: None, name: "Zoom".into() };
        m.update(Action::Open { kind: app, axis: Axis::Right, beside: None });
        // No recogniser on this Mac: an app pane says so, and offers nothing.
        assert_eq!(m.update(start(2)), [Effect::Notice { text: listen::NO_SOURCE.into() }]);
        assert!(m.listen.on.is_none() && !m.listen_offered(2));
        // With one, the app pane offers a start, and the terminal still does not; a client cannot
        // say there is one.
        m.update(Action::ListenAvailable { on: true }.sent_by(Origin::Socket));
        assert!(!m.listen_offered(2));
        m.update(Action::ListenAvailable { on: true }.sent_by(Origin::System));
        assert!(m.listen_offered(2) && !m.listen_offered(1));
        m.update(Action::ListenAvailable { on: false }.sent_by(Origin::System));
        m.update(Action::ListenFixture.sent_by(Origin::System));
        assert!(m.listen_offered(1) && m.listen_offered(2));
    }

    #[test]
    fn one_at_a_time() {
        let mut m = model();
        m.update(start(2));
        assert_eq!(m.update(start(1)), [Effect::Notice { text: listen::ONE_AT_A_TIME.into() }]);
        assert_eq!(m.listen.pane(), Some(2));
        assert!(!m.listen_offered(1), "no header offers a second");
    }

    #[test]
    fn the_key_starts_on_the_focused_pane_and_stops_from_anywhere() {
        let mut m = model();
        m.update(Action::Focus { pane: 2 });
        let toggle = |at| Action::ListenToggle { id: format!("k{at}"), at };
        let effects = m.update(toggle(1_000));
        assert_eq!(effects, opened("k1000", 2, "Zoom"));
        m.update(Action::Focus { pane: 1 });
        // Stopped before its streams ran: no file is kept.
        assert_eq!(m.update(toggle(2_000)), ended("k1000", Record::Failed));
        // On an empty pane, or none: the status bar says where to go.
        m.update(Action::Split { axis: Axis::Down, kind: Kind::Empty });
        assert_eq!(m.update(toggle(3)), [Effect::Notice { text: listen::FOCUS_APP.into() }]);
    }

    fn app(pid: i32, name: &str, windows: &[&str], playing: bool) -> listen::SoundApp {
        let windows: Vec<String> = windows.iter().map(|w| w.to_string()).collect();
        let window_ids = (0..windows.len() as u32).map(|n| pid as u32 * 100 + n).collect();
        listen::SoundApp { pid, name: name.into(), bundle: Some(format!("b.{name}")), windows, window_ids, playing }
    }

    fn picker(apps: Vec<listen::SoundApp>) -> Action {
        Action::ListenPicker { apps, id: "p1".into(), at: 5_000 }
    }

    fn rows(m: &Model) -> Vec<(String, Option<String>, bool)> {
        let items = m.picker.as_ref().map(|picker| picker.items.clone()).unwrap_or_default();
        let row = |item: PickItem| match item {
            PickItem::Sound { name, window, playing, .. } => Some((name, window.map(|(title, _)| title), playing)),
            _ => None,
        };
        items.into_iter().filter_map(row).collect()
    }

    /// The owner: "the app playing sound now sorts first (mark it)". Each window a row, an app
    /// with none one row; only the user opens it, moves in it or picks; a pick hears that app,
    /// in a pane or not.
    #[test]
    fn the_picker_lists_the_app_playing_first_and_only_the_users_pick_starts_one() {
        let mut m = model();
        let apps = vec![
            app(30, "Slack", &[], false),
            app(20, "Chrome", &["meet.example.com", "Docs"], false),
            app(10, "Zoom", &["Zoom Meeting"], true),
        ];
        for origin in [Origin::Socket, Origin::System] {
            assert_eq!(m.refusal(&picker(apps.clone()), origin), Some(listen::USERS_ALONE));
            m.update(picker(apps.clone()).sent_by(origin));
            assert!(m.picker.is_none());
        }
        m.update(picker(apps.clone()));
        let row = |name: &str, window: Option<&str>, playing| (name.to_string(), window.map(String::from), playing);
        assert_eq!(
            rows(&m),
            [
                row("Zoom", Some("Zoom Meeting"), true),
                row("Chrome", Some("meet.example.com"), false),
                row("Chrome", Some("Docs"), false),
                row("Slack", None, false),
            ]
        );
        // Window titles are drawn, never in the state.
        let state = serde_json::to_string(&m.picker).unwrap();
        assert!(!state.contains("meet.example.com") && state.contains("\"playing\":true"), "{state}");
        // An installed-app list read meanwhile adds no row.
        let installed = crate::settings::Installed { name: "Bear".into(), path: "/Applications/Bear.app".into() };
        m.update(Action::SetInstalled { apps: vec![installed], running: Vec::new() }.sent_by(Origin::System));
        assert!(m.picker.as_ref().is_some_and(|picker| picker.installed.is_empty()));
        // A client moves nothing, picks nothing, types nothing in its filter, closes or replaces nothing.
        let theirs = [
            Action::PickerMove { delta: 1 },
            Action::PickerChoose { index: Some(1) },
            Action::FieldEdit { edit: crate::edit::Edit::Insert("slack".into()) },
            Action::ClosePicker,
            Action::OpenPicker { axis: crate::Axis::Right, new_tab: false, items: vec![PickItem::Terminal] },
        ];
        for action in theirs {
            assert_eq!(m.refusal(&action, Origin::Socket), Some(listen::PICK_USERS), "{action:?}");
            m.update(action.sent_by(Origin::Socket));
        }
        assert!(m.picker.as_ref().is_some_and(|picker| picker.selected == 0 && picker.transcribe));
        assert_eq!((rows(&m).len(), m.listen.on.is_none()), (4, true));
        // The user's filter finds a window by its title too.
        m.update(Action::PickerQuery { query: "docs".into() });
        let found: Vec<String> =
            m.picker.as_ref().unwrap().matches().iter().map(|item| item.label().to_string()).collect();
        assert_eq!(found, ["Chrome"]);
        m.update(Action::PickerQuery { query: String::new() });
        // The user's pick of Chrome's second window: Chrome's sound, its name in the file.
        let effects = m.update(Action::PickerChoose { index: Some(2) });
        let head = listen::file::Head {
            app: "Chrome".into(),
            bundle: Some("b.Chrome".into()),
            mic: false,
            started: 5_000,
            meetings: Vec::new(),
        };
        let source = Source::App { pid: 20 };
        assert_eq!(
            effects,
            [
                Effect::ListenWrite { id: "p1".into(), records: vec![Record::Begin(head)] },
                Effect::ListenOpen { id: "p1".into(), source, mic: false },
            ]
        );
        assert_eq!((m.listen.on.as_ref().map(|on| on.source), m.picker.is_none()), (Some(source), true));
        // In no pane: no dot anywhere but the rail; while it is on, no second picker.
        assert_eq!(m.listen_dot(), None);
        assert_eq!(m.update(picker(apps)), [Effect::Notice { text: listen::ONE_AT_A_TIME.into() }]);
        // An app the picker did not list is refused.
        m.update(Action::ListenStop { at: 6_000 });
        let unlisted = Action::ListenStart { source: Source::App { pid: 99 }, mic: false, id: "u".into(), at: 7_000 };
        assert_eq!(m.update(unlisted), [Effect::Notice { text: listen::NOT_LISTED.into() }]);
        // The flag off takes the picker with it: no pick could start one.
        m.update(picker(vec![app(10, "Zoom", &[], true)]));
        m.update(Action::SetFeatures { features: Features::all().with(Feature::Transcripts, false) });
        assert!(m.picker.is_none() && m.listen.apps.is_empty());
    }

    /// Opened idle: today's finished transcripts (the shell's reading of the folder), and one
    /// open read-only on the user's click alone; it closes when one starts.
    #[test]
    fn today_lists_the_finished_ones_and_the_users_click_opens_one() {
        let mut m = model();
        let head = |app: &str, started| listen::file::Head {
            app: app.into(),
            bundle: None,
            mic: false,
            started,
            meetings: Vec::new(),
        };
        let said = |at| listen::Record::Line { from: listen::Speaker::Them, at, text: "made-up".into() };
        let done = |at: u64| {
            let line = listen::file::line(&said(at + 1_000), &|_| 0).unwrap_or_default();
            listen::file::header(&head("Zoom", at), Some(at + 120_000), &|_| 0) + &line
        };
        let files = vec![
            ("2026-11-16-0900-zoom.md".to_string(), done(1_794_819_600_000)),
            ("2026-11-16-1000-zoom.md".to_string(), done(1_794_823_200_000)),
        ];
        let past = Action::ListenPast { files, today: "2026-11-16".into() };
        assert_eq!(m.refusal(&past, Origin::Socket), Some(listen::SHELLS_ALONE));
        m.update(past.sent_by(Origin::System));
        let names: Vec<&str> = m.transcripts.today.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["2026-11-16-1000-zoom.md", "2026-11-16-0900-zoom.md"]);
        assert_eq!(m.transcripts.today[0].length_ms, 120_000);
        let open = Action::ListenOpenPast { name: Some("2026-11-16-0900-zoom.md".into()) };
        // Only on the idle view, and only the user's.
        m.update(open.clone());
        assert_eq!(m.transcripts.open, None, "the drawer is not on the transcript");
        m.update(Action::ListenShow { on: true });
        m.update(open.clone().sent_by(Origin::Socket));
        assert_eq!(m.transcripts.open, None);
        m.update(open);
        assert_eq!(m.transcripts.open.as_deref(), Some("2026-11-16-0900-zoom.md"));
        // A name Today does not list opens nothing.
        m.update(Action::ListenOpenPast { name: Some("../x.md".into()) });
        assert_eq!(m.transcripts.open, None);
        m.update(Action::ListenOpenPast { name: Some("2026-11-16-1000-zoom.md".into()) });
        m.update(start(2));
        assert_eq!(m.transcripts.open, None, "a start shows the one on");
    }

    /// The owner: "we show the record + time in 3 spots". The pane's own dot while its tab is on
    /// show; off show, a dot on its tab (its space selected) or on its space in the top row.
    #[test]
    fn the_dot_marks_the_tab_or_space_of_the_pane_heard_only_while_it_is_off_show() {
        let mut m = model();
        m.update(start(2));
        assert_eq!(m.listen_dot(), None, "on show: the pane's header has it");
        let (tab, space) = (m.tabs[m.active].id, m.space().clone());
        m.update(Action::NewTab);
        assert_eq!(m.listen_dot(), Some(listen::Dot::Tab(tab)));
        m.update(Action::WorkbookNew { uid: "00000000000000a1".into() });
        assert_ne!(*m.space(), space);
        assert_eq!(m.listen_dot(), Some(listen::Dot::Space(space)));
        // Its pane's dot shows it: on show again, no dot elsewhere.
        m.update(Action::ListenShow { on: true });
        assert_eq!(m.listen_dot(), None);
        // None on, none anywhere.
        m.update(Action::NewTab);
        m.update(Action::ListenStop { at: 5_000 });
        assert_eq!(m.listen_dot(), None);
    }

    #[test]
    fn closing_the_pane_ends_it_and_a_client_cannot() {
        let mut m = model();
        m.update(start(2));
        // A client's close of it, by id or as the focused pane, or of its tab: refused.
        for close in [Action::Close { pane: Some(2) }, Action::Close { pane: None }, Action::CloseTab { index: None }] {
            assert_eq!(m.refusal(&close, Origin::Socket), Some(listen::SOURCE_ON), "{close:?}");
            m.update(close.sent_by(Origin::Socket));
        }
        assert_eq!(m.listen.pane(), Some(2));
        // Another pane, a client may close.
        m.update(Action::Close { pane: Some(1) }.sent_by(Origin::Socket));
        assert!(m.pane(1).is_none());
        // The shell's close (its window is gone) and the user's end it.
        let effects = m.update(Action::Close { pane: Some(2) }.sent_by(Origin::System));
        assert!(effects.contains(&Effect::ListenClose { id: "t2".into() }), "{effects:?}");
        // The drawer stays on the transcript, idle.
        assert!(m.listen.on.is_none() && m.listen.shown);
    }

    #[test]
    fn the_app_quitting_or_the_stream_failing_ends_it() {
        for why in [Ended::AppQuit, Ended::StreamFailed] {
            let mut m = model();
            m.update(start(2));
            // Another transcript's end ends nothing.
            m.update(Action::ListenEnded { id: "old".into(), why, at: 5 }.sent_by(Origin::System));
            assert!(m.listen.on.is_some());
            m.update(Action::ListenBegan { id: "t2".into(), at: 1_000 }.sent_by(Origin::System));
            let effects = m.update(Action::ListenEnded { id: "t2".into(), why, at: 5_000 }.sent_by(Origin::System));
            assert_eq!(effects, ended("t2", Record::End { at: 5_000 }));
            assert!(why.said().starts_with("Transcript stopped: "));
        }
        let mut m = model();
        m.update(start(2));
        let failed = Action::ListenFailed { id: "t2".into(), why: "no audio source in a test run".into() };
        assert_eq!(m.update(failed.sent_by(Origin::System)), ended("t2", Record::Failed));
        assert!(m.listen.on.is_none());
    }

    #[test]
    fn a_lost_stream_leaves_a_grey_mark_until_the_user_puts_it_away() {
        let mut m = model();
        m.update(start(2));
        m.update(Action::ListenBegan { id: "t2".into(), at: 1_000 }.sent_by(Origin::System));
        let lost = Action::ListenEnded { id: "t2".into(), why: Ended::StreamFailed, at: 5_000 };
        m.update(lost.sent_by(Origin::System));
        assert_eq!((m.listen.on.is_none(), m.listen.lost), (true, Some(2)));
        // Not in the state: the reply says only that none is on.
        assert_eq!(serde_json::to_value(&m).unwrap()["listen"], serde_json::Value::Null);
        // A client's Stop does not put it away; the user's does, and ends nothing.
        m.update(Action::ListenStop { at: 6_000 }.sent_by(Origin::Socket));
        assert_eq!(m.listen.lost, Some(2));
        assert!(m.update(Action::ListenStop { at: 6_000 }).is_empty());
        assert_eq!(m.listen.lost, None);
        // A new start puts it away too, and the app quitting leaves none.
        m.update(start(2));
        m.update(Action::ListenBegan { id: "t2".into(), at: 1_000 }.sent_by(Origin::System));
        m.update(Action::ListenEnded { id: "t2".into(), why: Ended::StreamFailed, at: 5_000 }.sent_by(Origin::System));
        m.update(Action::ListenStart { source: Source::Pane(1), mic: false, id: "t3".into(), at: 7_000 });
        assert_eq!(m.listen.lost, None);
        m.update(Action::ListenEnded { id: "t3".into(), why: Ended::AppQuit, at: 8_000 }.sent_by(Origin::System));
        assert_eq!(m.listen.lost, None);
        // Its pane closed: it goes with it.
        m.update(start(2));
        m.update(Action::ListenEnded { id: "t2".into(), why: Ended::StreamFailed, at: 5_000 }.sent_by(Origin::System));
        m.update(Action::Close { pane: Some(2) });
        assert_eq!(m.listen.lost, None);
    }

    #[test]
    fn the_flag_turned_off_ends_it() {
        let mut m = model();
        m.update(start(2));
        let off = crate::features::Features::all().with(crate::features::Feature::Transcripts, false);
        let effects = m.update(Action::SetFeatures { features: off.clone() }.sent_by(Origin::System));
        assert_eq!(effects, ended("t2", Record::Failed));
        assert!(m.update(start(2)).is_empty() && m.listen.on.is_none(), "off, it is absent");
        assert!(!m.listen_offered(2));
        // What the shell learnt with it off is kept for when it turns on.
        let mut m = Model::default();
        m.update(Action::SetFeatures { features: off }.sent_by(Origin::System));
        m.update(Action::ListenAvailable { on: true }.sent_by(Origin::System));
        m.update(Action::SetFeatures { features: crate::features::Features::all() }.sent_by(Origin::System));
        assert!(m.listen.available);
    }

    #[test]
    fn the_top_rows_mark_shows_its_pane_and_the_drawer_on_it() {
        let mut m = model();
        m.update(start(2));
        assert!(m.listen.shown, "a start opens the drawer on it");
        m.update(Action::ListenShow { on: false });
        m.update(Action::NewTab);
        let tab = m.active;
        m.update(Action::ListenShow { on: true });
        assert!(m.listen.shown && m.active != tab);
        assert!(m.layout.as_ref().is_some_and(|l| l.leaves().contains(&2)));
    }

    /// The transcript's header has no ×: its → (⌘⇧R) collapses the drawer into its rail, as on a conversation.
    #[test]
    fn the_drawers_key_closes_the_drawer_on_the_transcript() {
        let mut m = model();
        m.update(start(2));
        m.update(Action::TalkToggle);
        assert!(!m.listen.shown && m.listen.on.is_some(), "closed, and still on");
        m.update(Action::ListenShow { on: true });
        assert!(m.listen.shown);
    }

    /// Pause and Resume are in the header's ⋯: one item, the one that fits its status, the user's alone.
    #[test]
    fn the_headers_menu_pauses_and_resumes() {
        use crate::row_menu::{Item, MenuOf};
        let open = Action::RowMenu { row: MenuOf::Transcript, dots: None, keys: true };
        let items = |m: &Model| m.row_menu().map(|(_, items)| items.to_vec());
        let mut m = model();
        // Starting: nothing to pause, so no menu, and the palette's Pause or Resume does nothing.
        assert_eq!(m.listen_pause_action(5), None);
        m.update(start(2));
        m.update(open.clone());
        assert_eq!((items(&m), m.listen_pause_action(5)), (None, None));
        m.update(Action::ListenBegan { id: "t2".into(), at: 1_200 }.sent_by(Origin::System));
        m.update(open.clone());
        assert_eq!(items(&m), Some(vec![Item::Pause]));
        assert_eq!(m.listen_pause_action(5), Some(Action::ListenPause { at: 5 }));
        m.update(Action::Tick { now: 2_000 }.sent_by(Origin::System));
        let effects = m.update(Action::RowMenuChoose { index: None });
        assert!(effects.contains(&Effect::ListenPauseStreams { id: "t2".into() }), "{effects:?}");
        assert_eq!((status(&m), m.row_menu.is_none()), (Some(Status::Paused), true));
        m.update(open.clone());
        assert_eq!(items(&m), Some(vec![Item::Resume]));
        assert_eq!(m.listen_pause_action(5), Some(Action::ListenResume { at: 5 }));
        // A client neither chooses from it nor closes it.
        let choose = Action::RowMenuChoose { index: None };
        assert_eq!(m.refusal(&choose, Origin::Socket), Some(crate::workbook::USERS_ALONE));
        m.update(choose.sent_by(Origin::Socket));
        assert_eq!(status(&m), Some(Status::Paused));
        m.update(Action::RowMenuChoose { index: Some(0) });
        assert_eq!(status(&m), Some(Status::On));
        // The transcript put away: its menu goes with it.
        m.update(open);
        m.update(Action::TalkToggle);
        assert!(m.row_menu.is_none());
    }

    #[test]
    fn what_takes_keys_in_the_drawer_takes_it_from_the_transcript() {
        let mut m = model();
        m.update(start(2));
        m.update(Action::OpenCompose { text: String::new() });
        assert!(!m.listen.shown, "the new-task box is not typed in unseen");
        m.update(Action::CloseCompose);
        m.update(Action::ListenShow { on: true });
        assert!(m.listen.shown);
    }

    #[test]
    fn a_batch_writes_its_final_lines_and_an_empty_one_writes_too() {
        let mut m = model();
        m.update(start(2));
        m.update(Action::ListenBegan { id: "t2".into(), at: 1_500 }.sent_by(Origin::System));
        let line =
            |text: &str, start_ms, is_final| Heard { from: Speaker::Them, start_ms, text: text.into(), is_final };
        let lines = vec![line("made", 0, false), line("made-up words", 200, true), line("next", 300, false)];
        let effects = m.update(Action::ListenHeard { id: "t2".into(), lines }.sent_by(Origin::System));
        // The final line on the wall clock: the streams began 500 ms in, and it was 200 ms into them.
        let words = Record::Line { from: Speaker::Them, at: 1_700, text: "made-up words".into() };
        assert_eq!(effects, [Effect::ListenWrite { id: "t2".into(), records: vec![words] }]);
        // Nothing heard: the same one effect, with nothing in it.
        let effects = m.update(Action::ListenHeard { id: "t2".into(), lines: vec![] }.sent_by(Origin::System));
        assert_eq!(effects, [Effect::ListenWrite { id: "t2".into(), records: vec![] }]);
        // Another's batch writes nothing.
        assert!(m.update(Action::ListenHeard { id: "old".into(), lines: vec![] }.sent_by(Origin::System)).is_empty());
        // The pane closed: the file ends at the last tick's time, the line being heard kept first.
        m.update(Action::Tick { now: 9_000 }.sent_by(Origin::System));
        let effects = m.update(Action::Close { pane: Some(2) });
        let next = Record::Line { from: Speaker::Them, at: 9_000, text: "next".into() };
        let write = Effect::ListenWrite { id: "t2".into(), records: vec![next, Record::End { at: 9_000 }] };
        let close = Effect::ListenClose { id: "t2".into() };
        assert!(effects.windows(2).any(|two| two == [write.clone(), close.clone()]), "{effects:?}");
    }

    #[test]
    fn the_state_never_holds_what_was_heard() {
        let mut m = model();
        m.update(start(2));
        m.update(Action::ListenBegan { id: "t2".into(), at: 1_000 }.sent_by(Origin::System));
        let hostile = "SECRET <script>\u{202e} [link](http://x) \u{1b}]52;c;";
        m.update(heard(hostile, true));
        m.update(heard(&format!("{hostile} still"), false));
        let state = serde_json::to_string(&m).unwrap();
        assert!(!state.contains("SECRET"), "{state}");
        let listen = serde_json::to_value(&m).unwrap()["listen"].clone();
        assert_eq!(listen["status"], "on");
        assert_eq!(listen["source"], serde_json::json!({"pane": 2}));
    }

    /// A calendar of made-up meetings at `start`s (ms), 30 minutes each, read at 0.
    fn with_calendar(m: &mut Model, starts: &[u64]) {
        let calendar =
            Action::SetCalendar { on: true, notify: false, links_only: false, hide_free: false, colors: vec![] };
        m.update(calendar.sent_by(Origin::System));
        let event = |(n, start): (usize, &u64)| crate::calendar::Event {
            id: n as u64 + 1,
            start: *start,
            end: start + 1_800_000,
            all_day: false,
            title: format!("Made-up meeting {}", n + 1),
            location: "Room MADEUP".into(),
            link: None,
            show: crate::ics::Show::Busy,
        };
        let events = Some(starts.iter().enumerate().map(event).collect());
        let days = std::array::from_fn(|n| n as u64 * 86_400_000);
        m.update(Action::CalendarRead { events, days, at: 0, held_from: 0 }.sent_by(Origin::System));
    }

    fn begun(effects: &[Effect]) -> Option<&listen::file::Head> {
        effects.iter().find_map(|effect| match effect {
            Effect::ListenWrite { records, .. } => records.iter().find_map(|r| match r {
                Record::Begin(head) => Some(head),
                _ => None,
            }),
            _ => None,
        })
    }

    #[test]
    fn a_transcript_during_a_meeting_names_it_and_one_that_begins_later_gets_a_line() {
        let mut m = model();
        // 1 on since 0:00; 2 begins at 0:10; 3 at 0:40, after the transcript ends.
        with_calendar(&mut m, &[0, 600_000, 2_400_000]);
        let effects =
            m.update(Action::ListenStart { source: Source::Pane(2), mic: false, id: "t2".into(), at: 60_000 });
        let titles: Vec<&str> = begun(&effects).unwrap().meetings.iter().map(|m| m.title.as_str()).collect();
        assert_eq!(titles, ["Made-up meeting 1"]);
        m.update(Action::ListenBegan { id: "t2".into(), at: 60_000 }.sent_by(Origin::System));
        let tick = |m: &mut Model, now| m.update(Action::Tick { now }.sent_by(Origin::System));
        assert_eq!(tick(&mut m, 300_000), [], "nothing begins");
        let line = Record::Meeting(listen::file::Meeting {
            title: "Made-up meeting 2".into(),
            start: 600_000,
            end: 2_400_000,
        });
        assert_eq!(tick(&mut m, 601_000), [Effect::ListenWrite { id: "t2".into(), records: vec![line] }]);
        assert_eq!(tick(&mut m, 602_000), [], "once");
        // Paused through one's start: its line comes after the pause's, at the first tick resumed.
        m.update(Action::ListenPause { at: 700_000 });
        let later = listen::file::Meeting { title: "Made-up meeting 4".into(), start: 900_000, end: 2_700_000 };
        with_calendar(&mut m, &[0, 600_000, 2_400_000, 900_000]);
        assert_eq!(tick(&mut m, 901_000), [], "paused");
        let resumed = m.update(Action::ListenResume { at: 1_000_000 });
        assert!(
            matches!(resumed.as_slice(), [Effect::ListenWrite { records, .. }, ..] if matches!(records.as_slice(), [Record::Paused { .. }]))
        );
        let line = Record::Meeting(later);
        assert_eq!(tick(&mut m, 1_001_000), [Effect::ListenWrite { id: "t2".into(), records: vec![line] }]);
        // The state says neither title nor place.
        let state = serde_json::to_string(&m).unwrap();
        assert!(!state.contains("Made-up meeting") && !state.contains("MADEUP"), "{state}");
    }

    #[test]
    fn with_no_meeting_on_or_no_calendar_the_header_names_none() {
        let start = Action::ListenStart { source: Source::Pane(2), mic: false, id: "t2".into(), at: 60_000 };
        let mut m = model();
        with_calendar(&mut m, &[3_600_000]);
        assert_eq!(begun(&m.update(start.clone())).unwrap().meetings, []);
        let mut m = model();
        with_calendar(&mut m, &[0]);
        m.update(
            Action::SetCalendar { on: false, notify: false, links_only: false, hide_free: false, colors: vec![] }
                .sent_by(Origin::System),
        );
        assert_eq!(begun(&m.update(start)).unwrap().meetings, []);
    }

    /// An app picked is heard whole: each pane of its windows is marked, none of another app's.
    #[test]
    fn the_panes_marked_are_the_source_or_every_window_of_the_app_heard() {
        let mut m = model();
        assert_eq!(m.listen_marked(), Vec::<u32>::new(), "none on");
        m.update(start(2));
        assert_eq!(m.listen_marked(), [2], "the source pane");
        m.update(Action::ListenStop { at: 2_000 });
        assert_eq!(m.listen_marked(), Vec::<u32>::new(), "stopped");
        // Zoom's second window, and Chrome's, in panes 3 and 4.
        for (window, name) in [(8, "Zoom"), (9, "Chrome")] {
            let kind = Kind::App { window, title: "w".into(), bundle: Some(format!("b.{name}")), name: name.into() };
            m.update(Action::Open { kind, axis: Axis::Right, beside: None });
        }
        // Pane 2's Zoom has no bundle: matched by name; pane 3's by bundle; Chrome's not.
        m.update(picker(vec![app(10, "Zoom", &["Zoom Meeting"], true), app(20, "Chrome", &[], false)]));
        m.update(Action::PickerChoose { index: Some(0) });
        assert_eq!(m.listen_marked(), [2, 3]);
        // Ended by the shell: no mark at once.
        let ended = Action::ListenEnded { id: "p1".into(), why: Ended::AppQuit, at: 6_000 };
        m.update(Action::ListenBegan { id: "p1".into(), at: 5_000 }.sent_by(Origin::System));
        m.update(ended.sent_by(Origin::System));
        assert_eq!(m.listen_marked(), Vec::<u32>::new());
        let other = listen::SoundApp { bundle: Some("b.Other".into()), ..app(30, "Zoom", &[], true) };
        m.update(picker(vec![other]));
        m.update(Action::PickerChoose { index: Some(0) });
        assert_eq!(m.listen_marked(), [2], "pane 3's bundle differs; pane 2 has none: by name");
    }

    /// The owner: "the zoom meeting ended, and then I joined a new one. The transcript didn't
    /// start a new session". Its meeting's window gone: the file closes; the next words begin a
    /// new file with its own header; the streams run on throughout.
    #[test]
    fn a_meeting_that_ends_closes_its_file_and_the_next_words_begin_another() {
        let mut m = model();
        with_calendar(&mut m, &[0, 7_200_000]);
        // Pane 2's window (7) is the meeting's; Zoom's home window (50) stays.
        m.update(start(2));
        m.update(Action::ListenBegan { id: "t2".into(), at: 1_000 }.sent_by(Origin::System));
        let look = |windows: &[u32], at| Action::ListenWindows { id: "t2".into(), windows: windows.to_vec(), at };
        let heard_at = |m: &mut Model, start_ms| {
            let line = Heard { from: Speaker::Them, start_ms, text: "made-up".into(), is_final: true };
            m.update(Action::ListenHeard { id: "t2".into(), lines: vec![line] }.sent_by(Origin::System))
        };
        assert_eq!(m.update(look(&[50, 7], 2_000).sent_by(Origin::System)), []);
        heard_at(&mut m, 1_000);
        // Gone at 1:00, still gone 10 s later: the file ends at 1:00, the streams stay.
        assert_eq!(m.update(look(&[50], 60_000).sent_by(Origin::System)), []);
        let ended = m.update(look(&[50], 70_000).sent_by(Origin::System));
        assert_eq!(ended, [Effect::ListenWrite { id: "t2".into(), records: vec![Record::End { at: 60_000 }] }]);
        assert_eq!(status(&m), Some(Status::On));
        assert_eq!(m.listen.pane(), Some(2), "still on: its dot stays");
        // A new meeting at 2:00, in a window of its own; its first words, at 2:05, begin a file.
        m.update(look(&[50, 9], 7_200_000).sent_by(Origin::System));
        let begun = heard_at(&mut m, 7_300_000 - 1_000);
        let [Effect::ListenWrite { records, .. }] = begun.as_slice() else { panic!("{begun:?}") };
        let Some(Record::Begin(head)) = records.first() else { panic!("{records:?}") };
        assert_eq!(head.started, 7_300_000);
        let titles: Vec<&str> = head.meetings.iter().map(|m| m.title.as_str()).collect();
        assert_eq!(titles, ["Made-up meeting 2"], "the new meeting's header");
        assert!(matches!(records.get(1), Some(Record::Line { at: 7_300_000, .. })), "{records:?}");
        assert_eq!(m.listen.on.as_ref().map(|on| on.rows().count()), Some(1), "the drawer shows the new one");
        // Its window gone too: that file ends as well.
        m.update(look(&[50], 8_000_000).sent_by(Origin::System));
        let again = m.update(look(&[50], 8_010_000).sent_by(Origin::System));
        assert_eq!(again, [Effect::ListenWrite { id: "t2".into(), records: vec![Record::End { at: 8_000_000 }] }]);
        // Nothing heard for 30 minutes after: the transcript ends, and the bar says why.
        assert_eq!(m.update(Action::Tick { now: 8_010_000 + listen::ARMED_MAX - 1 }.sent_by(Origin::System)), []);
        let over = m.update(Action::Tick { now: 8_010_000 + listen::ARMED_MAX }.sent_by(Origin::System));
        assert!(over.contains(&Effect::Notice { text: listen::ARMED_ENDED.into() }), "{over:?}");
        assert!(over.contains(&Effect::ListenClose { id: "t2".into() }), "{over:?}");
        assert_eq!(m.listen.on, None);
    }

    #[test]
    fn a_client_reports_no_windows_and_the_users_stop_while_armed_stops_all() {
        let mut m = model();
        m.update(start(2));
        m.update(Action::ListenBegan { id: "t2".into(), at: 1_000 }.sent_by(Origin::System));
        let look = |windows: &[u32], at| Action::ListenWindows { id: "t2".into(), windows: windows.to_vec(), at };
        assert_eq!(m.refusal(&look(&[], 0), Origin::Socket), Some(listen::SHELLS_ALONE));
        m.update(look(&[7], 2_000).sent_by(Origin::System));
        m.update(look(&[], 3_000).sent_by(Origin::System));
        m.update(look(&[], 13_000).sent_by(Origin::System));
        assert!(m.listen.on.as_ref().is_some_and(|on| on.session.armed.is_some()));
        let stopped = m.update(Action::ListenStop { at: 20_000 });
        assert!(stopped.contains(&Effect::ListenClose { id: "t2".into() }), "{stopped:?}");
        assert_eq!(m.listen.on, None);
    }
}
