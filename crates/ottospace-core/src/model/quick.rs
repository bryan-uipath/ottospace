//! The quick note box (⌘⇧N, `overlay_panels`): the new-task box centred over the panes, with a pill
//! for each piece of context it will attach (`docs/ai/attention/plan.md` §6). ↵ files a note for
//! later, nothing starts; ⌘↵ starts it now in its own space and goes there. Nothing without a pill
//! is attached, and × on a pill leaves it out. Only the user opens it, or takes a pill off.

use serde::Serialize;

use super::{Action, Effect, Model, Owner};
use crate::calendar;
use crate::capture::{self, Note, NoteMeeting, NoteTranscript};
use crate::complete::Enter;
use crate::talk::Subject;
use crate::tasks::{self, TaskId};

/// Why a client's opening of the box, or its taking off a pill, is refused.
pub const QUICK_USERS: &str = "only the user opens the quick note box or takes off its pills";

/// A piece of context to attach, as its pill shows it.
#[derive(Clone, Debug, PartialEq)]
pub enum Pill {
    /// The meeting on the calendar now, by its event's id. Its title is drawn, never in the state.
    Meeting { event: u64 },
    /// The transcript being written, and how far in it is (ms).
    Transcript { at_ms: u64 },
    /// The workspace or task in front.
    Front { of: Owner },
}

/// A pill's kind: what the state says of it, and what × names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PillKind {
    Meeting,
    Transcript,
    Front,
}

impl Pill {
    pub fn kind(&self) -> PillKind {
        match self {
            Pill::Meeting { .. } => PillKind::Meeting,
            Pill::Transcript { .. } => PillKind::Transcript,
            Pill::Front { .. } => PillKind::Front,
        }
    }
}

/// The open quick box: when it opened, its pills, and the word its note's folder is named by.
#[derive(Clone, Debug, PartialEq)]
pub struct Quick {
    pub now: u64,
    pub pills: Vec<Pill>,
    pub word: String,
}

impl Model {
    /// The pills of the quick box, while it is open.
    pub fn quick_pills(&self) -> Option<&[Pill]> {
        self.quick.as_ref().filter(|_| self.compose.is_some()).map(|quick| quick.pills.as_slice())
    }

    /// When the quick box opened (its "Transcript · 10:42" is of then).
    pub fn quick_at(&self) -> Option<u64> {
        self.quick.as_ref().filter(|_| self.compose.is_some()).map(|quick| quick.now)
    }

    /// ⌘⇧N: the box, centred, with what is there now as pills. Open already: nothing changes.
    pub(super) fn quick_open(&mut self, now: u64, word: String, effects: &mut Vec<Effect>) {
        if self.compose.is_some() {
            return;
        }
        let meeting = self.calendar.events.iter().find(|e| calendar::meeting(e) && e.start <= now && now < e.end);
        let heard = self.listen.on.as_ref();
        let front = Some(self.shown_space().clone()).filter(|of| *of != Owner::Home);
        let pills = meeting
            .map(|e| Pill::Meeting { event: e.id })
            .into_iter()
            .chain(heard.map(|on| Pill::Transcript { at_ms: on.elapsed(now) }))
            .chain(front.map(|of| Pill::Front { of }))
            .collect();
        self.open_box(String::new(), false, effects);
        self.quick = Some(Quick { now, pills, word });
    }

    /// × on a pill: it is left out.
    pub(super) fn quick_pill_off(&mut self, kind: PillKind) {
        if let Some(quick) = &mut self.quick {
            quick.pills.retain(|pill| pill.kind() != kind);
        }
    }

    /// ↵ or ⇧↵ (`open` false): the line filed as a task for later, nothing started. ⌘↵: started now,
    /// in its own space, which is shown. Either way its pills are its note's (`Effect::NoteKept`).
    pub(super) fn quick_submit(&mut self, open: bool, effects: &mut Vec<Effect>) {
        if !open {
            match self.enter() {
                Enter::Take(row) => return self.accept(row),
                Enter::Wait => return,
                Enter::Add => {}
            }
        }
        let line = self.compose_line().cloned().unwrap_or_default();
        let Some(quick) = self.quick.clone() else { return };
        let Some(id) = self.file_titled(&line, None, None) else { return };
        self.compose = None;
        self.compose_reset();
        self.keep_note(id, &quick, effects);
        if open && self.by_user {
            self.talk_to(Subject::Task(id), true, effects);
            self.act(Action::TaskGo { id: Some(id) }, effects);
        }
    }

