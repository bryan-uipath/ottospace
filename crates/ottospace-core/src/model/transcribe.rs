//! The model's side of a transcript (`crate::listen`): which pane may be heard, and what a start,
//! pause, stop, or the shell's news of the streams does. Who may send each is `Model::refusal`.

use super::{Action, Effect, Kind, Model, PaneId};
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

    /// The red dot of the pane being heard, off show: none while its tab is on show (its header
    /// has the dot), and none for a source in no pane.
    pub fn listen_dot(&self) -> Option<listen::Dot> {
        let pane = self.listen.pane()?;
        let at = self.tab_panes().iter().position(|panes| panes.contains(&pane))?;
        let tab = self.tabs.get(at).filter(|_| at != self.active)?;
        Some(if tab.space == *self.space() { listen::Dot::Tab(tab.id) } else { listen::Dot::Space(tab.space.clone()) })
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
            // The shell's alone: a key or click of the user's reports nothing.
            _ if self.by_user => {}
            Action::ListenFixture => self.listen.fixture = true,
            Action::ListenAvailable { on } => self.listen.available = on,
            Action::ListenBegan { id, at } => self.listen.began(&id, at),
            // A batch, on the shell's fixed tick, empty or not: the file's write is one effect either way,
            // so neither the reply nor the trace says when someone spoke.
            Action::ListenHeard { id, lines } => {
                if self.listen.on.as_ref().is_some_and(|on| on.id == id) {
                    let records = lines
                        .into_iter()
                        .filter_map(|line| self.listen.heard(&id, line.from, line.start_ms, &line.text, line.is_final))
                        .collect();
                    effects.push(Effect::ListenWrite { id, records });
                }
            }
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

    fn listen_start(&mut self, source: Source, mic: bool, id: String, at: u64, effects: &mut Vec<Effect>) {
        let refused = match self.listen.on {
            Some(_) => Some(listen::ONE_AT_A_TIME),
            None => self.listen_refusal(source.pane()),
        };
        if let Some(why) = refused {
            return effects.push(Effect::Notice { text: why.into() });
        }
        let head = self.listen_head(source, mic, at);
        if self.listen.start(source, mic, id.clone(), at).is_ok() {
            effects.push(Effect::ListenWrite { id: id.clone(), records: vec![Record::Begin(head)] });
            effects.push(Effect::ListenOpen { id, source, mic });
        }
    }

    /// What the file's header says of a transcript of `source`: its app's name as ottospace reads
    /// it, never a window title. A test run's terminal pane is `Terminal`.
    fn listen_head(&self, source: Source, mic: bool, started: u64) -> listen::file::Head {
        let kind = source.pane().and_then(|pane| self.pane(pane)).map(|pane| &pane.kind);
        let (app, bundle) = match kind {
            Some(Kind::App { name, bundle, .. }) => (name.clone(), bundle.clone()),
            _ => ("Terminal".into(), None),
        };
        listen::file::Head { app, bundle, mic, started }
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
    use crate::listen::{self, Ended, Heard, Record, Source, Speaker, Status};
    use crate::model::{Action, Effect, Kind, Model, Origin};
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
        let head = listen::file::Head { app: app.into(), bundle: None, mic: false, started: 1_000 };
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

    /// The transcript's header has no ×: ⌘⇧L closes the drawer on it, as on a conversation.
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
}
