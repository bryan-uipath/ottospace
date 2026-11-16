//! The model's side of a transcript (`crate::listen`): which pane may be heard, and what a start,
//! pause, stop, or the shell's news of the streams does. Who may send each is `Model::refusal`.

use super::{Action, Effect, Kind, Model, PaneId};
use crate::listen::{self, Source};

impl Model {
    /// Why `pane` cannot be heard now, if it cannot. Slice 1 has no audio: only a test run's
    /// fixture is a source, and then a terminal pane stands in for an app's (a test run hosts none).
    pub fn listen_refusal(&self, pane: Option<PaneId>) -> Option<&'static str> {
        let kind = pane.and_then(|pane| self.pane(pane)).map(|pane| &pane.kind);
        match kind {
            None | Some(Kind::Empty) => Some(listen::FOCUS_APP),
            Some(Kind::Terminal) if !self.listen.fixture => Some(listen::FOCUS_APP),
            Some(_) if !self.listen.fixture => Some(listen::NO_SOURCE),
            Some(_) => None,
        }
    }

    /// Whether the pane's header offers `Transcribe`: none is on, and it could start there.
    pub fn listen_offered(&self, pane: PaneId) -> bool {
        self.features.on(crate::features::Feature::Transcripts)
            && self.listen.on.is_none()
            && self.listen_refusal(Some(pane)).is_none()
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

    pub(super) fn listen_act(&mut self, action: Action, effects: &mut Vec<Effect>) {
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
                if self.listen.pause(at)
                    && let Some(on) = &self.listen.on
                {
                    effects.push(Effect::ListenPauseStreams { id: on.id.clone() });
                }
            }
            Action::ListenResume { at } => {
                if self.listen.resume(at)
                    && let Some(on) = &self.listen.on
                {
                    effects.push(Effect::ListenResumeStreams { id: on.id.clone() });
                }
            }
            Action::ListenStop { .. } => {
                self.listen_end(None, effects);
            }
            // The top row's mark: the drawer on the transcript, in its pane's tab.
            Action::ListenShow { on: true } => {
                if let Some(pane) = self.listen.pane() {
                    self.show_tab_of(pane, effects);
                    self.listen.shown = true;
                }
            }
            Action::ListenShow { on: false } => self.listen.shown = false,
            // The shell's alone: a key or click of the user's reports nothing.
            _ if self.by_user => {}
            Action::ListenFixture => self.listen.fixture = true,
            Action::ListenBegan { id, at } => self.listen.began(&id, at),
            Action::ListenHeard { id, from, start_ms, text, is_final } => {
                self.listen.heard(&id, from, start_ms, &text, is_final)
            }
            // The shell that reports the end says why on the notice bar (`Ended::said`).
            Action::ListenFailed { id, .. } | Action::ListenEnded { id, .. } => {
                self.listen_end(Some(&id), effects);
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
        if self.listen.start(source, mic, id.clone(), at).is_ok() {
            effects.push(Effect::ListenOpen { id, source, mic });
        }
    }

    /// Ends the transcript on (of `id`, if given), and its streams. True if one ended.
    pub(super) fn listen_end(&mut self, id: Option<&str>, effects: &mut Vec<Effect>) -> bool {
        let ended = self.listen.end(id);
        let ends = ended.is_some();
        effects.extend(ended.map(|id| Effect::ListenClose { id }));
        ends
    }
}

#[cfg(test)]
mod tests {
    use crate::listen::{self, Ended, Source, Speaker, Status};
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
        let (id, from) = ("t2".to_string(), Speaker::Them);
        Action::ListenHeard { id, from, start_ms: 0, text: text.into(), is_final }.sent_by(Origin::System)
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
        assert_eq!(m.update(start(2)), [Effect::ListenOpen { id: "t2".into(), source: Source::Pane(2), mic: false }]);
        m.update(Action::ListenBegan { id: "t2".into(), at: 1_200 }.sent_by(Origin::System));
        for origin in [Origin::Socket, Origin::System] {
            for action in [Action::ListenPause { at: 2 }, Action::ListenStop { at: 2 }] {
                assert!(m.update(action.sent_by(origin)).is_empty());
            }
        }
        assert_eq!(status(&m), Some(Status::On));
        assert_eq!(m.update(Action::ListenPause { at: 2_000 }), [Effect::ListenPauseStreams { id: "t2".into() }]);
        for origin in [Origin::Socket, Origin::System] {
            assert!(m.update(Action::ListenResume { at: 3 }.sent_by(origin)).is_empty());
        }
        assert_eq!(m.update(Action::ListenResume { at: 3_000 }), [Effect::ListenResumeStreams { id: "t2".into() }]);
        assert_eq!(m.update(Action::ListenStop { at: 4_000 }), [Effect::ListenClose { id: "t2".into() }]);
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
        // No sound yet outside a test run: an app pane says so, and offers nothing.
        assert_eq!(m.update(start(2)), [Effect::Notice { text: listen::NO_SOURCE.into() }]);
        assert!(m.listen.on.is_none() && !m.listen_offered(2));
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
        let opened = m.update(toggle(1));
        assert_eq!(opened, [Effect::ListenOpen { id: "k1".into(), source: Source::Pane(2), mic: false }]);
        m.update(Action::Focus { pane: 1 });
        assert_eq!(m.update(toggle(2)), [Effect::ListenClose { id: "k1".into() }]);
        // On an empty pane, or none: the status bar says where to go.
        m.update(Action::Split { axis: Axis::Down, kind: Kind::Empty });
        assert_eq!(m.update(toggle(3)), [Effect::Notice { text: listen::FOCUS_APP.into() }]);
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
        assert!(m.listen.on.is_none() && !m.listen.shown);
    }

    #[test]
    fn the_app_quitting_or_the_stream_failing_ends_it() {
        for why in [Ended::AppQuit, Ended::StreamFailed] {
            let mut m = model();
            m.update(start(2));
            // Another transcript's end ends nothing.
            m.update(Action::ListenEnded { id: "old".into(), why, at: 5 }.sent_by(Origin::System));
            assert!(m.listen.on.is_some());
            let effects = m.update(Action::ListenEnded { id: "t2".into(), why, at: 5 }.sent_by(Origin::System));
            assert_eq!(effects, [Effect::ListenClose { id: "t2".into() }]);
            assert!(why.said().starts_with("Transcript stopped: "));
        }
        let mut m = model();
        m.update(start(2));
        let failed = Action::ListenFailed { id: "t2".into(), why: "no audio source in a test run".into() };
        assert_eq!(m.update(failed.sent_by(Origin::System)), [Effect::ListenClose { id: "t2".into() }]);
        assert!(m.listen.on.is_none());
    }

    #[test]
    fn the_flag_turned_off_ends_it() {
        let mut m = model();
        m.update(start(2));
        let off = crate::features::Features::all().with(crate::features::Feature::Transcripts, false);
        let effects = m.update(Action::SetFeatures { features: off }.sent_by(Origin::System));
        assert_eq!(effects, [Effect::ListenClose { id: "t2".into() }]);
        assert!(m.update(start(2)).is_empty() && m.listen.on.is_none(), "off, it is absent");
        assert!(!m.listen_offered(2));
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
