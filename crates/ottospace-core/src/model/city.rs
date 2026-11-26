//! The code city behind an empty place (`atlas`, flag `code_city`): which empty places are on
//! show, which folder each is the map of, and which files are lit. The shell reads the folders'
//! files and draws.

use std::collections::BTreeMap;

use crate::PaneId;
use crate::activity::{Pulse, Station};
use crate::city_shell;
use crate::features::Feature;
use crate::glance::{Glance, Words};
use crate::layout::Axis;
use crate::model::spaces::Owner;
use crate::model::{Action, Effect, Model, Origin};
use crate::scene::View;
use crate::talk::Subject;
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

/// An agent working now in the city's checkout: whose, its task's identity colour (none for a
/// session no task has), and the file it is at (`Activity::at`; none: a shell line, between calls).
#[derive(Clone, Debug, PartialEq)]
pub struct Agent<'a> {
    pub who: Who<'a>,
    pub colour: Option<u8>,
    pub at: Option<&'a str>,
    /// At no file: the checkout's paths its running shell line names (`city_shell::paths`), files
    /// or folders, relative to it; the shell takes the first that is in the city.
    pub near: Vec<String>,
    /// A call of it runs now (for a session read from its file: a call it started has not ended).
    pub busy: bool,
    /// The sub-agents it runs now (a chat's `Agent`/`Task` calls not ended; none known for a
    /// session read from its file).
    pub subagents: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Who<'a> {
    Task(TaskId),
    /// A session no task has, at work (a `claude` the user runs in a terminal): its runner's
    /// name, its id and its title, as read from its file (`SetSessions`).
    Session {
        runner: &'a str,
        id: &'a str,
        title: Option<&'a str>,
    },
    /// An agent CLI one of our terminal panes runs, known by its process alone (`PaneAgents`): no
    /// file of it read, so nothing of what it does; its runner's name.
    Pane {
        pane: PaneId,
        runner: &'a str,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct City<'a> {
    /// The checkout it is the map of.
    pub dir: &'a str,
    pub lit: Vec<Lit<'a>>,
    /// Its agents that work now, by task, then the sessions no task has: each floats over its
    /// file, or at the city's edge.
    pub agents: Vec<Agent<'a>>,
    /// Where the user turned, moved and zoomed its camera (the fitted view until they do).
    pub view: View,
    /// What its working agents read and searched lately, oldest first: the shell plays each once.
    pub glimpses: Vec<Glimpse>,
}

/// A read or a search of an agent, as the city's places: `scope` the files or folders it covers
/// (a search's empty scope: the whole checkout), `found` the files a search's result names. Its
/// `id` is its task's and its call's: the same glimpse keeps it as more is known.
#[derive(Clone, Debug, PartialEq)]
pub struct Glimpse {
    pub id: String,
    pub search: bool,
    pub scope: Vec<String>,
    pub found: Vec<String>,
    pub colour: Option<u8>,
}

/// `glance` in the checkout `dir`; none for a read of no file there, or a search elsewhere (a
/// `cd` out of the checkout, or operands all outside it). A search with no operands covers where
/// it ran: the whole checkout, or the folder its `cd` went to. Its result is read from there.
fn glimpse(task: TaskId, colour: Option<u8>, glance: &Glance, dir: &str) -> Option<Glimpse> {
    let mut base = String::new();
    let scope = match &glance.words {
        Words::Path(path) => vec![city_shell::path(path, dir, "")?],
        Words::Stage { cds, operands } => {
            for cd in cds {
                base = city_shell::path(cd, dir, &base)?;
            }
            let inside: Vec<String> = operands.iter().filter_map(|word| city_shell::path(word, dir, &base)).collect();
            if !operands.is_empty() && inside.is_empty() {
                return None;
            }
            if operands.is_empty() { vec![base.clone()] } else { inside }
        }
        Words::All => Vec::new(),
    };
    let scope: Vec<String> = scope.into_iter().filter(|path| !path.is_empty()).collect();
    if !glance.search && scope.is_empty() {
        return None;
    }
    let found = glance.found.iter().filter_map(|word| city_shell::path(word, dir, &base)).filter(|p| !p.is_empty());
    let (id, search) = (format!("{task}:{}", glance.id), glance.search);
    Some(Glimpse { id, search, scope, found: found.collect(), colour })
}

/// A step of the city's camera, the user's pointer's (the shell translates it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CameraStep {
    /// Turned round the point looked at, in degrees.
    Orbit {
        yaw: f32,
        pitch: f32,
    },
    /// The point looked at moved over the plan, in plan units, and the picture slid up by `up`,
    /// a share of its height.
    Pan {
        dx: f32,
        dz: f32,
        up: f32,
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
    /// One frame of a spin (the developer's timed turn): `SPIN_DEGREES` round, level.
    Spin,
}

/// A spin turns the camera once round, a frame a step, between a `Grab` and a `Cancel` (which
/// puts it back): `SPIN_STEPS` of `SPIN_DEGREES`, as a steady drag would.
pub const SPIN_STEPS: usize = 240;
pub const SPIN_DEGREES: f32 = 360. / SPIN_STEPS as f32;

/// Only the user's pointer moves a city's camera: a client's drag is refused.
pub(crate) const USERS_ALONE: &str = "only the user's pointer moves the city's camera";
pub(crate) const OPEN_USERS: &str = "only the user's double-click opens a file or folder from the city";

/// Each place's camera (a tab, or an empty pane, showing a city), each checkout's last one used,
/// and the drag in flight with the view it started from.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Cameras {
    /// By folder: the view last used in any place of the checkout. A place with no camera of its
    /// own starts from it; it is kept in the layout file (`layout.json`'s `cameras`).
    pub(crate) views: BTreeMap<String, View>,
    /// By place and the checkout it shows: its own, once the user moved it or another place of
    /// its checkout on show. Two places of one checkout (a tab folded out, two panes) turn
    /// apart; a tab reused for another checkout starts afresh. Dropped with its tab or pane.
    places: BTreeMap<(At, String), View>,
    /// A drag in flight: its place and checkout, and both cameras before it (Esc puts both back).
    grab: Option<Grab>,
}

#[derive(Clone, Debug, PartialEq)]
struct Grab {
    place: (At, String),
    from: View,
    folder_was: Option<View>,
}

/// An empty place on show: a tab with no pane (by its id), or an empty pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum At {
    Tab(u32),
    Pane(PaneId),
}

