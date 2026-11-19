//! A new window of a hosted app: where it went, and why, in the log; and the chip its app's pane
//! shows while it floats (`Zoom opened “Zoom Meeting” · no slot for this window · Put in a pane ×`).
//! The core decides (`rules::decide`) and keeps the offer (`Model::offers`); this draws and hosts.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, StatefulInteractiveElement,
    Styled, Window, div, rgb,
};
use ottospace_core::model::{Action, Kind, Origin, PaneId};
use ottospace_core::{rules, theme};

use crate::{Hosted, Workspace, capture, cut_text, event_log};

/// What a new window was, for the log line. Never its title.
pub(crate) struct Seen<'a> {
    pub(crate) bundle: Option<&'a str>,
    pub(crate) subrole: &'a str,
}

/// One line in `<home>/log` a placement: the app, the window's kind and size, its pane's size,
/// where it went and why (the rule, if one decided). Never the title.
pub(crate) fn log(seen: &Seen, shape: &rules::Shape, decided: &rules::Decision) {
    let place = match decided.place {
        rules::Place::Split => "split",
        rules::Place::Float { offer: true } => "offer",
        rules::Place::Float { offer: false } => "float",
        rules::Place::Leave => "leave",
    };
    let rule = decided.rule.map(|(source, action)| format!("{}: {}", source.label(), action.name()));
    event_log::event(
        "place",
        serde_json::json!({
            "bundle": seen.bundle,
            "subrole": seen.subrole,
            "modal": shape.modal,
            "sheet": shape.sheet,
            "size": shape.size,
            "pane": shape.pane,
            "min_width": shape.min_width,
            "place": place,
            "why": decided.why.name(),
            "rule": rule,
        }),
    );
}

/// A `window_fixture` request's window.
pub(crate) struct Fixture {
    pub(crate) pane: PaneId,
    pub(crate) title: String,
    pub(crate) subrole: String,
    pub(crate) size: [f64; 2],
    pub(crate) modal: bool,
}

/// The chip's controls, for clicks and sneak labels: `offer:<window>`, `offer-dismiss:<window>`.
fn accept_control(window: u32) -> String {
    format!("offer:{window}")
}

fn dismiss_control(window: u32) -> String {
    format!("offer-dismiss:{window}")
}

