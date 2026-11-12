//! Draws the lattice: the home screen (the empty workspace) and the patch in an empty pane.
//! Where tasks sit is the core's choice (`ottospace_core::home`); this file only draws.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, Context, Div, EntityId, FontWeight, Hsla, Path, PathBuilder, Pixels, Point, SharedString,
    Window, canvas, div, point, prelude::*, px, relative, rgb,
};
use ottospace_core::home::{HOME, Home, NEAR, View};
use ottospace_core::tasks::{Group, TaskId};
use ottospace_core::{PaneId, Rect, theme};

use crate::{Workspace, open_task, rect, task_tags_cell, task_title_cells};

/// The patch of lattice in an empty pane: a nearer, smaller view of the same floor.
const PATCH: View = View { cx: 0.5, horizon: 0.05, fx: 0.58, fy: 0.91, sin: 0.5, cos: 0.866, dist: 3.9, lo: -3, hi: 4 };
/// Where the new-task box starts, as a part of the height.
const BOX_TOP: f32 = 0.18;
/// Lines are cut into pieces about this long, in points, so each can fade by itself.
const PIECE: f32 = 24.;
/// Half the side of a node's click target, and the room kept between a node and the text.
const TARGET: f32 = 14.;
const ROOM: f32 = TARGET + 4.;
/// Where the text is until a first frame has measured it: the box, then the legend.
const FIRST_GUESS: [Rect; 2] = [Rect { x: 0.26, y: 0., w: 0.48, h: 0.63 }, Rect { x: 0., y: 0.9, w: 0.36, h: 0.1 }];

/// What a lattice's lines were built for: its bounds, and the text the lines fade behind.
type Key = (Bounds<Pixels>, [Bounds<Pixels>; 2]);
/// A lattice's lines, built: each path with its colour.
type Lines = Rc<Vec<(Path<Pixels>, Hsla)>>;

/// What the last frame drew of the home screen, and the lattices kept between frames.
#[derive(Default)]
pub struct Drawn {
    /// The home screen's area, the new-task box with its list, and the legend (empty: none).
    area: Bounds<Pixels>,
    panel: Bounds<Pixels>,
    legend: Bounds<Pixels>,
    /// Each node's click target.
    nodes: Vec<(TaskId, Bounds<Pixels>)>,
    /// Built lines by where they are drawn (`None`: the home screen), with the bounds and
    /// text they were built for. Building is the costly part; a frame that changes neither
    /// paints the kept paths.
    lattices: HashMap<Option<PaneId>, (Key, Lines)>,
    builds: usize,
    paints: usize,
}

pub type Shared = Rc<RefCell<Drawn>>;

impl Drawn {
    /// Where the text is, with room for a node beside it, in fractions of the area: the core
    /// puts no node there.
    pub fn keep_out(&self) -> Vec<Rect> {
        let (w, h) = (f32::from(self.area.size.width), f32::from(self.area.size.height));
        if w < 1. || h < 1. || self.panel.size.width < px(1.) {
            return FIRST_GUESS.to_vec();
        }
        let part = |b: Bounds<Pixels>| {
            let [x, y, bw, bh] = rect(b);
            let (x, y) = (x - f32::from(self.area.origin.x), y - f32::from(self.area.origin.y));
            Rect { x: (x - ROOM) / w, y: (y - ROOM) / h, w: (bw + 2. * ROOM) / w, h: (bh + 2. * ROOM) / h }
        };
        [self.panel, self.legend].into_iter().filter(|b| b.size.width >= px(1.)).map(part).collect()
    }

    /// For the state snapshot: what was drawn, in window points, and how often lines were
    /// built and painted.
    pub fn snapshot(&self) -> serde_json::Value {
        let nodes: BTreeMap<TaskId, [f32; 4]> = self.nodes.iter().map(|(id, b)| (*id, rect(*b))).collect();
        serde_json::json!({
            "area": rect(self.area), "panel": rect(self.panel), "nodes": nodes,
            "lattice_builds": self.builds, "lattice_paints": self.paints,
        })
    }
}

