//! A recorded session's events through the model, as a live chat's: the city a task working in
//! a checkout shows after each, to see why it showed what it did (`ottospace city-replay`).

use crate::activity::Pulse;
use crate::features::Features;
use crate::layout::Axis;
use crate::model::city::City;
use crate::model::{Action, Model, Origin};
use crate::repos::Repo;
use crate::talk::Subject;
use crate::transcript::Event;

/// One task, started in the listed checkout it replays in, its workspace's empty tab on show.
pub struct Replay {
    model: Model,
}

const TASK: Subject = Subject::Task(1);

impl Replay {
    /// A model as the user's: `dir` listed, a task started there, its tab empty. `features`: the
    /// user's flags (the code city's on, or there is none).
    pub fn new(dir: &str, features: Features) -> Self {
        let mut model = Model::default();
        model.update(Action::SetFeatures { features }.sent_by(Origin::System));
        let repo = Repo { name: "replay".into(), path: dir.into(), branch: None, worktrees: vec![] };
        model.update(Action::SetRepos { repos: vec![repo] }.sent_by(Origin::System));
        // The user's acts: a click on the checkout, a task written and started there.
        for act in [
            Action::RepoClick { path: dir.into(), axis: Axis::Right },
            Action::OpenCompose { text: "replay".into() },
            Action::ComposeEnter,
        ] {
            model.update(act.sent_by(Origin::User));
        }
        model.update(Action::AgentStarted { task: TASK, session: None, at: 0 }.sent_by(Origin::System));
        // The click opened a terminal: closed, the tab is the floor.
        model.update(Action::Close { pane: None }.sent_by(Origin::User));
        Self { model }
    }

    /// The task's agent said `events` at `at` (ms).
    pub fn step(&mut self, events: Vec<Event>, at: u64) {
        self.model.update(Action::AgentEvents { task: TASK, events, at }.sent_by(Origin::System));
    }

    pub fn city(&self) -> Option<City<'_>> {
        let at = *self.model.empty_places().first()?;
        self.model.city_at(at)
    }

    /// Its chat's state: an agent floats only while it is `Running`.
    pub fn pulse(&self) -> Option<Pulse> {
        self.model.chat(&TASK).map(|chat| chat.transcript.pulse())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::Station;
    use crate::model::city::Who;
    use crate::transcript::Text;

    fn start(id: &str, tool: &str, station: Station, label: &str, detail: &str) -> Event {
        let (id, tool, label, detail) = (id.into(), tool.into(), label.into(), Text::new(detail));
        Event::ToolStart { id, parent: None, tool, station, label, detail, diff: None, gist: None }
    }

    /// A read floats the agent over its file, lit, and glimpsed; a shell line that reads nothing
    /// the city knows (`duped report .`) leaves it floating at no file, with no glimpse.
    #[test]
    fn a_replay_shows_the_city_a_live_chat_would() {
        let mut r = Replay::new("/code/app", Features::all());
        assert_eq!(r.city().map(|c| c.dir), Some("/code/app"));
        let read = start("r", "Read", Station::Files, "src/main.rs", "/code/app/src/main.rs");
        r.step(vec![Event::TurnStarted, read], 1);
        let city = r.city().unwrap();
        assert_eq!(
            city.agents.iter().map(|a| (a.who, a.at)).collect::<Vec<_>>(),
            [(Who::Task(1), Some("src/main.rs"))]
        );
        assert_eq!(city.lit.iter().map(|l| l.label).collect::<Vec<_>>(), ["src/main.rs"]);
        assert_eq!(
            city.glimpses.iter().map(|g| g.scope.clone()).collect::<Vec<_>>(),
            [vec!["src/main.rs".to_string()]]
        );
        let end = Event::ToolEnd { id: "r".into(), failed: false, output: None };
        let shell = start("s", "Bash", Station::Shell, "duped report", "duped report . --json");
        r.step(vec![end, shell], 2);
        let city = r.city().unwrap();
        assert_eq!(city.agents.iter().map(|a| (a.at, a.near.len())).collect::<Vec<_>>(), [(None, 0)]);
        assert_eq!(city.glimpses.len(), 1, "the read's, kept for the turn");
        assert_eq!(r.pulse(), Some(Pulse::Running));
    }
}
