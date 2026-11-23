//! Slots on screen (`docs/ai/slots.md`, the `slots` flag): ⋯ on an app pane's header, the dialog
//! (a checklist made from the pane's window), a slot's pane saying what it waits for and what it
//! offers, and the shell's side: looks at the slots' apps, hosting what the core names, giving it back.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px, rgb,
};
use ottospace_core::features::Feature;
use ottospace_core::marks::same_app;
use ottospace_core::model::slots::Choice;
use ottospace_core::model::{Action, Kind, Origin, PaneId};
use ottospace_core::slot::{Look, Matching, Pick, Seen, Status};
use ottospace_core::theme;
use ottospace_mac::{ax, display, recovery};

use crate::{Hosted, Workspace, capture, dialog};

/// A clickable word in the dialog or a slot's pane, labelled for sneak mode as `id`.
fn word(id: String, label: impl Into<SharedString>, on: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(id))
        .flex_none()
        .px_2()
        .py_0p5()
        .rounded_sm()
        .border_1()
        .border_color(rgb(if on { theme::PRIMARY } else { theme::BORDER }))
        .text_color(rgb(if on { theme::PRIMARY } else { theme::TEXT }))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(theme::SELECTED)))
        .child(label.into())
        // The press must not reach a header: an unsteady click would start a drag.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// A ticked row: `☑ Window id is zm.meeting.window.main`.
fn tick(on: bool) -> &'static str {
    if on { "☑" } else { "☐" }
}

