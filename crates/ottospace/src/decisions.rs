//! The Decisions card pane (`decisions` flag): the task, why it is first, the ask card as the
//! drawer draws it, and Skip and Later. Its keys are the core's (`Model::decision_key`). The
//! conversation is the drawer's: ⌘L talks the card over.

use gpui::{AnyElement, Context, Div, FontWeight, KeyDownEvent, Window, div, prelude::*, px, rgb};
use ottospace_core::model::chat::Key;
use ottospace_core::model::decisions::{LATER_MS, Sorting};
use ottospace_core::talk::Subject;
use ottospace_core::{Action, theme};

use crate::Workspace;
use crate::ui::{self, Button, Face, State};

/// The card's column at most this wide, in the middle of its pane.
const WIDTH: f32 = 720.;

impl Workspace {
    /// The card pane's body: its header, the ask card, its foot.
    pub(crate) fn decision_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(run) = self.model.decisions.as_ref() else { return div().into_any_element() };
        if let Some(sort) = &run.sort {
            return self.sort_body(sort, (run.at, run.of), cx);
        }
        let Some((task, ticket)) = run.task.zip(run.ticket) else { return div().into_any_element() };
        let subject = Subject::Task(task);
        let chat = self.model.chat(&subject);
        let card = chat.and_then(|chat| Some((chat, chat.form()?)));
        let card = card.map(|(chat, form)| self.ask_card(&subject, chat, form, ticket, Some(task), cx));
        let title = self.model.tasks.get(task).map(|t| t.title.clone()).unwrap_or_default();
        // Why it is first: from the queue's inputs alone. Its own entry: a held card may not be first.
        let why = run.queue.iter().find(|w| (w.task, w.ticket) == (task, ticket)).map(|own| {
            let age = ottospace_core::tasks::age(own.since, crate::now_ms());
            let waiting = if age == "now" { "just asked".to_string() } else { format!("waiting {age}") };
            format!("P{} · {waiting} · its agent asks", own.priority)
        });
        let header = div()
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .pt_3()
            .pb_2()
            .children(self.task_badge(task, 22.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(crate::cut_text(title).text_sm().font_weight(FontWeight::SEMIBOLD))
                    .children(why.map(|why| div().text_xs().text_color(rgb(theme::MUTED)).child(why))),
            )
            .child(div().text_xs().text_color(rgb(theme::MUTED)).child(format!("{} of {}", run.at, run.of)));
        let skip = ui::dialog::keyed("decision-skip", Button::Ghost(Face::Label("Skip".into())), &["S"], State::Rest)
            .child(self.controls.mark("decision:skip"))
            .on_click(cx.listener(move |this, _, window, cx| {
                drop(this.dispatch(Action::DecisionsSkip { task, ticket }, this.input, window, cx))
            }));
        let later =
            ui::dialog::keyed("decision-later", Button::Ghost(Face::Label("Later".into())), &["L"], State::Rest)
                .child(self.controls.mark("decision:later"))
                .on_click(cx.listener(move |this, _, window, cx| {
                    let until = crate::now_ms().saturating_add(LATER_MS);
                    drop(this.dispatch(Action::DecisionsLater { task, ticket, until }, this.input, window, cx))
                }));
        // Skip and Later first: in a narrow pane the hint wraps under them, never pushes them out.
        let foot =
            div().flex_none().flex().flex_wrap().items_center().gap_1().px_3().pb_3().child(skip).child(later).child(
                div()
                    .w_full()
                    .text_xs()
                    .text_color(rgb(theme::MUTED))
                    .child("1–9 pick · ↵ send · ⌘L talk it over · Esc close"),
            );
        div()
            .size_full()
            .flex()
            .justify_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(WIDTH))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(header)
                    .children(card.map(|card| card.flex_shrink().min_h_0()))
                    .child(foot),
            )
            .into_any_element()
    }

    /// Triage (`decision_sources`, the look provisional): its card, "Sort 3 new items", or the
    /// item being sorted: where it came from, what came in, what the filer suggests and claims
    /// (counted only once the user says), and its keys. One thing at a time; never a list.
    fn sort_body(&self, sort: &Sorting, (at, of): (usize, usize), cx: &mut Context<Self>) -> AnyElement {
        let muted = |text: String| div().text_xs().text_color(rgb(theme::MUTED)).child(text);
        let now = crate::now_ms();
        let (head, body, keys): (String, Vec<AnyElement>, Vec<(&str, &str, &str)>) = match sort.item {
            None => {
                let n = sort.left;
                let line = muted(format!("Each takes one key. {} sorted in this run.", sort.done)).into_any_element();
                let keys = vec![("sort-start", "Sort (↵)", "enter"), ("sort-skip", "Not now (s)", "s")];
                (format!("Sort {n} new item{}", if n == 1 { "" } else { "s" }), vec![line], keys)
            }
            Some(id) => {
                let Some(d) = self.model.filed.get(id) else { return div().into_any_element() };
                let from = format!("{} · {}", d.source.name(), ottospace_core::tasks::age(d.filed, now));
                let role = sort.role.map(|role| match role {
                    ottospace_core::filed::Role::Core => "core",
                    ottospace_core::filed::Role::Ancillary => "ancillary",
                });
                let suggest = match (d.suggest.projects.first(), role) {
                    (Some(project), Some(role)) => format!("Suggested: {project} · {role} (c)"),
                    (Some(project), None) => format!("Suggested: {project}"),
                    (None, Some(role)) => format!("Suggested: {role} (c)"),
                    (None, None) => "No project suggested".into(),
                };
                let due = d.claims.due.map(|due| match due > now {
                    true => format!("due in {}", ottospace_core::tasks::age(now, due)),
                    false => format!("overdue {}", ottospace_core::tasks::age(due, now)),
                });
                let waits = d.claims.person_waits.then(|| "someone waits".to_string());
                let claims: Vec<String> = waits.into_iter().chain(due).collect();
                let counted = if sort.claims { "counted (u)" } else { "not counted (u)" };
                let claims = (!claims.is_empty()).then(|| format!("Claims: {} · {counted}", claims.join(" · ")));
                let evidence = (!d.evidence.is_empty()).then(|| format!("{} pieces of evidence", d.evidence.len()));
                // Sorted before, then changed by its filer: what changed, so it is not taken for what was sorted.
                let revised =
                    (!d.revised.is_empty()).then(|| format!("Changed since you sorted it: {}", d.revised.join(", ")));
                let body = [
                    Some(muted(from).into_any_element()),
                    Some(
                        crate::cut_text(d.summary.clone())
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .into_any_element(),
                    ),
                    Some(muted(d.question.clone()).into_any_element()),
                    Some(muted(suggest).into_any_element()),
                    revised.map(|line| div().text_xs().text_color(rgb(theme::TEXT)).child(line).into_any_element()),
                    claims.map(|line| muted(line).into_any_element()),
                    evidence.map(|line| muted(line).into_any_element()),
                ];
                let keys = vec![
                    ("sort-keep", "Keep (↵)", "enter"),
                    ("sort-now", "Now (t)", "t"),
                    ("sort-later", "Later (l)", "l"),
                    ("sort-not-mine", "Not mine (n)", "n"),
                    ("sort-dismiss", "Dismiss (d)", "d"),
                ];
                ("Is this a priority, and where does it slot in?".into(), body.into_iter().flatten().collect(), keys)
            }
        };
        // A click is its key: the core says what the key does.
        let buttons = keys.into_iter().enumerate().map(|(n, (id, label, key))| {
            let kind = if n == 0 { Button::Primary(label.into()) } else { Button::Ghost(Face::Label(label.into())) };
            ui::button(id, kind, State::Rest).on_click(cx.listener(move |this, _, window, cx| {
                let mods = ottospace_core::keys::Mods::default();
                if let Key::Act(action) = this.model.sort_click(key, mods, crate::now_ms()) {
                    drop(this.dispatch(action, this.input, window, cx));
                }
            }))
        });
        // P trusts exactly the scope shown here: a channel, a project, a repository, a routine.
        let scope =
            sort.item.and_then(|id| self.model.filed.get(id)).map(|d| {
                match ottospace_core::filed::scope_label(d.source, &d.key) {
                    Some(scope) => format!(" · p skip triage for {scope} from now"),
                    None => String::new(),
                }
            });
        let hint = match scope {
            None => "↵ sort · s not now · Esc close".to_string(),
            Some(scope) => format!("u count its claims · c core or ancillary{scope} · z undo · Esc close"),
        };
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex_1().min_w_0().child(crate::cut_text(head).text_sm().font_weight(FontWeight::SEMIBOLD)))
            .child(muted(format!("{at} of {of}")));
        let column = div()
            .w_full()
            .max_w(px(WIDTH))
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .child(header)
            .children(body)
            .child(div().flex().flex_wrap().items_center().gap_1().pt_2().children(buttons))
            .child(muted(hint));
        div().size_full().flex().justify_center().child(column).into_any_element()
    }

    /// A key while the card pane has the keyboard. True: it was the card's.
    pub(crate) fn decision_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) -> bool {
        let key = &event.keystroke;
        // Held, only a move repeats: a held 1, ↵ or S acts once, not on the next card too.
        if event.is_held && !matches!(key.key.as_str(), "up" | "down" | "j" | "k") {
            return true;
        }
        match self.model.decision_key(&key.key, crate::mods(key.modifiers), crate::now_ms()) {
            Key::Act(action) => {
                self.dispatch(action, self.input, window, cx);
                cx.notify();
                true
            }
            Key::Swallow | Key::Field | Key::Newline => true,
            Key::Pass => false,
        }
    }

    /// Decisions' progress on its row in the top row: a thin bar along its top, `at - 1` of `of` cards done.
    pub(crate) fn decisions_progress(&self) -> Option<Div> {
        let run = self.model.decisions.as_ref()?;
        let done = (run.at.saturating_sub(1)) as f32 / run.of.max(1) as f32;
        let track = div().absolute().top_0().left_0().right_0().h(px(2.)).bg(rgb(theme::BORDER));
        Some(track.child(div().h_full().w(gpui::relative(done)).bg(rgb(theme::PICK))))
    }
}
