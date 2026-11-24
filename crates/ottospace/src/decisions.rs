//! The Decisions card pane (`decisions` flag): the task, why it is first, the ask card as the
//! drawer draws it, and Skip and Later. Its keys are the core's (`Model::decision_key`). The
//! conversation is the drawer's: ⌘L talks the card over.

use gpui::{AnyElement, Context, Div, FontWeight, KeyDownEvent, Window, div, prelude::*, px, rgb};
use ottospace_core::model::chat::Key;
use ottospace_core::model::decisions::{LATER_MS, Sorting};
use ottospace_core::model::focus::WeekCard;
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
        // What this frame draws: a key or click names it, and one on another card or revision takes nothing.
        self.drawn_decision.set(self.model.on_show());
        if let Some(card) = run.week {
            return self.week_body(card, (run.at, run.of), cx);
        }
        if let Some(sort) = &run.sort {
            return self.sort_body(sort, (run.at, run.of), cx);
        }
        let Some((task, ticket)) = run.task.zip(run.ticket) else { return div().into_any_element() };
        let subject = Subject::Task(task);
        let chat = self.model.chat(&subject);
        let card = chat.and_then(|chat| Some((chat, chat.form()?)));
        let card = card.map(|(chat, form)| self.ask_card(&subject, chat, form, ticket, Some(task), cx));
        let title = self.model.tasks.get(task).map(|t| t.title.clone()).unwrap_or_default();
        // Why it is here: the rank's inputs alone ("Login rewrite · an agent waits · P3 · waiting
        // 5 min"); the hover gives the sum. Its own entry: a held card may not be first.
        let why = run.queue.iter().find(|w| (w.task, w.ticket) == (task, ticket)).map(|own| {
            let now = crate::now_ms();
            let age = ottospace_core::tasks::age(own.since, now);
            let waiting = if age == "now" { "just asked".to_string() } else { format!("waiting {age}") };
            let area = own.area.and_then(|at| self.model.focus.areas().get(at)).map(|a| a.name.as_str());
            let due = |at| ottospace_core::calendar::clock(at, crate::calendar::zone_offset(at));
            let line = ottospace_core::rank::line(&own.why, area, now, due);
            let tip: gpui::SharedString = ottospace_core::rank::sum(&own.why, area, now, due).into();
            div()
                .id("decision-why")
                .text_xs()
                .text_color(rgb(theme::MUTED))
                .child(format!("{line} · P{} · {waiting}", own.priority))
                .child(self.controls.mark("decision:why"))
                .tooltip(move |_, cx| gpui::AnyView::from(cx.new(|_| crate::Tooltip(tip.clone()))))
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
                    .children(why),
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
        // Each button: its id, its label, the key it is (its cap), and the key the core reads.
        type Keyed = (&'static str, &'static str, &'static str, &'static str);
        let (head, body, keys): (String, Vec<AnyElement>, Vec<Keyed>) = match sort.item {
            None if sort.left == 0 => {
                let line = muted(format!("{} sorted in this run.", sort.done)).into_any_element();
                ("All sorted".into(), vec![line], vec![("sort-undo", "Undo", "Z", "z")])
            }
            None => {
                let n = sort.left;
                let line = muted(format!("Each takes one key. {} sorted in this run.", sort.done)).into_any_element();
                let keys = vec![("sort-start", "Sort", "↵", "enter"), ("sort-skip", "Not now", "S", "s")];
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
                    ("sort-keep", "Keep", "↵", "enter"),
                    ("sort-now", "Now", "T", "t"),
                    ("sort-later", "Later", "L", "l"),
                    ("sort-not-mine", "Not mine", "N", "n"),
                    ("sort-dismiss", "Dismiss", "D", "d"),
                ];
                ("Is this a priority, and where does it slot in?".into(), body.into_iter().flatten().collect(), keys)
            }
        };
        // A click is its key: the core says what the key does.
        let buttons = keys.into_iter().enumerate().map(|(n, (id, label, cap, key))| {
            let kind = if n == 0 { Button::Primary(label.into()) } else { Button::Ghost(Face::Label(label.into())) };
            ui::dialog::keyed(id, kind, &[cap], State::Rest).on_click(cx.listener(move |this, _, window, cx| {
                let mods = ottospace_core::keys::Mods::default();
                let drawn = this.drawn_decision.get();
                if let Key::Act(action) = this.model.sort_click(key, mods, crate::now_ms(), drawn) {
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
            None if sort.left == 0 => "z takes the last back · Esc close".to_string(),
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
            .children(self.changed_line())
            .children(body)
            .child(div().flex().flex_wrap().items_center().gap_1().pt_2().children(buttons))
            .child(muted(hint));
        div().size_full().flex().justify_center().child(column).into_any_element()
    }

    /// The week's focus card (`decision_sources`): an agent's proposal, shown exactly as it would
    /// be set (each area's name and links) beside the focus now, and where the agent read it; or
    /// "Keep last week's focus?". The words are drawn as they are: data, never followed.
    fn week_body(&self, card: WeekCard, (at, of): (usize, usize), cx: &mut Context<Self>) -> AnyElement {
        let muted = |text: String| div().text_xs().text_color(rgb(theme::MUTED)).child(text);
        let focus = &self.model.focus;
        let names =
            |areas: &[ottospace_core::focus::Area]| areas.iter().map(|a| a.name.clone()).collect::<Vec<_>>().join(", ");
        let (title, purpose, areas, lines, keys): (&str, &str, Vec<_>, Vec<String>, [_; 3]) = match card {
            WeekCard::Proposal(_) => {
                let Some(p) = focus.pending.as_ref() else { return div().into_any_element() };
                let now = focus.areas();
                let gone: Vec<String> =
                    now.iter().filter(|a| !p.areas.contains(a)).map(|a| format!("− {}", a.name)).collect();
                let new: Vec<String> =
                    p.areas.iter().filter(|a| !now.contains(a)).map(|a| format!("+ {}", a.name)).collect();
                let mut lines = vec![match now.is_empty() {
                    true => "Now: nothing in focus this week".to_string(),
                    false => format!("Now: {}", names(now)),
                }];
                if !gone.is_empty() || !new.is_empty() {
                    lines.push(format!("Changes: {}", gone.into_iter().chain(new).collect::<Vec<_>>().join("  ")));
                }
                if let Some(from) = &p.provenance {
                    let at = from.lines.map(|(a, b)| format!(", lines {a}–{b}")).unwrap_or_default();
                    lines.push(format!("Read from {}{at}", from.path));
                    let quote = from.quote.lines().take(3).collect::<Vec<_>>().join(" / ");
                    if !quote.is_empty() {
                        lines.push(format!("“{quote}”"));
                    }
                }
                let keys = [
                    ("week-set", "Set it", "↵", "enter"),
                    ("week-edit", "Edit", "E", "e"),
                    ("week-keep", "Keep the focus now", "N", "n"),
                ];
                (
                    "This week's focus, proposed by an agent",
                    "Nothing changes until you set it.",
                    p.areas.clone(),
                    lines,
                    keys,
                )
            }
            WeekCard::Carry => {
                let last = focus.last().map(|w| w.areas.clone()).unwrap_or_default();
                let keys = [
                    ("week-carry", "Keep", "↵", "enter"),
                    ("week-edit", "Edit", "E", "e"),
                    ("week-none", "None", "N", "n"),
                ];
                ("Keep last week's focus?", "A new week: nothing is in focus until you say.", last, Vec::new(), keys)
            }
        };
        let (projects, checkouts) = (self.model.project_names(), self.model.checkouts());
        let rows = areas.iter().enumerate().map(|(n, a)| {
            // A link to nothing listed is shown as such: the agent is never told which.
            let unlisted = a.unlisted(&projects, &checkouts);
            let mark = |l: &String| if unlisted.contains(l) { format!("{l} (not listed)") } else { l.clone() };
            let links: Vec<String> = a.projects.iter().chain(a.repos.iter()).map(mark).collect();
            let links = if links.is_empty() { "(no link)".to_string() } else { format!("→ {}", links.join(", ")) };
            div()
                .flex()
                .gap_2()
                .text_sm()
                .child(muted(format!("{}", n + 1)))
                .child(crate::cut_text(a.name.clone()))
                .child(muted(links))
                .into_any_element()
        });
        let body = div()
            .flex()
            .flex_col()
            .gap_1()
            .children(self.changed_line())
            .children(rows)
            .children(lines.into_iter().map(|l| muted(l).into_any_element()));
        // A click is its key: the core says what the key does.
        let buttons: Vec<AnyElement> = keys
            .into_iter()
            .rev()
            .enumerate()
            .map(|(n, (id, label, cap, key))| {
                let kind =
                    if n == 2 { Button::Primary(label.into()) } else { Button::Ghost(Face::Label(label.into())) };
                ui::dialog::action(id, kind, &[cap], State::Rest)
                    .child(self.controls.mark(format!("decision:{id}")))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let drawn = this.drawn_decision.get();
                        if let Key::Act(action) = this.model.week_click(key, crate::now_ms(), drawn) {
                            drop(this.dispatch(action, this.input, window, cx));
                        }
                    }))
                    .into_any_element()
            })
            .collect();
        let close = ui::dialog::close("week-card-close", State::Rest).on_click(
            cx.listener(|this, _, window, cx| drop(this.dispatch(Action::DecisionsClose, this.input, window, cx))),
        );
        let panel = ui::dialog::frame(
            ui::dialog::FORM,
            title,
            Some(purpose.into()),
            close,
            Some(body),
            ui::dialog::actions(None, Some(format!("{at} of {of}").into()), buttons),
        );
        div().size_full().flex().items_center().justify_center().child(panel).into_any_element()
    }

    /// While the card on show changed since it was first drawn: says so; the next key only says the
    /// user saw it, so nothing is taken on words they did not read.
    fn changed_line(&self) -> Option<AnyElement> {
        let changed = self.model.decisions.as_ref().is_some_and(|run| run.changed);
        changed.then(|| {
            div()
                .text_xs()
                .text_color(rgb(theme::TEXT))
                .child("Changed while on show: read it again. Any key goes on; nothing is taken.")
                .into_any_element()
        })
    }

    /// A key while the card pane has the keyboard. True: it was the card's.
    pub(crate) fn decision_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) -> bool {
        let key = &event.keystroke;
        // Held, only a move repeats: a held 1, ↵ or S acts once, not on the next card too.
        if event.is_held && !matches!(key.key.as_str(), "up" | "down" | "j" | "k") {
            return true;
        }
        let drawn = self.drawn_decision.get();
        match self.model.decision_key_drawn(&key.key, crate::mods(key.modifiers), crate::now_ms(), drawn) {
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
