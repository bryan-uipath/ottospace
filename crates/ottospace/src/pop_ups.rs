//! A new window of a hosted app: where it went, and why, in the log; and the chip its app's pane
//! shows while it floats (`Zoom opened “Zoom Meeting” · no slot for this window · ◫ split right ⊟ split down ×`).
//! The core decides (`rules::decide`) and keeps the offer (`Model::offers`); this draws and hosts.

use gpui::{
    AnyElement, AppContext, Context, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, rgb,
};
use ottospace_core::layout::Axis;
use ottospace_core::model::offers::Offer;
use ottospace_core::model::offers::Placed;
use ottospace_core::model::{Action, Effect, Kind, Origin, PaneId};
use ottospace_core::{rules, theme};

use ottospace_core::slot::Seen as SlotSeen;

use crate::{Hosted, Mark, Marks, Workspace, capture, cut_text, event_log, mark};

/// What a new window was, for the log line. Never its title.
pub(crate) struct Seen<'a> {
    pub(crate) bundle: Option<&'a str>,
    pub(crate) subrole: &'a str,
}

/// One line in `<home>/log` a placement: the app, the window's kind and size, its pane's size,
/// where it went and why (the rule, if one decided; the slot, if one took or offers it). Never the
/// title. `placed`: what the core made of an offer (`Effect::Placed`), so the log says what the
/// pane shows.
pub(crate) fn log(seen: &Seen, shape: &rules::Shape, decided: &rules::Decision, placed: Option<Placed>) {
    let place = match (decided.place, placed) {
        (_, Some(placed)) => placed.name(),
        (rules::Place::Split, _) => "split",
        (rules::Place::Float { offer: true }, _) => "offer",
        (rules::Place::Float { offer: false }, _) => "float",
        (rules::Place::Leave, _) => "leave",
    };
    let why = match placed.and_then(Placed::slot) {
        Some(_) => "slot",
        None => decided.why.name(),
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
            "why": why,
            "rule": rule,
            "slot": placed.and_then(Placed::slot),
        }),
    );
}

/// One offer in words: `“Chat” · no slot for this window`.
fn offer_words(offer: &Offer) -> String {
    match offer.title.is_empty() {
        true => format!("a window · {}", offer.why),
        false => format!("“{}” · {}", offer.title, offer.why),
    }
}

/// An offer chip's tooltip: its whole line, wrapped past the offer list's width. Where it is
/// drawn is noted for a test run.
struct OfferTip(gpui::SharedString, Marks);

impl gpui::Render for OfferTip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let tip = crate::ui::tooltip(self.0.clone(), crate::ui::Tip::Hint).max_w(gpui::px(420.));
        tip.relative().child(mark(&self.1, Mark::OfferTip))
    }
}

/// A `window_fixture` request's window.
pub(crate) struct Fixture {
    pub(crate) pane: PaneId,
    pub(crate) title: String,
    pub(crate) identifier: Option<String>,
    pub(crate) subrole: String,
    pub(crate) size: [f64; 2],
    pub(crate) modal: bool,
}

/// The chip's controls, for clicks and sneak labels: `offer:<window>` (split right, or Bring it
/// here in a pane that waits), `offer-down:<window>`, `offer-dismiss:<window>`, `offer-list:<pane>`.
fn accept_control(window: u32) -> String {
    format!("offer:{window}")
}

fn down_control(window: u32) -> String {
    format!("offer-down:{window}")
}

fn dismiss_control(window: u32) -> String {
    format!("offer-dismiss:{window}")
}

fn list_control(pane: PaneId) -> String {
    format!("offer-list:{pane}")
}

