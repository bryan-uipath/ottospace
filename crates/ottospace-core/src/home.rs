//! The home screen: what the empty workspace shows. Listed tasks sit as nodes on a lattice, a
//! grid on a floor seen in perspective. Left to right is the project, near to far is the age.

use serde::Serialize;

use crate::tasks::{Group, Task, TaskId, Tasks};

/// Most tasks shown as nodes: the first rows of the inbox.
pub const MAX_NODES: usize = 12;
/// Most tasks listed under "Needs you".
pub const MAX_NEEDS_YOU: usize = 3;

/// A camera over the lattice's floor. Screen positions are fractions of the drawn area (0,0 top
/// left; 1,1 bottom right), so the shell draws the lines and the nodes with one projection.
#[derive(Clone, Copy, Debug)]
pub struct View {
    /// The vanishing point of "straight ahead".
    pub cx: f32,
    pub horizon: f32,
    /// How far one unit to the side, and the camera's height, reach at depth 1.
    pub fx: f32,
    pub fy: f32,
    /// The grid is turned by this angle: its sine and cosine.
    pub sin: f32,
    pub cos: f32,
    /// The camera stands this far behind crossing (0, 0).
    pub dist: f32,
    /// Grid lines run from `lo` to `hi` in both directions.
    pub lo: i32,
    pub hi: i32,
}

/// Nothing nearer the camera than this is drawn.
pub const NEAR: f32 = 0.6;

/// The home screen's camera: two vanishing points, the grid turned 34 degrees.
pub const HOME: View = View {
    cx: 0.4624,
    horizon: 0.2788,
    fx: 0.5925,
    fy: 1.9712,
    sin: 0.559_193,
    cos: 0.829_038,
    dist: 6.5,
    lo: -9,
    hi: 15,
};

impl View {
    /// Distance from the camera. Lines fade and nodes shrink with it.
    pub fn depth(&self, x: f32, z: f32) -> f32 {
        x * self.sin + z * self.cos + self.dist
    }

    /// Where the floor point (x, z) is drawn. Only for points with `depth` over `NEAR`.
    pub fn project(&self, x: f32, z: f32) -> (f32, f32) {
        let w = self.depth(x, z);
        (self.cx + self.fx * (x * self.cos - z * self.sin) / w, self.horizon + self.fy / w)
    }
}

/// One task on the lattice.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Node {
    pub task: TaskId,
    /// Its inbox heading: the node's colour.
    pub group: Group,
    /// Its project's place, from 0 on the left. Tasks of no project share the last lane.
    pub lane: usize,
    /// Its rank by age among the shown tasks: 0 is the newest and the nearest.
    pub depth: usize,
    /// The grid crossing (x, z) it sits on.
    pub crossing: (i32, i32),
}

/// How many listed tasks each heading has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Counts {
    pub needs_you: usize,
    pub running: usize,
    pub captured: usize,
    pub on_hold: usize,
    pub done: usize,
}

#[derive(Debug, Serialize)]
pub struct Home<'a> {
    pub nodes: Vec<Node>,
    /// The first tasks that need the user, most urgent first.
    pub needs_you: Vec<&'a Task>,
    pub counts: Counts,
}

/// True where a node may sit: on screen, clear of the new-task box (top middle) and the legend
/// (bottom left).
fn open(x: f32, y: f32) -> bool {
    let on_screen = (0.04..0.96).contains(&x) && (0.47..0.94).contains(&y);
    let under_box = (0.26..0.74).contains(&x) && y < 0.63;
    let under_legend = x < 0.36 && y > 0.90;
    on_screen && !under_box && !under_legend
}

/// A crossing (x, z), with where it is drawn.
type Spot = ((i32, i32), (f32, f32));

/// The crossings a node may sit on.
fn crossings(view: &View) -> Vec<Spot> {
    let range = || view.lo..=view.hi;
    let all = range().flat_map(|x| range().map(move |z| (x, z)));
    let seen = all.filter(|(x, z)| view.depth(*x as f32, *z as f32) > NEAR + 0.4);
    seen.map(|(x, z)| ((x, z), view.project(x as f32, z as f32))).filter(|(_, (x, y))| open(*x, *y)).collect()
}

