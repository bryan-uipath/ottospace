//! Sneak mode in the shell: where ottospace's own controls were drawn, the label pills over them,
//! and what a typed label does. Labels, keys and leaving are the core's (`ottospace_core::sneak`).
//!
//! The focused app pane's elements get labels too, in the app's own tint (`sneak_apps`): the user's
//! letter presses one there, and sets no value. See `docs/ai/sneak-apps.md`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::time::Duration;

use gpui::{
    AnyElement, Bounds, BoxShadow, Context, Div, FontWeight, Hsla, IntoElement, KeyDownEvent, Keystroke, Pixels,
    Window, black, canvas, div, point, prelude::*, px, rgb,
};
use ottospace_core::ax::{Frame, Source, Target};
use ottospace_core::marks::Field;
use ottospace_core::sneak::{Sneak, SneakKey, sneak_key};
use ottospace_core::{Action, Axis, Origin, PaneId, theme};

use crate::{SECTIONS, SPLITS, Workspace, control};

/// The mode's own shortcuts: ⌘; on the home row, and ⌘⌃G. Not ⌃G alone: that is BEL in a
/// terminal (readline and fzf abort, vim's file info).
pub const KEYS: [&str; 2] = ["cmd-;", "cmd-ctrl-g"];

/// What a socket client is told when its request would get in the way of the user's labels.
pub const ON: &str = "sneak mode is on";

fn is_own(key: &Keystroke) -> bool {
    KEYS.iter().any(|own| Keystroke::parse(own).is_ok_and(|own| own.modifiers == key.modifiers && own.key == key.key))
}

/// Whether sneak mode itself takes the key: a letter, ⌫, Esc, or its own shortcut. The only keys a
/// socket client may send while the labels are up.
pub fn takes(key: &Keystroke) -> bool {
    let plain = !key.modifiers.platform && !key.modifiers.control && !key.modifiers.alt;
    is_own(key) || (plain && sneak_key(&key.key).is_some())
}

/// Bounds in window points as the core's plain rect.
pub fn frame(bounds: Bounds<Pixels>) -> Frame {
    Frame {
        x: f32::from(bounds.origin.x).into(),
        y: f32::from(bounds.origin.y).into(),
        w: f32::from(bounds.size.width).into(),
        h: f32::from(bounds.size.height).into(),
    }
}

/// A label chip: sneak mode's pills, and the accessibility overlay's.
pub fn chip() -> Div {
    div()
        .flex_none()
        .px_1()
        .rounded_sm()
        .whitespace_nowrap()
        .bg(rgb(theme::SNEAK))
        .text_color(rgb(theme::BACKGROUND))
        .text_xs()
        // The raised-panel shadow, scaled down for a chip.
        .shadow(vec![BoxShadow {
            color: black().opacity(0.7),
            offset: point(px(0.), px(2.)),
            blur_radius: px(6.),
            spread_radius: px(0.),
        }])
}

#[derive(Default)]
struct Painted {
    /// Counts frames, so a caller can wait for the next one.
    frame: u64,
    /// Each control's id and the part of it that shows, in window points.
    controls: BTreeMap<String, Bounds<Pixels>>,
}

/// ottospace's own controls as the last frame drew them: what sneak mode puts labels on.
#[derive(Clone, Default)]
pub struct Controls(Rc<RefCell<Painted>>);

impl Controls {
    /// A new frame: a control that is not drawn again is gone.
    pub fn begin(&self) {
        let mut painted = self.0.borrow_mut();
        painted.frame += 1;
        painted.controls.clear();
    }

    /// An invisible layer over its parent that notes where the parent is. Goes beside a control's
    /// click handler; `Workspace::activate` must know the id. e.g. `rail:apps`, `tab:2`, `pane:1`.
    pub fn mark(&self, id: impl Into<String>) -> AnyElement {
        let (painted, id) = (self.0.clone(), id.into());
        let note = move |bounds: Bounds<Pixels>, window: &mut Window, _: &mut gpui::App| {
            // Only the part that shows: a row scrolled out of its list gets no label.
            let shown = bounds.intersect(&window.content_mask().bounds);
            if shown.size.width > px(1.) && shown.size.height > px(1.) {
                painted.borrow_mut().controls.insert(id, shown);
            }
        };
        canvas(note, |_, _, _, _| {}).absolute().top_0().left_0().size_full().into_any_element()
    }

    /// Where the control `id` was last drawn, if it was.
    pub fn bounds(&self, id: &str) -> Option<Bounds<Pixels>> {
        self.0.borrow().controls.get(id).copied()
    }