/// An invisible layer over its parent that notes the parent's bounds. If `note` returns true,
/// the layout changed something the next frame needs: the view is drawn again.
fn measure(
    drawn: &Shared,
    view: EntityId,
    note: impl FnOnce(&mut Drawn, Bounds<Pixels>) -> bool + 'static,
) -> AnyElement {
    let drawn = drawn.clone();
    let prepaint = move |bounds, _: &mut Window, cx: &mut App| {
        if note(&mut drawn.borrow_mut(), bounds) {
            // A notify during a draw is dropped, so send it after.
            cx.defer(move |cx| cx.notify(view));
        }
    };
    canvas(prepaint, |_, _, _, _| {}).absolute().size_full().into_any_element()
}

/// A place where the lines fade: an ellipse, and how much of a line is left at its centre.
struct Quiet {
    centre: Point<Pixels>,
    radius: (f32, f32),
    left: f32,
}

impl Quiet {
    /// 0 at the centre to 1 at the edge and beyond.
    fn reach(&self, at: Point<Pixels>) -> f32 {
        let (dx, dy) =
            (f32::from(at.x - self.centre.x) / self.radius.0, f32::from(at.y - self.centre.y) / self.radius.1);
        (dx * dx + dy * dy).sqrt().min(1.)
    }

    /// How much of a line shows at `at`: `left` in the middle, all of it from the edge on.
    fn keep(&self, at: Point<Pixels>) -> f32 {
        let reach = self.reach(at);
        self.left + (1. - self.left) * ((reach - 0.55) / 0.45).max(0.)
    }
}

/// Builds the grid lines of `view` over `bounds` as vector paths, so they are sharp at any size.
/// A line fades with distance (gone at `far`) and by `keep`, a factor for each screen point.
fn build_lattice(
    view: &View,
    bounds: Bounds<Pixels>,
    (base, far): (f32, f32),
    keep: impl Fn(Point<Pixels>) -> f32,
) -> Vec<(Path<Pixels>, Hsla)> {
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let screen = |(x, y): (f32, f32)| (x * w, y * h);
    // Pieces of one opacity and width go into one path: about thirty paths, not a thousand.
    let mut paths: BTreeMap<(u8, u8), PathBuilder> = BTreeMap::new();
    let mut line = |a: (f32, f32), b: (f32, f32)| {
        // Keep the part in front of the camera. Depth is linear along the line.
        let (da, db) = (view.depth(a.0, a.1), view.depth(b.0, b.1));
        let cut = |t: f32| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        let (a, b, da, db) = match (da > NEAR, db > NEAR) {
            (true, true) => (a, b, da, db),
            (true, false) => (a, cut((da - NEAR) / (da - db)), da, NEAR),
            (false, true) => (cut((da - NEAR) / (da - db)), b, NEAR, db),
            (false, false) => return,
        };
        let (p, q) = (screen(view.project(a.0, a.1)), screen(view.project(b.0, b.1)));
        // Keep the part inside the bounds (Liang-Barsky), with room for the line's width.
        let (mut t0, mut t1) = (0f32, 1f32);
        for (start, delta, max) in [(p.0, q.0 - p.0, w), (p.1, q.1 - p.1, h)] {
            for (dir, room) in [(-delta, start + 4.), (delta, max + 4. - start)] {
                if dir == 0. && room < 0. {
                    return;
                } else if dir < 0. {
                    t0 = t0.max(room / dir);
                } else if dir > 0. {
                    t1 = t1.min(room / dir);
                }
            }
        }
        if t0 >= t1 {
            return;
        }
        let length = ((q.0 - p.0).powi(2) + (q.1 - p.1).powi(2)).sqrt() * (t1 - t0);
        let pieces = (length / PIECE).ceil().clamp(1., 400.) as usize;
        let at = |t: f32| bounds.origin + point(px(p.0 + (q.0 - p.0) * t), px(p.1 + (q.1 - p.1) * t));
        let mut last = None;
        for i in 0..pieces {
            let (s0, s1) = (t0 + (t1 - t0) * i as f32 / pieces as f32, t0 + (t1 - t0) * (i + 1) as f32 / pieces as f32);
            let middle = (s0 + s1) / 2.;
            // 1/depth is linear on screen.
            let depth = 1. / (1. / da + (1. / db - 1. / da) * middle);
            let opacity = base * (1. - depth / far).max(0.).powf(1.6) * keep(at(middle));
            let key = ((opacity * 40.).round() as u8, (3.2 / depth).clamp(0.5, 2.2).mul_add(4., 0.5) as u8);
            if key.0 == 0 {
                last = None;
                continue;
            }
            let path = paths.entry(key).or_insert_with(|| PathBuilder::stroke(px(f32::from(key.1) / 4.)));
            if last != Some(key) {
                path.move_to(at(s0));
            }
            path.line_to(at(s1));
            last = Some(key);
        }
    };
    let (lo, hi) = (view.lo as f32, view.hi as f32);
    for k in view.lo..=view.hi {
        line((k as f32, lo), (k as f32, hi));
        line((lo, k as f32), (hi, k as f32));
    }
    let color = |opacity: u8| Hsla::from(rgb(theme::PRIMARY)).opacity(f32::from(opacity) / 40.);
    paths.into_iter().filter_map(|((opacity, _), path)| Some((path.build().ok()?, color(opacity)))).collect()
}

