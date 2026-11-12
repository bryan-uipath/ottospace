//! Draws the lattice: the home screen (the empty workspace) and the patch in an empty pane.
//! Where tasks sit is the core's choice (`ottospace_core::home`); this file only draws.

use std::collections::BTreeMap;

use gpui::{
    AnyElement, Bounds, BoxShadow, Context, Div, FontWeight, Hsla, PathBuilder, Pixels, Point, SharedString, Window,
    black, canvas, div, point, prelude::*, px, relative, rgb,
};
use ottospace_core::home::{HOME, NEAR, View};
use ottospace_core::tasks::Group;
use ottospace_core::{Action, theme};

use crate::Workspace;

/// The patch of lattice in an empty pane: a nearer, smaller view of the same floor.
const PATCH: View = View { cx: 0.5, horizon: 0.05, fx: 0.58, fy: 0.91, sin: 0.5, cos: 0.866, dist: 3.9, lo: -3, hi: 4 };
/// Where the new-task box starts, as a part of the height.
const BOX_TOP: f32 = 0.18;
/// Lines are cut into pieces about this long, in points, so each can fade by itself.
const PIECE: f32 = 24.;

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

/// Paints the grid lines of `view` over `bounds` as vector paths, so they are sharp at any size.
/// A line fades with distance (gone at `far`) and by `keep`, a factor for each screen point.
fn paint_lattice(
    view: &View,
    bounds: Bounds<Pixels>,
    (base, far): (f32, f32),
    keep: impl Fn(Point<Pixels>) -> f32,
    window: &mut Window,
) {
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
    for ((opacity, _), path) in paths {
        if let Ok(path) = path.build() {
            window.paint_path(path, Hsla::from(rgb(theme::PRIMARY)).opacity(f32::from(opacity) / 40.));
        }
    }
}

/// A disc centred on its parent's top-left corner.
fn disc(radius: f32, color: Hsla) -> Div {
    div().absolute().left(px(-radius)).top(px(-radius)).size(px(radius * 2.)).rounded_full().bg(color)
}

/// A node at (x, y), as parts of the parent's size: a soft glow, the body, a bright core.
/// Nearer nodes (smaller `depth`) are larger.
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
fn dot(color: u32) -> Div {
    div().size(px(8.)).flex_none().rounded_full().bg(rgb(color))
}

