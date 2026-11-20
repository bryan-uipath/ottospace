//! The code city behind an empty place (`atlas`, flag `code_city`): which empty places are on
//! show, which folder each is the map of, and which files are lit. The shell reads the folders'
//! files and draws.

use std::collections::BTreeMap;

use crate::PaneId;
use crate::features::Feature;
use crate::model::Model;
use crate::model::spaces::Owner;
use crate::scene::View;
use crate::tasks::TaskId;

/// A file an agent is at, as its call names it (the last two parts of its path), and the
/// identity colour of the agent's task.
#[derive(Clone, Debug, PartialEq)]
pub struct Lit<'a> {
    pub label: &'a str,
    pub colour: Option<u8>,
    /// Its call runs still. Ended, the file may be new: the shell reads the files again.
    pub running: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct City<'a> {
    /// The checkout it is the map of.
    pub dir: &'a str,
    pub lit: Vec<Lit<'a>>,
    /// Where the user turned, moved and zoomed its camera (the fitted view until they do).
    pub view: View,
}

/// A step of the city's camera, the user's pointer's (the shell translates it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CameraStep {
    /// Turned round the point looked at, in degrees.
    Orbit {
        yaw: f32,
        pitch: f32,
    },
    /// The point looked at moved over the plan, in plan units.
    Pan {
        dx: f32,
        dz: f32,
    },
    /// Nearer by `factor` (under 1: in), keeping `about` (plan units from the city's middle).
    Zoom {
        factor: f32,
        about: [f32; 2],
    },
    /// A drag began: Esc puts the camera back to here (`Cancel`).
    Grab,
    /// The drag ended where it is.
    Let,
    Cancel,
    /// The fitted view again (a double click on the ground).
    Reset,
}

/// Only the user's pointer moves a city's camera: a client's drag is refused.
pub(crate) const USERS_ALONE: &str = "only the user's pointer moves the city's camera";

/// Each tab's camera of each folder, kept while ottospace runs (not across a restart), and the
/// drag in flight with the view it started from.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Cameras {
    views: BTreeMap<(u32, String), View>,
    grab: Option<((u32, String), View)>,
}

/// An empty place on show: a tab with no pane (by its id), or an empty pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum At {
    Tab(u32),
    Pane(PaneId),
}

impl Model {
    /// The empty places on show: in the main window and each folded-out one, the tab if it has
    /// no pane, else its empty panes.
    pub fn empty_places(&self) -> Vec<At> {
        let out = self.tabs_out().into_iter().filter_map(|(id, _)| self.tabs.iter().position(|tab| tab.id == id));
        let shown = std::iter::once(self.main_at()).chain(out);
        shown
            .filter_map(|at| Some((self.tabs.get(at)?.id, self.tree_at(at))))
            .flat_map(|(tab, tree)| match tree {
                None => vec![At::Tab(tab)],
                Some(tree) => tree.leaves().into_iter().filter(|id| self.is_empty_pane(*id)).map(At::Pane).collect(),
            })
            .collect()
    }

    fn is_empty_pane(&self, id: PaneId) -> bool {
        self.pane(id).is_some_and(|pane| pane.kind == crate::Kind::Empty)
    }

    /// The folder an empty place is the map of, the nearest that has one: its pane's (an empty
    /// pane keeps none), its tab's (opened in a checkout, `open_repo`), its space's (a task's
    /// checkout, a workspace's default). Only a checkout listed in Repos. Home has none.
    pub fn empty_folder(&self, at: At) -> Option<&str> {
        let tab_id = match at {
            At::Tab(id) => id,
            At::Pane(pane) => self.tab_of(pane)?,
        };
        let tab = self.tabs.iter().find(|tab| tab.id == tab_id)?;
        let own = tab.folder.as_deref().filter(|path| self.repo_listed(path));
        own.or_else(|| match &tab.space {
            Owner::Task { id } => self.task_worktree(*id),
            Owner::Home => None,
            of => self.space_worktree(of),
        })
    }