    fn frame(&self) -> u64 {
        self.0.borrow().frame
    }

    fn drew(&self, id: &str) -> bool {
        self.0.borrow().controls.contains_key(id)
    }

    fn targets(&self) -> Vec<Target> {
        let target = |(id, bounds): (&String, &Bounds<Pixels>)| Target {
            frame: frame(*bounds),
            source: Source::Own { id: id.clone() },
        };
        self.0.borrow().controls.iter().map(target).collect()
    }
}

/// The label pills: typed letters dimmed, labels the letters rule out not drawn.
pub fn labels(sneak: &Sneak) -> impl IntoElement + use<> {
    let dim = Hsla::from(rgb(theme::BACKGROUND)).opacity(0.35);
    let pills = sneak.visible().map(|t| {
        let f = t.target.frame;
        let (typed, rest) = t.label.split_at(sneak.typed.len().min(t.label.len()));
        // A label in another app is tinted as APP mode is: the press goes to that app.
        let app = matches!(t.target.source, Source::App { .. });
        let pill = chip()
            .when(app, |pill| pill.bg(rgb(theme::APP)))
            .flex()
            .line_height(px(15.))
            .font_weight(FontWeight::BOLD)
            .child(div().text_color(dim).child(typed.to_uppercase()))
            .child(rest.to_uppercase());
        // A pane's label sits in its middle, a small control's on it, a row's or bar's at its left.
        let body = matches!(&t.target.source, Source::Own { id } if id.starts_with("pane:"));
        div()
            .absolute()
            .left(px(f.x as f32))
            .top(px(f.y as f32))
            .w(px(f.w as f32))
            .h(px(f.h as f32))
            .flex()
            .items_center()
            .when(body || f.w <= 40., |d| d.justify_center())
            .when(!body && f.w > 40., |d| d.pl(px(2.)))
            .child(pill)
    });
    div().absolute().top_0().left_0().size_full().children(pills)
}

impl Workspace {
    /// ⌘; or ⌘⌃G: labels on the controls of the last painted frame, or out of the mode if it is on.
    pub(crate) fn toggle_sneak(&mut self, window: &Window, cx: &mut Context<Self>) {
        let targets = self.controls.targets();
        self.dispatch(Action::Sneak { targets }, self.input, window, cx);
    }

    /// Tells the core what the last painted frame drew, if a labelled control moved or went:
    /// its label follows it or goes with it. Every tick, and before a key is read as a label.
    pub(crate) fn sync_sneak(&mut self, window: &Window, cx: &mut Context<Self>) {
        let targets = self.controls.targets();
        if self.model.sneak.as_ref().is_some_and(|sneak| sneak.is_stale(&targets)) {
            self.dispatch(Action::SneakDrawn { targets }, Origin::System, window, cx);
        }
    }