/// An empty pane: a patch of the lattice with one place waiting, and a hint under it.
pub fn empty_pane(hint: &'static str) -> impl IntoElement + use<> {
    // The patch fades out all round, so no edge of it looks cut off.
    let fade = |bounds: Bounds<Pixels>| Quiet {
        centre: bounds.center(),
        radius: (f32::from(bounds.size.width) / 2., f32::from(bounds.size.height) / 2.),
        left: 0.,
    };
    let lines = canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let fade = fade(bounds);
            let keep = |at| (1. - ((fade.reach(at) - 0.25) / 0.7).clamp(0., 1.)).powi(2);
            paint_lattice(&PATCH, bounds, (0.75, 8.2), keep, window);
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
    pub(crate) fn render_home(&self, cx: &mut Context<Self>) -> AnyElement {
        let home = ottospace_core::home::home(&self.model.tasks);
        let counts = home.counts;
        let any = !self.model.tasks.inbox().is_empty();
        let muted = || div().text_xs().text_color(rgb(theme::MUTED));

        // Far nodes first, so a near node's glow lies over them.
        let mut nodes = home.nodes.clone();
        nodes.sort_by(|a, b| {
            let depth = |n: &ottospace_core::home::Node| HOME.depth(n.crossing.0 as f32, n.crossing.1 as f32);
            depth(b).total_cmp(&depth(a))
        });
        let nodes = nodes.into_iter().filter_map(|n| {
            let (id, title) = (n.task, self.model.tasks.get(n.task)?.title.clone());
            let group: SharedString = format!("home-node-{id}").into();
            // The label opens toward the middle, so it stays inside the window.
            let on_right = HOME.project(n.crossing.0 as f32, n.crossing.1 as f32).0 > 0.6;
            let label = div()
                .absolute()
                .top(px(-30.))
                .when(on_right, |d| d.right(px(14.)))
                .when(!on_right, |d| d.left(px(14.)))
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
                .left(px(-14.))
                .top(px(-14.))
                .size(px(28.))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::TaskOpen { id: Some(id) }, window, cx);
                    cx.notify();
                }))
                .child(div().absolute().left(px(14.)).top(px(14.)).size_0().child(label));
            Some(node(&HOME, n.crossing, theme::task(n.group)).child(target))
        });

        let line = div()
            .id("home-new-task")
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .rounded_lg()
            .bg(rgb(theme::RAISED))
            .border_1()
            .border_color(rgb(theme::BORDER_STRONG))
            .shadow(vec![BoxShadow {
                color: black().opacity(0.7),
                offset: point(px(0.), px(12.)),
                blur_radius: px(32.),
                spread_radius: px(0.),
            }])
            .cursor_pointer()
            .hover(|s| s.border_color(rgb(theme::COMPOSE)))
            .on_click(cx.listener(|this, _, window, cx| this.new_task(window, cx)))
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
                muted().flex().justify_between().child("#project  @repo  p1–p9").child("click, or ⌘N from anywhere"),
            );

        let rows = home.needs_you.iter().enumerate().map(|(i, task)| {
            let id = task.id;
            let urgent = if task.priority == 1 { theme::ERROR } else { theme::MUTED };
            div()
                .id(("home-needs-you", id as usize))
                .min_h(px(36.))
                .px_3()
                .flex()
                .items_center()
                .gap_3()
                .when(i > 0, |d| d.border_t_1().border_color(rgb(theme::BORDER)))
                .text_sm()
                .cursor_pointer()
                .hover(|s| s.bg(rgb(theme::SELECTED)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dispatch(Action::TaskOpen { id: Some(id) }, window, cx);
                    cx.notify();
                }))
                .child(dot(theme::task(Group::NeedsYou)))
                .child(div().flex_none().text_xs().text_color(rgb(urgent)).child(format!("P{}", task.priority)))
                .child(div().flex_1().min_w_0().truncate().child(task.title.clone()))
                .children(
                    task.project
                        .as_ref()
                        .map(|p| muted().flex_none().max_w(px(140.)).truncate().child(format!("#{p}"))),
                )
        });
        let listed = home.needs_you.len();
        let needs_you = (listed > 0).then(|| {
            let heading = div()
                .px_1()
                .flex()
                .items_baseline()
                .gap_2()
                .text_xs()
                .child(div().font_weight(FontWeight::SEMIBOLD).text_color(rgb(theme::SNEAK)).child("Needs you"))
                .child(muted().child(counts.needs_you.to_string()))
                .child(div().flex_1())
                .child(muted().child("⌘⇧N next"));
            let list =
                div().rounded_lg().bg(rgb(theme::RAISED)).border_1().border_color(rgb(theme::BORDER)).overflow_hidden();
            div().flex().flex_col().gap_2().child(heading).child(list.children(rows))
        });
        let under = if any {
            muted()
                .px_1()
                .flex()
                .items_center()
                .gap_2()
                .child(dot(theme::task(Group::Running)))
                .child(format!("Running · {}", counts.running))
        } else {
            muted()
                .flex()
                .justify_center()
                .child("⌘N files a task · ⌘D opens a terminal · the Repos list opens one in a repository")
        };
        let panel = div().absolute().top(relative(BOX_TOP)).left_0().w_full().px_4().flex().justify_center().child(
            div().w(px(560.)).max_w_full().flex().flex_col().gap_3().child(line).children(needs_you).child(under),
        );

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
                .child(item(Group::NeedsYou, counts.needs_you))
                .child(item(Group::Running, counts.running))
                .child(item(Group::Captured, counts.captured))
                .children((counts.on_hold > 0).then(|| item(Group::OnHold, counts.on_hold)))
                .child(item(Group::Done, counts.done))
        });

        // The lines go quiet behind the text: the box and its list, and the legend.
        let tall = 110. + if listed > 0 { 40. + 37. * listed as f32 } else { 0. };
        let lines = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                let top = bounds.origin.y + px(h * BOX_TOP);
                let text = Quiet {
                    centre: point(bounds.origin.x + px(w / 2.), top + px(tall / 2.)),
                    radius: (430., tall / 2. + 120.),
                    left: 0.05,
                };
                let legend =
                    Quiet { centre: bounds.bottom_left() + point(px(190.), px(-20.)), radius: (300., 50.), left: 0. };
                // A haze just under the horizon, where the lines would run together.
                let horizon = bounds.origin.y + px(h * HOME.horizon);
                let haze = |at: Point<Pixels>| (f32::from(at.y - horizon) / 120.).clamp(0., 1.);
                let keep = |at| text.keep(at) * if any { legend.keep(at) } else { 1. } * haze(at);
                paint_lattice(&HOME, bounds, (0.6, 19.), keep, window);
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
