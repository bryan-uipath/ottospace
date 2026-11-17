//! The model's side of slots (`crate::slot`): where each is in its life, which window it offers
//! or takes, and what the user's moves and the shell's looks do. Who may send each action is
//! `Model::refusal`; a slot is the user's (`docs/ai/slots.md` §6).

use std::collections::BTreeSet;

use serde::Serialize;

use super::{Action, Effect, Kind, Model, PaneId};
use crate::features::Feature;
use crate::marks::same_app;
use crate::slot::{self, Look, Matching, Seen, Slot, Status};

/// A slot as the model runs it. In the state: its status, how many windows match and its
/// matching. What it selects, the windows it offers and passes over: never.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Run {
    #[serde(skip)]
    pub slot: Slot,
    #[serde(skip)]
    pub bundle: Option<String>,
    #[serde(skip)]
    pub name: String,
    pub status: Status,
    /// The windows it could offer now, the one offered among them.
    pub matches: usize,
    pub matching: Matching,
    /// The window it offers.
    #[serde(skip)]
    pub offered: Option<u32>,
    /// The window it asked the shell to host (`Effect::HostSlot`), until `SlotHosted` says.
    #[serde(skip)]
    pub pending: Option<u32>,
    /// The process of its window, while filled and Away: a window of the same one comes back.
    #[serde(skip)]
    pub pid: Option<i32>,
    /// Windows passed over for this run of the app: Not this one, Give Back. By window id, never
    /// by title (a recurring meeting keeps its title); forgotten when the window closes.
    #[serde(skip)]
    pub declined: BTreeSet<u32>,
    /// Every window a look listed since the slot last began to wait. `None` until its first look:
    /// Automatic takes only a window first listed after it (§3).
    #[serde(skip)]
    pub known: Option<BTreeSet<u32>>,
    /// Since when it is Away.
    #[serde(skip)]
    pub away: Option<u64>,
}

impl Run {
    fn new(slot: Slot, bundle: Option<String>, name: String) -> Self {
        let matching = slot.matching;
        Run {
            slot,
            bundle,
            name,
            status: Status::Empty,
            matches: 0,
            matching,
            offered: None,
            pending: None,
            pid: None,
            declined: BTreeSet::new(),
            known: None,
            away: None,
        }
    }

    fn of(&self, look: &Look) -> bool {
        same_app((self.bundle.as_deref(), &self.name), (look.bundle.as_deref(), &look.name))
    }

    /// It waits again from now: windows open now are not new to it.
    fn waits(&mut self) {
        (self.offered, self.known) = (None, None);
    }

    /// Whether it takes `seen` without asking: its own process's window back while Away (Manual
    /// too), or for Automatic a window new to it that the user is not in (`frontmost`).
    fn takes(&self, seen: &Seen, frontmost: Option<u32>) -> bool {
        let new = self.known.as_ref().is_some_and(|known| !known.contains(&seen.window));
        match (self.away, self.slot.matching) {
            (Some(_), _) => Some(seen.pid) == self.pid,
            (None, Matching::Automatic) => new && Some(seen.window) != frontmost,
            (None, Matching::Manual) => false,
        }
    }
}

impl Model {
    /// The window slot `pane` offers, as its look found it: for the pane to draw, never the state.
    pub fn slot_offer(&self, pane: PaneId) -> Option<&Seen> {
        let run = self.slots.get(&pane)?;
        let look = self.slot_looks.iter().find(|look| run.of(look))?;
        look.windows.iter().find(|seen| Some(seen.window) == run.offered)
    }

    /// True if a slot that waits selects `window`: a restored app pane passes over it
    /// (`marks::Window::slot`), or it would take the slot's window.
    pub fn slot_selects(&self, window: u32) -> bool {
        let waiting = self.slots.iter().filter(|(id, _)| !self.slot_filled(**id)).map(|(_, run)| run);
        waiting.into_iter().any(|run| {
            let look = self.slot_looks.iter().find(|look| run.of(look));
            look.is_some_and(|look| {
                look.candidates(&run.slot, &self.window_rules, |_| false).iter().any(|s| s.window == window)
            })
        })
    }

    fn slot_filled(&self, pane: PaneId) -> bool {
        self.pane(pane).is_some_and(|pane| matches!(pane.kind, Kind::App { .. }))
    }

    /// Whether a restored app pane of this app waits for a slot's first look on the tab on show,
    /// so it does not take the window the slot is for.
    pub(crate) fn waits_for_slot_look(&self, app: (Option<&str>, &str)) -> bool {
        let shown = self.layout.as_ref().map(super::Node::leaves).unwrap_or_default();
        self.slots.iter().any(|(id, run)| {
            shown.contains(id)
                && !self.slot_filled(*id)
                && run.known.is_none()
                && same_app((run.bundle.as_deref(), &run.name), app)
        })
    }

    /// Whether a client's action would take a slot out of the user's layout or move it.
    pub(super) fn moves_slot(&self, action: &Action) -> bool {
        let slot = |id: &PaneId| self.slots.contains_key(id);
        match action {
            Action::Close { pane } => pane.or(self.focused).as_ref().is_some_and(slot),
            Action::CloseTab { index } => {
                self.tab_panes().get(index.unwrap_or(self.active)).is_some_and(|panes| panes.iter().any(slot))
            }
            Action::SwapDir { dir } => {
                let to = self.focused.and_then(|from| self.layout.as_ref()?.neighbor(from, *dir));
                self.focused.as_ref().is_some_and(slot) || to.as_ref().is_some_and(slot)
            }
            Action::MovePane { pane, .. } => pane.or(self.focused).as_ref().is_some_and(slot),
            Action::DropPane { source, target, .. } => slot(source) || slot(target),
            _ => false,
        }
    }

