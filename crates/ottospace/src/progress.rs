//! Shell half of a task's progress: the Tasks section's State | Project switch, the Project
//! view's groups with their bar of states, the Today footer, and a task's hover card. What each
//! says is the core's (`Model::project_groups`, `Model::today`, `Model::card`); this draws it.

use gpui::{
    AnyElement, AnyView, AppContext, Context, Div, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px, relative, rgb,
};
use ottospace_core::model::{Card, ProjectGroup, TasksView};
use ottospace_core::progress::count;
use ottospace_core::sessions::Row;
use ottospace_core::tasks::{Group, Segments};
use ottospace_core::{Action, theme};

use crate::{Tooltip, Workspace, home};

/// The segments' colours, in the bar's order: needs you, running, later, done.
const SEGMENTS: [u32; 4] = [theme::SNEAK, theme::PRIMARY, 0x71717a, theme::PICK];

/// An age as a card says it: `3h ago`, but `now`.
fn ago(age: String) -> String {
    if age == "now" { age } else { format!("{age} ago") }
}

/// A hover card's frame: its title, then a dot and its state. The task's, the session's and a pull request's.
pub(crate) fn frame(title: String, color: u32, state: String) -> Div {
    div()
        .w(px(330.))
        .p_3()
        .flex()
        .flex_col()
        .gap_2()
        .rounded_md()
        .bg(rgb(theme::RAISED))
        .border_1()
        .border_color(rgb(theme::BORDER_STRONG))
        .text_color(rgb(theme::TEXT))
        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(title))
        .child(
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .child(home::dot(color))
                .child(div().text_xs().truncate().child(state)),
        )
}

/// A named fact of a hover card: `Started  3h ago`.
pub(crate) fn fact(name: &'static str, value: String) -> Div {
    let name = div().w(px(72.)).text_color(rgb(theme::MUTED)).child(name);
    div().flex().gap_2().text_xs().child(name).child(crate::cut_text(value))
}

/// A meeting's hover card, on the band and on a Calendar row: its title, a dot in its colour and
/// its time, then its place, link, status and the calls merged into it. Other people's text.
pub struct MeetingCard(pub ottospace_core::calendar::Facts, pub u32);

impl Render for MeetingCard {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (facts, color) = (&self.0, self.1);
        frame(facts.title.clone(), color, facts.when.clone())
            .children(facts.place.clone().map(|place| fact("Place", place)))
            .children(facts.link.map(|link| fact("Link", link.to_string())))
            .children(facts.status.map(|status| fact("Shows as", status.to_string())))
            .when(!facts.more.is_empty(), |d| d.child(fact("Also", facts.more.join(", "))))
    }
}

/// A tooltip that shows a meeting's card.
pub type MeetingTip = Box<dyn Fn(&mut Window, &mut gpui::App) -> AnyView + 'static>;

pub fn meeting_tip(facts: ottospace_core::calendar::Facts, color: u32) -> MeetingTip {
    Box::new(move |_, cx| AnyView::from(cx.new(|_| MeetingCard(facts.clone(), color))))
}

/// A task's hover card: shown as a tooltip, after the tooltip's own delay.
pub struct CardView(pub Card);

impl Render for CardView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let card = &self.0;
        let muted = || div().text_xs().text_color(rgb(theme::MUTED));
        let color = card.group.map_or(theme::MUTED, theme::task);
        let priority = Some(format!("P{}", card.priority));
        let state = [priority, Some(card.state.to_string()), card.live.clone(), card.model.clone()];
        let state = state.into_iter().flatten().collect::<Vec<_>>();
        let chip = |tag: &String| {
            div()
                .px_1p5()
                .rounded_sm()
                .bg(rgb(theme::BORDER_STRONG))
                .text_xs()
                .text_color(rgb(theme::TEXT))
                .child(tag.clone())
        };
        let cell = |text: String| div().w(px(52.)).text_right().child(text);
        let tokens = card.tokens.iter().map(|row| {
            let name = if row.sub { format!("  {} (sub-agent)", row.model) } else { row.model.clone() };
            div()
                .flex()
                .text_xs()
                .child(crate::cut_text(name).flex_1())
                .child(cell(count(row.input)))
                .child(cell(count(row.output)))
        });
        let context = card.context.zip(card.window).filter(|(_, window)| *window > 0).map(|(used, window)| {
            let share = (used as f32 / window as f32).clamp(0., 1.);
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(muted().child(format!("CONTEXT  {} of {}", count(used), count(window))))
                .child(
                    div()
                        .h(px(4.))
                        .w_full()
                        .rounded_full()
                        .bg(rgb(theme::BORDER_STRONG))
                        .child(div().h_full().w(relative(share)).rounded_full().bg(rgb(theme::PRIMARY))),
                )
        });
        frame(card.title.clone(), color, state.join(" · "))
            .when(!card.tags.is_empty(), |d| {
                d.child(div().flex().flex_wrap().gap_1().children(card.tags.iter().map(chip)))
            })
            .children(card.filed.clone().map(|age| fact("Filed", ago(age))))
            .child(fact("Started", card.started.clone().map_or("not yet".into(), ago)))
            .child(fact("Agent time", card.agent_time.clone()))
            .child(fact("Workspace", card.workspace.clone()))
            .when(!card.tokens.is_empty(), |d| {
                let head = muted()
                    .flex()
                    .child(div().flex_1().child("TOKENS"))
                    .child(cell("in".into()))
                    .child(cell("out".into()));
                d.child(div().flex().flex_col().gap_0p5().child(head).children(tokens))
            })
            .children(context)
    }
}

