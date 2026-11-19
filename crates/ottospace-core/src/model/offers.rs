//! A new window of a hosted app never splits a pane by itself (`rules::place`): it floats, and its
//! app's pane offers it (`Zoom opened “Chat” · ◫ ⊟ ×`: split right, split down, dismiss). Several
//! group in one chip (`Zoom opened 3 windows ▾`), whose list holds each. Only the user takes one.
//! Its title is drawn in the chip, never in the state while offered; once the user puts it in a
//! pane it is an app pane, whose title the state carries as every app pane's.
//! An app pane whose window closes waits, empty, for another window of its app when the window
//! was shared (one it found open); the next new window of that app is offered in its body
//! (`Bring it here ↵ ×`), several such panes in reading order. A pane whose window it asked for
//! (owned) closes with it. No delay: the owner's call, 2026-11-19, "explicit, not magical".

use serde::Serialize;

use super::{Action, Effect, Kind, Model, Pane, PaneId};
use crate::layout::Axis;

/// Only the user puts a window an app opened by itself into a pane, or waves the offer away.
pub const USERS_ALONE: &str = "an offered window is the user's: only they put it in a pane or dismiss the offer";
/// Only ottospace says a window opened or closed.
pub const SHELLS_ALONE: &str = "only ottospace reports a new window";

/// A window offered by its app's pane. In the state: the pane and the window id, never its title.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Offer {
    pub pane: PaneId,
    pub window: u32,
    #[serde(skip)]
    pub pid: i32,
    /// Drawn in the chip, never sent on the socket.
    #[serde(skip)]
    pub title: String,
    /// Why it is in no pane, in plain words: `no slot for this window`.
    #[serde(skip)]
    pub why: String,
    /// Its app, as the pane that first offered it names it: the offer moves to another of its panes.
    #[serde(skip)]
    pub app: (Option<String>, String),
    /// The way the user chose to split it in, once they took it: kept until it is in a pane.
    #[serde(skip)]
    pub split: Option<Axis>,
}

/// Where a window that would be offered went: the log's line says so (`Effect::Placed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placed {
    /// Its app's pane offers it: no slot selects it.
    Offer,
    /// Slot `slot` offers it (Manual), or takes it at its next look (Automatic).
    SlotOffer { slot: PaneId },
    /// Slot `slot`, Away, takes its window back.
    SlotReturn { slot: PaneId },
}

impl Placed {
    /// For the log: `offer`, `slot_offer`, `slot_return`.
    pub fn name(self) -> &'static str {
        match self {
            Placed::Offer => "offer",
            Placed::SlotOffer { .. } => "slot_offer",
            Placed::SlotReturn { .. } => "slot_return",
        }
    }

    pub fn slot(self) -> Option<PaneId> {
        match self {
            Placed::Offer => None,
            Placed::SlotOffer { slot } | Placed::SlotReturn { slot } => Some(slot),
        }
    }
}

impl Model {
    /// The newest offer of app pane `pane`, the one its header shows.
    pub fn offer_of(&self, pane: PaneId) -> Option<&Offer> {
        self.offers.iter().rev().find(|offer| offer.pane == pane)
    }

    /// Every offer of app pane `pane`, newest first: more than one, its chip groups them.
    pub fn offers_at(&self, pane: PaneId) -> Vec<&Offer> {
        self.offers.iter().rev().filter(|offer| offer.pane == pane).collect()
    }

    /// True if `pane` is an empty pane that waits for a window of its app and offers one now.
    pub fn offer_waits(&self, pane: PaneId) -> bool {
        self.offer_panes.contains_key(&pane) && self.offer_of(pane).is_some()
    }

    /// The app empty pane `pane` waits for a window of, since its shared window closed.
    pub fn waits_for(&self, pane: PaneId) -> Option<&(Option<String>, String)> {
        self.offer_panes.get(&pane)
    }

    /// The first pane on show, in reading order, that waits for a window of `app` and offers none.
    pub fn waiting_pane(&self, app: (Option<&str>, &str)) -> Option<PaneId> {
        let shown = self.layout.as_ref().map(super::Node::leaves).unwrap_or_default();
        shown.into_iter().find(|id| {
            self.offer_of(*id).is_none()
                && self.offer_panes.get(id).is_some_and(|(b, n)| crate::marks::same_app((b.as_deref(), n), app))
        })
    }