    /// What the pills left on point at, kept in the task's own folder with its brief.
    fn keep_note(&mut self, id: TaskId, quick: &Quick, effects: &mut Vec<Effect>) {
        let Some(dir) = tasks::materials_dir(id, &quick.word) else { return };
        let mut note = Note { filed: quick.now, ..Note::default() };
        for pill in &quick.pills {
            match pill {
                Pill::Meeting { event } => {
                    let e = self.calendar.events.iter().find(|e| e.id == *event);
                    note.meeting = e.map(|e| NoteMeeting { title: e.title.clone(), start: e.start, end: e.end });
                }
                // The file is the shell's to name: it writes it.
                Pill::Transcript { at_ms } => {
                    note.transcript = Some(NoteTranscript { file: String::new(), at_ms: *at_ms })
                }
                Pill::Front { of } => note.front = Some(self.space_title(of)),
            }
        }
        let Some(task) = self.tasks.note(id, dir.clone()).cloned() else { return };
        let brief = capture::note_brief(&task, note, &quick.word);
        effects.push(Effect::NoteKept { dir, brief: Box::new(brief) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Axis;
    use crate::features::{Feature, Features};
    use crate::listen::Source;
    use crate::model::{Kind, Origin};
    use crate::tasks::State;

    const WORD: &str = "k3f9a2c41b7d";
    const TITLE: &str = "Made-up review SECRET";

    /// A workspace of one terminal, a meeting on from 0 to 30 minutes, `overlay_panels` on.
    fn model() -> Model {
        let mut m = Model::default();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        let calendar =
            Action::SetCalendar { on: true, notify: false, links_only: false, hide_free: false, colors: vec![] };
        m.update(calendar.sent_by(Origin::System));
        let event = crate::calendar::Event {
            id: 7,
            start: 0,
            end: 1_800_000,
            all_day: false,
            title: TITLE.into(),
            location: "Room MADEUP".into(),
            link: None,
            show: crate::ics::Show::Busy,
        };
        let days = std::array::from_fn(|n| n as u64 * 86_400_000);
        m.update(Action::CalendarRead { events: Some(vec![event]), days, at: 0, held_from: 0 }.sent_by(Origin::System));
        let features = Features::all().with(Feature::OverlayPanels, true);
        m.update(Action::SetFeatures { features }.sent_by(Origin::System));
        m
    }

    fn open(m: &mut Model, at: u64) {
        m.update(Action::QuickOpen { now: at, word: WORD.into() });
    }

    fn type_line(m: &mut Model, text: &str) {
        m.update(Action::FieldEdit { edit: crate::edit::Edit::Insert(text.into()) });
    }

    fn kept(effects: &[Effect]) -> Option<(&String, &capture::Brief)> {
        effects.iter().find_map(|effect| match effect {
            Effect::NoteKept { dir, brief } => Some((dir, brief.as_ref())),
            _ => None,
        })
    }

    #[test]
    fn its_pills_are_the_meeting_now_and_what_is_in_front_and_enter_files_a_note_for_later() {
        let mut m = model();
        open(&mut m, 600_000);
        let kinds: Vec<PillKind> = m.quick_pills().into_iter().flatten().map(Pill::kind).collect();
        assert_eq!(kinds, [PillKind::Meeting, PillKind::Front]);
        assert!(m.compose_centred() && m.quick_at() == Some(600_000));
        // The title is drawn from the calendar: never in the state.
        assert!(!serde_json::to_string(&m).unwrap().contains("SECRET"));
        type_line(&mut m, "follow up on the export");
        let effects = m.update(Action::ComposeEnter);
        let task = m.tasks.list.last().expect("filed").clone();
        assert_eq!(
            (task.title.as_str(), task.state, m.compose.is_none()),
            ("follow up on the export", State::Captured, true)
        );
        assert!(m.talk.as_ref().and_then(|talk| talk.task.task()) != Some(task.id), "nothing started");
        let (dir, brief) = kept(&effects).expect("its note");
        assert_eq!(
            (dir.as_str(), task.note.as_deref(), task.materials_dir()),
            ("1-k3f9a2c41b7d", Some("1-k3f9a2c41b7d"), Some("1-k3f9a2c41b7d"))
        );
        let note = &brief.note.as_ref().expect("a note").content;
        assert_eq!(note.meeting.as_ref().map(|m| m.title.as_str()), Some(TITLE));
        assert_eq!((note.filed, note.front.as_deref(), &note.transcript), (600_000, Some("Workspace 2"), &None));
        // In the brief the title is quoted, as data.
        let text = capture::markdown(brief);
        let line = text.lines().find(|line| line.contains("SECRET")).expect("the title");
        assert!(line.starts_with(capture::QUOTE), "{line}");
        assert!(!serde_json::to_string(&m).unwrap().contains("SECRET"), "the task holds no title of a meeting");
    }

    #[test]
    fn a_pill_taken_off_is_not_attached_and_none_outside_a_meeting() {
        let mut m = model();
        open(&mut m, 600_000);
        m.update(Action::QuickPill { kind: PillKind::Meeting });
        type_line(&mut m, "later");
        let effects = m.update(Action::ComposeSubmit { open: false });
        let note = &kept(&effects).expect("its note").1.note.as_ref().expect("a note").content;
        assert_eq!((&note.meeting, note.front.as_deref()), (&None, Some("Workspace 2")));
        // After the meeting: no meeting pill.
        open(&mut m, 2_000_000);
        let kinds: Vec<PillKind> = m.quick_pills().into_iter().flatten().map(Pill::kind).collect();
        assert_eq!(kinds, [PillKind::Front]);
        // Esc: closed, nothing filed, no pills left.
        let tasks = m.tasks.list.len();
        m.update(Action::ComposeEscape);
        assert!((m.compose.is_none(), m.quick_pills().is_none(), m.tasks.list.len()) == (true, true, tasks));
    }

    #[test]
    fn a_transcript_being_written_is_a_pill_of_how_far_in() {
        let mut m = model();
        let app = Kind::App { window: 7, title: "Call".into(), bundle: None, name: "Zoom".into() };
        m.update(Action::Open { kind: app, axis: Axis::Right, beside: None });
        m.update(Action::ListenFixture.sent_by(Origin::System));
        m.update(Action::ListenStart { source: Source::Pane(1), mic: false, id: "t1".into(), at: 1_000 });
        open(&mut m, 61_000);
        let pill = m.quick_pills().into_iter().flatten().find(|pill| pill.kind() == PillKind::Transcript).cloned();
        assert_eq!(pill, Some(Pill::Transcript { at_ms: 60_000 }));
    }

    /// ⌘↵ starts it now and shows its own space; the meeting is still its note's.
    #[test]
    fn command_enter_starts_it_in_its_own_space() {
        let mut m = model();
        open(&mut m, 600_000);
        type_line(&mut m, "draft the reply");
        let effects = m.update(Action::ComposeSubmit { open: true });
        let id = m.tasks.list.last().map(|task| task.id).expect("filed");
        assert!(kept(&effects).is_some());
        assert_eq!(*m.shown_space(), m.task_space(id));
        assert_eq!(m.talk.as_ref().and_then(|talk| talk.task.task()), Some(id));
    }

    /// No client opens it or takes off a pill; the flag off, ⌘⇧N does nothing.
    #[test]
    fn only_the_user_opens_it_or_takes_off_a_pill() {
        let mut m = model();
        for origin in [Origin::Socket, Origin::System] {
            m.update(Action::QuickOpen { now: 600_000, word: WORD.into() }.sent_by(origin));
            assert!(m.compose.is_none(), "{origin:?}");
            assert_eq!(m.refusal(&Action::QuickPill { kind: PillKind::Front }, origin), Some(QUICK_USERS));
        }
        open(&mut m, 600_000);
        m.update(Action::QuickPill { kind: PillKind::Meeting }.sent_by(Origin::Socket));
        assert_eq!(m.quick_pills().map(<[Pill]>::len), Some(2));
        m.update(Action::ComposeEscape);
        let off = Features::all().with(Feature::OverlayPanels, false);
        m.update(Action::SetFeatures { features: off }.sent_by(Origin::System));
        open(&mut m, 600_000);
        assert!(m.compose.is_none());
    }
}
