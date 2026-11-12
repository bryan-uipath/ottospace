//! The home screen: what the empty workspace shows. Listed tasks sit as nodes on a lattice, a
//! grid on a floor seen in perspective. Left to right is the project; within a project the older task is farther.

use serde::Serialize;

use crate::layout::Rect;
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

impl Counts {
    pub fn total(&self) -> usize {
        self.needs_you + self.running + self.captured + self.on_hold + self.done
    }
}

/// True where a node may sit: on screen, and outside every rectangle of `keep_out` (the text
/// drawn over the lattice). All in fractions of the drawn area.
fn open(x: f32, y: f32, keep_out: &[Rect]) -> bool {
    let on_screen = (0.04..0.96).contains(&x) && (0.47..0.94).contains(&y);
    on_screen && !keep_out.iter().any(|r| (r.x..r.x + r.w).contains(&x) && (r.y..r.y + r.h).contains(&y))
}

/// A crossing (x, z) a node may sit on, with how far across it is drawn and its depth.
type Spot = ((i32, i32), f32, f32);

fn spots(view: &View, keep_out: &[Rect]) -> Vec<Spot> {
    let range = || view.lo..=view.hi;
    let all = range().flat_map(|x| range().map(move |z| (x, z)));
    let seen = all.filter(|(x, z)| view.depth(*x as f32, *z as f32) > NEAR + 0.4);
    let drawn = seen.map(|(x, z)| ((x, z), view.project(x as f32, z as f32), view.depth(x as f32, z as f32)));
    drawn.filter(|(_, (x, y), _)| open(*x, *y, keep_out)).map(|(at, (x, _), depth)| (at, x, depth)).collect()
}

/// Gives each lane the free crossings in its strip of the width (and the nearest beside it, if
/// it has more tasks than those), then hands them out by age: within a lane an older task is
/// always farther. A lane with too few crossings left shows its newest tasks only.
fn place(shown: &[(Group, &Task)], spots: &[Spot]) -> Vec<Node> {
    let mut projects: Vec<&str> = shown.iter().filter_map(|(_, t)| t.project.as_deref()).collect();
    projects.sort_unstable();
    projects.dedup();
    let lane = |task: &Task| {
        let named = task.project.as_deref().and_then(|p| projects.iter().position(|q| *q == p));
        named.unwrap_or(projects.len())
    };
    let lanes = shown.iter().map(|(_, t)| lane(t) + 1).max().unwrap_or(1);
    // Ids rise with age: the highest id is the newest task, at depth 0.
    let mut by_age: Vec<&(Group, &Task)> = shown.iter().collect();
    by_age.sort_by_key(|(_, t)| std::cmp::Reverse(t.id));
    let oldest = (shown.len().max(2) - 1) as f32;
    let mut free = spots.to_vec();
    let mut nodes = Vec::new();
    for at in 0..lanes {
        let mine: Vec<(usize, &(Group, &Task))> =
            by_age.iter().copied().enumerate().filter(|(_, (_, t))| lane(t) == at).collect();
        let (left, right) = (at as f32 / lanes as f32, (at + 1) as f32 / lanes as f32);
        let outside = |x: f32| (left - x).max(x - right).max(0.);
        free.sort_by(|a, b| outside(a.1).total_cmp(&outside(b.1)));
        let inside = free.iter().take_while(|spot| outside(spot.1) == 0.).count();
        let mut pool = free[..inside.max(mine.len()).min(free.len())].to_vec();
        pool.sort_by(|a, b| a.2.total_cmp(&b.2));
        let count = mine.len().min(pool.len());
        // Each task aims at the depth its age gives it; then no two share one, in age order.
        let aim = |rank: usize| (rank as f32 / oldest * (pool.len() - 1) as f32).round() as usize;
        let mut picks: Vec<usize> = mine[..count].iter().map(|(rank, _)| aim(*rank)).collect();
        for i in 1..count {
            picks[i] = picks[i].max(picks[i - 1] + 1);
        }
        for i in (0..count).rev() {
            picks[i] = picks[i].min(pool.len() - (count - i));
        }
        for ((depth, (group, task)), pick) in mine.into_iter().zip(picks) {
            let crossing = pool[pick].0;
            free.retain(|spot| spot.0 != crossing);
            nodes.push(Node { task: task.id, group: *group, lane: at, depth, crossing });
        }
    }
    nodes
}