    /// The window ↵ brings in: the one the focused waiting pane offers, while ottospace has the
    /// keyboard in normal mode.
    pub fn offer_enter(&self) -> Option<u32> {
        let id = self.focused.filter(|_| self.mode() == super::Mode::Normal && !self.keys)?;
        self.offer_of(id).filter(|_| self.offer_waits(id)).map(|offer| offer.window)
    }

    /// App pane `pane`'s shared window is gone while its app runs: the pane stays, empty, and
    /// waits for another window of the app (one offered there already shows at once). False: it
    /// closes instead, its window being owned (asked for, it closes with its pane) or its app gone.
    pub(super) fn offer_keeps(&mut self, pane: PaneId, app_running: bool, effects: &mut Vec<Effect>) -> bool {
        let Some(Pane { kind: Kind::App { bundle, name, .. }, owned: false, .. }) = self.pane(pane) else {
            return false;
        };
        if !app_running {
            return false;
        }
        let app = (bundle.clone(), name.clone());
        if self.live == Some(pane) {
            self.leave(effects);
        }
        if self.listen.pane() == Some(pane) {
            self.listen_end(None, None, effects);
        }
        if let Some(p) = self.panes.iter_mut().find(|p| p.id == pane) {
            (p.kind, p.fit) = (Kind::Empty, None);
        }
        self.offer_panes.insert(pane, app);
        true
    }

    pub(super) fn offer_act(&mut self, action: Action, effects: &mut Vec<Effect>) {
        match action {
            // The offer stays until its window is in a pane (`settle_offers`): a host that failed
            // (a busy app) can be tried again.
            Action::OfferAccept { window, axis } => {
                if let Some(offer) = self.offers.iter_mut().find(|offer| offer.window == window) {
                    offer.split = Some(axis);
                    effects.push(Effect::HostOffer { pane: offer.pane, pid: offer.pid, window });
                }
            }
            Action::OfferDismiss { window } => self.offers.retain(|offer| offer.window != window),
            // Only a chip that groups offers has a list; ▾ again closes it.
            Action::OfferList { pane } => {
                let open = pane.filter(|id| self.offer_list != Some(*id) && self.offers_at(*id).len() > 1);
                self.offer_list = open;
            }
            // The shell's alone from here: a key or click of the user's reports no window.
            _ if self.by_user => {}
            Action::WindowOffered { pane, seen, why } => {
                let app = match self.pane(pane).map(|p| p.kind.clone()) {
                    Some(Kind::App { bundle, name, .. }) => (bundle, name),
                    // A pane that waits for a window of its app: no app pane of it is left.
                    Some(Kind::Empty) if let Some(app) = self.waits_for(pane) => app.clone(),
                    _ => return,
                };
                let (bundle, name) = app;
                // A slot comes first: its own window back, or one it offers (`docs/ai/slots.md`).
                let window = seen.window;
                let placed = match self.slot_new_window((bundle.as_deref(), &name), &seen, effects) {
                    Some((slot, true)) => Placed::SlotReturn { slot },
                    Some((slot, false)) => Placed::SlotOffer { slot },
                    None => {
                        self.offers.retain(|offer| offer.window != window);
                        let crate::slot::Seen { pid, title, .. } = seen;
                        // A pane that waits for a window of its app takes it in its body, the first in reading order.
                        let pane = self.waiting_pane((bundle.as_deref(), &name)).unwrap_or(pane);
                        let app = (bundle.clone(), name.clone());
                        self.offers.push(Offer { pane, window, pid, title, why, app, split: None });
                        Placed::Offer
                    }
                };
                effects.push(Effect::Placed { window, placed });
            }
            Action::OfferGone { window } => self.offers.retain(|offer| offer.window != window),
            _ => {}
        }
    }