    /// The record's `slot` of pane `id`, if it is one: what its record keeps beside its app.
    pub(crate) fn slot_record(&self, id: PaneId) -> Option<serde_json::Value> {
        let run = self.slots.get(&id).and_then(|run| serde_json::to_value(&run.slot).ok());
        run.or_else(|| self.slot_dormant.get(&id).cloned())
    }

    /// A restored pane's record names a slot: it starts waiting, never filled by an old window id.
    pub(crate) fn restore_slot(&mut self, id: PaneId, record: &crate::marks::RecordKind) -> bool {
        let crate::marks::RecordKind::App { bundle, name, extra, .. } = record else { return false };
        let Some(value) = extra.get("slot") else { return false };
        let slot = serde_json::from_value::<Slot>(value.clone()).ok();
        // With the flag off, or written by a newer build: a plain app pane that keeps it as it was.
        let Some(slot) = slot.filter(|_| self.features.on(Feature::Slots)) else {
            self.slot_dormant.insert(id, value.clone());
            return false;
        };
        let Some(pane) = self.panes.iter_mut().find(|pane| pane.id == id) else { return false };
        pane.kind = Kind::Slot { name: name.clone(), bundle: bundle.clone() };
        self.slots.insert(id, Run::new(slot, bundle.clone(), name.clone()));
        true
    }

    pub(super) fn slot_act(&mut self, action: Action, effects: &mut Vec<Effect>) {
        match action {
            Action::SlotMake { pane, pid, slot } => self.slot_make(pane, pid, slot),
            Action::SlotClear { pane } => {
                if self.slots.remove(&pane).is_some() {
                    self.slot_emptied(pane);
                }
            }
            Action::SlotAccept { pane } => {
                let offer = self.slot_offer(pane).map(|seen| (seen.pid, seen.window));
                if let (Some((pid, window)), Some(run)) = (offer, self.slots.get_mut(&pane)) {
                    run.pending = Some(window);
                    effects.push(Effect::HostSlot { pane, pid, window });
                }
            }
            Action::SlotDecline { pane } => {
                if let Some(run) = self.slots.get_mut(&pane)
                    && let Some(window) = run.offered.take()
                {
                    run.declined.insert(window);
                }
            }
            Action::SlotNext { pane } => self.slot_next(pane),
            Action::SlotLookAgain { pane } => {
                if let Some(run) = self.slots.get_mut(&pane) {
                    (run.declined, run.offered) = (BTreeSet::new(), None);
                }
            }
            Action::SlotGiveBack { pane } => {
                let window = match self.pane(pane).map(|p| &p.kind) {
                    Some(Kind::App { window, .. }) if self.slots.contains_key(&pane) => *window,
                    _ => return,
                };
                if self.live == Some(pane) || self.peek.as_ref().is_some_and(|peek| peek.pane == pane) {
                    self.leave(effects);
                }
                // Unhosting stops a transcript of it.
                if self.listen.pane() == Some(pane) {
                    self.listen_end(None, None, effects);
                }
                self.slot_unfill(pane);
                if let Some(run) = self.slots.get_mut(&pane) {
                    // Passed over, or an Automatic slot would take it straight back.
                    run.declined.insert(window);
                }
                effects.push(Effect::GiveBack { pane });
            }
            // The shell's alone from here: a key or click of the user's reports nothing.
            _ if self.by_user => {}
            Action::SlotWindows { look } => self.slot_windows(look, effects),
            Action::SlotHosted { pane, kind } => self.slot_hosted(pane, kind, effects),
            Action::SlotTick { at } => {
                for run in self.slots.values_mut() {
                    if run.away.is_some_and(|since| at >= since.saturating_add(slot::AWAY_MS)) {
                        run.away = None;
                        run.waits();
                    }
                }
            }
            _ => {}
        }
    }

    /// `pane`'s window is gone. A slot waits for it (Away while its app runs); any other pane
    /// closes, as a `Close` from the shell does.
    pub(super) fn window_gone(&mut self, pane: PaneId, app_running: bool, at: u64, effects: &mut Vec<Effect>) {
        // The shell's alone: the user's key or click reports no window gone.
        if self.by_user {
            return;
        }
        // A slot that waits already: a late or second report changes nothing.
        if self.slots.contains_key(&pane) && !self.slot_filled(pane) {
            return;
        }
        if !self.slots.contains_key(&pane) {
            return self.act(Action::Close { pane: Some(pane) }, effects);
        }
        // The keyboard comes back to ottospace, as the shell is told (`Effect::Parked`).
        if self.live == Some(pane) || self.peek.as_ref().is_some_and(|peek| peek.pane == pane) {
            self.leave(effects);
        }
        // Its window going stops a transcript of it, as the pane closing would (slice 5: a meeting's holds it).
        if self.listen.pane() == Some(pane) {
            self.listen_end(None, None, effects);
        }
        self.slot_unfill(pane);
        let Some(run) = self.slots.get_mut(&pane) else { return };
        run.away = app_running.then_some(at);
        if !app_running {
            run.pid = None;
            let (bundle, name) = (run.bundle.clone(), run.name.clone());
            let of = |look: &&mut Look| same_app((bundle.as_deref(), &name), (look.bundle.as_deref(), &look.name));
            if let Some(look) = self.slot_looks.iter_mut().find(of) {
                (look.running, look.windows) = (false, Vec::new());
            }
        }
    }

    /// A slot that ended while it waited: its pane is an empty one, and a mark of it stands for none.
    fn slot_emptied(&mut self, id: PaneId) {
        if let Some(pane) = self.panes.iter_mut().find(|p| p.id == id && matches!(p.kind, Kind::Slot { .. })) {
            pane.kind = Kind::Empty;
            self.marks.forget_pane(id);
        }
    }