/// Paints the lines kept for `slot`, building them first if the bounds or the text moved.
fn paint_lattice(
    drawn: &Shared,
    slot: Option<PaneId>,
    key: Key,
    build: impl FnOnce() -> Vec<(Path<Pixels>, Hsla)>,
    window: &mut Window,
) {
    let lines = {
        let mut drawn = drawn.borrow_mut();
        drawn.paints += 1;
        match drawn.lattices.get(&slot) {
            Some((kept, lines)) if *kept == key => lines.clone(),
            _ => {
                // Closed panes leave their lines behind: start over before they pile up.
                if drawn.lattices.len() > 32 {
                    drawn.lattices.clear();
                }
                let lines: Lines = Rc::new(build());
                drawn.builds += 1;
                drawn.lattices.insert(slot, (key, lines.clone()));
                lines
            }
        }
    };
    for (path, color) in lines.iter() {
        window.paint_path(path.clone(), *color);
    }
}

/// A disc centred on its parent's top-left corner.
fn disc(radius: f32, color: Hsla) -> Div {
    div().absolute().left(px(-radius)).top(px(-radius)).size(px(radius * 2.)).rounded_full().bg(color)
}

/// A node on a crossing of `view`: a soft glow, the body, a bright core. Nearer nodes are larger.
fn node(view: &View, crossing: (i32, i32), color: u32) -> Div {
    let (x, z) = (crossing.0 as f32, crossing.1 as f32);
    let (left, top) = view.project(x, z);
    // A far task is still a dot you can see.
    let radius = (5. * (3.4 / view.depth(x, z)).min(2.4)).max(2.3);
    let color = Hsla::from(rgb(color));
    div()
        .absolute()
        .left(relative(left))
        .top(relative(top))
        .size_0()
        .child(disc(radius * 4.2, color.opacity(0.10)))
        .child(disc(radius * 2.2, color.opacity(0.22)))
        .child(disc(radius, color))
        .child(disc(radius * 0.38, gpui::white().opacity(0.75)).left(px(-radius * 0.63)).top(px(-radius * 0.63)))
}

/// A coloured dot before a label.
pub fn dot(color: u32) -> Div {
    div().size(px(8.)).flex_none().rounded_full().bg(rgb(color))
}

/// An empty pane: a patch of the lattice with one place waiting, and a hint under it.
pub fn empty_pane(hint: SharedString, pane: PaneId, drawn: &Shared) -> impl IntoElement + use<> {
    let drawn = drawn.clone();
    let lines = canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            // The patch fades out all round, so no edge of it looks cut off.
            let build = || {
                let (centre, half) = (bounds.center(), bounds.size.map(|side| f32::from(side) / 2.));
                let fade = Quiet { centre, radius: (half.width, half.height), left: 0. };
                let keep = |at| (1. - ((fade.reach(at) - 0.25) / 0.7).clamp(0., 1.)).powi(2);
                build_lattice(&PATCH, bounds, (0.75, 8.2), keep)
            };
            paint_lattice(&drawn, Some(pane), (bounds, Default::default()), build, window);
        },
    );
    let patch = div().relative().w(px(480.)).max_w_full().h(px(280.)).child(lines.absolute().size_full());
    div()
        .flex()
        .flex_col()
        .items_center()
        .max_w_full()
        .child(patch.child(node(&PATCH, (0, 1), theme::task(Group::Captured))))
        .child(div().text_sm().text_color(rgb(theme::MUTED)).child(hint))
}