/// Puts each task on the free crossing nearest to where its lane and depth point. A task with
/// no free crossing left is not shown.
fn place(shown: &[(Group, &Task)], crossings: &[Spot]) -> Vec<Node> {
    let mut projects: Vec<&str> = shown.iter().filter_map(|(_, t)| t.project.as_deref()).collect();
    projects.sort_unstable();
    projects.dedup();
    let lane = |task: &Task| {
        let named = task.project.as_deref().and_then(|p| projects.iter().position(|q| *q == p));
        named.unwrap_or(projects.len())
    };
    let lanes = shown.iter().map(|(_, t)| lane(t) + 1).max().unwrap_or(1);
    // Ids rise with age: the highest id is the newest task.
    let mut by_age: Vec<&(Group, &Task)> = shown.iter().collect();
    by_age.sort_by_key(|(_, t)| std::cmp::Reverse(t.id));
    let mut in_lane = vec![0usize; lanes];
    let mut used: Vec<(i32, i32)> = Vec::new();
    let mut nodes = Vec::new();
    for (depth, (group, task)) in by_age.into_iter().enumerate() {
        let lane = lane(task);
        // Tasks of one lane spread over its width, so they do not queue up in one column.
        let across = (in_lane[lane] as f32 * 0.618 + 0.5).fract();
        in_lane[lane] += 1;
        let x = 0.06 + 0.88 * (lane as f32 + 0.1 + 0.8 * across) / lanes as f32;
        // Under the box the floor is short: the farthest place there is nearer.
        let far = if (0.26..0.74).contains(&x) { 0.66 } else { 0.48 };
        let y = 0.90 - (0.90 - far) * depth as f32 / (shown.len().max(2) - 1) as f32;
        // The drawn area is wider than tall (about 5:3).
        let apart = |(cx, cy): (f32, f32)| ((cx - x) * 1.66).powi(2) + (cy - y).powi(2);
        let free = crossings.iter().filter(|(at, _)| !used.contains(at));
        let Some((crossing, _)) = free.min_by(|a, b| apart(a.1).total_cmp(&apart(b.1))) else { continue };
        used.push(*crossing);
        nodes.push(Node { task: task.id, group: *group, lane, depth, crossing: *crossing });
    }
    nodes
}

