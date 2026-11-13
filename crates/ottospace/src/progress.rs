//! Shell half of a task's progress: the Tasks section's State | Project switch, the Project
//! view's groups with their bar of states, the Today footer, and a task's hover card. What each
//! says is the core's (`Model::project_groups`, `Model::today`, `Model::card`); this draws it.

use gpui::{
    AnyElement, AnyView, AppContext, Context, Div, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px, relative, rgb,
};
use ottospace_core::model::{Card, ProjectGroup, TasksView};
use ottospace_core::progress::count;
use ottospace_core::tasks::Segments;
use ottospace_core::{Action, theme};

use crate::{Tooltip, Workspace, home};

/// The segments' colours, in the bar's order: needs you, running, later, done.
const SEGMENTS: [u32; 4] = [theme::SNEAK, theme::PRIMARY, 0x71717a, theme::PICK];

/// A task's hover card: shown as a tooltip, after the tooltip's own delay.
pub struct CardView(pub Card);

impl Render for CardView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let card = &self.0;
        let muted = || div().text_xs().text_color(rgb(theme::MUTED));
        let color = card.group.map_or(theme::MUTED, theme::task);
        let state = [Some(card.state.to_string()), card.live.clone()].into_iter().flatten().collect::<Vec<_>>();
        let chip = |tag: &String| {
            div()
                .px_1p5()
                .rounded_sm()
                .bg(rgb(theme::BORDER_STRONG))
                .text_xs()
                .text_color(rgb(theme::TEXT))
                .child(tag.clone())
        };
        let fact = |name: &'static str, value: String| {
            div().flex().gap_2().child(muted().w(px(72.)).child(name)).child(div().text_xs().child(value))
        };
        let cell = |text: String| div().w(px(52.)).text_right().child(text);
        let tokens = card.tokens.iter().map(|row| {
            let name = if row.sub { format!("  {} (sub-agent)", row.model) } else { row.model.clone() };
            div()
                .flex()
                .text_xs()
                .child(div().flex_1().min_w_0().truncate().child(name))
                .child(cell(count(row.input)))
                .child(cell(count(row.output)))
                .child(cell(count(row.cached)))
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
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(card.title.clone()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(home::dot(color))
                    .child(div().text_xs().truncate().child(state.join(" · "))),
            )
            .when(!card.tags.is_empty(), |d| {
                d.child(div().flex().flex_wrap().gap_1().children(card.tags.iter().map(chip)))
            })
            .child(fact("Started", card.started.clone().map_or("not yet".into(), |age| format!("{age} ago"))))
            .child(fact("Agent time", card.agent_time.clone()))
            .child(fact("Workspace", card.workspace.clone()))
            .when(!card.tokens.is_empty(), |d| {
                let head = muted()
                    .flex()
                    .child(div().flex_1().child("TOKENS"))
                    .child(cell("in".into()))
                    .child(cell("out".into()))
                    .child(cell("cached".into()));
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
    /// The Tasks header's switch: State | Project. A click picks one.
    pub(crate) fn tasks_view_switch(&self, cx: &mut Context<Self>) -> Div {
        let view = self.model.tasks_view;
        let option = |name: &'static str, label: &'static str, of: TasksView| {
            let on = view == of;
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
                .on_click(cx.listener(move |this, _, window, cx| this.click_tasks_view(of, window, cx)))
        };
        div()
            .ml_2()
            .p(px(2.))
            .flex()
            .rounded_md()
            .border_1()
            .border_color(rgb(theme::BORDER))
            .child(option("tasks-view:state", "State", TasksView::State))
            .child(option("tasks-view:project", "Project", TasksView::Project))
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
                    .child(div().w(px(8.)).child(if group.folded { "▸" } else { "▾" }))
                    .child(div().flex_1().min_w_0().truncate().child(name))
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
            for row in &group.rows {
                if let Some(task) = self.model.tasks.get(row.id) {
                    rows.push(self.task_row(task, row, self.model.list == Some(*listed), cx));
                }
                *listed += 1;
            }
            if group.done > 0 {
                let project = group.project.clone();
                let selected = self.model.list == Some(*listed);
                rows.push(
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
                        .child(div().w(px(8.)).child(if group.done_open { "▾" } else { "▸" }))
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
                        rows.push(self.task_row(task, row, self.model.list == Some(*listed), cx));
                    }
                    *listed += 1;
                }
            }
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
