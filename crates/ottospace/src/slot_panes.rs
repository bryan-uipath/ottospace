//! Slots on screen (`docs/ai/slots.md`, the `slots` flag): ⋯ on an app pane's header, the dialog
//! (its rule in one line over a form, `ui::dialog`), a slot's pane saying what it waits for and what it
//! offers, and the shell's side: looks at the slots' apps, hosting what the core names, giving it back.

use std::sync::Arc;

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, img, prelude::FluentBuilder, px, rgb, svg,
};
use ottospace_core::features::Feature;
use ottospace_core::marks::same_app;
use ottospace_core::model::slots::{By, Choice};
use ottospace_core::model::{Action, Kind, Origin, PaneId};
use ottospace_core::row_menu::MenuOf;
use ottospace_core::slot::{Look, Matching, Pick, Seen, Status, Test};
use ottospace_core::theme;
use ottospace_mac::{ax, display, recovery};

use crate::{Hosted, Workspace, capture, dialog, ui};

/// The most windows the dialog lists; its status line counts them all.
const LISTED: usize = 5;

impl Workspace {
    /// ⋯ in an app pane's or a slot's header: its menu (`MenuOf::Pane`). None with the flag off.
    pub(crate) fn slot_dots(&self, pane: PaneId, cx: &mut Context<Self>) -> Option<AnyElement> {
        let kind = self.model.pane(pane).map(|p| &p.kind).filter(|_| self.on(Feature::Slots))?;
        if !matches!(kind, Kind::App { .. } | Kind::Slot { .. }) {
            return None;
        }
        let id = format!("slot-dots:{pane}");
        let open = self.model.row_menu().is_some_and(|(menu, _)| menu.row == MenuOf::Pane(pane));
        let dots = div()
            .id(SharedString::from(id.clone()))
            .flex_none()
            .px_1()
            .rounded_sm()
            .cursor_pointer()
            .text_color(rgb(theme::MUTED))
            .when(open, |d| d.bg(rgb(theme::BORDER_STRONG)).text_color(rgb(theme::TEXT)))
            .hover(|s| s.text_color(rgb(theme::TEXT)))
            .child("⋯")
            .child(self.controls.mark(id))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(
                cx.listener(move |this, _, window, cx| this.open_row_menu(MenuOf::Pane(pane), false, window, cx)),
            );
        Some(dots.into_any_element())
    }

    /// A filled slot says so in its header: `slot`.
    pub(crate) fn slot_mark(&self, pane: PaneId) -> Option<AnyElement> {
        let filled = self.model.slots.get(&pane).is_some_and(|run| run.status == Status::Filled);
        let mark = div().flex_none().text_color(rgb(theme::MUTED)).child("slot");
        (filled && self.on(Feature::Slots)).then(|| mark.into_any_element())
    }

    /// The user's Keep or Edit in ⋯'s menu: a look at the pane's app now, for the dialog to be made from.
    pub(crate) fn slot_dialog_open(&mut self, pane: PaneId, window: &Window, cx: &mut Context<Self>) {
        let app = match self.model.pane(pane).map(|p| &p.kind) {
            Some(Kind::App { bundle, name, .. } | Kind::Slot { bundle, name }) => (bundle.clone(), name.clone()),
            _ => return,
        };
        let look = self.look_at(app.0.as_deref(), &app.1, Some(pane));
        self.offer_send(Action::SlotDialogOpen { pane, look }, window, cx);
    }

    /// What a look at `app` finds now: its windows the hosting judges (standard, dialogs, panels),
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
                        sheet: w.sheet(),
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
                self.open_row_menu(MenuOf::Pane(pane), true, window, cx);
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

    /// What a control of the open dialog does: `slot-dialog:by-title`, `slot-dialog:keep`, ...
    fn dialog_action(&self, name: &str) -> Option<Action> {
        self.model.slot_dialog.as_ref()?;
        let set = |choice| Some(Action::SlotDialogSet { choice });
        match name {
            "by-any" => set(Choice::By(By::Any)),
            "by-id" => set(Choice::By(By::Id)),
            "by-title" | "words" => set(Choice::By(By::Title)),
            "is" => set(Choice::Test(Test::Is)),
            "contains" => set(Choice::Test(Test::Contains)),
            "starts-with" => set(Choice::Test(Test::StartsWith)),
            "manual" => set(Choice::Matching(Matching::Manual)),
            "automatic" => set(Choice::Matching(Matching::Automatic)),
            "largest" => set(Choice::Pick(Pick::Largest)),
            "smallest" => set(Choice::Pick(Pick::Smallest)),
            "any" => set(Choice::Pick(Pick::Any)),
            "keep" => Some(Action::SlotDialogKeep),
            "cancel" => Some(Action::SlotDialogClose),
            _ => name
                .strip_prefix("display-")
                .and_then(|n| n.parse().ok())
                .and_then(|n| set(Choice::Pick(Pick::Display(n)))),
        }
    }