impl Workspace {
    /// One of the chip's small buttons.
    fn offer_button(&self, id: String, label: impl Into<gpui::SharedString>, colour: u32) -> gpui::Stateful<gpui::Div> {
        div()
            .id(gpui::SharedString::from(id.clone()))
            .flex_none()
            .px_1()
            .rounded_sm()
            .cursor_pointer()
            .text_color(rgb(colour))
            .hover(|s| s.bg(rgb(theme::SELECTED)))
            .child(label.into())
            .child(self.controls.mark(id))
            // The press must not reach the header or the pane: an unsteady click would start a drag.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    }

    /// An offer's three choices: ◫ split right, ⊟ split down, ×.
    fn offer_choices(&self, window: u32, cx: &mut Context<Self>) -> [AnyElement; 3] {
        let send = |action: Action| {
            cx.listener(move |this: &mut Self, _: &gpui::ClickEvent, window_: &mut Window, cx| {
                this.offer_send(action.clone(), window_, cx)
            })
        };
        let right = Action::OfferAccept { window, axis: Axis::Right };
        let down = Action::OfferAccept { window, axis: Axis::Down };
        [
            self.offer_button(accept_control(window), "◫ split right", theme::PRIMARY).on_click(send(right)),
            self.offer_button(down_control(window), "⊟ split down", theme::PRIMARY).on_click(send(down)),
            self.offer_button(dismiss_control(window), "×", theme::MUTED)
                .on_click(send(Action::OfferDismiss { window })),
        ]
        .map(IntoElement::into_any_element)
    }

    /// The chip in app pane `pane`'s header while windows its app opened float: one offer's
    /// title, why it is in no pane and its three choices; several, `Zoom opened 3 windows ▾`,
    /// whose list (`offer_list`) holds each. None with no offer.
    pub(crate) fn offer_header(&self, pane: PaneId, cx: &mut Context<Self>) -> Option<AnyElement> {
        let offers = self.model.offers_at(pane);
        let offer = offers.first()?;
        let app = match self.model.pane(pane).map(|p| &p.kind) {
            Some(Kind::App { name, .. }) => name.clone(),
            _ => return None,
        };
        let chip = div()
            .flex()
            .min_w_0()
            .items_center()
            .gap_1()
            .px_1()
            .rounded_sm()
            .border_1()
            .border_color(rgb(theme::BORDER_STRONG));
        if offers.len() > 1 {
            let open = self.model.offer_list == Some(pane);
            let what = format!("{app} opened {} windows {}", offers.len(), if open { "▴" } else { "▾" });
            let button = self.offer_button(list_control(pane), what, theme::TEXT).on_click(
                cx.listener(move |this, _, window_, cx| this.offer_send(Action::OfferList { pane }, window_, cx)),
            );
            return Some(chip.child(button).into_any_element());
        }
        let what = format!("{app} opened {}", offer_words(offer));
        // Hover: the whole line, cut short in a narrow pane. Not while a window is live: it would
        // draw under that window.
        let words = cut_text(what.clone())
            .id(gpui::SharedString::from(format!("offer-words:{pane}")))
            .text_color(rgb(theme::MUTED))
            .child(mark(&self.marks, Mark::OfferWords(pane)))
            .when(self.model.live.is_none(), |words| {
                let marks = self.marks.clone();
                words
                    .tooltip(move |_, cx| gpui::AnyView::from(cx.new(|_| OfferTip(what.clone().into(), marks.clone()))))
            });
        let chip = chip.child(words);
        Some(chip.children(self.offer_choices(offer.window, cx)).into_any_element())
    }

    /// The open list of app pane `pane`'s grouped offers, over the top right of its body: each
    /// window, why it is in no pane, and its three choices.
    pub(crate) fn offer_list(&self, pane: PaneId, cx: &mut Context<Self>) -> Option<AnyElement> {
        (self.model.offer_list == Some(pane)).then_some(())?;
        let offers = self.model.offers_at(pane);
        let rows = offers.iter().map(|offer| {
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(cut_text(offer_words(offer)).flex_1().text_color(rgb(theme::TEXT)))
                .children(self.offer_choices(offer.window, cx))
        });
        let list = div()
            .id(gpui::SharedString::from(format!("offer-list-open:{pane}")))
            .absolute()
            .top_1()
            .right_1()
            .max_w(gpui::px(420.))
            .p_1()
            .flex()
            .flex_col()
            .gap_1()
            .rounded_sm()
            .border_1()
            .border_color(rgb(theme::BORDER_STRONG))
            .bg(rgb(theme::RAISED))
            .text_xs()
            .occlude()
            // A press here is the list's, not the pane's: it would enter the app.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(rows);
        Some(list.into_any_element())
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
        let bring = button(accept_control(window), "Bring it here ↵", theme::PRIMARY).on_click(cx.listener(
            move |this, _, window_, cx| this.offer_send(Action::OfferAccept { window, axis: Axis::Right }, window_, cx),
        ));
        // ×: the pane closes; the window floats on.
        let close = button(format!("offer-close:{pane}"), "×", theme::MUTED)
            .on_click(cx.listener(move |this, _, window_, cx| this.close_pane(pane, window_, cx)));
        let buttons = div().flex().gap_2().child(bring).child(close);
        let body = div().flex().flex_col().items_center().gap_2();
        let body = body.child(div().text_sm().text_color(rgb(theme::TEXT)).child(said)).children(title);
        Some(body.child(buttons).into_any_element())
    }

    /// Sends an offer's (or a slot's) action as whoever is acting; refused, the status bar says why.
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
            "offer" => self.offer_send(Action::OfferAccept { window: id, axis: Axis::Right }, window, cx),
            "offer-down" => self.offer_send(Action::OfferAccept { window: id, axis: Axis::Down }, window, cx),
            "offer-dismiss" => self.offer_send(Action::OfferDismiss { window: id }, window, cx),
            "offer-list" => self.offer_send(Action::OfferList { pane: id }, window, cx),
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

    /// A new window that would float with an offer: the core gives it to a slot or to its app
    /// pane's chip (`Action::WindowOffered`), and the log's line says which.
    pub(crate) fn offer_window(
        &mut self,
        pane: PaneId,
        seen: SlotSeen,
        (logged, shape, decided): (&Seen, &rules::Shape, &rules::Decision),
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        // The core's look gains the window: the next look is sent, not taken for one already sent.
        self.slot_seen.clear();
        let action = Action::WindowOffered { pane, seen, why: decided.reason() };
        let effects = self.dispatch(action, Origin::System, window, cx);
        let placed = effects.iter().find_map(|effect| match effect {
            Effect::Placed { placed, .. } => Some(*placed),
            _ => None,
        });
        log(logged, shape, decided, placed);
    }

    /// `window_fixture`: a new window of app pane `pane`'s app, as if just seen (a test run hosts
    /// none). Judged, logged and offered as a real one; its id is returned in the reply's offers.
    pub(crate) fn window_fixture(
        &mut self,
        new: Fixture,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let Fixture { pane, title, identifier, subrole, size, modal } = new;
        let Some(Kind::App { bundle, name, .. }) = self.model.pane(pane).map(|p| p.kind.clone()) else {
            return Err(format!("pane {pane} is no app pane"));
        };
        let facts = rules::Facts {
            app: &name,
            bundle: bundle.as_deref(),
            title: &title,
            subrole: &subrole,
            identifier: identifier.as_deref(),
        };
        let pane_size = self.cells.get(&pane).map(|b| b.get().size).map(|s| [f64::from(s.width), f64::from(s.height)]);
        let shape = rules::Shape { modal, size: Some(size), pane: pane_size, ..Default::default() };
        let decided = rules::decide(&crate::config::live().windows, &facts, &shape);
        let logged = Seen { bundle: bundle.as_deref(), subrole: &subrole };
        self.next_fixture_window += 1;
        let id = 800_000 + self.next_fixture_window;
        match decided.place {
            rules::Place::Float { offer: true } => {
                self.offer_fixtures.insert(id, title.clone());
                let frame = ottospace_core::ax::Frame::new(0., 0., size[0], size[1]);
                let seen = SlotSeen { pid: 0, window: id, title, identifier, subrole: subrole.clone(), frame };
                self.offer_window(pane, seen, (&logged, &shape, &decided), window, cx);
            }
            // A `split` rule's: a pane with no window, as `app_fixture` makes.
            rules::Place::Split => {
                log(&logged, &shape, &decided, None);
                let kind = Kind::App { window: id, title, bundle, name };
                self.dispatch(Action::Adopt { kind, beside: Some(pane), seen: None }, Origin::System, window, cx);
            }
            rules::Place::Float { offer: false } | rules::Place::Leave => log(&logged, &shape, &decided, None),
        }
        Ok(())
    }
}