    /// The city an empty place draws, with the flag on: the map of its folder. Lit: the files
    /// the agents are at now (`Activity::files`), each in its task's colour: in a task's space
    /// that task's; elsewhere every task that works in that folder (`task_worktree`, as the
    /// Repos section's dots).
    pub fn city_at(&self, at: At) -> Option<City<'_>> {
        let empty = match at {
            At::Tab(id) => self.tabs.iter().position(|tab| tab.id == id).is_some_and(|i| self.tree_at(i).is_none()),
            At::Pane(id) => self.is_empty_pane(id),
        };
        if !self.features.on(Feature::CodeCity) || !empty {
            return None;
        }
        let dir = self.empty_folder(at)?;
        let space = match at {
            At::Tab(id) => self.tabs.iter().find(|tab| tab.id == id).map(|tab| &tab.space),
            At::Pane(pane) => {
                self.tab_of(pane).and_then(|id| self.tabs.iter().find(|tab| tab.id == id)).map(|tab| &tab.space)
            }
        };
        let own: Option<TaskId> = match space {
            Some(Owner::Task { id }) => Some(*id),
            _ => None,
        };
        let agents = self.chats.iter().filter_map(|chat| {
            let task = chat.subject.task()?;
            let works = own.map_or(self.task_worktree(task) == Some(dir), |own| own == task);
            works.then(|| Some((self.tasks.get(task)?.colour, chat.transcript.files())))?
        });
        let lit = agents
            .flat_map(|(colour, files)| files.into_iter().map(move |(label, running)| Lit { label, colour, running }));
        let view = self.city_view(at);
        Some(City { dir, lit: lit.collect(), view })
    }

    /// Whose camera a place's is: its tab's, of its folder. Panes of one tab share it.
    fn camera_of(&self, at: At) -> Option<(u32, String)> {
        let tab = match at {
            At::Tab(id) => id,
            At::Pane(pane) => self.tab_of(pane)?,
        };
        Some((tab, self.empty_folder(at)?.to_string()))
    }

    /// The camera of a place's city: as the user left it, else the fitted view.
    pub fn city_view(&self, at: At) -> View {
        self.camera_of(at).and_then(|key| self.cameras.views.get(&key).copied()).unwrap_or_default()
    }

    /// A drag of a city's camera is in flight: Esc cancels it.
    pub fn camera_grabbed(&self) -> bool {
        self.cameras.grab.is_some()
    }

    /// Moves a place's camera by the user's `step`. Clamped: never under the ground, never
    /// past the city's footprint, never nearer than a building or farther than the whole city.
    pub(crate) fn camera(&mut self, at: At, step: CameraStep) {
        let Some(key) = self.camera_of(at).filter(|_| self.city_at(at).is_some()) else { return };
        let now = self.cameras.views.get(&key).copied().unwrap_or_default();
        let view = match step {
            CameraStep::Orbit { yaw, pitch } => now.orbited(yaw, pitch),
            CameraStep::Pan { dx, dz } => now.panned(dx, dz),
            CameraStep::Zoom { factor, about } => now.zoomed(factor, about),
            CameraStep::Grab => {
                self.cameras.grab = Some((key, now));
                return;
            }
            CameraStep::Let => {
                self.cameras.grab = None;
                return;
            }
            CameraStep::Cancel => match self.cameras.grab.take() {
                Some((grabbed, from)) if grabbed == key => from,
                _ => return,
            },
            CameraStep::Reset => View::default(),
        };
        // A step of no number moves nothing.
        if view.sound() {
            self.cameras.views.insert(key, view);
        }
    }

    /// Every camera inside its clamps, and a drag only of a camera kept: for the model's check.
    pub fn cameras_sound(&self) -> Result<(), String> {
        let bad = self.cameras.views.iter().find(|(_, view)| !view.sound());
        match bad {
            Some((key, view)) => Err(format!("camera {key:?} out of its clamps: {view:?}")),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::Station;
    use crate::features::Features;
    use crate::layout::Axis;
    use crate::model::{Action, Kind, Origin};
    use crate::repos::{Repo, Worktree};
    use crate::scene::View;
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

    /// The city of the main window's first empty place.
    fn city(m: &Model) -> Option<City<'_>> {
        m.empty_places().first().and_then(|at| m.city_at(*at))
    }

    fn labels(m: &Model) -> Option<Vec<&str>> {
        city(m).map(|city| city.lit.iter().map(|lit| lit.label).collect())
    }

    /// Two listed checkouts, `/code/app` (with a worktree `/code/app-fix`) and `/code/web`.
    fn listed() -> Model {
        let mut m = Model::default();
        let fix = Worktree { name: "app-fix".into(), path: "/code/app-fix".into(), branch: None };
        let app = Repo { name: "app".into(), path: "/code/app".into(), branch: None, worktrees: vec![fix] };
        let web = Repo { name: "web".into(), path: "/code/web".into(), branch: None, worktrees: vec![] };
        m.update(Action::SetRepos { repos: vec![app, web] });
        m
    }

    fn dir(m: &Model) -> Option<&str> {
        city(m).map(|city| city.dir)
    }

    #[test]
    fn a_tasks_empty_tab_is_the_city_of_its_checkout_and_only_with_the_flag() {
        let mut m = task_in_app();
        let colour = m.tasks.get(1).and_then(|task| task.colour);
        assert_eq!(dir(&m), Some("/code/app"));
        // A pane in the tab: the tab is no longer the floor.
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        assert_eq!(city(&m), None);
        m.update(Action::Close { pane: None });
        assert!(city(&m).is_some());
        // Home, and the flag off: none.
        m.update(Action::SpaceOpen { space: Owner::Home });
        assert_eq!(city(&m), None);
        m.update(Action::SpaceOpen { space: Owner::Task { id: 1 } });
        assert!(colour.is_some() && city(&m).is_some());
        m.update(Action::SetFeatures { features: Features::default() }.sent_by(Origin::System));
        assert_eq!(city(&m), None);
    }

    #[test]
    fn a_file_is_lit_while_its_call_runs_and_the_last_one_while_the_agent_works() {
        let mut m = task_in_app();
        let colour = m.tasks.get(1).and_then(|task| task.colour);
        events(&mut m, vec![Event::TurnStarted, call("a", Station::Files, "src/main.rs")]);
        assert_eq!(city(&m).map(|city| city.lit), Some(vec![Lit { label: "src/main.rs", colour, running: true }]));
        // Ended: still where the agent was last, while its turn runs. A second call beside it.
        events(&mut m, vec![end("a")]);
        assert_eq!(city(&m).map(|city| city.lit), Some(vec![Lit { label: "src/main.rs", colour, running: false }]));
        events(&mut m, vec![call("b", Station::Files, "src/lib.rs")]);
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

    /// The owner's case: a Repos row (a checkout, or its worktree) opens a terminal there;
    /// closed, the tab is that checkout's city, as is an empty pane in it. Home never is.
    #[test]
    fn a_repository_tab_is_its_checkouts_city_after_its_terminal_closes() {
        for path in ["/code/app", "/code/app-fix"] {
            let mut m = listed();
            assert_eq!(m.empty_places(), [At::Tab(crate::model::HOME_TAB)]);
            assert_eq!(dir(&m), None, "Home has no folder");
            m.update(Action::RepoClick { path: path.into(), axis: Axis::Right });
            assert_eq!(city(&m), None, "a terminal covers it");
            m.update(Action::Close { pane: None });
            assert_eq!(dir(&m), Some(path));
            // An empty pane beside a terminal there: the same city.
            m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
            m.update(Action::Split { axis: Axis::Right, kind: Kind::Empty });
            let places = m.empty_places();
            assert!(matches!(places[..], [At::Pane(_)]), "{places:?}");
            assert_eq!(m.city_at(places[0]).map(|city| city.dir), Some(path));
        }
    }

    /// `open_repo` into a workspace's empty tab: the tab keeps the checkout when its terminal
    /// closes, and it is nearer than the workspace's default.
    #[test]
    fn a_tab_opened_in_a_checkout_keeps_it_nearer_than_its_workspaces() {
        let mut m = listed();
        m.update(Action::RepoClick { path: "/code/app".into(), axis: Axis::Right });
        m.update(Action::NewTab);
        assert_eq!(dir(&m), Some("/code/app"), "the workspace's default");
        m.update(Action::OpenRepo { path: "/code/web".into(), axis: Axis::Right });
        m.update(Action::Close { pane: None });
        assert_eq!(dir(&m), Some("/code/web"), "the tab's own, nearer");
        // Unlisted, it is no city: the workspace's again.
        let app = Repo { name: "app".into(), path: "/code/app".into(), branch: None, worktrees: vec![] };
        m.update(Action::SetRepos { repos: vec![app] });
        assert_eq!(dir(&m), Some("/code/app"));
    }

    /// A workspace's empty place lights every task that works in its folder (as the Repos
    /// dots decide), each in its colour; a task's lights only that task's.
    #[test]
    fn a_workspace_lights_every_task_in_its_folder_and_a_task_only_its_own() {
        let mut m = task_in_app();
        let one = m.tasks.get(1).and_then(|task| task.colour);
        events(&mut m, vec![Event::TurnStarted, call("a", Station::Files, "src/main.rs")]);
        // A second task in the same checkout, at another file.
        m.update(Action::OpenCompose { text: "second >app".into() });
        m.update(Action::ComposeEnter);
        let two = 2;
        m.update(Action::AgentStarted { task: Subject::Task(two), session: None, at: 1 }.sent_by(Origin::System));
        let read = vec![Event::TurnStarted, call("b", Station::Files, "src/lib.rs")];
        m.update(Action::AgentEvents { task: Subject::Task(two), events: read, at: 5 }.sent_by(Origin::System));
        assert_eq!(m.task_worktree(two), Some("/code/app"));
        m.update(Action::SpaceOpen { space: Owner::Task { id: 1 } });
        assert_eq!(labels(&m), Some(vec!["src/main.rs"]), "a task's: its own");
        // A workspace of the checkout (`open_repo` from Home makes one): both, each in its colour.
        m.update(Action::SpaceOpen { space: Owner::Home });
        m.update(Action::OpenRepo { path: "/code/app".into(), axis: Axis::Right });
        m.update(Action::Close { pane: None });
        assert!(matches!(m.shown_space(), Owner::Workbook { .. }), "{:?}", m.shown_space());
        let lit = city(&m).map(|city| city.lit.into_iter().map(|lit| (lit.label, lit.colour)).collect::<Vec<_>>());
        let colour = m.tasks.get(two).and_then(|task| task.colour);
        assert_eq!(lit, Some(vec![("src/main.rs", one), ("src/lib.rs", colour)]));
    }

    /// The user's camera moves, stays in its clamps, survives switching tabs, and comes back
    /// to the fitted view on Reset; a client's step is refused; with the flag off nothing moves.
    #[test]
    fn the_users_camera_moves_within_its_clamps_and_a_clients_is_refused() {
        let mut m = task_in_app();
        let at = m.empty_places()[0];
        let step = |m: &mut Model, step: CameraStep, origin: Origin| {
            m.update(Action::CityCamera { at, step }.sent_by(origin));
            m.city_at(at).map(|city| city.view)
        };
        assert_eq!(step(&mut m, CameraStep::Orbit { yaw: 30., pitch: 0. }, Origin::Socket), Some(View::default()));
        let turned = step(&mut m, CameraStep::Orbit { yaw: 30., pitch: 90. }, Origin::User).unwrap();
        assert_eq!((turned.yaw, turned.pitch), (View::default().yaw + 30., crate::scene::PITCH_RANGE.1));
        let near = step(&mut m, CameraStep::Zoom { factor: 0.001, about: [0., 0.] }, Origin::User).unwrap();
        assert_eq!(near.zoom, crate::scene::ZOOM_RANGE.0);
        let moved = step(&mut m, CameraStep::Pan { dx: 5., dz: f32::NAN }, Origin::User).unwrap();
        assert!(moved.sound() && moved.pan[0] > 0., "{moved:?}");
        // Another tab and back: the same view. A new file list keeps it too (it is the tab's).
        m.update(Action::SpaceOpen { space: Owner::Home });
        m.update(Action::SpaceOpen { space: Owner::Task { id: 1 } });
        assert_eq!(m.city_at(at).map(|city| city.view), Some(moved));
        assert_eq!(step(&mut m, CameraStep::Reset, Origin::User), Some(View::default()));
        // The flag off: nothing moves; on again, as it was.
        m.update(Action::SetFeatures { features: Features::default() }.sent_by(Origin::System));
        m.update(Action::CityCamera { at, step: CameraStep::Orbit { yaw: 10., pitch: 0. } });
        let on = Features::parse_list("code_city").unwrap();
        m.update(Action::SetFeatures { features: on }.sent_by(Origin::System));
        assert_eq!(m.city_at(at).map(|city| city.view), Some(View::default()));
    }

    /// Esc during a drag puts the camera back where the drag began; a drag let go keeps it.
    #[test]
    fn a_cancelled_drag_puts_the_camera_back() {
        let mut m = task_in_app();
        let at = m.empty_places()[0];
        let go = |m: &mut Model, step| drop(m.update(Action::CityCamera { at, step }));
        go(&mut m, CameraStep::Grab);
        go(&mut m, CameraStep::Orbit { yaw: 40., pitch: -5. });
        go(&mut m, CameraStep::Pan { dx: 0.2, dz: 0.1 });
        assert!(m.camera_grabbed());
        go(&mut m, CameraStep::Cancel);
        assert_eq!((m.city_view(at), m.camera_grabbed()), (View::default(), false));
        go(&mut m, CameraStep::Grab);
        go(&mut m, CameraStep::Orbit { yaw: 40., pitch: 0. });
        go(&mut m, CameraStep::Let);
        go(&mut m, CameraStep::Cancel);
        assert_eq!(m.city_view(at).yaw, View::default().yaw + 40.);
    }
}