    /// The user's key while the dialog is open (no ⌘: those go to their bindings): ↵ keeps, Esc
    /// closes; O and T choose, while the title words do not have the keys; they edit those, ⌥ and
    /// ⌃ keys too. `None`: not the dialog's. `Some(false)`: text, for the input method to type.
    pub(crate) fn slot_dialog_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        if self.input != Origin::User || self.model.slot_dialog.is_none() || self.model.picker.is_some() {
            return None;
        }
        let typing = self.model.field().is_some_and(|(of, _)| of == "slot");
        let m = event.keystroke.modifiers;
        let plain = !(m.platform || m.control || m.alt);
        let action = match event.keystroke.key.as_str() {
            _ if m.platform => return None,
            "enter" | "escape" if !plain => None,
            "enter" => Some(Action::SlotDialogKeep),
            "escape" => Some(Action::SlotDialogClose),
            _ if typing => return Some(self.field_key(event, window, cx)),
            "o" if plain => self.dialog_action("manual"),
            "t" if plain => self.dialog_action("automatic"),
            _ => None,
        };
        if let Some(action) = action {
            self.offer_send(action, window, cx);
        }
        Some(true)
    }

    /// The dialog (board H4): its rule in one line on top, a form under it to loosen or tighten
    /// it, each choice with how many windows it takes; the app's windows open now, those it takes
    /// ticked; Keep slot (or Save) with ↵. × and Esc close it.
    pub(crate) fn render_slot_dialog(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let dialog = self.model.slot_dialog.as_ref()?;
        let app = self.slot_app(dialog.pane)?;
        let rules = &crate::config::live().windows;
        let typing = self.model.field().is_some_and(|(of, _)| of == "slot");
        // A control the user clicks or labels: `slot-dialog:<name>`, its action the dialog's.
        let live = |el: gpui::Stateful<gpui::Div>, name: &str, cx: &mut Context<Self>| {
            let id = format!("slot-dialog:{name}");
            let action = self.dialog_action(name);
            el.child(self.controls.mark(id))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| {
                    if let Some(action) = action.clone() {
                        this.offer_send(action, window, cx)
                    }
                }))
        };
        let segment = |name: &str, label: String, key: Option<&'static str>, on: bool, cx: &mut Context<Self>| {
            let id = SharedString::from(format!("slot-dialog-seg:{name}"));
            live(ui::dialog::segment(id, label, key.filter(|_| !typing), on, ui::State::Rest), name, cx)
        };
        // Window: any, by id (when there is one), by title; each with how many it takes now.
        let mut bys = vec![("by-any", "Any window", By::Any)];
        if dialog.has_id {
            bys.push(("by-id", "Window id", By::Id));
        }
        bys.push(("by-title", "Title", By::Title));
        let bys: Vec<_> = bys
            .into_iter()
            .map(|(name, label, by)| {
                let label = format!("{label} · {}", dialog.count(by, rules));
                segment(name, label, None, dialog.by == by, cx)
            })
            .collect();
        let said = match dialog.by {
            By::Any => "Any window of the app: Several says which.".to_string(),
            By::Id => {
                let id = dialog.kept.as_ref().and_then(|k| k.identifier.clone());
                let id = id.or_else(|| dialog.window.as_ref().and_then(|w| w.identifier.clone())).unwrap_or_default();
                if id.is_empty() {
                    "A window the app gives no id.".into()
                } else {
                    format!("The app's own name for it: {id}")
                }
            }
            By::Title => "A window whose title passes the test.".into(),
        };
        // Title: its test and words. There while the rule is another, dimmed: a click on it chooses it.
        let titled = dialog.by == By::Title;
        let tests: Vec<_> = [("is", Test::Is), ("contains", Test::Contains), ("starts-with", Test::StartsWith)]
            .into_iter()
            .map(|(name, test)| segment(name, test.label().into(), None, titled && dialog.test == test, cx))
            .collect();
        let words = match titled {
            true => self.render_field_of("slot", "words in the title", cx).into_any_element(),
            false => {
                div().truncate().text_color(rgb(theme::MUTED)).child(dialog.words.text().to_string()).into_any_element()
            }
        };
        let field = ui::dialog::text_box(typing).child(words);
        let field = match titled {
            true => field.into_any_element(),
            false => live(div().id("slot-dialog-words").flex_1().min_w_0().cursor_pointer().child(field), "words", cx)
                .into_any_element(),
        };
        let title_row = div()
            .flex_1()
            .flex()
            .gap(px(theme::SPACE_1_5))
            .when(!titled, |d| d.opacity(0.5))
            .child(ui::dialog::segments(tests, false))
            .child(field);
        // Several: which, when more than one matches.
        let mut picks = vec![
            segment("largest", "The largest".into(), None, dialog.pick == Pick::Largest, cx),
            segment("smallest", "The smallest".into(), None, dialog.pick == Pick::Smallest, cx),
            segment("any", "The frontmost".into(), None, dialog.pick == Pick::Any, cx),
        ];
        for n in (1..=dialog.look.displays.len() as u32).filter(|_| dialog.look.displays.len() > 1) {
            let name = format!("display-{n}");
            picks.push(segment(&name, format!("Display {n}"), None, dialog.pick == Pick::Display(n), cx));
        }
        let offers =
            [("manual", "Offer it", "O", Matching::Manual), ("automatic", "Take it", "T", Matching::Automatic)].map(
                |(name, label, key, matching)| segment(name, label.into(), Some(key), dialog.matching == matching, cx),
            );
        let does = match dialog.matching {
            Matching::Manual => "Shows it here with Put it here; nothing moves until you press it.",
            Matching::Automatic => {
                "Moves it in at once, on the tab on show. A window you are in is still only offered."
            }
        };
        // The one it takes first, then the rest front to back, at most `LISTED`, then how many more.
        let mut all = dialog.listed(rules);
        all.sort_by_key(|l| !l.taken);
        let more = all.len().saturating_sub(LISTED);
        let mut listed: Vec<ui::dialog::Listed> = all
            .into_iter()
            .take(LISTED)
            .map(|l| ui::dialog::Listed { taken: l.taken, title: l.seen.title.clone().into(), detail: l.detail.into() })
            .collect();
        if more > 0 {
            listed.push(ui::dialog::Listed {
                taken: false,
                title: format!("and {more} more").into(),
                detail: "".into(),
            });
        }
        let mut body = div()
            .child(ui::dialog::summary(dialog.headline(&app)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::SPACE_1_5))
                    .child(ui::dialog::row("Window", ui::dialog::segments(bys, false)))
                    .child(ui::dialog::consequence(said)),
            )
            .child(ui::dialog::row("Title", title_row));
        // A dialog's or panel's slot holds its kind only: said, not a choice (another kind needs the core's rules).
        if let Some(role) = dialog.kind_word() {
            let only = div().text_size(px(theme::TEXT_XS)).child(format!("Only a {role}, as this one"));
            body = body.child(ui::dialog::row("Kind", only));
        }
        body = body
            .child(ui::dialog::row("Several", ui::dialog::segments(picks, false)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::SPACE_1_5))
                    .child(ui::dialog::row("When it opens", ui::dialog::segments(offers, false)))
                    .child(ui::dialog::consequence(does)),
            )
            .children((!listed.is_empty()).then(|| ui::dialog::row("Open now", ui::dialog::window_list(listed))));
        let (title, purpose, keep) = match dialog.editing {
            true => (format!("{app} slot"), "Change what it waits for.", "Save"),
            false => (
                format!("Keep this pane for {app}"),
                "The pane stays when the window goes, and waits for the next one.",
                "Keep slot",
            ),
        };
        let status = format!("{} of {} windows match", dialog.matches(rules), dialog.look.windows.len());
        let keep = live(
            ui::dialog::action("slot-dialog-keep", ui::Button::Primary(keep.into()), &["↵"], ui::State::Rest),
            "keep",
            cx,
        );
        let close = live(ui::dialog::close("slot-dialog-close", ui::State::Rest), "cancel", cx);
        let panel = ui::dialog::frame(
            ui::dialog::FORM,
            title,
            Some(purpose.into()),
            close,
            Some(body),
            ui::dialog::actions(None, Some(status.into()), vec![keep.into_any_element()]),
        );
        Some(
            dialog::backdrop("slot-dialog-backdrop", ui::dialog::DIM, Action::SlotDialogClose, cx)
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

    /// A slot's app icon, from the cache by bundle id (`slot_icon`); the generic one without.
    fn slot_icon(&self, bundle: Option<&str>, side: f32, dim: bool) -> AnyElement {
        let read = |bundle: &str| {
            let dir = crate::config::get().home.as_ref().map(|home| home.join("icons"));
            let png = crate::slot_icon::png(bundle, dir.as_deref(), crate::ICON_PIXELS)?;
            Some(Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png)))
        };
        let image = bundle.and_then(|bundle| {
            let mut icons = self.slot_icons.borrow_mut();
            icons.entry(bundle.to_string()).or_insert_with(|| read(bundle)).clone()
        });
        let opacity = if dim { 0.5 } else { 1. };
        match image {
            Some(image) => img(image).size(px(side)).flex_none().opacity(opacity).into_any_element(),
            None => svg()
                .path("icons/apps.svg")
                .size(px(side))
                .flex_none()
                .text_color(rgb(theme::MUTED))
                .opacity(opacity)
                .into_any_element(),
        }
    }

    /// A slot that waits, in its pane: a dashed outline, its app's icon, what it waits for in the
    /// dialog's words and whether it offers or takes. Offering: a solid outline and Put it here.
    /// A small pane: the icon and one line.
    pub(crate) fn slot_body(&self, pane: PaneId, cx: &mut Context<Self>) -> Option<AnyElement> {
        let run = self.model.slots.get(&pane).filter(|_| self.on(Feature::Slots))?;
        let app = run.name.clone();
        let offered = (run.status == Status::Offered).then(|| self.model.slot_offer(pane)).flatten();
        let headline = match offered {
            Some(seen) => format!("“{}” is open", seen.title),
            None => format!("Waiting for a {app} window"),
        };
        let state = self.model.slot_note(pane);
        let mode = match run.matching {
            Matching::Manual => "Offers it here when one opens",
            Matching::Automatic => "Takes it here when one opens",
        };
        let size = self.cells.get(&pane).map(|cell| cell.get().size).unwrap_or_default();
        // Not laid out yet (a first frame): the full one.
        let small = size.width > px(0.) && (size.width < px(260.) || size.height < px(180.));
        let line = |text: String, colour: u32| div().text_sm().text_color(rgb(colour)).text_center().child(text);
        let icon = self.slot_icon(run.bundle.as_deref(), if small { 20. } else { 40. }, offered.is_none());
        let mut body = div().flex().items_center().gap_2().child(icon);
        if small {
            // One line, cut: a narrow pane never wraps it under the icon.
            let one = line(headline, theme::TEXT).min_w_0().whitespace_nowrap().overflow_hidden().text_ellipsis();
            body = body.flex_row().px_2().max_w_full().child(one);
        } else {
            body = body.flex_col().px_4().child(line(headline, theme::TEXT));
            body = body.child(line(run.slot.words(&app), theme::MUTED));
            body = body.children(state.map(|state| line(state, theme::MUTED)));
            if offered.is_none() {
                body = body.child(line(mode.into(), theme::MUTED));
            }
        }
        if offered.is_some() {
            let id = format!("slot-accept:{pane}");
            let accept =
                ui::button(SharedString::from(id.clone()), ui::Button::Primary("Put it here".into()), ui::State::Rest)
                    .child(self.controls.mark(id))
                    // The press must not reach a header: an unsteady click would start a drag.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.offer_send(Action::SlotAccept { pane }, window, cx)
                        }),
                    );
            body = body.child(accept);
            // Quieter: pass over it, or the next that matches.
            let quiet = |name: &str, label: String, action: Action, cx: &mut Context<Self>| {
                let id = format!("{name}:{pane}");
                div()
                    .id(SharedString::from(id.clone()))
                    .text_xs()
                    .text_color(rgb(theme::MUTED))
                    .cursor_pointer()
                    .hover(|s| s.text_color(rgb(theme::TEXT)))
                    .child(label)
                    .child(self.controls.mark(id))
                    .on_click(cx.listener(move |this, _, window, cx| this.offer_send(action.clone(), window, cx)))
            };
            let mut more = div().flex().gap_3();
            more = more.child(quiet("slot-decline", "Not this one".into(), Action::SlotDecline { pane }, cx));
            if run.matches > 1 {
                let next = format!("{} more match", run.matches - 1);
                more = more.child(quiet("slot-next", next, Action::SlotNext { pane }, cx));
            }
            body = body.children((!small).then_some(more));
        }
        // Waiting: dashed and quiet; offering: solid, in the accent. Drawn still, so Reduce Motion has nothing to stop.
        let outline = div().absolute().inset(px(10.)).rounded_md().border_1().flex().items_center().justify_center();
        let outline = match offered {
            Some(_) => outline.border_color(rgb(theme::PRIMARY)),
            None => outline.border_dashed().border_color(rgb(theme::BORDER_STRONG)),
        };
        Some(outline.child(body).into_any_element())
    }

    /// `slot_windows`: what a look at an app found, as the test says (a test run hosts none).
    pub(crate) fn slot_windows_fixture(&mut self, look: Look, window: &Window, cx: &mut Context<Self>) {
        let of = |old: &Look| same_app((old.bundle.as_deref(), &old.name), (look.bundle.as_deref(), &look.name));
        self.slot_fixtures.retain(|old| !of(old));
        self.slot_fixtures.push(look.clone());
        self.dispatch(Action::SlotWindows { look }, Origin::System, window, cx);
    }
}