/// A tooltip that shows task `id`'s card, as it is when drawn. None: the task is gone.
pub fn card_tip(workspace: &Workspace, id: u32) -> Option<impl Fn(&mut Window, &mut gpui::App) -> AnyView + 'static> {
    let card = workspace.model.card(id, crate::now_ms())?;
    Some(move |_: &mut Window, cx: &mut gpui::App| AnyView::from(cx.new(|_| CardView(card.clone()))))
}

/// A pull request's hover card, in its mark's colour: title, `Draft → main`, checks, review,
/// merge. Other people's text, cleaned (`prs::Pr::card`), never a link.
pub struct PrCard(pub ottospace_core::prs::Card, pub u32);

impl Render for PrCard {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let card = &self.0;
        frame(card.title.clone(), self.1, card.state.clone())
            .child(fact("Checks", card.checks.clone()))
            .child(fact("Review", card.review.clone()))
            .child(fact("Merge", card.merge.clone()))
    }
}

/// An Elsewhere row's hover card: what its one line leaves out. Its title is quoted text.
pub struct SessionCard(pub Row);

impl Render for SessionCard {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let row = &self.0;
        // A wait says why ("needs you · permission"), quiet or not.
        let state = match (row.waits_for, row.active) {
            (None, false) => "idle".into(),
            _ => row.doing.clone().unwrap_or_else(|| "active".into()),
        };
        let color = match (row.waits_for, row.active) {
            (Some(_), _) => theme::task(Group::NeedsYou),
            (None, true) => theme::task(Group::Running),
            (None, false) => theme::BORDER_STRONG,
        };
        frame(row.title().to_string(), color, state)
            .child(fact("Repository", row.name.clone()))
            .child(fact("Runner", row.runner.clone()))
            .children(row.model.clone().map(|model| fact("Model", model)))
            .children(row.age.clone().map(|age| fact("Last active", ago(age))))
            .children(row.pane.map(|pane| fact("Runs in", format!("pane {pane}"))))
    }
}

/// The bar of a project's states: one segment a state, as wide as its share of the tasks.
fn bar(segments: Segments) -> Div {
    let shares = segments.shares();
    let parts = shares.iter().zip(SEGMENTS).filter(|(share, _)| **share > 0.);
    div()
        .w(px(64.))
        .h(px(6.))
        .flex()
        .gap(px(1.))
        .rounded_full()
        .overflow_hidden()
        .children(parts.map(|(share, color)| div().h_full().w(relative(*share)).bg(rgb(color))))
}