impl Workspace {
    /// The home screen: the lattice with a node for each listed task, the new-task line with
    /// the tasks that need the user under it, and a legend.
    pub(crate) fn render_home(&self, home: Home<'_>, cx: &mut Context<Self>) -> AnyElement {
        let (counts, view, drawn) = (home.counts, cx.entity_id(), &self.home);
        let any = counts.total() > 0;
        let muted = || div().text_xs().text_color(rgb(theme::MUTED));
        drawn.borrow_mut().nodes.clear();

        // Far nodes first, so a near node's glow lies over them.
        let mut nodes = home.nodes;
        nodes.sort_by(|a, b| {
            let depth = |n: &ottospace_core::home::Node| HOME.depth(n.crossing.0 as f32, n.crossing.1 as f32);
            depth(b).total_cmp(&depth(a))
        });
        let nodes: Vec<Div> = nodes
            .into_iter()
            .filter_map(|n| {
                let (id, title) = (n.task, self.model.tasks.get(n.task)?.title.clone());
                let group: SharedString = format!("home-node-{id}").into();
                // The label opens toward the middle, so it stays inside the window.
                let on_right = HOME.project(n.crossing.0 as f32, n.crossing.1 as f32).0 > 0.6;
                let label = div()
                    .absolute()
                    .top(px(-30.))
                    .when(on_right, |d| d.right(px(TARGET)))
                    .when(!on_right, |d| d.left(px(TARGET)))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(theme::SELECTED))
                    .border_1()
                    .border_color(rgb(theme::BORDER_STRONG))
                    .text_xs()
                    .whitespace_nowrap()
                    .child(title)
                    .invisible()
                    .group_hover(group.clone(), |s| s.visible());
                // Larger than the dot, so a far node is not hard to hit.
                let target = div()
                    .id(("home-node", id as usize))
                    .group(group)
                    .absolute()
                    .left(px(-TARGET))
                    .top(px(-TARGET))
                    .size(px(TARGET * 2.))
                    .cursor_pointer()
                    .on_click(open_task(id, cx))
                    .child(self.controls.mark(format!("home-node:{id}")))
                    .child(measure(drawn, view, move |drawn, bounds| {
                        drawn.nodes.push((id, bounds));
                        false
                    }))
                    .child(div().absolute().left(px(TARGET)).top(px(TARGET)).size_0().child(label));
                Some(node(&HOME, n.crossing, theme::task(n.group)).child(target))
            })
            .collect();

        // `occlude`: a click on the text must not reach anything under it.
        let line = crate::dialog::panel(theme::BORDER_STRONG)
            .id("home-new-task")
            .occlude()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .cursor_pointer()
            .hover(|s| s.border_color(rgb(theme::COMPOSE)))
            .on_click(cx.listener(|this, _, window, cx| this.new_task(window, cx)))
            .child(self.controls.mark("home:new_task"))
            .child(
                div()
                    .min_h(px(28.))
                    .flex()
                    .items_center()
                    .text_base()
                    .text_color(rgb(theme::MUTED))
                    .child("What needs doing?"),
            )
            .child(
                muted()
                    .flex()
                    .justify_between()
                    .child("#project  >repo  @agent  /path  $skill  p1–p9")
                    .child("click, or ⌘N from anywhere"),
            );