    /// The `sneak` request: the mode is on when it replies, with the labels of a frame painted
    /// after the request came. It never replies with the controls of an earlier frame.
    pub(crate) fn serve_sneak(&mut self, reply: Sender<String>, window: &Window, cx: &mut Context<Self>) {
        // The request before this one (a split, a section) may not be painted yet: draw.
        let seen = self.controls.frame();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            // Two seconds at most: a window nobody can see is not painted.
            let mut drawn = false;
            for _ in 0..250 {
                drawn = this.update(cx, |this, _| this.controls.frame()).is_ok_and(|frame| frame != seen);
                if drawn {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(8)).await;
            }
            // Gone: the reply is dropped with the closure, and the client is told the app is closing.
            let _ = this.update_in(cx, |this, window, cx| {
                if !drawn {
                    let why = "the window has not been drawn: sneak mode labels what is on screen";
                    return drop(reply.send(control::error(why)));
                }
                // Enter only: if ⌘; / ⌘⌃G or another caller got here first, the mode stays on.
                let targets = this.controls.targets();
                this.dispatch(Action::SneakEnter { targets }, Origin::Socket, window, cx);
                this.sync_sneak(window, cx);
                match this.model.sneak {
                    // In a test run, once the labels are drawn too.
                    Some(_) => this.answer(reply, window, cx),
                    None => {
                        let why = "sneak mode starts from normal, list or talk mode: leave the app or close the box";
                        drop(reply.send(control::error(why)))
                    }
                }
            });
        })
        .detach();
    }

    /// Before a key's shortcut runs: any ⌘ shortcut but the mode's own leaves sneak mode first.
    /// `false`, always: the key goes on to its binding.
    pub(crate) fn shortcut_leaves_sneak(&mut self, key: &Keystroke, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.model.sneak.is_some() && key.modifiers.platform && !is_own(key) {
            self.dispatch(Action::LeaveSneak, self.input, window, cx);
        }
        false
    }

    /// Sneak mode takes every key: a letter types a label or, if no label goes on with it,
    /// leaves; ⌫ takes one back; Esc leaves. Other keys do nothing. (⌘ shortcuts left already.)
    pub(crate) fn sneak_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        // Nothing reaches the terminal under the labels.
        cx.stop_propagation();
        // ⌃A is not `a`. A held key types once.
        let plain = !key.modifiers.control && !key.modifiers.alt && !event.is_held;
        let action = match sneak_key(&key.key).filter(|_| plain) {
            Some(SneakKey::Type(letter)) => Action::SneakType { text: letter.to_string() },
            Some(SneakKey::Back) => Action::SneakBackspace,
            Some(SneakKey::Leave) => Action::LeaveSneak,
            None => return,
        };
        // The letter is read against the frame on screen now.
        self.sync_sneak(window, cx);
        self.dispatch(action, self.input, window, cx);
        // If that ended the mode, the key is still down: its repeats are not for the terminal.
        self.sneak_spent = self.model.sneak.is_none();
    }

    /// Selects a tab by its id: its place on the bar, and its label.
    pub(crate) fn select_tab(&mut self, tab: u32, window: &Window, cx: &mut Context<Self>) {
        if let Some(index) = self.model.tabs.iter().position(|t| t.id == tab) {
            self.dispatch(Action::SelectTab { index }, self.input, window, cx);
        }
    }

    /// Closes a tab by its id: its × on the bar, and the ×'s label.
    pub(crate) fn close_tab(&mut self, tab: u32, window: &Window, cx: &mut Context<Self>) {
        if let Some(index) = self.model.tabs.iter().position(|t| t.id == tab) {
            self.dispatch(Action::CloseTab { index: Some(index) }, self.input, window, cx);
        }
    }

    /// Closes a pane: its ×, and the ×'s label.
    pub(crate) fn close_pane(&mut self, pane: PaneId, window: &Window, cx: &mut Context<Self>) {
        self.dispatch(Action::Close { pane: Some(pane) }, self.input, window, cx);
    }

    /// Fits a pane to its app's window: the word `fit` in its header, and the word's label.
    pub(crate) fn fit_pane(&mut self, pane: PaneId, window: &Window, cx: &mut Context<Self>) {
        self.dispatch(Action::Fit { pane: Some(pane) }, self.input, window, cx);
    }

    /// What choosing a pane's body does, by a click or by its label: focus, or for an app pane
    /// the keyboard, which raises its window. `true`: an app pane went live just now.
    pub(crate) fn choose_pane(&mut self, pane: PaneId, window: &Window, cx: &mut Context<Self>) -> bool {
        if !self.hosted.contains_key(&pane) {
            self.dispatch(Action::Focus { pane }, self.input, window, cx);
            return false;
        }
        // Already live: the real window is over the pane and takes its own input.
        if self.model.live == Some(pane) {
            return false;
        }
        self.dispatch(Action::Enter { pane }, self.input, window, cx);
        true
    }

    /// Does what a click on the control `id` does: the same `Action` or method its click handler
    /// runs. No mouse event is made, and no control inside another app is pressed. Only a control
    /// the last frame drew: an id that is not on screen does nothing.
    pub(crate) fn activate(&mut self, id: &str, window: &Window, cx: &mut Context<Self>) {
        if !self.controls.drew(id) {
            return;
        }
        let (kind, name) = id.split_once(':').unwrap_or((id, ""));
        let number = name.parse::<u32>().ok();
        if self.activate_drawer(kind, name, window, cx) {
            return;
        }
        let action = match (kind, name) {
            ("rail", _) => {
                let section = SECTIONS.iter().find(|(_, _, label)| label.eq_ignore_ascii_case(name));
                section.map(|(section, ..)| Action::ToggleSection { section: *section })
            }
            ("bar", "close") => Some(Action::CloseSidebar),
            ("bar", "add") => {
                if let Some(section) = self.model.sidebar {
                    self.bar_add(section, window, cx);
                }
                None
            }
            ("repo-remove", _) => {
                self.remove_repo(name, window, cx);
                None
            }
            ("app", _) => {
                let _ = self.open_app(name, Axis::Right, false, window, cx);
                None
            }
            ("closed-app" | "app-pin" | "app-hide", _) => {
                self.click_app_control(kind, name, Axis::Right, window, cx);
                None
            }
            ("apps-more" | "apps-all" | "apps-filter" | "apps-undo", _) => {
                self.click_apps_control(kind, window, cx);
                None
            }
            ("repo", _) => Some(Action::OpenRepo { path: name.into(), axis: Axis::Right }),
            // The one verb, as a click: the user's starts the conversation, a client's shows the tabs.
            ("task" | "home-row" | "home-node", _) if self.input == ottospace_core::Origin::User => {
                number.map(|id| Action::TaskTalk { id })
            }
            ("task" | "home-row" | "home-node", _) => number.map(|id| Action::TaskOpen { id: Some(id) }),
            ("tasks-view", _) => {
                if let Ok(view) = serde_json::from_value(serde_json::json!(name)) {
                    self.click_tasks_view(view, window, cx);
                }
                None
            }
            // `project:` with no name is the group of no project.
            ("project" | "project-done", _) => {
                let project = Some(name.to_string()).filter(|name| !name.is_empty());
                self.click_project(project, kind == "project-done", window, cx);
                None
            }
            ("task-heading", _) => {
                if let Ok(heading) = serde_json::from_value(serde_json::json!(name)) {
                    self.click_heading(heading, window, cx);
                }
                None
            }
            ("task-snooze" | "task-hold" | "task-dismiss", _) => {
                if let Some(id) = number {
                    self.click_row_button(kind, id, window, cx);
                }
                None
            }
            ("session" | "session-open" | "session-adopt" | "session-hide", _) => {
                if let Some((runner, id)) = name.split_once(':') {
                    self.click_session_control(kind, runner.into(), id.into(), window, cx);
                }
                None
            }
            ("sessions-older", _) => {
                self.click_older_sessions(window, cx);
                None
            }
            ("task-undo", _) => {
                self.click_undo(window, cx);
                None
            }
            ("task-new", _) => {
                self.new_task(window, cx);
                None
            }
            ("home", "new_task") => {
                self.new_task(window, cx);
                None
            }
            // The right bar, the tool strip and the way back: `draft:add`, `draft:edit:2`.
            ("draft", _) => {
                self.click_capture(name, window, cx);
                None
            }
            ("inspector" | "inspector-row" | "inspector-fold", _) => {
                self.click_inspector(kind, name, window, cx);
                None
            }
            ("mark-undo", _) => Some(Action::MarkUndo),
            (
                "workbook" | "workbook-open" | "workbook-add" | "workbook-fold" | "workbook-rename" | "workbook-delete",
                _,
            ) => {
                if let Some(id) = number {
                    self.click_workbook(kind, id, None, window, cx);
                }
                None
            }
            // A tab of a workbook: `workbook-tab-open:3:1`.
            ("workbook-tab" | "workbook-tab-open" | "workbook-tab-rename" | "workbook-tab-remove", _) => {
                let place = name.split_once(':').and_then(|(id, at)| Some((id.parse().ok()?, at.parse().ok()?)));
                if let Some((id, at)) = place {
                    self.click_workbook(kind, id, Some(at), window, cx);
                }
                None
            }
            ("workbook-undo", _) => {
                self.undo_workbook(window, cx);
                None
            }
            ("bar", "save_tab") | ("workbook-save", _) => {
                self.save_workbook(window, cx);
                None
            }
            ("top", "task_leave") => Some(Action::TaskLeave),
            ("top", _) => {
                if let Some((.., axis, new_tab)) = SPLITS.iter().find(|(top, ..)| *top == name) {
                    self.top_button(*axis, *new_tab, window, cx);
                }
                None
            }
            ("header", _) => number.map(|pane| Action::Focus { pane }),
            ("unfilled-open", _) => number.map(|pane| Action::UnfilledOpen { pane }),
            _ => {
                match (kind, number) {
                    ("tab", Some(tab)) => self.select_tab(tab, window, cx),
                    ("tab-close", Some(tab)) => self.close_tab(tab, window, cx),
                    ("close", Some(pane)) => self.close_pane(pane, window, cx),
                    ("fit", Some(pane)) => self.fit_pane(pane, window, cx),
                    ("mark", Some(mark)) => self.choose_mark(mark, Axis::Right, window, cx),
                    ("mark-key", Some(mark)) => self.edit_mark(mark, Field::Key, window, cx),
                    ("mark-rename", Some(mark)) => self.edit_mark(mark, Field::Name, window, cx),
                    ("mark-remove", Some(mark)) => self.remove_mark(mark, window, cx),
                    // The click a mouse also replays into the app has no place here.
                    ("pane", Some(pane)) => drop(self.choose_pane(pane, window, cx)),
                    _ => {}
                }
                None
            }
        };
        if let Some(action) = action {
            self.dispatch(action, self.input, window, cx);
        }
    }
}