    /// A filled slot's pane waits again: its kind is the slot's, its window none.
    fn slot_unfill(&mut self, id: PaneId) {
        let Some(run) = self.slots.get_mut(&id) else { return };
        if let Some(pane) = self.panes.iter_mut().find(|p| p.id == id) {
            (pane.kind, pane.fit) = (Kind::Slot { name: run.name.clone(), bundle: run.bundle.clone() }, None);
        }
        run.waits();
        (run.pending, run.status) = (None, Status::Empty);
    }

    /// The user's Keep: a live app pane becomes a filled slot, or a slot takes its new settings.
    fn slot_make(&mut self, id: PaneId, pid: i32, slot: Slot) {
        if let Some(run) = self.slots.get_mut(&id) {
            run.matching = slot.matching;
            (run.slot, run.declined, run.offered) = (slot, BTreeSet::new(), None);
            return;
        }
        let Some(Kind::App { bundle, name, .. }) = self.pane(id).map(|pane| &pane.kind) else { return };
        let mut run = Run::new(slot, bundle.clone(), name.clone());
        (run.status, run.pid) = (Status::Filled, Some(pid));
        self.slots.insert(id, run);
    }

    /// `SlotNext`: the candidate after the one offered, in the pick's order, round again.
    fn slot_next(&mut self, pane: PaneId) {
        let Some(run) = self.slots.get(&pane) else { return };
        let Some(look) = self.slot_looks.iter().find(|look| run.of(look)) else { return };
        let claimed = self.slot_claims(Some(pane));
        let passed = |window: u32| claimed.contains(&window) || run.declined.contains(&window);
        let found: Vec<u32> = look.candidates(&run.slot, &self.window_rules, passed).iter().map(|s| s.window).collect();
        let at = found.iter().position(|window| Some(*window) == run.offered);
        let next = at.map_or(found.first(), |at| found.get((at + 1) % found.len())).copied();
        if let Some(run) = self.slots.get_mut(&pane) {
            run.offered = next.or(run.offered);
        }
    }

    /// The windows no slot may offer: in a pane, asked for by a slot, or offered by another one
    /// (`but`: this one's own offer counts as free).
    fn slot_claims(&self, but: Option<PaneId>) -> BTreeSet<u32> {
        let held = self.panes.iter().filter_map(|pane| match pane.kind {
            Kind::App { window, .. } => Some(window),
            _ => None,
        });
        let asked = self
            .slots
            .iter()
            .filter(|(id, _)| Some(**id) != but)
            .flat_map(|(_, run)| run.pending.into_iter().chain(run.offered));
        held.chain(asked).collect()
    }

    fn slot_windows(&mut self, look: Look, effects: &mut Vec<Effect>) {
        let waited = self.waits_for_slot_look((look.bundle.as_deref(), &look.name));
        let listed: BTreeSet<u32> = look.windows.iter().map(|seen| seen.window).collect();
        let app = (look.bundle.clone(), look.name.clone());
        let at = self
            .slot_looks
            .iter()
            .position(|old| same_app((old.bundle.as_deref(), &old.name), (look.bundle.as_deref(), &look.name)));
        match at {
            Some(at) => self.slot_looks[at] = look,
            None => self.slot_looks.push(look),
        }
        let shown = self.layout.as_ref().map(super::Node::leaves).unwrap_or_default();
        // What a slot earlier in the order offers: no later one takes it (several slots, §3).
        let mut offered_here = BTreeSet::new();
        for id in shown {
            let filled = self.slot_filled(id);
            let ours = |run: &&Run| same_app((run.bundle.as_deref(), &run.name), (app.0.as_deref(), &app.1));
            let Some(run) = self.slots.get(&id).filter(|run| !filled && run.pending.is_none()).filter(ours) else {
                continue;
            };
            let Some(look) = self.slot_looks.iter().find(|look| run.of(look)) else { continue };
            // Looked at: a plain pane of its app waits no more, and the app's windows once it
            // starts opened while the slot waited.
            if !look.running {
                self.slots.get_mut(&id).map(|run| run.known.get_or_insert_default());
                continue;
            }
            // A closed window is passed over no more: its id may come back as another window.
            let declined: BTreeSet<u32> = run.declined.intersection(&listed).copied().collect();
            let claimed = self.slot_claims(Some(id));
            let passed =
                |window: u32| claimed.contains(&window) || declined.contains(&window) || offered_here.contains(&window);
            let found = look.candidates(&run.slot, &self.window_rules, passed);
            let take = found.iter().find(|seen| run.takes(seen, look.frontmost)).map(|seen| (seen.pid, seen.window));
            if take.is_none() {
                let kept = run.offered.filter(|window| found.iter().any(|seen| seen.window == *window));
                offered_here.extend(kept.or(found.first().map(|seen| seen.window)));
            }
            let windows: Vec<u32> = look.windows.iter().map(|seen| seen.window).collect();
            let Some(run) = self.slots.get_mut(&id) else { continue };
            run.declined = declined;
            run.known.get_or_insert_default().extend(windows);
            if let Some((pid, window)) = take {
                (run.pending, run.offered) = (Some(window), None);
                effects.push(Effect::HostSlot { pane: id, pid, window });
            }
        }
        // A restored app pane of this app waited for the slot to look first.
        if waited {
            self.settle_slots(effects);
            self.retry_unfilled(effects);
        }
    }