        let rows: Vec<_> = (home.needs_you.iter().enumerate())
            .map(|(i, task)| {
                div()
                    .id(("home-needs-you", task.id as usize))
                    .min_h(px(36.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .when(i > 0, |d| d.border_t_1().border_color(rgb(theme::BORDER)))
                    .text_sm()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(theme::SELECTED)))
                    .on_click(open_task(task.id, cx))
                    .child(self.controls.mark(format!("home-row:{}", task.id)))
                    .child(dot(theme::task(Group::NeedsYou)))
                    .children(task_title_cells(task))
                    .children(task_tags_cell(self.model.task_tags(task), 140.))
            })
            .collect();
        let needs_you = (!rows.is_empty()).then(|| {
            let title = div().font_weight(FontWeight::SEMIBOLD).text_color(rgb(theme::task(Group::NeedsYou)));
            let heading = div()
                .px_1()
                .flex()
                .items_baseline()
                .gap_2()
                .text_xs()
                .child(title.child(Group::NeedsYou.title()))
                .child(muted().child(counts.needs_you.to_string()))
                .child(div().flex_1())
                .child(muted().child("⌘⇧N next"));
            let list = div()
                .occlude()
                .rounded_lg()
                .bg(rgb(theme::RAISED))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .overflow_hidden();
            div().flex().flex_col().gap_2().child(heading).child(list.children(rows))
        });
        let running = any.then(|| {
            let label = format!("{} · {}", Group::Running.title(), counts.running);
            div().flex().items_center().gap_2().child(dot(theme::task(Group::Running))).child(label)
        });
        let under = muted()
            .px_1()
            .flex()
            .flex_wrap()
            .gap_x_4()
            .when(any, |d| d.justify_between())
            .when(!any, |d| d.justify_center())
            .children(running)
            .child("⌘E to open a pane · ⌘D for a terminal");
        let text = div()
            .relative()
            .w(px(560.))
            .max_w_full()
            .flex()
            .flex_col()
            .gap_3()
            .child(measure(drawn, view, |drawn, bounds| std::mem::replace(&mut drawn.panel, bounds) != bounds))
            .child(line)
            .children(needs_you)
            .child(under);
        let panel =
            div().absolute().top(relative(BOX_TOP)).left_0().w_full().px_4().flex().justify_center().child(text);

        if !any {
            drawn.borrow_mut().legend = Bounds::default();
        }
        let legend = any.then(|| {
            let item = |group: Group, count: usize| {
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(dot(theme::task(group)).size(px(7.)))
                    .child(format!("{} {count}", group.title()))
            };
            div()
                .absolute()
                .left(px(20.))
                .bottom(px(14.))
                .flex()
                .gap_4()
                .text_size(px(11.))
                .text_color(rgb(theme::MUTED))
                .child(measure(drawn, view, |drawn, bounds| std::mem::replace(&mut drawn.legend, bounds) != bounds))
                .child(item(Group::NeedsYou, counts.needs_you))
                .child(item(Group::Running, counts.running))
                .child(item(Group::Captured, counts.captured))
                .children((counts.on_hold > 0).then(|| item(Group::OnHold, counts.on_hold)))
                .child(item(Group::Done, counts.done))
        });

        let shared = drawn.clone();
        let lines = canvas(
            {
                let drawn = drawn.clone();
                move |bounds, _: &mut Window, cx: &mut App| {
                    if std::mem::replace(&mut drawn.borrow_mut().area, bounds) != bounds {
                        cx.defer(move |cx| cx.notify(view));
                    }
                }
            },
            move |bounds, _, window, _| {
                let (panel, legend) = (shared.borrow().panel, shared.borrow().legend);
                // The lines go quiet behind the text (the box and its list, the legend), and in
                // a haze just under the horizon, where they would run together.
                let build = || {
                    let around = |b: Bounds<Pixels>, room: (f32, f32), left: f32| Quiet {
                        centre: b.center(),
                        radius: (f32::from(b.size.width) / 2. + room.0, f32::from(b.size.height) / 2. + room.1),
                        left,
                    };
                    let (text, legend) = (around(panel, (150., 120.), 0.05), around(legend, (110., 40.), 0.));
                    let horizon = bounds.origin.y + bounds.size.height * HOME.horizon;
                    let haze = |at: Point<Pixels>| (f32::from(at.y - horizon) / 120.).clamp(0., 1.);
                    let keep = |at| text.keep(at) * if any { legend.keep(at) } else { 1. } * haze(at);
                    build_lattice(&HOME, bounds, (0.6, 19.), keep)
                };
                paint_lattice(&shared, None, (bounds, [panel, legend]), build, window);
            },
        );
        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .child(lines.absolute().size_full())
            .children(nodes)
            .child(panel)
            .children(legend)
            .into_any_element()
    }
}