impl Workspace {
    /// A switch of a few words, one of them on: (sneak id, label, on). A click on one calls
    /// `click` with its place, and goes no further (a heading under it does not fold).
    fn switch<F>(&self, options: &[(&'static str, &'static str, bool)], click: F, cx: &mut Context<Self>) -> Div
    where
        F: Fn(&mut Self, usize, &Window, &mut Context<Self>) + Clone + 'static,
    {
        let option = |at: usize, (name, label, on): (&'static str, &'static str, bool)| {
            let click = click.clone();
            div()
                .id(name)
                .relative()
                .px_1p5()
                .rounded_sm()
                .cursor_pointer()
                .text_color(rgb(if on { theme::TEXT } else { theme::MUTED }))
                .when(on, |d| d.bg(rgb(theme::SELECTED)))
                .hover(|s| s.text_color(rgb(theme::TEXT)))
                .child(label)
                .child(self.controls.mark(name))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    click(this, at, window, cx);
                }))
        };
        let container = div().ml_2().p(px(2.)).flex().rounded_md().border_1().border_color(rgb(theme::BORDER));
        container.children(options.iter().copied().enumerate().map(|(at, o)| option(at, o)))
    }

    /// The Tasks header's switch: State | Project. A click picks one.
    pub(crate) fn tasks_view_switch(&self, cx: &mut Context<Self>) -> Div {
        let view = self.model.tasks_view;
        let options = [
            ("tasks-view:state", "State", view == TasksView::State),
            ("tasks-view:project", "Project", view == TasksView::Project),
        ];
        let views = [TasksView::State, TasksView::Project];
        self.switch(&options, move |this, at, window, cx| this.click_tasks_view(views[at], window, cx), cx)
    }

    pub(crate) fn click_tasks_view(&mut self, view: TasksView, window: &Window, cx: &mut Context<Self>) {
        self.dispatch(Action::TasksView { view }, self.input, window, cx);
        cx.notify();
    }

    pub(crate) fn click_project(
        &mut self,
        project: Option<String>,
        done: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(Action::ToggleProject { project, done }, self.input, window, cx);
        cx.notify();
    }

    /// The Project view: per project its line (fold mark, `#name`, the bar of states), its open
    /// tasks, then "Done · n". `listed`: rows of the list before; each line is a row, as list mode walks them.
    pub(crate) fn render_projects(
        &self,
        groups: &[ProjectGroup],
        listed: &mut usize,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut rows = Vec::new();
        for group in groups {
            let key = group.project.clone().unwrap_or_default();
            let name = group.project.as_ref().map_or("no project".to_string(), |p| format!("#{p}"));
            let (project, tip) = (group.project.clone(), group.bar.tip());
            let selected = self.model.list == Some(*listed);
            rows.push(
                div()
                    .id(gpui::ElementId::Name(format!("project:{key}").into()))
                    .px_2()
                    .pt_2()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_xs()
                    .text_color(rgb(theme::MUTED))
                    .cursor_pointer()
                    .hover(|s| s.text_color(rgb(theme::TEXT)))
                    .when(selected, |d| d.pb_0p5().rounded_md().bg(rgb(theme::SELECTED)))
                    .child(crate::ui::fold_mark(!group.folded).text_color(rgb(theme::MUTED)))
                    .child(crate::cut_text(name).flex_1())
                    .child(
                        div()
                            .id(gpui::ElementId::Name(format!("project-bar:{key}").into()))
                            .child(bar(group.bar))
                            .tooltip(move |_, cx| AnyView::from(cx.new(|_| Tooltip(tip.clone().into())))),
                    )
                    .relative()
                    .child(self.controls.mark(format!("project:{key}")))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.click_project(project.clone(), false, window, cx)),
                    )
                    .into_any_element(),
            );
            *listed += 1;
            if group.folded {
                continue;
            }
            let mut group_rows = Vec::new();
            for row in &group.rows {
                if let Some(task) = self.model.tasks.get(row.id) {
                    group_rows.push(self.task_row(task, row, self.model.list == Some(*listed), cx));
                }
                *listed += 1;
            }
            if group.done > 0 {
                let project = group.project.clone();
                let selected = self.model.list == Some(*listed);
                group_rows.push(
                    div()
                        .id(gpui::ElementId::Name(format!("project-done:{key}").into()))
                        .pl(px(22.))
                        .py_0p5()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .text_xs()
                        .text_color(rgb(theme::MUTED))
                        .cursor_pointer()
                        .hover(|s| s.text_color(rgb(theme::TEXT)))
                        .when(selected, |d| d.rounded_md().bg(rgb(theme::SELECTED)))
                        .child(crate::ui::fold_mark(group.done_open).text_color(rgb(theme::MUTED)))
                        .child(format!("Done · {}", group.done))
                        .relative()
                        .child(self.controls.mark(format!("project-done:{key}")))
                        .on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.click_project(project.clone(), true, window, cx)
                            }),
                        )
                        .into_any_element(),
                );
                *listed += 1;
                for row in &group.done_rows {
                    if let Some(task) = self.model.tasks.get(row.id) {
                        group_rows.push(self.task_row(task, row, self.model.list == Some(*listed), cx));
                    }
                    *listed += 1;
                }
            }
            rows.extend(crate::well(group_rows, 4.));
        }
        rows
    }

    /// Under the list: `Today  2 done · 1 running · 1h 05m`.
    pub(crate) fn render_today(&self) -> Div {
        let today = self.model.today(crate::day_start(crate::now_ms()));
        div()
            .flex_none()
            .px_3()
            .py_2()
            .flex()
            .gap_2()
            .border_t_1()
            .border_color(rgb(theme::BORDER))
            .text_xs()
            .text_color(rgb(theme::MUTED))
            .child(div().text_color(rgb(theme::TEXT)).child("Today"))
            .child(today.line())
    }
}

/// What the state reply says of the Tasks section's progress: the Project view as drawn, the
/// footer, and every listed task's card (numbers and names; the title is the user's own).
pub fn snapshot(workspace: &Workspace) -> serde_json::Value {
    let now = crate::now_ms();
    let model = &workspace.model;
    let today = model.today(crate::day_start(now));
    let cards: serde_json::Map<String, serde_json::Value> = model
        .tasks
        .inbox()
        .iter()
        .filter_map(|(_, task)| Some((task.id.to_string(), serde_json::json!(model.card(task.id, now)?))))
        .collect();
    serde_json::json!({
        "view": model.tasks_view,
        "projects": model.project_groups(now),
        "today": { "done": today.done, "running": today.running, "agent_ms": today.agent_ms, "line": today.line() },
        "cards": cards,
    })
}
