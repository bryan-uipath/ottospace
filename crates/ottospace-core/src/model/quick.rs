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
    /// The meeting on the calendar as the box opened: what is drawn is what is kept. Its title is
    /// drawn, never in the state.
    Meeting(NoteMeeting),
    /// The transcript being written as the box opened: its file (the shell's) and how far in (ms).
    Transcript(NoteTranscript),
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
            Pill::Meeting(_) => PillKind::Meeting,
            Pill::Transcript(_) => PillKind::Transcript,
            Pill::Front { .. } => PillKind::Front,
        }
    }
}

/// The open quick box: when it opened, its pills, the word its note's folder is named by, and
/// its project: that of the workspace or task in front as it opened, or the one its picker took
/// (none: untagged). A `#project` typed in the line wins.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Quick {
    pub now: u64,
    pub pills: Vec<Pill>,
    pub word: String,
    pub project: Option<String>,
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

    /// ⌘⇧N: the box, centred, with what is there now as pills. `transcript`: the file of the one
    /// being written, as the shell has it. Open already: only an empty box of the drawer's own
    /// (nothing open yet) becomes it; else nothing changes.
    pub(super) fn quick_open(&mut self, now: u64, word: String, transcript: Option<String>, effects: &mut Vec<Effect>) {
        let empty = self.compose_line().is_some_and(String::is_empty) && self.capture.is_none() && self.quick.is_none();
        if self.compose.is_some() && !empty {
            return;
        }
        let meeting = self.calendar.events.iter().find(|e| calendar::meeting(e) && e.start <= now && now < e.end);
        let meeting = meeting.map(|e| NoteMeeting { title: e.title.clone(), start: e.start, end: e.end });
        let heard = self.listen.on.as_ref().zip(transcript);
        let heard = heard.map(|(on, file)| NoteTranscript { file, at_ms: on.elapsed(now) });
        // What the user is in: a tab out's window's, else the main window's.
        let front = Some(self.space().clone()).filter(|of| *of != Owner::Home);
        let pills = meeting
            .map(Pill::Meeting)
            .into_iter()
            .chain(heard.map(Pill::Transcript))
            .chain(front.clone().map(|of| Pill::Front { of }))
            .collect();
        // The project of the task in front there (in a shared space, the one in front, not its owner).
        let task = front.as_ref().and_then(|of| match of {
            Owner::Task { id } => self.front(of).or(Some(*id)),
            of => self.front(of),
        });
        let project = task.and_then(|id| self.tasks.get(id)).and_then(|task| task.project.clone());
        if self.compose.is_none() {
            self.open_box(String::new(), false, effects);
        }
        self.quick = Some(Quick { now, pills, word, project });
    }

    /// The quick note's project, as its picker shows it: the line's `#project`, else its own.
    pub fn quick_project(&self) -> Option<String> {
        let quick = self.quick.as_ref().filter(|_| self.compose.is_some())?;
        self.compose_line().and_then(|line| self.draft(line).project).or_else(|| quick.project.clone())
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
        // Its ✎'s captures name it when nothing is typed, as a draft's do.
        let named = self.capture_title().filter(|_| self.draft(&line).title.is_empty());
        let Some(id) = self.file_titled(&line, None, named) else { return };
        // Its project, unless the line names one: the line is filed as typed.
        let task = self.tasks.get(id).filter(|task| task.project.is_none()).map(|task| task.repo.clone());
        if let (Some(repo), Some(project)) = (task, quick.project.clone()) {
            self.tasks.tag(id, Some(project), repo);
        }
        self.compose = None;
        self.compose_reset();
        let note = self.quick_note(&quick);
        match self.capture.is_some() {
            true => self.capture_keep_noted(id, Some(note), effects),
            false => self.keep_note(id, note, &quick.word, effects),
        }
        if open && self.by_user {
            self.talk_to(Subject::Task(id), true, effects);
            self.act(Action::TaskGo { id: Some(id) }, effects);
        }
    }

    /// What the pills left on point at.
    fn quick_note(&self, quick: &Quick) -> Note {
        let mut note = Note { filed: quick.now, ..Note::default() };
        for pill in &quick.pills {
            match pill {
                Pill::Meeting(meeting) => note.meeting = Some(meeting.clone()),
                Pill::Transcript(heard) => note.transcript = Some(heard.clone()),
                Pill::Front { of } => note.front = Some(self.space_title(of)),
            }
        }
        note
    }

    /// The note, kept in the task's own folder with its brief.
    fn keep_note(&mut self, id: TaskId, note: Note, word: &str, effects: &mut Vec<Effect>) {
        let Some(dir) = tasks::materials_dir(id, word) else { return };
        let Some(task) = self.tasks.note(id, dir.clone()).cloned() else { return };
        let brief = capture::note_brief(&task, note, word);
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
        m.update(Action::QuickOpen { now: at, word: WORD.into(), transcript: Some("/t/today.md".into()) });
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
        assert_eq!(pill, Some(Pill::Transcript(NoteTranscript { file: "/t/today.md".into(), at_ms: 60_000 })));
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

    /// What is drawn is what is kept: a calendar read again while it is open changes no pill. Its
    /// plan says nothing starts; folded and reopened (⌘⇧R) it is still the quick note.
    #[test]
    fn its_pills_are_kept_as_drawn_and_it_stays_a_note_when_folded() {
        let mut m = model();
        open(&mut m, 600_000);
        type_line(&mut m, "ask about it");
        assert_eq!(m.compose_plan().map(|plan| plan.starts), Some(false));
        let days = std::array::from_fn(|n| n as u64 * 86_400_000);
        m.update(Action::CalendarRead { events: Some(vec![]), days, at: 1, held_from: 0 }.sent_by(Origin::System));
        m.update(Action::OverlayDismiss);
        assert!(m.compose.is_none());
        m.update(Action::TalkToggle);
        assert_eq!(m.quick_pills().map(<[Pill]>::len), Some(2), "back with its pills");
        let effects = m.update(Action::ComposeEnter);
        let note = &kept(&effects).expect("its note").1.note.as_ref().expect("a note").content;
        assert_eq!(note.meeting.as_ref().map(|m| m.title.as_str()), Some(TITLE));
        assert_eq!(m.tasks.list.last().map(|task| task.state), Some(State::Captured));
    }

    /// Its project is that of the task in front, filed with the note; its one picker, the
    /// project's, changes it, and × (or "no project") leaves the note untagged. No agent, no repo.
    #[test]
    fn its_project_is_that_of_the_task_in_front_and_its_pill_takes_it_off() {
        let mut m = model();
        m.update(Action::TaskFromLine { line: "ship it #canvas".into(), known: Default::default(), open: true });
        // A line that ends in a backslash files as typed, under the project.
        open(&mut m, 600_000);
        type_line(&mut m, "check C:\\");
        m.update(Action::ComposeEnter);
        let task = m.tasks.list.last().expect("filed");
        assert_eq!((task.title.as_str(), task.project.as_deref()), ("check C:\\", Some("canvas")));
        open(&mut m, 600_000);
        assert_eq!(
            (m.quick_project().as_deref(), m.compose_pickers(), m.compose_choices()),
            (Some("canvas"), &['#'][..], None)
        );
        let said = "Type, then ↵ files it for later under #canvas · ⌘↵ starts it now · Esc";
        assert_eq!(m.compose_status().as_deref(), Some(said));
        type_line(&mut m, "ask about it");
        m.update(Action::ComposeEnter);
        let task = m.tasks.list.last().expect("filed");
        assert_eq!((task.title.as_str(), task.project.as_deref()), ("ask about it", Some("canvas")));
        // × on its pill: untagged, and the line says so. Its list, if open, closes: ↵ takes no row.
        open(&mut m, 600_000);
        m.update(Action::ComposePicker { mark: Some('#') });
        m.update(Action::ComposePick { mark: '#', name: None });
        assert_eq!((m.quick_project(), m.picker_menu()), (None, None));
        type_line(&mut m, "untagged");
        assert_eq!(m.compose_status().as_deref(), Some("↵ files it for later · ⌘↵ starts it now · Esc"));
        m.update(Action::ComposeEnter);
        assert_eq!(m.tasks.list.last().and_then(|task| task.project.clone()), None);
        // Its list, from Tab: "no project" first, the project on; a row taken is the note's, off the line.
        open(&mut m, 600_000);
        m.update(Action::ComposeComplete);
        let menu = m.picker_menu().expect("the project's list");
        assert_eq!((menu.mark, menu.rows[menu.selected.unwrap_or(0)].name.as_str()), ('#', "canvas"));
        m.update(Action::ComposeMove { delta: -1 });
        m.update(Action::ComposeEnter);
        assert_eq!((m.quick_project(), m.compose_line().map(String::as_str)), (None, Some("")));
        // Tab: its list, then the line again. There is no agent or repository to pick.
        m.update(Action::ComposeComplete);
        assert_eq!(m.picker_menu().map(|menu| menu.mark), Some('#'));
        m.update(Action::ComposeComplete);
        assert_eq!(m.picker_menu(), None);
    }

    /// Two tasks in one space: the project is that of the one in front, not of the space's owner.
    #[test]
    fn its_project_is_that_of_the_task_in_front_of_a_shared_space() {
        let mut m = model();
        for line in ["first #alpha", "second #beta"] {
            m.update(Action::TaskFromLine { line: line.into(), known: Default::default(), open: true });
        }
        let second = m.tasks.list.last().map(|task| task.id);
        assert_eq!(m.front(m.space()), second, "the second in front");
        open(&mut m, 600_000);
        assert_eq!(m.quick_project().as_deref(), Some("beta"));
    }

    /// With nothing open the drawer's own empty box shows: ⌘⇧N makes it the quick note.
    #[test]
    fn the_empty_box_of_nothing_open_becomes_the_quick_note() {
        let mut m = Model::default();
        let features = Features::all().with(Feature::OverlayPanels, true);
        m.update(Action::SetFeatures { features }.sent_by(Origin::System));
        m.update(Action::OpenCompose { text: String::new() });
        open(&mut m, 0);
        assert!(m.quick_pills().is_some());
    }

    /// No client opens it or takes off a pill; the flag off, ⌘⇧N does nothing.
    #[test]
    fn only_the_user_opens_it_or_takes_off_a_pill() {
        let mut m = model();
        for origin in [Origin::Socket, Origin::System] {
            m.update(Action::QuickOpen { now: 600_000, word: WORD.into(), transcript: None }.sent_by(origin));
            assert!(m.compose.is_none(), "{origin:?}");
            assert_eq!(m.refusal(&Action::QuickPill { kind: PillKind::Front }, origin), Some(QUICK_USERS));
        }
        open(&mut m, 600_000);
        m.update(Action::QuickPill { kind: PillKind::Meeting }.sent_by(Origin::Socket));
        assert_eq!(m.quick_pills().map(<[Pill]>::len), Some(2));
        // Nor types in it, files or closes it.
        for theirs in [
            Action::FieldEdit { edit: crate::edit::Edit::Insert("do X".into()) },
            Action::ComposeSubmit { open: false },
            Action::ComposeEnter,
            Action::CloseCompose,
            Action::OpenCompose { text: "x".into() },
        ] {
            assert!(m.refusal(&theirs, Origin::Socket).is_some(), "{theirs:?}");
            m.update(theirs.sent_by(Origin::Socket));
        }
        assert!(m.quick_pills().is_some() && m.compose_line().is_some_and(String::is_empty));
        m.update(Action::ComposeEscape);
        let off = Features::all().with(Feature::OverlayPanels, false);
        m.update(Action::SetFeatures { features: off }.sent_by(Origin::System));
        open(&mut m, 600_000);
        assert!(m.compose.is_none());
    }
}