    fn slot_hosted(&mut self, id: PaneId, kind: Option<Kind>, effects: &mut Vec<Effect>) {
        let Some(run) = self.slots.get_mut(&id) else { return };
        let Some(asked) = run.pending.take() else { return };
        let Some(kind @ Kind::App { window, .. }) =
            kind.filter(|kind| matches!(kind, Kind::App { window, .. } if *window == asked))
        else {
            return;
        };
        // Its tab went out of sight meanwhile: filling it would park the window (rule 2).
        if !self.layout.as_ref().is_some_and(|layout| layout.leaves().contains(&id)) {
            return;
        }
        // A window is in one pane at most: another took it meanwhile.
        if self.panes.iter().any(|p| p.id != id && matches!(p.kind, Kind::App { window: w, .. } if w == window)) {
            return;
        }
        let pid = self
            .slot_looks
            .iter()
            .flat_map(|look| &look.windows)
            .find(|seen| seen.window == asked)
            .map(|seen| seen.pid);
        self.slot_fill(id, kind, pid, effects);
    }

    /// Slot `id`, waiting, holds the window of `kind` (of process `pid`, when known).
    fn slot_fill(&mut self, id: PaneId, kind: Kind, pid: Option<i32>, effects: &mut Vec<Effect>) {
        let Some(pane) = self.panes.iter_mut().find(|p| p.id == id && matches!(p.kind, Kind::Slot { .. })) else {
            return;
        };
        pane.kind = kind;
        if let Some(run) = self.slots.get_mut(&id) {
            (run.away, run.offered, run.known) = (None, None, None);
            run.pid = pid.or(run.pid);
        }
        effects.push(Effect::Opened(id));
    }

    /// A new window of a hosted app, with its facts (`Action::Adopt`). The first slot on the tab
    /// on show that waits and selects it takes it (`Run::takes`) or offers it; one adoption, no
    /// race with the slot's look. False: no slot selects it, and it is a new pane.
    pub(super) fn slot_adopt(&mut self, kind: &Kind, seen: Seen, effects: &mut Vec<Effect>) -> bool {
        let Kind::App { bundle, name, window, .. } = kind else { return false };
        if *window != seen.window {
            return false;
        }
        let app = (bundle.as_deref(), name.as_str());
        // The app's last look with this window in front: what the slots choose from.
        let at = self.slot_looks.iter().position(|look| same_app((look.bundle.as_deref(), &look.name), app));
        let mut look = at.map(|at| self.slot_looks[at].clone()).unwrap_or_else(|| Look {
            bundle: bundle.clone(),
            name: name.clone(),
            ..Look::default()
        });
        look.running = true;
        look.windows.retain(|old| old.window != seen.window);
        look.windows.insert(0, seen.clone());
        let shown = self.layout.as_ref().map(super::Node::leaves).unwrap_or_default();
        let chosen = shown.into_iter().find_map(|id| {
            let filled = self.slot_filled(id);
            let run = self.slots.get(&id).filter(|run| !filled && run.pending.is_none() && run.of(&look))?;
            let claimed = self.slot_claims(Some(id));
            let passed = |window: u32| claimed.contains(&window) || run.declined.contains(&window);
            let found = look.candidates(&run.slot, &self.window_rules, passed);
            found.iter().any(|s| s.window == seen.window).then(|| (id, run.takes(&seen, look.frontmost)))
        });
        let Some((id, take)) = chosen else { return false };
        match at {
            Some(at) => self.slot_looks[at] = look,
            None => self.slot_looks.push(look),
        }
        if take {
            self.slot_fill(id, kind.clone(), Some(seen.pid), effects);
        } else if let Some(known) = self.slots.get_mut(&id).and_then(|run| run.known.as_mut()) {
            // Offered (by the settle after this step), and no longer new to it.
            known.insert(seen.window);
        }
        true
    }

