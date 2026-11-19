//! The code city behind a task's empty tab (`atlas`, flag `code_city`): which folder it is the
//! map of, and which files are lit. The shell reads the folder's files and draws.

use crate::features::Feature;
use crate::model::Model;
use crate::model::spaces::Owner;

/// A file an agent is at, as its call names it (the last two parts of its path), and the
/// identity colour of the agent's task.
#[derive(Clone, Debug, PartialEq)]
pub struct Lit<'a> {
    pub label: &'a str,
    pub colour: Option<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct City<'a> {
    /// The checkout it is the map of.
    pub dir: &'a str,
    pub lit: Vec<Lit<'a>>,
}

impl Model {
    /// The city to draw in place of the lattice: the tab on show is a task's and has no panes,
    /// and the task works in a listed checkout. Lit: the files that the agents of every task
    /// working in that checkout are at now (`Activity::files`), each in its task's colour.
    pub fn city(&self) -> Option<City<'_>> {
        if !self.features.on(Feature::CodeCity) || self.tree_at(self.main_at()).is_some() {
            return None;
        }
        let Owner::Task { id } = self.shown_space() else { return None };
        let dir = self.task_worktree(*id)?;
        let agents = self.chats.iter().filter_map(|chat| {
            let task = chat.subject.task().filter(|task| self.task_worktree(*task) == Some(dir))?;
            Some((self.tasks.get(task)?.colour, chat.transcript.files()))
        });
        let lit = agents.flat_map(|(colour, files)| files.into_iter().map(move |label| Lit { label, colour }));
        Some(City { dir, lit: lit.collect() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::Station;
    use crate::features::Features;
    use crate::layout::Axis;
    use crate::model::{Action, Kind, Origin};
    use crate::repos::Repo;
    use crate::talk::Subject;
    use crate::transcript::{Event, Outcome, Text};

    /// A task started in the listed checkout `/code/app`, its workspace on show.
    fn task_in_app() -> Model {
        let mut m = Model::default();
        let app = Repo { name: "app".into(), path: "/code/app".into(), branch: None, worktrees: vec![] };
        m.update(Action::SetRepos { repos: vec![app] });
        m.update(Action::RepoClick { path: "/code/app".into(), axis: Axis::Right });
        m.update(Action::OpenCompose { text: "first".into() });
        m.update(Action::ComposeEnter);
        m.update(Action::AgentStarted { task: Subject::Task(1), session: None, at: 1 }.sent_by(Origin::System));
        // The click opened a terminal there: closed, the tab is the floor.
        m.update(Action::Close { pane: None });
        m
    }

    fn events(m: &mut Model, events: Vec<Event>) {
        m.update(Action::AgentEvents { task: Subject::Task(1), events, at: 5 }.sent_by(Origin::System));
    }

    fn call(id: &str, station: Station, label: &str) -> Event {
        let (id, tool, label) = (id.into(), "Read".into(), label.into());
        Event::ToolStart { id, sub: false, tool, station, label, detail: Text::default(), diff: None, gist: None }
    }

    fn end(id: &str) -> Event {
        Event::ToolEnd { id: id.into(), failed: false, output: None }
    }

    fn labels(m: &Model) -> Option<Vec<&str>> {
        m.city().map(|city| city.lit.iter().map(|lit| lit.label).collect())
    }

    #[test]
    fn a_tasks_empty_tab_is_the_city_of_its_checkout_and_only_with_the_flag() {
        let mut m = task_in_app();
        let colour = m.tasks.get(1).and_then(|task| task.colour);
        assert_eq!(m.city().map(|city| city.dir), Some("/code/app"));
        // A pane in the tab: the tab is no longer the floor.
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        assert_eq!(m.city(), None);
        m.update(Action::Close { pane: None });
        assert!(m.city().is_some());
        // Home, and the flag off: none.
        m.update(Action::SpaceOpen { space: Owner::Home });
        assert_eq!(m.city(), None);
        m.update(Action::SpaceOpen { space: Owner::Task { id: 1 } });
        assert!(colour.is_some() && m.city().is_some());
        m.update(Action::SetFeatures { features: Features::default() }.sent_by(Origin::System));
        assert_eq!(m.city(), None);
    }

    #[test]
    fn a_file_is_lit_while_its_call_runs_and_the_last_one_while_the_agent_works() {
        let mut m = task_in_app();
        let colour = m.tasks.get(1).and_then(|task| task.colour);
        events(&mut m, vec![Event::TurnStarted, call("a", Station::Files, "src/main.rs")]);
        assert_eq!(m.city().map(|city| city.lit), Some(vec![Lit { label: "src/main.rs", colour }]));
        // Ended: still where the agent was last, while its turn runs. A second call beside it.
        events(&mut m, vec![end("a"), call("b", Station::Files, "src/lib.rs")]);
        assert_eq!(labels(&m), Some(vec!["src/lib.rs"]));
        events(&mut m, vec![call("c", Station::Files, "docs/a.md")]);
        assert_eq!(labels(&m), Some(vec!["docs/a.md", "src/lib.rs"]));
        // A shell line names no file: the agent is at none, whatever it read before.
        events(&mut m, vec![end("b"), end("c"), call("d", Station::Shell, "cargo test")]);
        assert_eq!(labels(&m), Some(vec![]));
        // The turn over: nothing is lit.
        events(&mut m, vec![end("d"), call("e", Station::Files, "src/main.rs"), end("e")]);
        assert_eq!(labels(&m), Some(vec!["src/main.rs"]));
        events(&mut m, vec![Event::TurnFinished { outcome: Outcome::Completed }]);
        assert_eq!(labels(&m), Some(vec![]));
    }
}