/// The file a session at work is at: its label, while its station is files (as serialized).
fn file_of(s: &crate::sessions::Found) -> Option<&str> {
    let file = s.station.as_deref().is_some_and(|station| station.eq_ignore_ascii_case(Station::Files.name()));
    s.label.as_deref().filter(|_| file)
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
    /// its tasks' (its own and those that joined it); elsewhere every task that works in that
    /// folder (`task_worktree`, as the Repos section's dots); everywhere, the sessions no task has.
    pub fn city_at(&self, at: At) -> Option<City<'_>> {
        let empty = match at {
            At::Tab(id) => self.tabs.iter().position(|tab| tab.id == id).is_some_and(|i| self.tree_at(i).is_none()),
            At::Pane(id) => self.is_empty_pane(id),
        };
        if !empty {
            return None;
        }
        self.city_of(at)
    }

    /// The city of pane `pane`'s folder, as an empty place would draw it, whatever the pane holds:
    /// for a test run's city under a terminal (`city_under`, `city_live_layer`).
    pub fn city_under(&self, pane: crate::PaneId) -> Option<City<'_>> {
        self.city_of(At::Pane(pane))
    }

    /// The main window's tab whose city is drawn behind its panes (`city_behind`): one with
    /// panes, a folder, and a pane that lets the city show (`see_through` under 1).
    pub fn behind_tab(&self) -> Option<u32> {
        let on = self.features.on(Feature::CityBehind) && self.features.on(Feature::CodeCity);
        let tree = self.tree_at(self.main_at()).filter(|_| on)?;
        let tab = self.tabs.get(self.main_at())?.id;
        self.empty_folder(At::Tab(tab))?;
        tree.leaves().into_iter().any(|id| self.see_through(id) < 1.).then_some(tab)
    }

    /// The opacity a pane's kind asks for over a city: a terminal's and an explorer's as set,
    /// every other 1 (an app pane is another app's window: nothing of ours is under it).
    fn see_through(&self, id: PaneId) -> f32 {
        match self.pane(id).map(|pane| &pane.kind) {
            Some(crate::Kind::Terminal) => self.opacity.0,
            Some(crate::Kind::Explorer { .. }) => self.opacity.1,
            _ => 1.,
        }
    }

    /// The city behind the main window's tab's panes, as its empty tab would draw it, and where.
    pub fn city_behind(&self) -> Option<(At, City<'_>)> {
        let at = At::Tab(self.behind_tab()?);
        Some((at, self.city_of(at)?))
    }

    /// The panes of the tab whose city is behind them that let it show, with their opacity.
    pub fn see_through_panes(&self) -> Vec<(PaneId, f32)> {
        let Some(tree) = self.behind_tab().and_then(|_| self.tree_at(self.main_at())) else { return Vec::new() };
        tree.leaves().into_iter().map(|id| (id, self.see_through(id))).filter(|(_, o)| *o < 1.).collect()
    }

    /// How opaque pane `id`'s background is: under 1 only over the city behind its tab.
    pub fn pane_opacity(&self, id: PaneId) -> f32 {
        let behind = self.behind_tab().is_some_and(|tab| self.tab_of(id) == Some(tab));
        if behind { self.see_through(id) } else { 1. }
    }

    fn city_of(&self, at: At) -> Option<City<'_>> {
        if !self.features.on(Feature::CodeCity) {
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
        // A task's space: its own task, and those that joined it working in this checkout (one
        // may name another, `tidy >web`); elsewhere, every task of the checkout.
        let members = own.map(|id| self.members(&Owner::Task { id }));
        let works = |task: TaskId| match &members {
            Some(members) => Some(task) == own || (members.contains(&task) && self.task_worktree(task) == Some(dir)),
            None => self.task_worktree(task) == Some(dir),
        };
        let chats = self.chats.iter().filter_map(|chat| {
            let task = chat.subject.task()?;
            let works = works(task);
            works.then(|| Some((task, self.tasks.get(task)?.colour, &chat.transcript)))?
        });
        let chats: Vec<_> = chats.collect();
        let lit = (chats.iter()).flat_map(|(_, colour, transcript)| {
            transcript.files().into_iter().map(move |(label, running)| Lit { label, colour: *colour, running })
        });
        let working = chats.iter().filter(|(.., transcript)| transcript.pulse() == Pulse::Running);
        let mut tasks: Vec<(TaskId, Agent)> = (working)
            .map(|(task, colour, transcript)| {
                let at = transcript.file_at();
                let near = (transcript.shell_line().filter(|_| at.is_none()))
                    .map_or_else(Vec::new, |line| crate::city_shell::paths(line, dir));
                let (busy, subagents) = (transcript.calling(), transcript.subagents());
                (*task, Agent { who: Who::Task(*task), colour: *colour, at, near, busy, subagents })
            })
            .collect();
        // A task whose agent runs in a terminal (Codex, or `conversation_view: terminal`) has no
        // live chat: its session at work (its file read, as a terminal's is) stands for it. A live
        // chat decides for its own task (idle or asking: no badge, though its file was just written).
        // The sessions drawn here (by a task, or as one no task has): a pane's agent that is one of
        // them shows by it, not twice.
        let mut drawn: Vec<&crate::sessions::Found> = Vec::new();
        let in_terminal = (self.at_work.iter()).filter_map(|s| {
            let task = self.tasks.adopter(&s.runner, &s.id)?;
            let works = works(task.id);
            let chatted = self.chat(&Subject::Task(task.id)).is_some_and(|chat| chat.transcript.live);
            let agent = Agent {
                who: Who::Task(task.id),
                colour: task.colour,
                at: file_of(s),
                near: Vec::new(),
                busy: s.station.is_some(),
                subagents: s.subagents,
            };
            (works && !chatted).then_some((task.id, agent, s))
        });
        let in_terminal: Vec<(TaskId, Agent)> = (in_terminal.collect::<Vec<_>>().into_iter())
            .map(|(task, agent, s)| {
                drawn.push(s);
                (task, agent)
            })
            .collect();
        tasks.extend(in_terminal);
        tasks.sort_by_key(|(task, _)| *task);
        // A session no task has, at work in this checkout (as its Repos dot says), a task's space too.
        let free: Vec<&crate::sessions::Found> =
            (self.at_work.iter()).filter(|s| s.repo == dir && !self.tasks.adopted(&s.runner, &s.id)).collect();
        drawn.extend(free.iter().copied());
        let sessions = free.into_iter().map(|s| Agent {
            who: Who::Session { runner: &s.runner, id: &s.id, title: s.title.as_deref() },
            colour: None,
            at: file_of(s),
            near: Vec::new(),
            busy: s.station.is_some(),
            subagents: s.subagents,
        });
        // An agent CLI in one of our panes, working in this checkout, no session of it on show
        // here: by its process alone (no setting needed), hovering. A task's space: its own panes.
        // Drawn: the session its pane runs (`in_panes`); its session not known, one of its runner
        // in its folder. Another session in the same folder hides no known pane's.
        let shown = |p: &crate::sessions::PaneAgent| match self.in_panes.iter().find(|at| at.pane == p.pane) {
            Some(at) => drawn.iter().any(|s| s.id == at.id),
            None => drawn.iter().any(|s| s.runner == p.runner && s.cwd == p.cwd),
        };
        // In this checkout, not one listed below it (a worktree inside it).
        let inside = |cwd: &str| self.checkout_at(cwd) == Some(dir);
        let mine = |pane: PaneId| match own {
            None => true,
            Some(task) => self
                .tab_of(pane)
                .and_then(|id| self.tabs.iter().find(|tab| tab.id == id))
                .is_some_and(|tab| tab.space == Owner::Task { id: task }),
        };
        let in_panes = (self.pane_agents.iter())
            .filter(|p| inside(&p.cwd) && mine(p.pane) && !shown(p) && self.pane(p.pane).is_some())
            .map(|p| Agent {
                who: Who::Pane { pane: p.pane, runner: &p.runner },
                colour: None,
                at: None,
                near: Vec::new(),
                busy: false,
                subagents: 0,
            });
        let agents: Vec<Agent> = tasks.into_iter().map(|(_, agent)| agent).chain(sessions).chain(in_panes).collect();
        // A session's file is lit too: in its task's colour if a task has it, else in none.
        let lit = lit.chain(agents.iter().filter_map(|agent| match agent.who {
            Who::Session { .. } => Some(Lit { label: agent.at?, colour: None, running: true }),
            Who::Task(task) if !chats.iter().any(|(t, ..)| *t == task) => {
                Some(Lit { label: agent.at?, colour: agent.colour, running: true })
            }
            Who::Task(_) | Who::Pane { .. } => None,
        }));
        // Its sub-agents' files too, in its colour: they may work in another worktree of the
        // repository, whose file a building here has by name (the last two parts of its path).
        let colour_of = |s: &crate::sessions::Found| self.tasks.adopter(&s.runner, &s.id).and_then(|task| task.colour);
        let subs = drawn.iter().flat_map(|s| {
            let colour = colour_of(s);
            s.sub_files.iter().map(move |label| Lit { label, colour, running: true })
        });
        let lit = lit.chain(subs);
        let view = self.city_view(at);
        let glimpses = (chats.iter().filter(|(.., transcript)| transcript.pulse() == Pulse::Running))
            .flat_map(|(task, colour, transcript)| {
                transcript.glances.iter().filter_map(move |glance| glimpse(*task, *colour, glance, dir))
            })
            .collect();
        Some(City { dir, lit: lit.collect(), agents, view, glimpses })
    }

    /// What a click on an agent's icon over a city does: the user's own goes to its task (as
    /// its row does), or opens a session's conversation (as its row does); a client's click, a
    /// task gone, or an agent known by its pane alone, nothing.
    pub fn city_agent_click(&self, who: Who, origin: Origin) -> Option<Action> {
        if origin != Origin::User {
            return None;
        }
        match who {
            Who::Task(task) => self.tasks.live(task).is_some().then_some(Action::TaskGo { id: Some(task) }),
            // A session no task has: its read-only conversation, as its row opens it.
            Who::Session { runner, id, .. } => {
                Some(Action::SessionOpen { runner: runner.to_string(), id: id.to_string() })
            }
            Who::Pane { .. } => None,
        }
    }

    /// The checkout whose last camera a place starts from: its folder's.
    fn camera_of(&self, at: At) -> Option<String> {
        Some(self.empty_folder(at)?.to_string())
    }

    /// The camera of a place's city: its own as the user left it, else its checkout's last one
    /// used (a new tab of it starts there), else the fitted view.
    pub fn city_view(&self, at: At) -> View {
        let Some(folder) = self.camera_of(at) else { return View::default() };
        let own = self.cameras.places.get(&(at, folder.clone())).copied();
        own.or_else(|| self.cameras.views.get(&folder).copied()).unwrap_or_default()
    }

    /// A drag of a city's camera is in flight: Esc cancels it.
    pub fn camera_grabbed(&self) -> bool {
        self.cameras.grab.is_some()
    }

    /// The place's camera is moving: a drag of its own camera in flight that has moved it from
    /// where the drag began. A press that has not moved, or another place's drag, is no motion
    /// (its picture stays full size and named).
    pub fn camera_moving(&self, at: At) -> bool {
        let Some(grab) = &self.cameras.grab else { return false };
        grab.place.0 == at && self.camera_of(at).as_ref() == Some(&grab.place.1) && self.city_view(at) != grab.from
    }

    /// Moves a place's camera by the user's `step`. Clamped: never under the ground, never
    /// past the city's footprint, never nearer than a building or farther than the whole city.
    pub(crate) fn camera(&mut self, at: At, step: CameraStep) {
        // A drag ends, or is put back, even if its city has gone meanwhile (a pane opened over it).
        match step {
            CameraStep::Let => return self.cameras.grab = None,
            CameraStep::Cancel => {
                if let Some(Grab { place, from, folder_was }) = self.cameras.grab.take() {
                    let folder = place.1.clone();
                    self.cameras.places.insert(place, from);
                    match folder_was {
                        Some(was) => self.cameras.views.insert(folder, was),
                        None => self.cameras.views.remove(&folder),
                    };
                }
                return;
            }
            _ => {}
        }
        let Some(key) = self.camera_of(at).filter(|_| self.city_at(at).is_some()) else { return };
        let now = self.city_view(at);
        let view = match step {
            CameraStep::Orbit { yaw, pitch } => now.orbited(yaw, pitch),
            CameraStep::Spin => now.orbited(SPIN_DEGREES, 0.),
            CameraStep::Pan { dx, dz, up } => now.panned(dx, dz, up),
            CameraStep::Zoom { factor, about } => now.zoomed(factor, about),
            CameraStep::Grab => {
                let folder_was = self.cameras.views.get(&key).copied();
                self.cameras.grab = Some(Grab { place: (at, key), from: now, folder_was });
                return;
            }
            // Taken above.
            CameraStep::Let | CameraStep::Cancel => return,
            CameraStep::Reset => View::default(),
        };
        // A step of no number moves nothing.
        if !view.sound() {
            return;
        }
        // Cameras of places gone go; every other place of this checkout on show keeps where it
        // is (it showed the checkout's last camera); then this place's own, and the checkout's.
        let (tabs, panes): (Vec<u32>, Vec<crate::PaneId>) =
            (self.tabs.iter().map(|tab| tab.id).collect(), self.panes.iter().map(|pane| pane.id).collect());
        self.cameras.places.retain(|(place, _), _| match place {
            At::Tab(id) => tabs.contains(id),
            At::Pane(id) => panes.contains(id),
        });
        let behind = self.behind_tab().map(At::Tab);
        for other in self.empty_places().into_iter().chain(behind).filter(|other| *other != at) {
            if self.camera_of(other).as_deref() == Some(key.as_str()) {
                let shown = self.city_view(other);
                self.cameras.places.entry((other, key.clone())).or_insert(shown);
            }
        }
        self.cameras.places.insert((at, key.clone()), view);
        self.cameras.views.insert(key, view);
    }

    /// The user's double-click on a building or district of `at`'s city: the file `id` (from the
    /// checkout's root) in a viewer, as Go to opens one; a folder (`folder`) in a terminal there, as
    /// Repos opens a checkout. In the empty pane the city is in (its tab's, for an empty tab, if
    /// the keyboard is there). Nothing for a place that shows no city, or a path that is not a
    /// plain one inside the checkout (`..`, `.`, empty parts, absolute).
    pub(crate) fn city_open(&mut self, at: At, id: &str, folder: bool, effects: &mut Vec<Effect>) {
        // A file that shares a folder's name has `/` at the end of its id (`atlas::repo`); a folder never.
        let id = if folder { id } else { id.strip_suffix('/').unwrap_or(id) };
        let plain = !id.is_empty() && id.split('/').all(|part| !matches!(part, "" | "." | ".."));
        let shown = plain && self.empty_places().contains(&at);
        let Some(dir) = self.city_at(at).filter(|_| shown).map(|city| city.dir.trim_end_matches('/').to_string())
        else {
            return;
        };
        let path = format!("{dir}/{id}");
        let pane = match at {
            At::Pane(pane) => Some(pane),
            At::Tab(tab) if self.tabs.get(self.active).is_some_and(|shown| shown.id == tab) => None,
            At::Tab(_) => return,
        };
        if folder {
            if let Some(pane) = pane {
                self.focus(pane, effects);
            }
            self.open_terminal_at(Axis::Right, Some(path), effects);
        } else {
            self.open_file(path, Axis::Right, pane, false, None, effects);
        }
    }

    /// Every camera inside its clamps, and a drag only of a camera kept: for the model's check.
    pub fn cameras_sound(&self) -> Result<(), String> {
        let bad = self.cameras.views.values().chain(self.cameras.places.values()).find(|view| !view.sound());
        match bad {
            Some(view) => Err(format!("a camera out of its clamps: {view:?}")),
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
    use crate::transcript::{Event, Outcome, Text};

    /// A task started in the listed checkout `/code/app`, its workspace on show.
    fn task_in_app() -> Model {
        crate::city_replay::Replay::new("/code/app", Features::all()).into_model()
    }

    fn events(m: &mut Model, events: Vec<Event>) {
        m.update(Action::AgentEvents { task: Subject::Task(1), events, at: 5 }.sent_by(Origin::System));
    }

    fn call(id: &str, station: Station, label: &str) -> Event {
        let (id, tool, label) = (id.into(), "Read".into(), label.into());
        Event::ToolStart { id, parent: None, tool, station, label, detail: Text::default(), diff: None, gist: None }
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

    /// A task started in another task's workspace joins its space: its agent floats on that
    /// space's city too, not only the space's own task's.
    #[test]
    fn a_task_that_joined_a_space_floats_on_its_city() {
        let mut m = task_in_app();
        m.update(Action::OpenCompose { text: "second".into() });
        m.update(Action::ComposeEnter);
        assert_eq!(m.space_tasks(&Owner::Task { id: 1 }), [1, 2], "task 2 joined task 1's space");
        m.update(Action::AgentStarted { task: Subject::Task(2), session: None, at: 1 }.sent_by(Origin::System));
        let start = call("r", Station::Shell, "duped report");
        m.update(
            Action::AgentEvents { task: Subject::Task(2), events: vec![Event::TurnStarted, start], at: 5 }
                .sent_by(Origin::System),
        );
        let agents = city(&m).map(|c| c.agents.iter().map(|a| a.who).collect::<Vec<_>>()).unwrap_or_default();
        assert!(agents.contains(&Who::Task(2)), "{agents:?}");
        // One that joined but works in another checkout (`>web`): not on this city.
        let web = Repo { name: "web".into(), path: "/code/web".into(), branch: None, worktrees: vec![] };
        let app = Repo { name: "app".into(), path: "/code/app".into(), branch: None, worktrees: vec![] };
        m.update(Action::SetRepos { repos: vec![app, web] });
        m.update(Action::OpenCompose { text: "third >web".into() });
        m.update(Action::ComposeEnter);
        assert!(m.space_tasks(&Owner::Task { id: 1 }).contains(&3), "task 3 joined task 1's space");
        assert_eq!(m.task_worktree(3), Some("/code/web"));
        m.update(Action::AgentStarted { task: Subject::Task(3), session: None, at: 1 }.sent_by(Origin::System));
        let start = call("w", Station::Shell, "cargo test");
        m.update(
            Action::AgentEvents { task: Subject::Task(3), events: vec![Event::TurnStarted, start], at: 5 }
                .sent_by(Origin::System),
        );
        let agents = city(&m).map(|c| c.agents.iter().map(|a| a.who).collect::<Vec<_>>()).unwrap_or_default();
        assert!(!agents.contains(&Who::Task(3)), "{agents:?}");
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

    /// A new tab the user opens (⌘T) from a tab that keeps a checkout (`open_repo` made it, from
    /// any origin) carries that checkout: its city, and ⌘D there opens a terminal in it. A
    /// client's new tab carries none. A workspace's default is still the user's alone to set.
    #[test]
    fn a_new_tab_carries_its_tabs_checkout() {
        for by in [Origin::User, Origin::Socket] {
            let mut m = listed();
            // A client's `open_repo` from Home: a workspace with no default, its tab the checkout's.
            m.update(Action::OpenRepo { path: "/code/web".into(), axis: Axis::Right }.sent_by(Origin::Socket));
            m.update(Action::Close { pane: None });
            assert_eq!(dir(&m), Some("/code/web"), "{by:?}: the tab keeps it");
            m.update(Action::NewTab.sent_by(by));
            let carried = (by == Origin::User).then_some("/code/web");
            assert_eq!(dir(&m), carried, "{by:?}: the new tab");
            if by == Origin::User {
                m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
                let pane = m.focus_terminal(true).and_then(|id| m.pane(id)).and_then(|p| p.cwd.clone());
                assert_eq!(pane.as_deref(), Some("/code/web"), "⌘D opens in it");
            }
        }
        // Moved to another space, a tab is that space's: its checkout goes with the move.
        let mut m = listed();
        m.update(Action::OpenRepo { path: "/code/web".into(), axis: Axis::Right }.sent_by(Origin::Socket));
        m.update(Action::Close { pane: None });
        let tab = m.tabs[m.active].id;
        m.update(Action::RepoClick { path: "/code/app".into(), axis: Axis::Right });
        let other = m.tabs[m.active].space.clone();
        assert_ne!(Some(&other), m.tabs.iter().find(|t| t.id == tab).map(|t| &t.space));
        m.update(Action::TabPlace { tab: Some(tab), at: crate::model::TabAt::End(other) });
        assert_eq!(m.tabs.iter().find(|t| t.id == tab).and_then(|t| t.folder.clone()), None);
    }

    /// A workspace's empty place lights every task that works in its folder (as the Repos
    /// dots decide), each in its colour; a task's lights only that task's.
    #[test]
    fn a_workspace_lights_every_task_in_its_folder_and_a_task_only_its_own() {
        let mut m = task_in_app();
        let one = m.tasks.get(1).and_then(|task| task.colour);
        events(&mut m, vec![Event::TurnStarted, call("a", Station::Files, "src/main.rs")]);
        // A second task in the same checkout, at another file: started from Home, a space of its own.
        m.update(Action::SpaceOpen { space: Owner::Home });
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

    fn agents(m: &Model) -> Option<Vec<(TaskId, Option<&str>)>> {
        let task = |agent: &Agent| match agent.who {
            Who::Task(task) => task,
            Who::Session { .. } | Who::Pane { .. } => 0,
        };
        city(m).map(|city| city.agents.iter().map(|agent| (task(agent), agent.at)).collect())
    }

    /// A session no task has, at work in the checkout (a `claude` in the user's terminal): an
    /// agent of the city at the file it reads, that file lit, in a task's space of the checkout
    /// too; idle, gone.
    #[test]
    fn a_session_in_a_terminal_floats_over_its_file() {
        let mut m = listed();
        m.update(Action::RepoClick { path: "/code/app".into(), axis: Axis::Right });
        m.update(Action::Close { pane: None });
        let found = |active: bool, station: &str, label: &str| crate::sessions::Found {
            runner: "claude".into(),
            id: "s1".into(),
            cwd: "/code/app".into(),
            repo: "/code/app".into(),
            started: Some(1),
            last_activity: Some(2),
            active,
            station: Some(station.into()),
            label: Some(label.into()),
            title: Some("fix the loop".into()),
            model: None,
            held: true,
            subagents: 0,
            sub_files: Vec::new(),
        };
        m.update(Action::SetSessions { found: vec![found(true, "files", "src/main.rs")], day: 0 });
        let city = city(&m).unwrap();
        let who = Who::Session { runner: "claude", id: "s1", title: Some("fix the loop") };
        let near = Vec::new();
        assert_eq!(
            city.agents,
            vec![Agent { who, colour: None, at: Some("src/main.rs"), near, busy: true, subagents: 0 }]
        );
        assert_eq!(city.lit, vec![Lit { label: "src/main.rs", colour: None, running: true }]);
        m.update(Action::SetSessions { found: vec![found(true, "shell", "cargo test")], day: 0 });
        assert_eq!(agents(&m), Some(vec![(0, None)]), "a shell line: at no file");
        m.update(Action::SetSessions { found: vec![found(false, "files", "src/main.rs")], day: 0 });
        assert_eq!(agents(&m), Some(vec![]));
        // A task's space of the checkout: there too.
        let mut m = task_in_app();
        m.update(Action::SetSessions { found: vec![found(true, "files", "src/main.rs")], day: 0 });
        let shown = m.empty_places().first().and_then(|at| m.city_at(*at));
        let who = shown.map(|c| c.agents.iter().map(|a| a.who).collect::<Vec<_>>());
        assert!(who.is_some_and(|w| w.contains(&Who::Session {
            runner: "claude",
            id: "s1",
            title: Some("fix the loop")
        })));
    }

    /// The owner's case: a `claude` in another terminal, its own file quiet while its sub-agents
    /// work (in worktrees of the repository, under a temp folder). Read at work by their files,
    /// it is an agent of the city with its sub-agents counted, and their files are lit by name.
    #[test]
    fn a_terminal_session_waiting_on_its_subagents_floats_with_them_and_lights_their_files() {
        let mut m = listed();
        m.update(Action::RepoClick { path: "/code/app".into(), axis: Axis::Right });
        m.update(Action::Close { pane: None });
        let found = crate::sessions::Found {
            runner: "claude".into(),
            id: "s1".into(),
            cwd: "/code/app".into(),
            repo: "/code/app".into(),
            started: Some(1),
            last_activity: Some(2),
            active: false,
            station: Some("agents".into()),
            label: Some("Explore".into()),
            title: None,
            model: None,
            held: true,
            subagents: 2,
            sub_files: vec!["src/lib.rs".into(), "docs/a.md".into()],
        };
        m.update(Action::SetSessions { found: vec![found], day: 0 }.sent_by(Origin::System));
        let city = city(&m).unwrap();
        assert_eq!(city.agents.iter().map(|a| (a.at, a.subagents)).collect::<Vec<_>>(), [(None, 2)]);
        let lit: Vec<&str> = city.lit.iter().map(|lit| lit.label).collect();
        assert_eq!(lit, ["src/lib.rs", "docs/a.md"]);
    }

    /// An agent floats over the file it is at now: its newest call's; with a shell line its
    /// newest, at none, still working; its turn over, it is gone.
    #[test]
    fn an_agent_is_at_its_newest_file_while_it_works() {
        let mut m = task_in_app();
        assert_eq!(agents(&m), Some(vec![(1, None)]), "its first turn runs, at no file yet");
        events(&mut m, vec![Event::TurnStarted, call("a", Station::Files, "src/main.rs")]);
        assert_eq!(agents(&m), Some(vec![(1, Some("src/main.rs"))]));
        assert_eq!(city(&m).unwrap().agents[0].colour, m.tasks.get(1).and_then(|task| task.colour));
        // Two calls run: the newer one's file, whichever sorts first.
        events(&mut m, vec![call("b", Station::Files, "docs/a.md")]);
        assert_eq!(agents(&m), Some(vec![(1, Some("docs/a.md"))]));
        // The newer ends: the older still runs, and is where it is.
        events(&mut m, vec![end("b"), call("c", Station::Shell, "cargo test")]);
        assert_eq!(agents(&m), Some(vec![(1, Some("src/main.rs"))]));
        events(&mut m, vec![end("a")]);
        assert_eq!(agents(&m), Some(vec![(1, None)]), "a shell line: at no file");
        events(&mut m, vec![end("c"), Event::TurnFinished { outcome: Outcome::Completed }]);
        assert_eq!(agents(&m), Some(vec![]));
    }

    /// The owner's case: a chat whose turn is a Skill call, then shell lines, no file read or
    /// edited. The agent is in the city from its first event, at no file (so it hovers).
    #[test]
    fn an_agent_of_a_skill_and_shell_lines_shows_from_its_first_event() {
        let mut m = task_in_app();
        let tool = |id: &str, tool: &str, station: Station, label: &str| Event::ToolStart {
            id: id.into(),
            parent: None,
            tool: tool.into(),
            station,
            label: label.into(),
            detail: Text::default(),
            diff: None,
            gist: None,
        };
        let turn = [
            vec![Event::TurnStarted],
            vec![tool("a", "Skill", Station::Skills, "review")],
            vec![end("a"), tool("b", "Bash", Station::Shell, "git status")],
            vec![end("b"), tool("c", "Bash", Station::Shell, "cargo test")],
            vec![end("c")],
        ];
        for (n, step) in turn.into_iter().enumerate() {
            events(&mut m, step);
            assert_eq!(agents(&m), Some(vec![(1, None)]), "after step {n}");
            assert_eq!(labels(&m), Some(vec![]), "nothing lit, step {n}");
        }
    }

    /// The owner's third case: shell lines only, each `cd <checkout> && <cmd> <path>`. While one
    /// runs, the agent is near the paths it names (the shell puts it over that building or
    /// plinth) and busy; between calls, near none and not busy; a file call wins over a line.
    #[test]
    fn a_shell_line_puts_its_agent_near_the_paths_it_names() {
        let mut m = task_in_app();
        let bash = |id: &str, line: &str| Event::ToolStart {
            id: id.into(),
            parent: None,
            tool: "Bash".into(),
            station: Station::Shell,
            label: line.split(' ').nth(3).unwrap_or("").into(),
            detail: Text::new(line),
            diff: None,
            gist: None,
        };
        let near = |m: &Model| city(m).map(|city| (city.agents[0].near.clone(), city.agents[0].busy));
        events(&mut m, vec![Event::TurnStarted]);
        assert_eq!(near(&m), Some((vec![], false)));
        let lines = [
            ("a", "cd /code/app && cat src/main.rs", vec!["src/main.rs"]),
            ("b", "cd /code/app && rg -n loop src/model docs", vec!["src/model"]),
            ("c", "cd /code/app && git log --oneline -5", vec![]),
            ("d", "cd /code/app && wc -l src/lib.rs src/main.rs", vec!["src/lib.rs", "src/main.rs"]),
        ];
        for (id, line, want) in lines {
            events(&mut m, vec![bash(id, line)]);
            let want: Vec<String> = want.into_iter().map(String::from).collect();
            assert_eq!(near(&m), Some((want, true)), "{line}");
            assert_eq!(agents(&m), Some(vec![(1, None)]), "{line}");
            events(&mut m, vec![end(id)]);
            assert_eq!(near(&m), Some((vec![], false)), "{line}: ended");
        }
        // A file call beside a running line: at the file, near nothing.
        events(&mut m, vec![bash("e", "cd /code/app && cat docs/a.md"), call("f", Station::Files, "src/x.rs")]);
        assert_eq!(agents(&m), Some(vec![(1, Some("src/x.rs"))]));
        assert_eq!(near(&m), Some((vec![], true)));
    }

    /// An agent CLI in one of our panes, working in the checkout, shows by its process alone (no
    /// session read, no setting): in a workspace's city of that checkout, hovering; not for
    /// another checkout; gone with its pane.
    #[test]
    fn an_agent_cli_in_a_pane_shows_by_its_process() {
        let mut m = listed();
        m.update(Action::RepoClick { path: "/code/app".into(), axis: Axis::Right });
        let pane = m.focus_terminal(true).unwrap();
        m.update(Action::NewTab);
        let found = |cwd: &str| crate::sessions::PaneAgent { pane, runner: "claude".into(), cwd: cwd.into() };
        m.update(Action::PaneAgents { found: vec![found("/code/app/src")] }.sent_by(Origin::System));
        fn who(m: &Model) -> Option<Vec<String>> {
            city(m).map(|c| c.agents.iter().map(|a| format!("{:?} {:?}", a.who, a.at)).collect())
        }
        let want = format!("{:?} None", Who::Pane { pane, runner: "claude" });
        assert_eq!(who(&m), Some(vec![want]));
        m.update(Action::PaneAgents { found: vec![found("/code/application")] }.sent_by(Origin::System));
        assert_eq!(who(&m), Some(vec![]), "another checkout's");
        // Its session read too (`sessions` on): shown by the session, not twice.
        let session = crate::sessions::Found {
            runner: "claude".into(),
            id: "s1".into(),
            cwd: "/code/app/src".into(),
            repo: "/code/app".into(),
            started: Some(1),
            last_activity: Some(2),
            active: true,
            station: None,
            label: None,
            title: None,
            model: None,
            held: true,
            subagents: 0,
            sub_files: Vec::new(),
        };
        m.update(Action::PaneAgents { found: vec![found("/code/app/src")] }.sent_by(Origin::System));
        m.update(Action::SetSessions { found: vec![session], day: 0 });
        assert_eq!(who(&m).map(|w| w.len()), Some(1));
        assert!(who(&m).is_some_and(|w| w[0].starts_with("Session")), "{:?}", who(&m));
        // Its pane known to run another session, idle: the one at work hides its icon no more.
        let mine = crate::sessions::InPane {
            pane,
            runner: "claude".into(),
            id: "s0".into(),
            last_activity: Some(1),
            waits_for: None,
            cwd: Some("/code/app/src".into()),
            busy: false,
        };
        m.update(Action::SessionsInPanes { found: vec![mine] }.sent_by(Origin::System));
        assert_eq!(who(&m).map(|w| w.len()), Some(2), "{:?}", who(&m));
        m.update(Action::SessionsInPanes { found: vec![] }.sent_by(Origin::System));
        m.update(Action::SetSessions { found: vec![], day: 0 });
        m.update(Action::PaneAgents { found: vec![found("/code/app")] }.sent_by(Origin::System));
        m.update(Action::Close { pane: Some(pane) });
        assert_eq!(who(&m).map(|w| w.len()), Some(0), "its pane closed");
    }

    /// An agent counts the sub-agents it runs: its `Agent`/`Task` calls not ended (a sub-agent's
    /// own calls are not its).
    #[test]
    fn an_agent_counts_the_subagents_it_runs() {
        let mut m = task_in_app();
        let agent = |id: &str, sub: bool| Event::ToolStart {
            id: id.into(),
            parent: sub.then(|| "p".into()),
            tool: "Agent".into(),
            station: Station::Agents,
            label: "Explore".into(),
            detail: Text::default(),
            diff: None,
            gist: None,
        };
        let subs = |m: &Model| city(m).map(|c| c.agents[0].subagents);
        events(&mut m, vec![Event::TurnStarted, agent("a", false), agent("b", false), agent("c", true)]);
        assert_eq!(subs(&m), Some(2));
        events(&mut m, vec![end("a")]);
        assert_eq!(subs(&m), Some(1));
        events(&mut m, vec![end("b")]);
        assert_eq!(subs(&m), Some(0));
        // Interrupted: a call whose end never comes runs no more.
        events(&mut m, vec![agent("d", false), Event::TurnFinished { outcome: Outcome::Interrupted }]);
        assert_eq!(city(&m).map(|c| c.agents.iter().map(|a| a.subagents).sum::<usize>()), Some(0));
    }

    /// Reads and searches as the city's glimpses: a read's file, a search's scope (none: the
    /// whole checkout) and, once it ends, the files its result names; outside the checkout, none.
    #[test]
    fn an_agents_reads_and_searches_are_glimpses_of_the_city() {
        let mut m = task_in_app();
        let start = |id: &str, tool: &str, station: Station, detail: &str| Event::ToolStart {
            id: id.into(),
            parent: None,
            tool: tool.into(),
            station,
            label: String::new(),
            detail: Text::new(detail),
            diff: None,
            gist: None,
        };
        let done = |id: &str, out: &str| Event::ToolEnd { id: id.into(), failed: false, output: Some(Text::new(out)) };
        events(
            &mut m,
            vec![
                Event::TurnStarted,
                start("r", "Read", Station::Files, "/code/app/src/main.rs"),
                start("x", "Read", Station::Files, "/etc/hosts"),
                start("g", "Grep", Station::Files, r#"{"pattern":"loop"}"#),
                start("b", "Bash", Station::Shell, "cd /code/app && rg -n loop src/model docs"),
                start("o", "Bash", Station::Shell, "rg password /etc"),
                start("e", "Bash", Station::Shell, "cd /elsewhere && rg -n loop"),
                start("c", "Bash", Station::Shell, "cd src && rg -n loop"),
            ],
        );
        let glimpses = |m: &Model| {
            city(m).map(|c| {
                c.glimpses
                    .iter()
                    .map(|g| (g.id.clone(), g.search, g.scope.clone(), g.found.clone()))
                    .collect::<Vec<_>>()
            })
        };
        let s = |v: &[&str]| v.iter().map(|p| p.to_string()).collect::<Vec<_>>();
        let want = vec![
            ("1:r".to_string(), false, s(&["src/main.rs"]), s(&[])),
            ("1:g".to_string(), true, s(&[]), s(&[])),
            ("1:b".to_string(), true, s(&["src/model", "docs"]), s(&[])),
            ("1:c".to_string(), true, s(&["src"]), s(&[])),
        ];
        assert_eq!(glimpses(&m), Some(want.clone()));
        events(
            &mut m,
            vec![done(
                "g",
                "/code/app/src/lib.rs:3:loop
src/main.rs:9:loop
/etc/x:1:loop",
            )],
        );
        // A result from where the search ran: `cd src && rg` names `ui/mod.rs`, `src/ui/mod.rs`.
        events(&mut m, vec![done("c", "ui/mod.rs:3:loop")]);
        let mut found = want;
        found[1].3 = s(&["src/lib.rs", "src/main.rs"]);
        found[3].3 = s(&["src/ui/mod.rs"]);
        assert_eq!(glimpses(&m), Some(found));
        // Its turn over: none.
        events(&mut m, vec![Event::TurnFinished { outcome: Outcome::Completed }]);
        assert_eq!(glimpses(&m), Some(vec![]));
    }

    /// A task whose agent runs in a terminal (no chat: Codex, or the terminal view) floats in its
    /// city by its session at work, at its file, lit in its colour; once the session is idle, it is
    /// gone. A task with a live chat floats by its chat alone, never twice.
    #[test]
    fn a_task_in_a_terminal_floats_by_its_session() {
        let mut m = Model::default();
        let app = Repo { name: "app".into(), path: "/code/app".into(), branch: None, worktrees: vec![] };
        m.update(Action::SetRepos { repos: vec![app] });
        m.update(Action::RepoClick { path: "/code/app".into(), axis: Axis::Right });
        m.update(Action::OpenCompose { text: "first".into() });
        m.update(Action::ComposeEnter);
        m.update(Action::Close { pane: None });
        assert!(m.chat(&Subject::Task(1)).is_none(), "its agent has no chat");
        let found = |id: &str, active: bool, station: &str, label: &str| crate::sessions::Found {
            runner: "codex".into(),
            id: id.into(),
            cwd: "/code/app".into(),
            repo: "/code/app".into(),
            started: Some(1),
            last_activity: Some(2),
            active,
            station: Some(station.into()),
            label: Some(label.into()),
            title: None,
            model: None,
            held: true,
            subagents: 0,
            sub_files: Vec::new(),
        };
        m.tasks.link(1, crate::sessions::Link { runner: "codex".into(), id: "s9".into(), state: None });
        m.update(Action::SetSessions { found: vec![found("s9", true, "files", "src/main.rs")], day: 0 });
        assert_eq!(agents(&m), Some(vec![(1, Some("src/main.rs"))]));
        let colour = m.tasks.get(1).and_then(|task| task.colour);
        assert_eq!(city(&m).unwrap().lit, vec![Lit { label: "src/main.rs", colour, running: true }]);
        m.update(Action::SetSessions { found: vec![found("s9", true, "shell", "cargo test")], day: 0 });
        assert_eq!(agents(&m), Some(vec![(1, None)]), "a shell line: at the edge");
        m.update(Action::SetSessions { found: vec![], day: 0 });
        assert_eq!(agents(&m), Some(vec![]));
        // A task with a live chat: its chat decides; its session at work adds nothing.
        let mut m = task_in_app();
        m.tasks.link(1, crate::sessions::Link { runner: "codex".into(), id: "s9".into(), state: None });
        m.update(Action::SetSessions { found: vec![found("s9", true, "files", "src/lib.rs")], day: 0 });
        assert_eq!(agents(&m), Some(vec![(1, None)]), "its turn runs, at no file yet: once");
    }

    /// A workspace's city floats every task working in its folder, by task; a task's only its own.
    #[test]
    fn a_workspace_floats_every_working_task_and_a_task_only_its_own() {
        let mut m = task_in_app();
        events(&mut m, vec![Event::TurnStarted, call("a", Station::Files, "src/main.rs")]);
        // Started from Home: not in task 1's space, though in its checkout.
        m.update(Action::SpaceOpen { space: Owner::Home });
        m.update(Action::OpenCompose { text: "second >app".into() });
        m.update(Action::ComposeEnter);
        m.update(Action::AgentStarted { task: Subject::Task(2), session: None, at: 1 }.sent_by(Origin::System));
        let read = vec![Event::TurnStarted, call("b", Station::Files, "src/main.rs")];
        m.update(Action::AgentEvents { task: Subject::Task(2), events: read, at: 5 }.sent_by(Origin::System));
        m.update(Action::SpaceOpen { space: Owner::Task { id: 1 } });
        assert_eq!(agents(&m), Some(vec![(1, Some("src/main.rs"))]));
        m.update(Action::SpaceOpen { space: Owner::Home });
        m.update(Action::OpenRepo { path: "/code/app".into(), axis: Axis::Right });
        m.update(Action::Close { pane: None });
        assert_eq!(agents(&m), Some(vec![(1, Some("src/main.rs")), (2, Some("src/main.rs"))]));
    }

    /// Only the user's own click on an agent goes to its task.
    #[test]
    fn only_the_users_click_on_an_agent_goes_to_its_task() {
        let m = task_in_app();
        assert_eq!(m.city_agent_click(Who::Task(1), Origin::User), Some(Action::TaskGo { id: Some(1) }));
        assert_eq!(m.city_agent_click(Who::Task(1), Origin::Socket), None);
        assert_eq!(m.city_agent_click(Who::Task(1), Origin::System), None);
        assert_eq!(m.city_agent_click(Who::Task(9), Origin::User), None, "no such task");
        let session = Who::Session { runner: "claude", id: "s1", title: None };
        let open = Action::SessionOpen { runner: "claude".into(), id: "s1".into() };
        assert_eq!(m.city_agent_click(session, Origin::User), Some(open), "its conversation, as its row opens it");
        assert_eq!(m.city_agent_click(session, Origin::Socket), None);
        assert_eq!(m.city_agent_click(Who::Pane { pane: 1, runner: "claude" }, Origin::User), None);
    }

    /// The user's double-click opens a building's file in a viewer and a district's folder in a
    /// terminal there, in the empty place the city is in; a client's is refused, as is a path out
    /// of the checkout, and a place that shows no city.
    #[test]
    fn the_users_double_click_opens_a_file_or_a_folder_from_the_city() {
        let open = |m: &mut Model, at: At, id: &str, folder: bool, by: Origin| {
            let before = m.panes.len();
            m.update(Action::CityOpen { at, id: id.into(), folder }.sent_by(by));
            (m.panes.len() > before).then(|| m.panes.last().map(|p| (p.kind.clone(), p.cwd.clone()))).flatten()
        };
        let file = |path: &str| Some((Kind::File { path: path.into() }, None));
        for (id, folder) in [("src/a.rs", false), ("src", true), ("../etc/x", false), ("", true), ("/etc", false)] {
            let mut m = task_in_app();
            let at = m.empty_places()[0];
            assert_eq!(open(&mut m, at, id, folder, Origin::Socket), None, "{id}: a client's");
            assert_eq!(open(&mut m, at, id, folder, Origin::System), None, "{id}: the system's");
        }
        let mut m = task_in_app();
        let at = m.empty_places()[0];
        for bad in ["../etc/passwd", "src/../../x", "./a", "a//b", "", "/etc/x"] {
            assert_eq!(open(&mut m, at, bad, false, Origin::User), None, "{bad}");
        }
        assert_eq!(open(&mut m, at, "src/a.rs", false, Origin::User), file("/code/app/src/a.rs"));
        // The viewer covers the floor: no city there now, so no second open.
        assert_eq!(open(&mut m, at, "src/b.rs", false, Origin::User), None);
        // A district: a terminal in its folder. A file that shares a folder's name (`a/`) is the file.
        let mut m = task_in_app();
        let at = m.empty_places()[0];
        assert_eq!(
            open(&mut m, at, "src/ui", true, Origin::User),
            Some((Kind::Terminal, Some("/code/app/src/ui".into())))
        );
        let mut m = task_in_app();
        assert_eq!(open(&mut m, at, "a/", false, Origin::User), file("/code/app/a"));
        // An empty pane's city: it fills that pane, beside a terminal.
        let mut m = task_in_app();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Empty });
        let empty = m.empty_places();
        let [At::Pane(pane)] = empty[..] else { panic!("{empty:?}") };
        m.update(Action::FocusCycle { delta: 1 });
        assert_eq!(open(&mut m, empty[0], "src/a.rs", false, Origin::User), None, "fills the pane: none new");
        assert_eq!(m.pane(pane).map(|p| p.kind.clone()), Some(Kind::File { path: "/code/app/src/a.rs".into() }));
        // A folder's id has no `/` at its end.
        let mut m = task_in_app();
        assert_eq!(open(&mut m, at, "src/", true, Origin::User), None);
        // A city not on show (its tab behind another): nothing opens, and the tab stays behind.
        let mut m = task_in_app();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Empty });
        let [hidden] = m.empty_places()[..] else { panic!() };
        m.update(Action::SpaceOpen { space: Owner::Home });
        let shown = m.active;
        assert_eq!(open(&mut m, hidden, "src/a.rs", false, Origin::User), None);
        assert_eq!((m.active, m.empty_places().contains(&hidden)), (shown, false));
        let At::Pane(pane) = hidden else { panic!() };
        assert_eq!(m.pane(pane).map(|p| p.kind.clone()), Some(Kind::Empty));
        // While the key reference has the keyboard, it waits, as any pane action.
        let mut m = task_in_app();
        m.update(Action::ToggleKeys);
        assert_eq!(open(&mut m, at, "src/a.rs", false, Origin::User), None);
        assert!(m.empty_places().contains(&at));
        // With the flag off, no city: nothing opens.
        let mut m = task_in_app();
        m.update(Action::SetFeatures { features: Features::default() }.sent_by(Origin::System));
        assert_eq!(open(&mut m, at, "src/a.rs", false, Origin::User), None);
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
        let moved = step(&mut m, CameraStep::Pan { dx: 5., dz: f32::NAN, up: f32::INFINITY }, Origin::User).unwrap();
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

    /// Two places of one checkout turn apart: each keeps its own camera. A new place starts from
    /// the checkout's last camera used, and that is what is kept for it (`layout.json`).
    #[test]
    fn each_place_has_its_own_camera_and_a_new_one_starts_from_the_last_used() {
        let mut m = task_in_app();
        // Two empty panes of the checkout, beside a terminal, both at the fitted view.
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Empty });
        m.update(Action::Split { axis: Axis::Down, kind: Kind::Empty });
        let [a, b] = m.empty_places()[..] else { panic!("{:?}", m.empty_places()) };
        m.update(Action::CityCamera { at: a, step: CameraStep::Orbit { yaw: 30., pitch: 0. } });
        let turned = m.city_view(a);
        assert!(turned != View::default());
        assert_eq!(m.city_view(b), View::default(), "b, untouched, stays where it was");
        m.update(Action::CityCamera { at: b, step: CameraStep::Orbit { yaw: -50., pitch: 0. } });
        assert_eq!(m.city_view(a), turned, "a turns apart from b");
        // A new place starts from the last used: b's. That is the one kept.
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Empty });
        let c = *m.empty_places().iter().find(|p| **p != a && **p != b).unwrap();
        assert_eq!(m.city_view(c), m.city_view(b));
        assert_eq!(m.cameras.views.get("/code/app"), Some(&m.city_view(b)), "kept: the last used");
        // Esc on a drag of a puts a and the checkout's last used back where they were.
        let last = m.city_view(b);
        for step in [CameraStep::Grab, CameraStep::Orbit { yaw: 5., pitch: 0. }, CameraStep::Cancel] {
            m.update(Action::CityCamera { at: a, step });
        }
        assert_eq!((m.city_view(a), m.cameras.views.get("/code/app")), (turned, Some(&last)));
    }

    /// Esc during a drag puts the camera back where the drag began; a drag let go keeps it.
    #[test]
    fn a_drag_is_motion_once_it_moves_and_only_for_its_own_checkout() {
        let mut m = task_in_app();
        let at = m.empty_places()[0];
        let go = |m: &mut Model, step| drop(m.update(Action::CityCamera { at, step }));
        go(&mut m, CameraStep::Grab);
        assert!(m.camera_grabbed() && !m.camera_moving(at), "a press that has not moved");
        go(&mut m, CameraStep::Orbit { yaw: 10., pitch: 0. });
        assert!(m.camera_moving(at));
        // Another place of no checkout of this drag: none.
        m.update(Action::SpaceOpen { space: Owner::Home });
        assert!(m.empty_places().iter().all(|other| *other == at || !m.camera_moving(*other)));
        m.update(Action::SpaceOpen { space: Owner::Task { id: 1 } });
        go(&mut m, CameraStep::Let);
        assert!(!m.camera_moving(at));
    }

    /// A terminal pane draws no city, but its folder's city can stand under it (a test run's
    /// `city_under`): the same folder the empty tab drew.
    #[test]
    fn a_terminal_pane_has_its_folders_city_under_it() {
        let mut m = task_in_app();
        let at = m.empty_places()[0];
        let dir = m.city_at(at).map(|city| city.dir.to_string());
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        let pane = m.focused.unwrap();
        assert!(m.city_at(At::Pane(pane)).is_none());
        assert_eq!(m.city_under(pane).map(|city| city.dir.to_string()), dir);
    }

    /// A tab's city behind its panes (`city_behind`): only while a terminal or an explorer in it
    /// lets it show; its tab's camera; the other panes, and every pane with none behind, opaque.
    #[test]
    fn a_tabs_city_is_behind_its_panes_only_while_one_lets_it_show() {
        let mut m = task_in_app();
        let tab = m.tabs[m.main_at()].id;
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        let terminal = m.focused.unwrap();
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Explorer { root: "/code/app".into() } });
        let explorer = m.focused.unwrap();
        m.update(Action::Split { axis: Axis::Down, kind: Kind::Empty });
        let empty = m.focused.unwrap();
        let settings = |m: &mut Model, text: &str| {
            m.update(Action::SettingsRead { text: text.into(), written: true }.sent_by(Origin::System))
        };
        let opacities = |m: &Model| [terminal, explorer, empty].map(|pane| m.pane_opacity(pane));
        // Opaque, as by default: nothing behind.
        assert!(m.city_behind().is_none());
        assert_eq!(opacities(&m), [1., 1., 1.]);
        settings(&mut m, r#"{"terminal_opacity": 0.85, "explorer_opacity": 0.2}"#);
        let (at, city) = m.city_behind().unwrap();
        assert_eq!((at, city.dir), (At::Tab(tab), "/code/app"));
        assert_eq!(city.view, m.city_view(At::Tab(tab)), "the tab's own camera");
        assert_eq!(opacities(&m), [0.85, 0.6, 1.], "kept in range; an empty pane draws its own city");
        assert_eq!(m.see_through_panes(), [(terminal, 0.85), (explorer, 0.6)]);
        // The empty pane's city turns; the one behind stays where it was.
        let behind = m.city_view(At::Tab(tab));
        m.update(Action::CityCamera { at: At::Pane(empty), step: CameraStep::Orbit { yaw: 30., pitch: 0. } });
        assert!(m.city_view(At::Pane(empty)) != behind);
        assert_eq!(m.city_view(At::Tab(tab)), behind);
        // Only the explorer lets it show: still behind.
        settings(&mut m, r#"{"explorer_opacity": 0.9}"#);
        assert_eq!(opacities(&m), [1., 0.9, 1.]);
        assert!(m.city_behind().is_some());
        // The flag off: none, all opaque.
        m.update(Action::SetFeatures { features: m.features.with(Feature::CityBehind, false) }.sent_by(Origin::System));
        assert!(m.city_behind().is_none());
        assert_eq!(opacities(&m), [1., 1., 1.]);
    }

    /// A spin is a full turn, in motion all the way (half-size pictures, as a drag), and leaves
    /// the camera where it was.
    #[test]
    fn a_spin_turns_once_round_in_motion_and_puts_the_camera_back() {
        let mut m = task_in_app();
        let at = m.empty_places()[0];
        let go = |m: &mut Model, step| drop(m.update(Action::CityCamera { at, step }));
        go(&mut m, CameraStep::Orbit { yaw: 20., pitch: 5. });
        let was = m.city_view(at);
        go(&mut m, CameraStep::Grab);
        for i in 1..=SPIN_STEPS {
            go(&mut m, CameraStep::Spin);
            let turned = (m.city_view(at).yaw - was.yaw).rem_euclid(360.);
            let want = (i as f32 * SPIN_DEGREES).rem_euclid(360.);
            assert!((turned - want).abs() < 1e-2 || (turned - want).abs() > 360. - 1e-2, "step {i}: {turned}");
            assert!(i == SPIN_STEPS || m.camera_moving(at), "step {i}: not in motion");
            assert_eq!(m.city_view(at).pitch, was.pitch);
        }
        go(&mut m, CameraStep::Cancel);
        assert_eq!((m.city_view(at), m.camera_grabbed()), (was, false));
    }

    #[test]
    fn a_cancelled_drag_puts_the_camera_back() {
        let mut m = task_in_app();
        let at = m.empty_places()[0];
        let go = |m: &mut Model, step| drop(m.update(Action::CityCamera { at, step }));
        go(&mut m, CameraStep::Grab);
        go(&mut m, CameraStep::Orbit { yaw: 40., pitch: -5. });
        go(&mut m, CameraStep::Pan { dx: 0.2, dz: 0.1, up: 0.1 });
        assert!(m.camera_grabbed());
        go(&mut m, CameraStep::Cancel);
        assert_eq!((m.city_view(at), m.camera_grabbed()), (View::default(), false));
        go(&mut m, CameraStep::Grab);
        go(&mut m, CameraStep::Orbit { yaw: 40., pitch: 0. });
        go(&mut m, CameraStep::Let);
        go(&mut m, CameraStep::Cancel);
        assert_eq!(m.city_view(at).yaw, View::default().yaw + 40.);
        // A pane opened over the city mid-drag: Esc still puts it back, and ends the drag.
        go(&mut m, CameraStep::Grab);
        go(&mut m, CameraStep::Orbit { yaw: 30., pitch: 0. });
        m.update(Action::Split { axis: Axis::Right, kind: Kind::Terminal });
        assert!(m.city_at(at).is_none());
        go(&mut m, CameraStep::Cancel);
        m.update(Action::Close { pane: None });
        assert_eq!((m.city_view(at).yaw, m.camera_grabbed()), (View::default().yaw + 40., false));
    }
}