/// What the home screen shows for these tasks.
pub fn home(tasks: &Tasks) -> Home<'_> {
    let inbox = tasks.inbox();
    let mut counts = Counts::default();
    for (group, _) in &inbox {
        *match group {
            Group::NeedsYou => &mut counts.needs_you,
            Group::Running => &mut counts.running,
            Group::Captured => &mut counts.captured,
            Group::OnHold => &mut counts.on_hold,
            Group::Done => &mut counts.done,
        } += 1;
    }
    let needs_you = inbox.iter().filter(|(g, _)| *g == Group::NeedsYou).map(|(_, t)| *t).take(MAX_NEEDS_YOU).collect();
    let shown = &inbox[..inbox.len().min(MAX_NODES)];
    Home { nodes: place(shown, &crossings(&HOME)), needs_you, counts }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::State;

    fn filed(list: &[(&str, State, Option<&str>)]) -> Tasks {
        let mut tasks = Tasks::default();
        for (title, state, project) in list {
            let id = tasks.add((*title).into(), 3, *state);
            tasks.tag(id, project.map(str::to_string), None);
        }
        tasks
    }

    fn node(home: &Home, task: TaskId) -> Node {
        home.nodes.iter().find(|n| n.task == task).cloned().expect("the task has a node")
    }

    #[test]
    fn no_tasks_no_nodes() {
        let tasks = Tasks::default();
        let home = home(&tasks);
        assert!(home.nodes.is_empty() && home.needs_you.is_empty());
        assert_eq!(home.counts, Counts::default());
    }

    #[test]
    fn lanes_are_projects_by_name_and_depth_is_age() {
        let tasks = filed(&[
            ("old", State::Captured, Some("minter")),
            ("ask", State::NeedsYou, Some("canvas")),
            ("loose", State::Running, None),
            ("new", State::Done, Some("minter")),
        ]);
        let home = home(&tasks);
        let facts = |id| {
            let n = node(&home, id);
            (n.lane, n.depth, n.group)
        };
        // canvas, minter, then the tasks of no project. The newest task (4) is the nearest.
        assert_eq!(facts(1), (1, 3, Group::Captured));
        assert_eq!(facts(2), (0, 2, Group::NeedsYou));
        assert_eq!(facts(3), (2, 1, Group::Running));
        assert_eq!(facts(4), (1, 0, Group::Done));
        // On screen: the first lane is left of the last, and the older task of a lane is farther.
        let at = |id| {
            let (x, z) = node(&home, id).crossing;
            (HOME.project(x as f32, z as f32).0, HOME.depth(x as f32, z as f32))
        };
        assert!(at(2).0 < at(3).0, "canvas is left of the tasks of no project");
        assert!(at(1).1 > at(4).1, "the old minter task is farther than the new one");
    }

    #[test]
    fn the_same_tasks_get_the_same_places_and_no_two_share_a_crossing() {
        let list: Vec<(String, State, Option<&str>)> = (0..MAX_NODES)
            .map(|i| (format!("task {i}"), State::Captured, [Some("a"), Some("b"), None][i % 3]))
            .collect();
        let list: Vec<(&str, State, Option<&str>)> = list.iter().map(|(t, s, p)| (t.as_str(), *s, *p)).collect();
        let tasks = filed(&list);
        let (first, again) = (home(&tasks).nodes, home(&tasks).nodes);
        assert_eq!(first, again);
        assert_eq!(first.len(), MAX_NODES);
        let mut places: Vec<(i32, i32)> = first.iter().map(|n| n.crossing).collect();
        places.sort_unstable();
        places.dedup();
        assert_eq!(places.len(), MAX_NODES, "every node has its own crossing");
        // Every node is on screen, clear of the box and the legend.
        for (x, z) in places {
            let (sx, sy) = HOME.project(x as f32, z as f32);
            assert!(open(sx, sy), "crossing ({x}, {z}) is drawn at ({sx}, {sy})");
        }
        // One project only: its tasks still spread out, and none is lost.
        let one: Vec<(&str, State, Option<&str>)> = list.iter().map(|(t, s, _)| (*t, *s, None)).collect();
        assert_eq!(home(&filed(&one)).nodes.len(), MAX_NODES);
    }

    #[test]
    fn more_tasks_than_places_shows_the_first_of_the_inbox() {
        let mut list = vec![("idea", State::Captured, None); MAX_NODES + 3];
        list.push(("question", State::NeedsYou, Some("support")));
        let tasks = filed(&list);
        let home = home(&tasks);
        assert_eq!(home.nodes.len(), MAX_NODES);
        // The task that needs the user is first in the inbox, so it is shown, filed last or not.
        assert_eq!(node(&home, MAX_NODES as TaskId + 4).group, Group::NeedsYou);
        assert_eq!((home.counts.captured, home.counts.needs_you), (MAX_NODES + 3, 1));
        // With fewer crossings than tasks, the tasks left over have no node.
        let inbox = tasks.inbox();
        let few = place(&inbox[..5], &crossings(&HOME)[..2]);
        assert_eq!(few.len(), 2);
        assert_ne!(few[0].crossing, few[1].crossing);
    }

    #[test]
    fn hidden_tasks_have_no_node_and_needs_you_lists_three() {
        let mut tasks = filed(&[
            ("snoozed", State::NeedsYou, None),
            ("dismissed", State::Running, None),
            ("cleaned", State::Cleaned, None),
            ("a", State::NeedsYou, None),
            ("b", State::NeedsYou, None),
            ("c", State::Stale, None),
            ("d", State::NeedsYou, None),
        ]);
        tasks.snooze(1, 1_000);
        tasks.dismiss(2);
        let home = home(&tasks);
        let mut shown: Vec<TaskId> = home.nodes.iter().map(|n| n.task).collect();
        shown.sort_unstable();
        assert_eq!(shown, [4, 5, 6, 7]);
        assert_eq!(home.needs_you.iter().map(|t| t.id).collect::<Vec<_>>(), [4, 5, 6]);
        assert_eq!(home.counts.needs_you, 4);
    }
}