impl Workspace {
    /// ⋯ in an app pane's or a slot's header: the slot dialog. None with the flag off.
    pub(crate) fn slot_dots(&self, pane: PaneId, cx: &mut Context<Self>) -> Option<AnyElement> {
        let kind = self.model.pane(pane).map(|p| &p.kind).filter(|_| self.on(Feature::Slots))?;
        if !matches!(kind, Kind::App { .. } | Kind::Slot { .. }) {
            return None;
        }
        let id = format!("slot-dots:{pane}");
        let dots = div()
            .id(SharedString::from(id.clone()))
            .flex_none()
            .px_1()
            .cursor_pointer()
            .text_color(rgb(theme::MUTED))
            .hover(|s| s.text_color(rgb(theme::TEXT)))
            .child("⋯")
            .child(self.controls.mark(id))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| this.slot_dialog_open(pane, window, cx)));
        Some(dots.into_any_element())
    }

    /// A filled slot says so in its header: `slot`.
    pub(crate) fn slot_mark(&self, pane: PaneId) -> Option<AnyElement> {
        let filled = self.model.slots.get(&pane).is_some_and(|run| run.status == Status::Filled);
        let mark = div().flex_none().text_color(rgb(theme::MUTED)).child("slot");
        (filled && self.on(Feature::Slots)).then(|| mark.into_any_element())
    }

    /// The user's ⋯: a look at the pane's app now, for the dialog to be made from.
    fn slot_dialog_open(&mut self, pane: PaneId, window: &Window, cx: &mut Context<Self>) {
        let app = match self.model.pane(pane).map(|p| &p.kind) {
            Some(Kind::App { bundle, name, .. } | Kind::Slot { bundle, name }) => (bundle.clone(), name.clone()),
            _ => return,
        };
        let look = self.look_at(app.0.as_deref(), &app.1, Some(pane));
        self.offer_send(Action::SlotDialogOpen { pane, look }, window, cx);
    }

    /// What a look at `app` finds now: its windows the hosting would take (standard, the rules'),
    /// front to back, the displays, and the window the user is in if the app is in front. A test
    /// run's look is its fixture's (`slot_windows`), with pane `pane`'s own window if it has none.
    pub(crate) fn look_at(&self, bundle: Option<&str>, name: &str, pane: Option<PaneId>) -> Look {
        let of = |look: &&Look| same_app((look.bundle.as_deref(), &look.name), (bundle, name));
        if !self.hosting {
            let mut look = self.slot_fixtures.iter().find(of).cloned().unwrap_or_else(|| Look {
                bundle: bundle.map(String::from),
                name: name.to_string(),
                running: true,
                ..Look::default()
            });
            if let Some(Kind::App { window, title, .. }) = pane.and_then(|id| self.model.pane(id)).map(|p| &p.kind)
                && !look.windows.iter().any(|seen| seen.window == *window)
            {
                let subrole = "AXStandardWindow".into();
                look.windows.insert(0, Seen { window: *window, title: title.clone(), subrole, ..Seen::default() });
            }
            return look;
        }
        let running = ax::running();
        // A run limited to one process (`OTTOSPACE_HOST_ONLY`) looks at no other app's windows.
        let allowed = |pid: i32| self.host_only.is_none_or(|only| only == pid);
        let found = running.iter().find(|(pid, b, n)| allowed(*pid) && same_app((b.as_deref(), n), (bundle, name)));
        let displays = display::all().into_iter().map(recovery::to_frame).collect();
        let Some((pid, ..)) = found else {
            return Look { bundle: bundle.map(String::from), name: name.to_string(), displays, ..Look::default() };
        };
        // Dialogs and panels too: one in a pane (the user put it there) is the dialog's window.
        let windows = ax::Window::all_of(*pid)
            .iter()
            .filter(|w| w.quick(ax::Window::judged))
            .filter_map(|w| {
                w.quick(|w| {
                    Some(Seen {
                        pid: *pid,
                        window: w.id()?,
                        title: w.title().unwrap_or_default(),
                        identifier: w.identifier(),
                        subrole: w.subrole().unwrap_or_default(),
                        frame: w.frame().map(recovery::to_frame).unwrap_or_default(),
                    })
                })
            })
            .collect();
        let frontmost = ax::is_frontmost(*pid).then(|| ax::Window::main_of(*pid).and_then(|w| w.id())).flatten();
        Look { bundle: bundle.map(String::from), name: name.to_string(), running: true, windows, displays, frontmost }
    }

    /// Every 2 s: Away's clock (`SlotTick`), and a look at each app the core asked about
    /// (`Effect::WatchSlots`), sent when it changed. No look in a test run: its looks are fixtures.
    pub(crate) fn look_at_slots(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.model.slots.values().any(|run| run.away.is_some()) {
            self.dispatch(Action::SlotTick { at: crate::now_ms() }, Origin::System, window, cx);
        }
        if !self.hosting {
            return;
        }
        for (bundle, name) in self.slot_watch.clone() {
            let look = self.look_at(bundle.as_deref(), &name, None);
            let of = |old: &&Look| same_app((old.bundle.as_deref(), &old.name), (bundle.as_deref(), &name));
            if self.slot_seen.iter().find(of) == Some(&look) {
                continue;
            }
            self.slot_seen.retain(|old| !of(&old));
            self.slot_seen.push(look.clone());
            self.dispatch(Action::SlotWindows { look }, Origin::System, window, cx);
        }
    }

    /// `Effect::HostSlot`: window `id` of `pid` into slot `pane`, then `SlotHosted` says what came
    /// of it. A test run's fixture window has no window: the pane holds none, as `app_fixture`'s.
    pub(crate) fn host_slot(&mut self, pane: PaneId, pid: i32, id: u32, window: &Window, cx: &mut Context<Self>) {
        let run = self.model.slots.get(&pane).map(|run| (run.bundle.clone(), run.name.clone()));
        let Some((bundle, name)) = run else { return };
        if !self.hosting {
            let title = self.slot_fixtures.iter().flat_map(|look| &look.windows).find(|seen| seen.window == id);
            let title = title.map(|seen| seen.title.clone()).or_else(|| self.offer_fixtures.get(&id).cloned());
            let kind = title.map(|title| Kind::App { window: id, title, bundle, name });
            self.dispatch(Action::SlotHosted { pane, kind }, Origin::System, window, cx);
            return;
        }
        // Never another process's than the run allows (`OTTOSPACE_HOST_ONLY`).
        let allowed = self.host_only.is_none_or(|only| only == pid);
        let found = ax::Window::all_of(pid).into_iter().find(|w| allowed && w.id() == Some(id));
        match found
            .ok_or_else(|| "the window is gone".to_string())
            .and_then(|w| capture::target(w).and_then(Hosted::new))
        {
            Ok(hosted) => {
                let action = Action::SlotHosted { pane, kind: Some(hosted.kind()) };
                // The shell's report: a fill only the shell's news makes.
                let _ = self.with_origin(Origin::System, |this| this.attach(hosted, action, window, cx));
            }
            Err(why) => {
                self.broke(format!("could not put the window in its slot: {why}"));
                self.dispatch(Action::SlotHosted { pane, kind: None }, Origin::System, window, cx);
            }
        }
    }

    /// `Effect::GiveBack`: slot `pane`'s window goes back where it was; the pane stays, waiting.
    pub(crate) fn give_back_slot(&mut self, pane: PaneId) {
        if let Some(hosted) = self.hosted.remove(&pane) {
            hosted.release();
        }
    }

    /// A sneak label (or a click) on ⋯, the dialog or a slot's pane. False: not one of these.
    pub(crate) fn click_slot(&mut self, kind: &str, name: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        let pane = name.parse::<PaneId>().ok();
        let action = match (kind, name, pane) {
            ("slot-dots", _, Some(pane)) => {
                self.slot_dialog_open(pane, window, cx);
                return true;
            }
            ("slot-close", _, Some(pane)) => {
                self.close_pane(pane, window, cx);
                return true;
            }
            ("slot-accept", _, Some(pane)) => Action::SlotAccept { pane },
            ("slot-decline", _, Some(pane)) => Action::SlotDecline { pane },
            ("slot-next", _, Some(pane)) => Action::SlotNext { pane },
            ("slot-dialog", name, _) => match self.dialog_action(name) {
                Some(action) => action,
                None => return false,
            },
            _ => return false,
        };
        self.offer_send(action, window, cx);
        true
    }

    /// What a control of the open dialog does: `slot-dialog:by-id`, `slot-dialog:keep`, ...
    fn dialog_action(&self, name: &str) -> Option<Action> {
        let dialog = self.model.slot_dialog.as_ref()?;
        let pane = dialog.pane;
        let set = |choice| Some(Action::SlotDialogSet { choice });
        match name {
            "by-id" => set(Choice::ById(!dialog.by_id)),
            "by-title" => set(Choice::ByTitle(!dialog.by_title)),
            "manual" => set(Choice::Matching(Matching::Manual)),
            "automatic" => set(Choice::Matching(Matching::Automatic)),
            "largest" => set(Choice::Pick(Pick::Largest)),
            "smallest" => set(Choice::Pick(Pick::Smallest)),
            "any" => set(Choice::Pick(Pick::Any)),
            "keep" => Some(Action::SlotDialogKeep),
            "cancel" => Some(Action::SlotDialogClose),
            "look-again" => Some(Action::SlotLookAgain { pane }),
            "give-back" => Some(Action::SlotGiveBack { pane }),
            "clear" => Some(Action::SlotClear { pane }),
            _ => name
                .strip_prefix("display-")
                .and_then(|n| n.parse().ok())
                .and_then(|n| set(Choice::Pick(Pick::Display(n)))),
        }
    }

    /// ↵ keeps, Esc closes, while the dialog is open: the user's keys only.
    pub(crate) fn slot_dialog_key(&mut self, key: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.input != Origin::User || self.model.slot_dialog.is_none() || self.model.picker.is_some() {
            return false;
        }
        match key {
            "enter" => self.offer_send(Action::SlotDialogKeep, window, cx),
            "escape" => self.offer_send(Action::SlotDialogClose, window, cx),
            _ => {}
        }
        true
    }

    /// The dialog: every row a choice, the slot it makes in words at its foot.
    pub(crate) fn render_slot_dialog(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let dialog = self.model.slot_dialog.as_ref()?;
        let app = self.slot_app(dialog.pane)?;
        let control = |name: &str| format!("slot-dialog:{name}");
        let button = |name: &str, label: String, on: bool, cx: &mut Context<Self>| {
            let action = self.dialog_action(name);
            word(control(name), label, on).child(self.controls.mark(control(name))).on_click(cx.listener(
                move |this, _, window, cx| {
                    if let Some(action) = action.clone() {
                        this.offer_send(action, window, cx)
                    }
                },
            ))
        };
        let row = || div().flex().items_center().gap_2().flex_wrap();
        let label = |text: &'static str| div().w(px(150.)).flex_none().text_color(rgb(theme::MUTED)).child(text);
        // The rows as the slot has them when it is edited, else from the pane's window.
        let (id_row, title_row) = dialog.rows();
        let mut rows = vec![row().child(label("App")).child(app.clone()).into_any_element()];
        if let Some(id) = id_row.filter(|_| dialog.has_id) {
            // A place kept for a window with no id (`kept_place`) waits for one with none.
            let id = if id.is_empty() { "none".to_string() } else { id };
            let text = format!("{} Window id is {id}", tick(dialog.by_id));
            let note = div().text_xs().text_color(rgb(theme::MUTED)).child("the app's own name for this window");
            rows.push(row().child(button("by-id", text, dialog.by_id, cx)).child(note).into_any_element());
        }
        if let Some(title) = title_row {
            let is = if title.contains('*') { "matches" } else { "is" };
            let text = format!("{} Title {is} “{title}”", tick(dialog.by_title));
            rows.push(row().child(button("by-title", text, dialog.by_title, cx)).into_any_element());
        }
        let matches = dialog.matches(&crate::config::live().windows);
        if matches > 1 {
            let mut picks = row().child(label("When several match"));
            let displays = dialog.look.displays.len().max(1) as u32;
            let choices = [("largest", "The largest", Pick::Largest), ("smallest", "The smallest", Pick::Smallest)];
            for (name, text, pick) in choices {
                picks = picks.child(button(name, text.into(), dialog.pick == pick, cx));
            }
            for n in (1..=displays).filter(|_| displays > 1) {
                let name = format!("display-{n}");
                picks = picks.child(button(&name, format!("On display {n}"), dialog.pick == Pick::Display(n), cx));
            }
            picks = picks.child(button("any", "Any".into(), dialog.pick == Pick::Any, cx));
            let note = div().text_xs().text_color(rgb(theme::MUTED)).child(format!("{matches} windows match now"));
            rows.push(picks.child(note).into_any_element());
        }
        let matching = row()
            .child(label("Matching"))
            .child(button("manual", "Manual: offer it".into(), dialog.matching == Matching::Manual, cx))
            .child(button("automatic", "Automatic: take it".into(), dialog.matching == Matching::Automatic, cx));
        rows.push(matching.into_any_element());
        let words = dialog.slot().words(&app);
        let waits = div().text_xs().text_color(rgb(theme::PRIMARY)).child(format!("Waits for: {words}"));
        let mut foot = div().flex().items_center().gap_2().justify_end();
        if dialog.editing {
            let filled = matches!(self.model.pane(dialog.pane).map(|p| &p.kind), Some(Kind::App { .. }));
            foot = foot.child(button("look-again", "Look Again".into(), false, cx));
            if filled {
                foot = foot.child(button("give-back", "Give Back the Window".into(), false, cx));
            }
            foot = foot.child(button("clear", "Clear Slot".into(), false, cx));
        }
        let foot =
            foot.child(button("cancel", "Cancel".into(), false, cx)).child(button("keep", "Keep ↵".into(), true, cx));
        let heading = if dialog.editing { "Edit this slot" } else { "Keep this pane as a slot" };
        let panel = dialog::floating(560.)
            .child(div().px_3().pt_3().text_sm().text_color(rgb(theme::TEXT)).child(heading))
            .child(div().p_3().flex().flex_col().gap_2().text_sm().children(rows).child(waits).child(foot))
            .child(dialog::hint("↵ keep · esc cancel"));
        Some(
            dialog::backdrop("slot-dialog-backdrop", 0.5, Action::SlotDialogClose, cx)
                .flex()
                .items_center()
                .justify_center()
                .child(panel),
        )
    }

    /// The app a slot or app pane is of, as its header names it.
    fn slot_app(&self, pane: PaneId) -> Option<String> {
        match self.model.pane(pane).map(|p| &p.kind)? {
            Kind::App { name, .. } | Kind::Slot { name, .. } => Some(name.clone()),
            _ => None,
        }
    }

    /// A slot that waits, in its pane: what it waits for, in the dialog's words, and what it offers.
    pub(crate) fn slot_body(&self, pane: PaneId, cx: &mut Context<Self>) -> Option<AnyElement> {
        let run = self.model.slots.get(&pane).filter(|_| self.on(Feature::Slots))?;
        let app = run.name.clone();
        let waits = format!("Waits for: {}", run.slot.words(&app));
        let said = match run.status {
            Status::Absent => format!("{app} is not running"),
            Status::Away => format!("{app}'s window is away"),
            Status::Offered => match self.model.slot_offer(pane) {
                Some(seen) => format!("“{}” is open", seen.title),
                None => "A window is open".into(),
            },
            Status::Empty | Status::Filled => format!("Waiting for a {app} window"),
        };
        let line = |text: String, colour: u32| div().text_sm().text_color(rgb(colour)).child(text);
        let mut body = div().flex().flex_col().items_center().gap_2().child(line(said, theme::TEXT));
        if run.status == Status::Offered {
            body = body.child(line(format!("Matches this slot: {}", run.slot.words(&app)), theme::MUTED));
            let act = |name: &str, label: String, action: Action, cx: &mut Context<Self>| {
                let id = format!("{name}:{pane}");
                word(id.clone(), label, name == "slot-accept")
                    .child(self.controls.mark(id))
                    .on_click(cx.listener(move |this, _, window, cx| this.offer_send(action.clone(), window, cx)))
            };
            let mut buttons = div().flex().gap_2();
            buttons = buttons.child(act("slot-accept", "Bring it here ↵".into(), Action::SlotAccept { pane }, cx));
            buttons = buttons.child(act("slot-decline", "Not this one".into(), Action::SlotDecline { pane }, cx));
            if run.matches > 1 {
                let more = format!("{} more match", run.matches - 1);
                buttons = buttons.child(act("slot-next", more, Action::SlotNext { pane }, cx));
            }
            body = body.child(buttons);
        } else {
            body = body.child(line(waits, theme::MUTED));
            body = body.child(line("⌘E Put something else here".into(), theme::MUTED));
        }
        // Kept when its window closed: as easy to be rid of as the pane was to get.
        let id = format!("slot-close:{pane}");
        let close = word(id.clone(), "Close this pane", false)
            .child(self.controls.mark(id))
            .on_click(cx.listener(move |this, _, window, cx| this.close_pane(pane, window, cx)));
        body = body.child(close);
        Some(body.into_any_element())
    }

    /// `slot_windows`: what a look at an app found, as the test says (a test run hosts none).
    pub(crate) fn slot_windows_fixture(&mut self, look: Look, window: &Window, cx: &mut Context<Self>) {
        let of = |old: &Look| same_app((old.bundle.as_deref(), &old.name), (look.bundle.as_deref(), &look.name));
        self.slot_fixtures.retain(|old| !of(old));
        self.slot_fixtures.push(look.clone());
        self.dispatch(Action::SlotWindows { look }, Origin::System, window, cx);
    }
}