    /// After every step: an offer lasts while its window is in no pane. One whose pane closed, or
    /// holds another app now, moves to another pane of its app; with none left, it goes (the
    /// window floats on). A waiting pane is one while it is empty and offers something.
    pub(super) fn settle_offers(&mut self) {
        let panes = &self.panes;
        self.offer_panes.retain(|id, _| panes.iter().any(|p| p.id == *id && p.kind == Kind::Empty));
        let held = |w: u32| panes.iter().any(|p| matches!(p.kind, Kind::App { window, .. } if window == w));
        let of_app = |id: PaneId, app: &(Option<String>, String)| {
            panes.iter().any(|p| {
                p.id == id
                    && matches!(&p.kind, Kind::App { bundle, name, .. }
                        if crate::marks::same_app((bundle.as_deref(), name), (app.0.as_deref(), &app.1)))
            })
        };
        let waiting = &self.offer_panes;
        let mut offers = std::mem::take(&mut self.offers);
        offers.retain_mut(|offer| {
            if held(offer.window) {
                return false;
            }
            if of_app(offer.pane, &offer.app) || waiting.contains_key(&offer.pane) {
                return true;
            }
            // Its pane went: the newest other pane of its app offers it.
            let other = panes.iter().rev().find(|p| of_app(p.id, &offer.app)).map(|p| p.id);
            other.map(|id| offer.pane = id).is_some()
        });
        self.offers = offers;
        // A list lasts while its chip groups offers.
        self.offer_list = self.offer_list.filter(|id| self.offers_at(*id).len() > 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Axis::{Down, Right};
    use crate::layout::Node;
    use crate::model::Origin;

    fn app(window: u32) -> Kind {
        Kind::App { window, title: "Main".into(), bundle: Some("us.zoom.xos".into()), name: "zoom.us".into() }
    }

    /// Window `window` of process 7, a standard one titled `title`, opened beside pane `pane`.
    fn new_window(pane: PaneId, window: u32, title: &str) -> Action {
        let subrole = "AXStandardWindow".into();
        let seen = crate::slot::Seen { pid: 7, window, title: title.into(), subrole, ..Default::default() };
        Action::WindowOffered { pane, seen, why: "no slot".into() }
    }

    /// Zoom's pane 1, and window 9 it opened by itself, offered.
    fn offered() -> Model {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        let offer = new_window(1, 9, "Zoom SECRET Meeting");
        m.update(offer.sent_by(Origin::System));
        m
    }

    #[test]
    fn a_new_window_is_offered_and_split_in_only_by_the_users_accept() {
        let mut m = offered();
        assert_eq!(m.offer_of(1).map(|offer| offer.window), Some(9));
        assert_eq!(m.panes.len(), 1, "no split by itself");
        // A client's accept or dismiss is refused, with a reason, and does nothing.
        for action in [Action::OfferAccept { window: 9, axis: Right }, Action::OfferDismiss { window: 9 }] {
            for origin in [Origin::Socket, Origin::System] {
                assert_eq!(m.refusal(&action, origin), Some(USERS_ALONE));
                assert!(m.update(action.clone().sent_by(origin)).is_empty());
            }
        }
        assert_eq!(m.offers.len(), 1);
        // The user's asks the shell to host it beside its app's pane; the offer goes.
        let effects = m.update(Action::OfferAccept { window: 9, axis: Right });
        assert!(effects.contains(&Effect::HostOffer { pane: 1, pid: 7, window: 9 }), "{effects:?}");
        // Kept until the window is in a pane: a failed host can be tried again.
        assert_eq!(m.offers.len(), 1);
        m.update(Action::Adopt { kind: app(9), beside: Some(1), seen: None }.sent_by(Origin::System));
        assert!(m.offers.is_empty());
    }

    #[test]
    fn an_offer_goes_when_dismissed_closed_hosted_or_its_pane_closes() {
        let mut m = offered();
        m.update(Action::OfferDismiss { window: 9 });
        assert!(m.offers.is_empty(), "dismissed: it stays floating");
        let mut m = offered();
        m.update(Action::OfferGone { window: 9 }.sent_by(Origin::System));
        assert!(m.offers.is_empty(), "its window closed");
        // Put in a pane some other way: in a pane, it is offered no more.
        let mut m = offered();
        m.update(Action::Open { kind: app(9), axis: Right, beside: None });
        assert!(m.offers.is_empty());
        let mut m = offered();
        m.update(Action::Close { pane: Some(1) });
        assert!(m.offers.is_empty());
    }

    #[test]
    fn only_the_shell_offers_and_the_state_never_holds_the_title() {
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        let offer = new_window(1, 9, "x");
        assert_eq!(m.refusal(&offer, Origin::Socket), Some(SHELLS_ALONE));
        assert_eq!(m.refusal(&Action::OfferGone { window: 9 }, Origin::Socket), Some(SHELLS_ALONE));
        m.update(offer.clone());
        assert!(m.offers.is_empty(), "the user's key reports no window");
        // Of a pane that is no app pane: none.
        m.update(new_window(5, 9, "x").sent_by(Origin::System));
        assert!(m.offers.is_empty());
        let m = offered();
        let state = serde_json::to_string(&m).unwrap_or_default();
        assert!(state.contains(r#""offers":[{"pane":1,"window":9}]"#), "{state}");
        assert!(!state.contains("SECRET"), "{state}");
    }

    #[test]
    fn a_window_offered_again_is_offered_once() {
        let mut m = offered();
        m.update(new_window(1, 9, "b").sent_by(Origin::System));
        m.update(new_window(1, 10, "c").sent_by(Origin::System));
        assert_eq!(m.offers.iter().map(|o| o.window).collect::<Vec<_>>(), [9, 10]);
        assert_eq!(m.offer_of(1).map(|o| o.title.as_str()), Some("c"), "the newest shows");
    }

    /// The app remade its window (closed it, opened another): the pane stays, empty, and offers
    /// the new one. Only the user's Bring it here (↵) fills it; × closes the pane.
    #[test]
    fn a_pane_whose_window_the_app_remade_waits_empty_and_offers_the_new_one() {
        let mut m = offered();
        m.update(Action::WindowGone { pane: 1, app_running: true, at: 0 }.sent_by(Origin::System));
        assert_eq!(m.pane(1).map(|p| p.kind.clone()), Some(Kind::Empty), "kept, empty");
        assert!(m.offer_waits(1) && m.offer_of(1).is_some_and(|o| o.window == 9));
        m.update(Action::Focus { pane: 1 });
        assert_eq!(m.offer_enter(), Some(9));
        // Bring it here asks the shell to host it in that pane.
        let effects = m.update(Action::OfferAccept { window: 9, axis: Right });
        assert!(effects.contains(&Effect::HostOffer { pane: 1, pid: 7, window: 9 }), "{effects:?}");
        m.update(Action::Fill { pane: 1, kind: app(9) }.sent_by(Origin::System));
        assert!(m.offers.is_empty() && !m.offer_waits(1));
        // ×: the pane closes, the window floats on.
        let mut m = offered();
        m.update(Action::WindowGone { pane: 1, app_running: true, at: 0 }.sent_by(Origin::System));
        m.update(Action::Close { pane: Some(1) });
        assert!(m.pane(1).is_none() && m.offers.is_empty());
    }

    /// The owner's call: a shared window's pane waits for another of its app; an owned one's
    /// closes; one whose app quit closes. No delay. The next window is offered in the waiting
    /// pane's body, two waiting panes in reading order.
    #[test]
    fn a_shared_windows_pane_waits_an_owned_ones_closes() {
        let gone = |pane, app_running| Action::WindowGone { pane, app_running, at: 0 }.sent_by(Origin::System);
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(gone(1, true));
        assert_eq!(m.pane(1).map(|p| p.kind.clone()), Some(Kind::Empty), "kept, empty");
        assert_eq!(m.waits_for(1).map(|app| app.1.as_str()), Some("zoom.us"));
        assert!(!m.offer_waits(1), "nothing to offer yet");
        // Its app's next window, reported for the waiting pane (no app pane is left), is offered there.
        m.update(new_window(1, 9, "SECRET chat").sent_by(Origin::System));
        assert!(m.offer_waits(1) && m.offer_of(1).is_some_and(|o| o.window == 9));
        // Dismissed, the pane still waits.
        m.update(Action::OfferDismiss { window: 9 });
        assert_eq!(m.waits_for(1).map(|app| app.1.as_str()), Some("zoom.us"));
        // Owned: it closes with its window.
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.panes[0].owned = true;
        m.update(gone(1, true));
        assert!(m.pane(1).is_none());
        // Its app quit: it closes.
        let mut m = Model::default();
        m.update(Action::Open { kind: app(1), axis: Right, beside: None });
        m.update(gone(1, false));
        assert!(m.pane(1).is_none());
        // Two waiting panes and the app's pane: new windows fill the waiting ones in reading order.
        let mut m = Model::default();
        for window in [1, 2, 3] {
            m.update(Action::Open { kind: app(window), axis: Right, beside: None });
        }
        m.update(gone(3, true));
        m.update(gone(2, true));
        m.update(new_window(1, 9, "a").sent_by(Origin::System));
        m.update(new_window(1, 10, "b").sent_by(Origin::System));
        m.update(new_window(1, 11, "c").sent_by(Origin::System));
        let at: Vec<(PaneId, u32)> = m.offers.iter().map(|o| (o.pane, o.window)).collect();
        assert_eq!(at, [(2, 9), (3, 10), (1, 11)], "waiting panes first, in reading order; then the chip");
        // Bring it here fills the waiting pane.
        m.update(Action::OfferAccept { window: 9, axis: Right });
        m.update(Action::Fill { pane: 2, kind: app(9) }.sent_by(Origin::System));
        assert!(m.waits_for(2).is_none() && m.pane(2).is_some_and(|p| p.kind == app(9)));
    }

    /// No offer vanishes while its app has a pane: one whose pane closes moves to another of its app.
    #[test]
    fn an_offer_whose_pane_closes_moves_to_another_pane_of_its_app() {
        let mut m = offered();
        m.update(Action::Open { kind: app(2), axis: Right, beside: None });
        m.update(Action::Close { pane: Some(1) });
        assert_eq!(m.offers.iter().map(|o| (o.pane, o.window)).collect::<Vec<_>>(), [(2, 9)]);
        // Another app in its place takes no offer: it moves on, or goes.
        let mut m = offered();
        let other = Kind::App { window: 3, title: "x".into(), bundle: None, name: "Slack".into() };
        m.update(Action::Close { pane: Some(1) });
        m.update(Action::Open { kind: other, axis: Right, beside: None });
        assert!(m.offers.is_empty());
    }

    /// The owner's call: each offer splits right or down, as the user picks; a client's pick is refused.
    #[test]
    fn an_offer_splits_the_way_the_user_picks() {
        let mut m = offered();
        let down = Action::OfferAccept { window: 9, axis: Down };
        assert_eq!(m.refusal(&down, Origin::Socket), Some(USERS_ALONE));
        assert!(m.update(down.clone()).contains(&Effect::HostOffer { pane: 1, pid: 7, window: 9 }));
        m.update(Action::Adopt { kind: app(9), beside: Some(1), seen: None }.sent_by(Origin::System));
        let below = Node::Split { axis: Down, ratio: 0.5, a: Box::new(Node::Leaf(1)), b: Box::new(Node::Leaf(2)) };
        assert_eq!(m.layout, Some(below), "split down, as picked");
        assert_eq!(m.focused, Some(1), "the focus stays where it was");
        // Untaken, a window the app opened splits right, as before (a `split` rule's).
        let mut m = offered();
        m.update(Action::Adopt { kind: app(10), beside: Some(1), seen: None }.sent_by(Origin::System));
        assert!(matches!(m.layout, Some(Node::Split { axis: Right, .. })), "{:?}", m.layout);
    }

    /// Several offers of one pane group in its chip; ▾ (the user's) opens their list, which lasts
    /// while more than one is left.
    #[test]
    fn several_offers_group_and_their_list_opens_only_for_the_user() {
        let mut m = offered();
        m.update(Action::OfferList { pane: Some(1) });
        assert_eq!(m.offer_list, None, "one offer: no list");
        m.update(new_window(1, 10, "b").sent_by(Origin::System));
        m.update(new_window(1, 11, "c").sent_by(Origin::System));
        assert_eq!(m.offers_at(1).iter().map(|o| o.window).collect::<Vec<_>>(), [11, 10, 9], "newest first");
        let list = Action::OfferList { pane: Some(1) };
        assert_eq!(m.refusal(&list, Origin::Socket), Some(USERS_ALONE));
        m.update(list.clone().sent_by(Origin::Socket));
        assert_eq!(m.offer_list, None);
        m.update(list.clone());
        assert_eq!(m.offer_list, Some(1));
        let state = serde_json::to_string(&m).unwrap_or_default();
        assert!(state.contains(r#""offer_list":1"#) && !state.contains("SECRET"), "{state}");
        // One dismissed: two left, still open. Another: one left, it closes.
        m.update(Action::OfferDismiss { window: 10 });
        assert_eq!(m.offer_list, Some(1));
        m.update(Action::OfferGone { window: 11 }.sent_by(Origin::System));
        assert_eq!(m.offer_list, None);
        // ▾ again closes it.
        m.update(new_window(1, 12, "d").sent_by(Origin::System));
        m.update(list.clone());
        m.update(list);
        assert_eq!(m.offer_list, None);
    }
}