    /// After every step: each slot's status and offer, from the last looks, in the order of the
    /// tab on show's panes (the first slot that selects a window offers it). A slot on a hidden
    /// tab offers nothing and looks again when its tab shows. Then the apps to look at.
    pub(super) fn settle_slots(&mut self, effects: &mut Vec<Effect>) {
        let panes = &self.panes;
        self.slots.retain(|id, _| panes.iter().any(|pane| pane.id == *id));
        self.slot_dormant.retain(|id, _| panes.iter().any(|pane| pane.id == *id));
        if !self.features.on(Feature::Slots) && !self.slots.is_empty() {
            for (id, run) in std::mem::take(&mut self.slots) {
                // A filled one stays an app pane and keeps its slot on its record for the flag's return.
                if self.slot_filled(id) {
                    self.slot_dormant.extend(serde_json::to_value(&run.slot).ok().map(|slot| (id, slot)));
                }
                self.slot_emptied(id);
            }
        }
        let shown = self.layout.as_ref().map(super::Node::leaves).unwrap_or_default();
        let filled: BTreeSet<PaneId> =
            self.panes.iter().filter(|pane| matches!(pane.kind, Kind::App { .. })).map(|pane| pane.id).collect();
        for (id, run) in self.slots.iter_mut().filter(|(id, _)| !shown.contains(id)) {
            run.offered = None;
            run.known = None;
            run.status = match run.status {
                _ if filled.contains(id) => Status::Filled,
                Status::Offered | Status::Filled => Status::Empty,
                status => status,
            };
        }
        // Offers go in order: a window is offered by one slot at a time.
        let order: Vec<PaneId> = shown.into_iter().filter(|id| self.slots.contains_key(id)).collect();
        for id in order {
            let claimed = self.slot_claims(Some(id));
            let Some(run) = self.slots.get(&id) else { continue };
            let look = self.slot_looks.iter().find(|look| run.of(look));
            let (status, matches, offered) = match look {
                _ if filled.contains(&id) => (Status::Filled, 0, None),
                _ if run.pending.is_some() => (run.status, run.matches, None),
                Some(look) if !look.running => (Status::Absent, 0, None),
                None => (if run.away.is_some() { Status::Away } else { Status::Empty }, 0, None),
                Some(look) => {
                    let passed = |window: u32| claimed.contains(&window) || run.declined.contains(&window);
                    let found = look.candidates(&run.slot, &self.window_rules, passed);
                    let kept = run.offered.filter(|window| found.iter().any(|seen| seen.window == *window));
                    let offered = kept.or(found.first().map(|seen| seen.window));
                    let status = match (run.away, offered) {
                        (Some(_), _) => Status::Away,
                        (None, Some(_)) => Status::Offered,
                        (None, None) => Status::Empty,
                    };
                    (status, found.len(), offered)
                }
            };
            if let Some(run) = self.slots.get_mut(&id) {
                (run.status, run.matches, run.offered) = (status, matches, offered);
                if status == Status::Absent {
                    run.away = None;
                }
            }
        }
        // Look at the apps of the slots on show that wait; none with no such slot.
        let shown = self.layout.as_ref().map(super::Node::leaves).unwrap_or_default();
        let mut apps: Vec<(Option<String>, String)> = Vec::new();
        for (_, run) in self.slots.iter().filter(|(id, run)| shown.contains(id) && run.status != Status::Filled) {
            if !apps
                .iter()
                .any(|(bundle, name)| same_app((bundle.as_deref(), name), (run.bundle.as_deref(), &run.name)))
            {
                apps.push((run.bundle.clone(), run.name.clone()));
            }
        }
        if apps != self.slot_watch {
            self.slot_watch = apps.clone();
            effects.push(Effect::WatchSlots { apps });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ax::Frame;
    use crate::layout::Axis::Right;
    use crate::model::Origin;
    use crate::slot::{Look, Pick};

    const BUNDLE: &str = "com.example.meetings";

    fn meetings(window: u32, title: &str) -> Kind {
        Kind::App { window, title: title.into(), bundle: Some(BUNDLE.into()), name: "Meetings".into() }
    }

    fn slot(title: &str, matching: Matching) -> Slot {
        let select = crate::rules::Selector { title: Some(title.into()), ..Default::default() };
        Slot { select, matching, ..Default::default() }
    }

    fn seen(pid: i32, window: u32, title: &str) -> Seen {
        let frame = Frame::new(0., 0., 800. + window as f64, 600.);
        Seen { pid, window, title: title.into(), subrole: "AXStandardWindow".into(), frame, ..Default::default() }
    }

    fn look(windows: Vec<Seen>) -> Action {
        let displays = vec![Frame::new(0., 0., 1920., 1080.)];
        let look = Look {
            bundle: Some(BUNDLE.into()),
            name: "Meetings".into(),
            running: true,
            windows,
            displays,
            frontmost: None,
        };
        Action::SlotWindows { look }
    }

    fn system(action: Action) -> crate::model::Envelope {
        action.sent_by(Origin::System)
    }

    /// A terminal, and beside it pane 2: Meetings' window 1, kept as a slot for `*Incident*` by the user.
    fn kept(matching: Matching) -> Model {
        let mut m = Model::default();
        m.update(Action::Split { axis: Right, kind: Kind::Terminal });
        m.update(Action::Open { kind: meetings(1, "Incident review"), axis: Right, beside: None });
        m.update(Action::SlotMake { pane: 2, pid: 7, slot: slot("*incident*", matching) });
        m
    }

    /// As `kept`, its window closed while Meetings runs, and Away over: the slot waits.
    fn waiting(matching: Matching) -> Model {
        let mut m = kept(matching);
        m.update(system(Action::WindowGone { pane: 2, app_running: true, at: 0 }));
        m.update(system(Action::SlotTick { at: slot::AWAY_MS }));
        m
    }

    fn status(m: &Model, pane: PaneId) -> Option<Status> {
        m.slots.get(&pane).map(|run| run.status)
    }

    fn hosts(effects: &[Effect]) -> Vec<(PaneId, u32)> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::HostSlot { pane, window, .. } => Some((*pane, *window)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn only_the_user_makes_fills_or_clears_a_slot() {
        let mut m = Model::default();
        m.update(Action::Open { kind: meetings(1, "Incident review"), axis: Right, beside: None });
        let make = Action::SlotMake { pane: 1, pid: 7, slot: slot("*", Matching::Manual) };
        for origin in [Origin::Socket, Origin::System] {
            assert_eq!(m.refusal(&make, origin), Some(slot::USERS_ALONE));
            m.update(make.clone().sent_by(origin));
        }
        assert!(m.slots.is_empty());
        m.update(make);
        assert_eq!(status(&m, 1), Some(Status::Filled));
        // Only the shell reports windows; a client moves no slot pane, nor closes it.
        assert_eq!(m.refusal(&look(Vec::new()), Origin::Socket), Some(slot::SHELLS_ALONE));
        for action in [
            Action::Close { pane: Some(1) },
            Action::CloseTab { index: None },
            Action::MovePane { pane: Some(1), tab: 0 },
            Action::DropPane { source: 1, target: 1, zone: crate::layout::Zone::Left },
        ] {
            assert_eq!(m.refusal(&action, Origin::Socket), Some(slot::SLOT_PANE), "{action:?}");
            assert_eq!(m.refusal(&action, Origin::User), None);
        }
        // No pane becomes a slot by its kind alone.
        let fake = Kind::Slot { name: "Meetings".into(), bundle: None };
        assert!(m.update(Action::Split { axis: Right, kind: fake }).is_empty());
        assert_eq!(m.panes.len(), 1);
    }

    #[test]
    fn a_closed_window_leaves_the_slot_waiting_and_a_clients_open_splits_beside_it() {
        let mut m = kept(Matching::Manual);
        m.update(system(Action::WindowGone { pane: 2, app_running: true, at: 5 }));
        let waits = Kind::Slot { name: "Meetings".into(), bundle: Some(BUNDLE.into()) };
        assert_eq!(m.pane(2).map(|p| &p.kind), Some(&waits));
        assert_eq!(status(&m, 2), Some(Status::Away));
        m.update(system(Action::SlotTick { at: 5 + slot::AWAY_MS - 1 }));
        assert_eq!(status(&m, 2), Some(Status::Away));
        m.update(system(Action::SlotTick { at: 5 + slot::AWAY_MS }));
        assert_eq!(status(&m, 2), Some(Status::Empty));
        // An empty pane would take the client's app; a slot does not: it splits.
        let open = Action::Open { kind: meetings(9, "x"), axis: Right, beside: Some(2) };
        let opened = m.update(open.sent_by(Origin::Socket));
        assert!(opened.contains(&Effect::Opened(3)), "{opened:?}");
        assert!(matches!(m.pane(2).map(|p| &p.kind), Some(Kind::Slot { .. })));
        // Any other pane's window going closes it.
        m.update(system(Action::WindowGone { pane: 3, app_running: true, at: 6 }));
        assert!(m.pane(3).is_none());
    }

    #[test]
    fn manual_offers_in_the_picks_order_and_fills_on_the_users_bring_it_here() {
        let mut m = waiting(Matching::Manual);
        let windows = vec![seen(7, 2, "Incident review"), seen(7, 3, "Incident retro"), seen(7, 4, "Lunch")];
        let effects = m.update(system(look(windows)));
        assert!(hosts(&effects).is_empty(), "Manual never fills from a look");
        let offer = |m: &Model| m.slot_offer(2).map(|s| s.window);
        assert_eq!((status(&m, 2), m.slots[&2].matches, offer(&m)), (Some(Status::Offered), 2, Some(2)));
        // 2 more match: the next, round again.
        m.update(Action::SlotNext { pane: 2 });
        assert_eq!(offer(&m), Some(3));
        m.update(Action::SlotNext { pane: 2 });
        assert_eq!(offer(&m), Some(2));
        // Not this one: passed over, the next offered.
        m.update(Action::SlotDecline { pane: 2 });
        assert_eq!((offer(&m), m.slots[&2].matches), (Some(3), 1));
        // A client's Bring it here does nothing; the user's asks the shell for that window.
        assert!(hosts(&m.update(Action::SlotAccept { pane: 2 }.sent_by(Origin::Socket))).is_empty());
        let effects = m.update(Action::SlotAccept { pane: 2 });
        assert!(effects.contains(&Effect::HostSlot { pane: 2, pid: 7, window: 3 }), "{effects:?}");
        // Only that window fills it, once hosted.
        m.update(system(Action::SlotHosted { pane: 2, kind: Some(meetings(4, "Lunch")) }));
        assert_eq!(status(&m, 2), Some(Status::Offered));
        m.update(Action::SlotAccept { pane: 2 });
        let effects = m.update(system(Action::SlotHosted { pane: 2, kind: Some(meetings(3, "Incident retro")) }));
        assert!(effects.contains(&Effect::Opened(2)));
        assert_eq!(status(&m, 2), Some(Status::Filled));
        assert_eq!(m.pane(2).map(|p| p.kind.clone()), Some(meetings(3, "Incident retro")));
    }

    #[test]
    fn a_window_that_comes_back_while_away_goes_in_without_asking() {
        let mut m = kept(Matching::Manual);
        m.update(system(Action::WindowGone { pane: 2, app_running: true, at: 0 }));
        m.update(system(look(Vec::new())));
        // Another process's window that matches is offered; the same process's goes back in.
        let effects = m.update(system(look(vec![seen(8, 5, "Incident 2")])));
        assert!(hosts(&effects).is_empty());
        let effects = m.update(system(look(vec![seen(8, 5, "Incident 2"), seen(7, 6, "Incident review")])));
        assert_eq!(hosts(&effects), [(2, 6)]);
        // The app quit instead: it waits for the app.
        let mut m = kept(Matching::Manual);
        m.update(system(Action::WindowGone { pane: 2, app_running: false, at: 0 }));
        assert_eq!(status(&m, 2), Some(Status::Empty));
        let gone = Look { bundle: Some(BUNDLE.into()), name: "Meetings".into(), ..Default::default() };
        m.update(system(Action::SlotWindows { look: gone }));
        assert_eq!(status(&m, 2), Some(Status::Absent));
    }

    #[test]
    fn automatic_takes_only_a_new_window_on_the_tab_on_show_and_not_the_one_in_use() {
        let mut m = waiting(Matching::Automatic);
        // What is open at the first look is offered: the user may be in it.
        assert!(hosts(&m.update(system(look(vec![seen(7, 2, "Incident review")])))).is_empty());
        assert_eq!(status(&m, 2), Some(Status::Offered));
        // A new window the user is in, in its app, is offered too.
        let mut front = look(vec![seen(7, 2, "Incident review"), seen(7, 3, "Incident retro")]);
        if let Action::SlotWindows { look } = &mut front {
            look.frontmost = Some(3);
        }
        assert!(hosts(&m.update(system(front))).is_empty());
        // A new one elsewhere is taken.
        let three = vec![seen(7, 2, "Incident review"), seen(7, 3, "Incident retro"), seen(7, 4, "Incident 3")];
        assert_eq!(hosts(&m.update(system(look(three)))), [(2, 4)]);
        // On a hidden tab it takes nothing, and offers nothing.
        let mut m = waiting(Matching::Automatic);
        m.update(system(look(Vec::new())));
        m.update(Action::NewTab);
        assert!(hosts(&m.update(system(look(vec![seen(7, 9, "Incident 9")])))).is_empty());
        assert_eq!((status(&m, 2), m.slots[&2].offered), (Some(Status::Empty), None));
    }

    #[test]
    fn a_decline_is_by_window_for_the_run_and_forgotten_when_it_closes_or_on_look_again() {
        let mut m = waiting(Matching::Manual);
        let review = || seen(7, 2, "Incident review");
        m.update(system(look(vec![review()])));
        m.update(Action::SlotDecline { pane: 2 });
        assert_eq!(status(&m, 2), Some(Status::Empty));
        // Another window of the same title is offered: not by title.
        m.update(system(look(vec![review(), seen(7, 5, "Incident review")])));
        assert_eq!(m.slot_offer(2).map(|s| s.window), Some(5));
        m.update(Action::SlotDecline { pane: 2 });
        m.update(Action::SlotLookAgain { pane: 2 });
        assert_eq!(m.slot_offer(2).map(|s| s.window), Some(2));
        // Closed, it is forgotten: its id may come back as another window.
        m.update(Action::SlotDecline { pane: 2 });
        m.update(system(look(vec![seen(7, 5, "Incident review")])));
        assert!(!m.slots[&2].declined.contains(&2));
    }

    #[test]
    fn of_two_slots_the_first_on_screen_offers_a_window_and_the_next_once_declined() {
        let mut m = waiting(Matching::Manual);
        m.update(Action::Open { kind: meetings(8, "Incident other"), axis: Right, beside: None });
        m.update(Action::SlotMake { pane: 3, pid: 7, slot: slot("incident*", Matching::Manual) });
        m.update(system(Action::WindowGone { pane: 3, app_running: true, at: 0 }));
        m.update(system(Action::SlotTick { at: slot::AWAY_MS }));
        m.update(system(look(vec![seen(7, 2, "Incident review")])));
        assert_eq!((m.slots[&2].offered, m.slots[&3].offered), (Some(2), None));
        m.update(Action::SlotDecline { pane: 2 });
        assert_eq!((m.slots[&2].offered, m.slots[&3].offered), (None, Some(2)));
    }

    #[test]
    fn the_window_rules_come_first_and_a_pick_narrows() {
        let mut m = waiting(Matching::Manual);
        let rules = r#"{"windows": [{"app": "Meetings", "title": "incident retro", "do": "ignore"}]}"#;
        m.update(system(Action::SettingsRead { text: rules.into(), written: false }));
        m.update(system(look(vec![seen(7, 3, "Incident retro")])));
        assert_eq!(status(&m, 2), Some(Status::Empty));
        let largest = Slot { pick: Pick::Largest, ..slot("*incident*", Matching::Manual) };
        m.update(Action::SlotMake { pane: 2, pid: 7, slot: largest });
        m.update(system(look(vec![seen(7, 4, "Incident a"), seen(7, 6, "Incident b"), seen(7, 3, "Incident retro")])));
        assert_eq!(m.slot_offer(2).map(|s| s.window), Some(6), "the larger");
    }

    #[test]
    fn the_state_says_where_a_slot_is_never_what_it_selects_or_a_windows_title() {
        let mut m = waiting(Matching::Manual);
        m.update(system(look(vec![seen(7, 2, "Incident SECRET review")])));
        let state = serde_json::to_string(&m).unwrap();
        assert!(state.contains(r#""slots":{"2":{"status":"offered","matches":1,"matching":"manual"}}"#), "{state}");
        assert!(!state.to_lowercase().contains("incident"), "{state}");
        assert!(state.contains(r#""kind":{"type":"slot","name":"Meetings","bundle":"com.example.meetings"}"#));
    }

    #[test]
    fn a_slot_is_kept_on_its_app_panes_record_and_comes_back_waiting() {
        let mut m = kept(Matching::Automatic);
        m.update(Action::Open { kind: meetings(5, "Lunch"), axis: Right, beside: None });
        let file = m.layout_file(&[], &[], 0);
        let text = crate::layout_file::write(&file);
        assert!(text.contains(r#""slot": {"#) && text.contains(r#""title": "*incident*""#), "{text}");
        let restore = |file: crate::layout_file::Layout, m: &mut Model| {
            let layout = Box::new(file);
            m.update(system(Action::RestoreLayout { layout, gone: Vec::new(), home: None, launch: false }))
        };
        let mut r = Model::default();
        let effects = restore(file.clone(), &mut r);
        let slot = r.slots.keys().next().copied().unwrap_or_default();
        assert!(matches!(r.pane(slot).map(|p| &p.kind), Some(Kind::Slot { .. })));
        assert_eq!(status(&r, slot), Some(Status::Empty));
        // The plain app pane of the same app waits for the slot's first look, then passes over its window.
        let host = |effects: &[Effect]| {
            effects.iter().any(|e| matches!(e, Effect::HostWindow { name, .. } if name == "Meetings"))
        };
        assert!(!host(&effects) && r.marks.unfilled().iter().all(|u| u.pane != slot));
        assert!(effects.contains(&Effect::WatchSlots { apps: vec![(Some(BUNDLE.into()), "Meetings".into())] }));
        let effects = r.update(system(look(vec![seen(7, 1, "Incident review"), seen(7, 5, "Lunch")])));
        assert!(host(&effects) && hosts(&effects).is_empty(), "Automatic offers what is open at a restore");
        assert!(r.slot_selects(1) && !r.slot_selects(5));
        // The app not running: the plain pane is asked for at once, and the slot is Absent.
        let mut quit = Model::default();
        restore(file.clone(), &mut quit);
        let gone = Look { bundle: Some(BUNDLE.into()), name: "Meetings".into(), ..Default::default() };
        assert!(host(&quit.update(system(Action::SlotWindows { look: gone }))));
        assert_eq!(quit.slots.values().map(|run| run.status).collect::<Vec<_>>(), [Status::Absent]);
        // With the flag off it is a plain app pane, as an older build reads it.
        let mut off = Model::default();
        off.update(system(Action::SetFeatures { features: crate::features::Features::default() }));
        restore(file, &mut off);
        assert!(off.slots.is_empty() && off.panes.iter().all(|p| !matches!(p.kind, Kind::Slot { .. })));
        // Hosted as a plain pane, it keeps its slot on the record for the flag's return.
        let kept = off.slot_dormant.keys().next().copied().unwrap_or_default();
        off.update(system(Action::Fill { pane: kept, kind: meetings(1, "Incident review") }));
        let text = crate::layout_file::write(&off.layout_file(&[], &[], 0));
        assert!(text.contains(r#""slot": {"#) && text.contains("*incident*"), "{text}");
        // Turned off mid-run, a filled slot keeps it too.
        let mut m = kept_slot_turned_off();
        let text = crate::layout_file::write(&m.layout_file(&[], &[], 0));
        assert!(m.slots.is_empty() && text.contains(r#""slot": {"#), "{text}");
        m.update(Action::Close { pane: Some(2) });
        assert!(m.slot_dormant.is_empty());
    }

    fn kept_slot_turned_off() -> Model {
        let mut m = kept(Matching::Manual);
        m.update(system(Action::SetFeatures { features: crate::features::Features::default() }));
        m
    }

    #[test]
    fn a_look_at_another_app_leaves_a_slot_as_it_was() {
        let mut m = waiting(Matching::Manual);
        m.update(system(look(vec![seen(7, 2, "Incident review")])));
        m.update(Action::SlotDecline { pane: 2 });
        let other =
            Look { bundle: Some("com.other".into()), name: "Other".into(), running: true, ..Default::default() };
        m.update(system(Action::SlotWindows { look: other }));
        assert!(m.slots[&2].declined.contains(&2), "a decline stands");
        // A late report of its window gone closes no slot that waits.
        m.update(system(Action::WindowGone { pane: 2, app_running: true, at: 1 }));
        assert_eq!(status(&m, 2), Some(Status::Empty));
    }

    #[test]
    fn a_window_two_slots_select_goes_to_the_first_even_when_the_second_would_take_it() {
        let mut m = waiting(Matching::Manual);
        m.update(Action::Open { kind: meetings(8, "Incident other"), axis: Right, beside: None });
        m.update(Action::SlotMake { pane: 3, pid: 7, slot: slot("incident*", Matching::Automatic) });
        m.update(system(Action::WindowGone { pane: 3, app_running: true, at: 0 }));
        m.update(system(Action::SlotTick { at: slot::AWAY_MS }));
        m.update(system(look(Vec::new())));
        let effects = m.update(system(look(vec![seen(7, 4, "Incident new")])));
        assert!(hosts(&effects).is_empty(), "{effects:?}");
        assert_eq!((m.slots[&2].offered, m.slots[&3].offered), (Some(4), None));
    }

    #[test]
    fn a_new_window_of_the_app_goes_to_the_slot_not_a_new_pane() {
        let adopt = |window: u32, pid: i32, title: &str| {
            let seen = seen(pid, window, title);
            system(Action::Adopt { kind: meetings(window, title), beside: Some(2), seen: Some(seen) })
        };
        // Manual: offered, and no pane opens for it.
        let mut m = waiting(Matching::Manual);
        m.update(system(look(Vec::new())));
        assert!(!m.update(adopt(4, 7, "Incident review")).iter().any(|e| matches!(e, Effect::Opened(_))));
        assert_eq!((m.panes.len(), m.slot_offer(2).map(|s| s.window)), (2, Some(4)));
        // One the slot does not select is a new pane, as before.
        assert!(m.update(adopt(5, 7, "Lunch")).contains(&Effect::Opened(3)));
        // Automatic, after its first look: taken into the slot's pane.
        let mut m = waiting(Matching::Automatic);
        m.update(system(look(Vec::new())));
        assert!(m.update(adopt(4, 7, "Incident review")).contains(&Effect::Opened(2)));
        assert_eq!(
            (status(&m, 2), m.pane(2).map(|p| p.kind.clone())),
            (Some(Status::Filled), Some(meetings(4, "Incident review")))
        );
        // Away: its process's window comes back in, Manual too.
        let mut m = kept(Matching::Manual);
        m.update(system(Action::WindowGone { pane: 2, app_running: true, at: 0 }));
        assert!(m.update(adopt(6, 7, "Incident review")).contains(&Effect::Opened(2)));
        // A client's facts are refused; the user's are not the shell's, and open a pane.
        let mut m = waiting(Matching::Automatic);
        m.update(system(look(Vec::new())));
        let socket = adopt(4, 7, "Incident review").action.sent_by(Origin::Socket);
        assert_eq!(m.refusal(&socket.action, Origin::Socket), Some(slot::SHELLS_ALONE));
        assert!(m.update(adopt(4, 7, "Incident review").action).contains(&Effect::Opened(3)));
    }

    #[test]
    fn give_back_and_clear_end_a_fill_or_the_slot_as_the_user_says() {
        let mut m = kept(Matching::Automatic);
        let effects = m.update(Action::SlotGiveBack { pane: 2 });
        assert!(effects.contains(&Effect::GiveBack { pane: 2 }));
        // Passed over, or the Automatic slot would take it straight back.
        m.update(system(look(vec![seen(7, 1, "Incident review")])));
        assert!(m.slots[&2].declined.contains(&1) && m.slots[&2].offered.is_none());
        m.update(Action::SlotClear { pane: 2 });
        assert_eq!((m.slots.is_empty(), m.pane(2).map(|p| p.kind.clone())), (true, Some(Kind::Empty)));
        // A filled slot cleared stays an app pane.
        let mut m = kept(Matching::Manual);
        m.update(Action::SlotClear { pane: 2 });
        assert_eq!(
            (m.slots.is_empty(), m.pane(2).map(|p| p.kind.clone())),
            (true, Some(meetings(1, "Incident review")))
        );
    }
}