impl Workspace {
    /// The chip in app pane `pane`'s header while a window its app opened floats: what opened,
    /// why it is in no pane, Put in a pane, ×. None with no offer.
    pub(crate) fn offer_header(&self, pane: PaneId, cx: &mut Context<Self>) -> Option<AnyElement> {
        let offer = self.model.offer_of(pane)?;
        let app = match self.model.pane(pane).map(|p| &p.kind) {
            Some(Kind::App { name, .. }) => name.clone(),
            _ => return None,
        };
        let window = offer.window;
        let what = if offer.title.is_empty() {
            format!("{app} opened a window · {}", offer.why)
        } else {
            format!("{app} opened “{}” · {}", offer.title, offer.why)
        };
        let button = |id: String, label: &'static str, colour: u32| {
            div()
                .id(gpui::SharedString::from(id.clone()))
                .flex_none()
                .px_1()
                .rounded_sm()
                .cursor_pointer()
                .text_color(rgb(colour))
                .hover(|s| s.bg(rgb(theme::SELECTED)))
                .child(label)
                .child(self.controls.mark(id))
                // The press must not reach the header: an unsteady click would start a drag.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let accept = button(accept_control(window), "Put in a pane", theme::PRIMARY).on_click(
            cx.listener(move |this, _, window_, cx| this.offer_send(Action::OfferAccept { window }, window_, cx)),
        );
        let dismiss = button(dismiss_control(window), "×", theme::MUTED).on_click(
            cx.listener(move |this, _, window_, cx| this.offer_send(Action::OfferDismiss { window }, window_, cx)),
        );
        let chip = div()
            .flex()
            .min_w_0()
            .items_center()
            .gap_1()
            .px_1()
            .rounded_sm()
            .border_1()
            .border_color(rgb(theme::BORDER_STRONG))
            .child(cut_text(what).text_color(rgb(theme::MUTED)))
            .child(accept)
            .child(dismiss);
        Some(chip.into_any_element())
    }

    /// An empty pane that waits for a window its app remade, in its body: `Zoom replaced its
    /// window`, its title, Bring it here ↵ and ×.
    pub(crate) fn offer_body(&self, pane: PaneId, cx: &mut Context<Self>) -> Option<AnyElement> {
        let offer = self.model.offer_of(pane).filter(|_| self.model.offer_waits(pane))?;
        let window = offer.window;
        let said = format!("{} replaced its window", offer.app.1);
        let title = (!offer.title.is_empty())
            .then(|| div().text_sm().text_color(rgb(theme::MUTED)).child(format!("“{}”", offer.title)));
        let button = |id: String, label: &'static str, colour: u32| {
            div()
                .id(gpui::SharedString::from(id.clone()))
                .px_2()
                .py_0p5()
                .rounded_sm()
                .border_1()
                .border_color(rgb(colour))
                .cursor_pointer()
                .text_color(rgb(colour))
                .hover(|s| s.bg(rgb(theme::SELECTED)))
                .child(label)
                .child(self.controls.mark(id))
        };
        let bring = button(accept_control(window), "Bring it here ↵", theme::PRIMARY).on_click(
            cx.listener(move |this, _, window_, cx| this.offer_send(Action::OfferAccept { window }, window_, cx)),
        );
        // ×: the pane closes; the window floats on.
        let close = button(format!("offer-close:{pane}"), "×", theme::MUTED)
            .on_click(cx.listener(move |this, _, window_, cx| this.close_pane(pane, window_, cx)));
        let buttons = div().flex().gap_2().child(bring).child(close);
        let body = div().flex().flex_col().items_center().gap_2();
        let body = body.child(div().text_sm().text_color(rgb(theme::TEXT)).child(said)).children(title);
        Some(body.child(buttons).into_any_element())
    }

    /// Sends an offer's action as whoever is acting; refused, the status bar says why.
    pub(crate) fn offer_send(&mut self, action: Action, window: &Window, cx: &mut Context<Self>) {
        if let Some(why) = self.model.refusal(&action, self.input) {
            self.error = Some(why.into());
            return cx.notify();
        }
        self.dispatch(action, self.input, window, cx);
        cx.notify();
    }

    /// A sneak label (or a click) on the chip. False: not one of its controls.
    pub(crate) fn click_offer(&mut self, kind: &str, name: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        let Ok(id) = name.parse::<u32>() else { return false };
        match kind {
            "offer-close" => self.close_pane(id, window, cx),
            "offer" => self.offer_send(Action::OfferAccept { window: id }, window, cx),
            "offer-dismiss" => self.offer_send(Action::OfferDismiss { window: id }, window, cx),
            _ => return false,
        }
        true
    }

    /// The user took the offer: window `id` of process `pid` in a new pane beside `pane`. A test
    /// run's fixture window has no window to host: a pane with none, as `app_fixture` makes.
    pub(crate) fn host_offer(&mut self, pane: PaneId, pid: i32, id: u32, window: &Window, cx: &mut Context<Self>) {
        // Already in a pane: a second click while the first was hosting.
        if self.model.panes.iter().any(|p| matches!(p.kind, Kind::App { window, .. } if window == id)) {
            return;
        }
        if let Some(title) = self.offer_fixtures.remove(&id) {
            // The offer's app: its pane may be one that waits, empty.
            let Some(name) = self.model.offers.iter().find(|o| o.window == id).map(|o| o.app.1.clone()) else {
                return;
            };
            let kind = Kind::App { window: id, title, bundle: None, name };
            self.dispatch(self.offer_into(pane, kind), self.input, window, cx);
            return;
        }
        let found = ottospace_mac::ax::Window::all_of(pid).into_iter().find(|w| w.id() == Some(id));
        match found
            .ok_or_else(|| "the window is gone".to_string())
            .and_then(|w| capture::target(w).and_then(Hosted::new))
        {
            Ok(hosted) => {
                let action = self.offer_into(pane, hosted.kind());
                self.attach(hosted, action, window, cx);
            }
            Err(why) => self.broke(format!("could not put the window in a pane: {why}")),
        }
    }

    /// Where an accepted window goes: into the empty pane that waits for it, else beside its app's.
    fn offer_into(&self, pane: PaneId, kind: Kind) -> Action {
        match self.model.offer_waits(pane) {
            true => Action::Fill { pane, kind },
            false => Action::Adopt { kind, beside: Some(pane), seen: None },
        }
    }

    /// Offers of windows that closed go (every tick). A fixture's stays until the test closes it.
    pub(crate) fn drop_gone_offers(&mut self, window: &Window, cx: &mut Context<Self>) {
        let gone: Vec<u32> = (self.model.offers.iter())
            .map(|offer| offer.window)
            .filter(|id| !self.offer_fixtures.contains_key(id) && !crate::recovery::window_exists(*id))
            .collect();
        for id in gone {
            self.dispatch(Action::OfferGone { window: id }, Origin::System, window, cx);
        }
    }

    /// `window_fixture`: a new window of app pane `pane`'s app, as if just seen (a test run hosts
    /// none). Judged, logged and offered as a real one; its id is returned in the reply's offers.
    pub(crate) fn window_fixture(
        &mut self,
        new: Fixture,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let Fixture { pane, title, subrole, size, modal } = new;
        let Some(Kind::App { bundle, name, .. }) = self.model.pane(pane).map(|p| p.kind.clone()) else {
            return Err(format!("pane {pane} is no app pane"));
        };
        let facts =
            rules::Facts { app: &name, bundle: bundle.as_deref(), title: &title, subrole: &subrole, identifier: None };
        let pane_size = self.cells.get(&pane).map(|b| b.get().size).map(|s| [f64::from(s.width), f64::from(s.height)]);
        let shape = rules::Shape { modal, size: Some(size), pane: pane_size, ..Default::default() };
        let decided = rules::decide(&crate::config::live().windows, &facts, &shape);
        log(&Seen { bundle: bundle.as_deref(), subrole: &subrole }, &shape, &decided);
        self.next_fixture_window += 1;
        let id = 800_000 + self.next_fixture_window;
        match decided.place {
            rules::Place::Float { offer: true } => {
                self.offer_fixtures.insert(id, title.clone());
                let action = Action::WindowOffered { pane, pid: 0, window: id, title, why: decided.reason() };
                self.dispatch(action, Origin::System, window, cx);
            }
            // A `split` rule's: a pane with no window, as `app_fixture` makes.
            rules::Place::Split => {
                let kind = Kind::App { window: id, title, bundle, name };
                self.dispatch(Action::Adopt { kind, beside: Some(pane), seen: None }, Origin::System, window, cx);
            }
            rules::Place::Float { offer: false } | rules::Place::Leave => {}
        }
        Ok(())
    }
}