/// What the home screen shows for these tasks. `keep_out`: where its text is drawn, in fractions
/// of the drawn area; no node sits there.
pub fn home<'a>(tasks: &'a Tasks, keep_out: &[Rect]) -> Home<'a> {
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
    Home { nodes: place(shown, &spots(&HOME, keep_out)), needs_you, counts }
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

    /// The text's rectangles as the shell passes them, for a drawn area of `w` by `h` points:
    /// the new-task box with three rows under it (top middle) and the legend (bottom left).
    fn text(w: f32, h: f32) -> Vec<Rect> {
        let wide = 592f32.min(w);
        let panel = Rect { x: (w - wide) / 2. / w, y: 0.18 - 16. / h, w: wide / w, h: 290. / h };
        vec![panel, Rect { x: 0., y: 1. - 46. / h, w: 420. / w, h: 46. / h }]
    }

    /// A large window and a small one.
    const AREAS: [(f32, f32); 2] = [(1384., 832.), (620., 440.)];

    fn node(home: &Home, task: TaskId) -> Node {
        home.nodes.iter().find(|n| n.task == task).cloned().expect("the task has a node")
    }

    fn depth(node: &Node) -> f32 {
        HOME.depth(node.crossing.0 as f32, node.crossing.1 as f32)
    }

    /// No node under the text or off screen, none sharing a crossing, and in every lane each
    /// older task farther than each newer one.
    fn check(nodes: &[Node], keep_out: &[Rect]) {
        for (i, a) in nodes.iter().enumerate() {
            let (x, y) = HOME.project(a.crossing.0 as f32, a.crossing.1 as f32);
            assert!(open(x, y, keep_out), "task {} is drawn at ({x}, {y})", a.task);
            for b in &nodes[i + 1..] {
                assert_ne!(a.crossing, b.crossing, "tasks {} and {} share a crossing", a.task, b.task);
                if a.lane == b.lane {
                    let (old, new) = if a.task < b.task { (a, b) } else { (b, a) };
                    assert!(depth(old) > depth(new), "task {} is older than {} but nearer", old.task, new.task);
                }
            }
        }
    }

    #[test]
    fn no_tasks_no_nodes() {
        let tasks = Tasks::default();
        let home = home(&tasks, &text(1384., 832.));
        assert!(home.nodes.is_empty() && home.needs_you.is_empty());
        assert_eq!((home.counts, home.counts.total()), (Counts::default(), 0));
    }

    #[test]
    fn lanes_are_projects_by_name_and_depth_is_age() {
        let tasks = filed(&[
            ("old", State::Captured, Some("minter")),
            ("ask", State::NeedsYou, Some("canvas")),
            ("loose", State::Running, None),
            ("new", State::Done, Some("minter")),
        ]);
        let home = home(&tasks, &text(1384., 832.));
        let facts = |id| {
            let n = node(&home, id);
            (n.lane, n.depth, n.group)
        };
        // canvas, minter, then the tasks of no project. The newest task (4) is the nearest.
        assert_eq!(facts(1), (1, 3, Group::Captured));
        assert_eq!(facts(2), (0, 2, Group::NeedsYou));
        assert_eq!(facts(3), (2, 1, Group::Running));
        assert_eq!(facts(4), (1, 0, Group::Done));
        // On screen: the first lane is left of the last.
        let across = |id| {
            let (x, z) = node(&home, id).crossing;
            HOME.project(x as f32, z as f32).0
        };
        assert!(across(2) < across(3), "canvas is left of the tasks of no project");
        check(&home.nodes, &text(1384., 832.));
    }

    #[test]
    fn in_a_lane_every_older_task_is_farther_and_none_is_under_the_text() {
        let projects: [&[Option<&str>]; 4] =
            [&[None], &[Some("a"), Some("b"), None], &[Some("a"), None, None, None], &[Some("a"), Some("b")]];
        for (w, h) in AREAS {
            let keep_out = text(w, h);
            for projects in projects {
                for count in [1, 2, 4, 7, MAX_NODES] {
                    let list: Vec<_> =
                        (0..count).map(|i| ("task", State::Captured, projects[i % projects.len()])).collect();
                    let tasks = filed(&list);
                    let (first, again) = (home(&tasks, &keep_out).nodes, home(&tasks, &keep_out).nodes);
                    assert_eq!(first, again, "the same tasks get the same places");
                    check(&first, &keep_out);
                    // The large window has a crossing for every task.
                    assert!(first.len() == count || w < 1000., "{count} tasks, {} nodes at {w} by {h}", first.len());
                }
            }
        }
        // The small window still shows some of twelve tasks, clear of the text.
        let tasks = filed(&[("task", State::Captured, None); MAX_NODES]);
        assert!(!home(&tasks, &text(620., 440.)).nodes.is_empty());
    }

    #[test]
    fn more_tasks_than_places_shows_the_first_of_the_inbox() {
        let mut list = vec![("idea", State::Captured, None); MAX_NODES + 3];
        list.push(("question", State::NeedsYou, Some("support")));
        let tasks = filed(&list);
        let home = home(&tasks, &text(1384., 832.));
        assert_eq!(home.nodes.len(), MAX_NODES);
        // The task that needs the user is first in the inbox, so it is shown, filed last or not.
        assert_eq!(node(&home, MAX_NODES as TaskId + 4).group, Group::NeedsYou);
        assert_eq!((home.counts.captured, home.counts.needs_you), (MAX_NODES + 3, 1));
        // With fewer crossings than tasks, the oldest tasks have no node.
        let inbox = tasks.inbox();
        let few = place(&inbox[1..6], &spots(&HOME, &[])[..2]);
        assert_eq!(few.iter().map(|n| n.task).collect::<Vec<_>>(), [5, 4]);
        check(&few, &[]);
        assert!(place(&inbox[..5], &[]).is_empty());
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
        let home = home(&tasks, &[]);
        let mut shown: Vec<TaskId> = home.nodes.iter().map(|n| n.task).collect();
        shown.sort_unstable();
        assert_eq!(shown, [4, 5, 6, 7]);
        assert_eq!(home.needs_you.iter().map(|t| t.id).collect::<Vec<_>>(), [4, 5, 6]);
        assert_eq!(home.counts.needs_you, 4);
    }
}
