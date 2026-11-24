//! The model's side of slots (`crate::slot`): where each is in its life, which window it offers
//! or takes, and what the user's moves and the shell's looks do. Who may send each action is
//! `Model::refusal`; a slot is the user's (`docs/ai/slots.md` §6).

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::{Action, Effect, Kind, Model, PaneId};
use crate::features::Feature;
use crate::marks::{Extra, PaneRecord, RecordKind, Unfilled, same_app};
use crate::slot::{self, Look, Matching, Seen, Slot, Status, Test};

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
    /// The window the user's Next chose: it stays the offer while it can, whatever the pairing.
    #[serde(skip)]
    pub picked: Option<u32>,
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
            picked: None,
        }
    }

    pub(crate) fn of(&self, look: &Look) -> bool {
        self.of_app((look.bundle.as_deref(), &look.name))
    }

    fn of_app(&self, app: (Option<&str>, &str)) -> bool {
        same_app((self.bundle.as_deref(), &self.name), app)
    }

    /// It waits again from now: windows open now are not new to it.
    fn waits(&mut self) {
        (self.offered, self.known, self.picked) = (None, None, None);
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

/// The slot dialog (⋯'s Create slot… or Edit slot…): a form under its rule, said in one line.
/// In the state: what is chosen; never a window's title or id, nor the words of a title rule.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Dialog {
    pub pane: PaneId,
    /// The app's windows as a look found them when it opened, front to back.
    #[serde(skip)]
    pub look: Look,
    /// The pane's window: its process, title and identifier.
    #[serde(skip)]
    pub window: Option<Seen>,
    /// The slot's own selector when the dialog edits one: its id stays the one it waits for, and
    /// a title it has beside its id (an older slot's) stays while the rule is by id.
    #[serde(skip)]
    pub kept: Option<crate::rules::Selector>,
    /// "Window id" is a choice: the window, or the slot, has one.
    pub has_id: bool,
    /// Which windows of the app: any, the one with the id, or by title.
    pub by: By,
    /// How a title rule tests the title, with `words`.
    pub test: Test,
    /// A title rule's words: the window's whole title to begin with, for the user to cut down.
    #[serde(skip)]
    pub words: crate::edit::Field,
    /// The window's kind when it is a dialog or panel (`AXDialog`), or the slot's: the rule names
    /// it, so the slot holds only that kind.
    #[serde(skip)]
    pub kind: Option<String>,
    pub has_kind: bool,
    pub pick: crate::slot::Pick,
    pub matching: Matching,
    /// A slot already: the dialog edits it.
    pub editing: bool,
}

/// Why a window of its app is not a waiting slot's, in the order they are said and checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Miss {
    /// The window rules leave it alone: ignored, floated, a sheet, tiny or another kind.
    Rules,
    NoId,
    OtherId,
    OtherTitle,
    OtherKind,
    /// Its pick leaves it out: on another display.
    OtherDisplay,
    InPane,
    PassedOver,
    /// Another slot that pairs by size, in a larger pane, has it.
    BiggerPane,
    /// Another slot, one that names its window, offers or asked for it.
    OtherSlot,
}

impl Miss {
    /// Its code in the log: `no_id`.
    pub fn name(self) -> &'static str {
        match self {
            Miss::Rules => "rules",
            Miss::NoId => "no_id",
            Miss::OtherId => "other_id",
            Miss::OtherTitle => "other_title",
            Miss::OtherKind => "other_kind",
            Miss::OtherDisplay => "other_display",
            Miss::InPane => "in_pane",
            Miss::PassedOver => "passed_over",
            Miss::BiggerPane => "bigger_pane",
            Miss::OtherSlot => "other_slot",
        }
    }

    /// After a count of `n`: `1 has no window id`, `2 have no window id`.
    pub fn words(self, n: usize) -> &'static str {
        let (one, more) = match self {
            Miss::Rules => ("is left alone by the window rules", "are left alone by the window rules"),
            Miss::NoId => ("has no window id", "have no window id"),
            Miss::OtherId => ("has another window id", "have another window id"),
            Miss::OtherTitle => ("has another title", "have another title"),
            Miss::OtherKind => ("is another kind", "are another kind"),
            Miss::OtherDisplay => ("is on another display", "are on another display"),
            Miss::InPane => ("is in another pane", "are in other panes"),
            Miss::PassedOver => ("was passed over", "were passed over"),
            Miss::BiggerPane => ("went to a bigger pane", "went to bigger panes"),
            Miss::OtherSlot => ("went to another slot", "went to other slots"),
        };
        if n == 1 { one } else { more }
    }
}

/// Which windows of the app a slot waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum By {
    /// Any window of the app (with `Pick`, which of several).
    Any,
    /// The window with the app's own name for it (`AXIdentifier`).
    Id,
    /// A window whose title passes `Test` with the dialog's words.
    Title,
}

/// A choice in the dialog. A title test chooses the rule by title too.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Choice {
    By(By),
    Test(Test),
    Pick(crate::slot::Pick),
    Matching(Matching),
}

/// A window the dialog lists: whether the rule takes it, and what tells it apart.
pub struct Listed<'a> {
    pub seen: &'a Seen,
    pub taken: bool,
    pub detail: String,
}

impl Dialog {
    /// The id the slot waits for: its own when edited, else the window's.
    fn id(&self) -> Option<String> {
        let kept = self.kept.as_ref().and_then(|select| select.identifier.clone());
        kept.or_else(|| self.window.as_ref().and_then(|seen| seen.identifier.clone()))
    }

    /// An older slot's title beside its id: kept while the rule stays by id.
    fn kept_title(&self, by: By) -> Option<String> {
        let kept = self.kept.as_ref().filter(|kept| kept.identifier.is_some() && by == By::Id)?;
        kept.title.clone()
    }

    /// The slot of what is chosen.
    pub fn slot(&self) -> Slot {
        self.slot_by(self.by)
    }

    fn slot_by(&self, by: By) -> Slot {
        let title = (by == By::Title).then(|| self.test.glob(self.words.text()));
        let select = crate::rules::Selector {
            identifier: self.id().filter(|_| by == By::Id),
            title: title.or_else(|| self.kept_title(by)),
            subrole: self.kind.clone(),
            app: None,
        };
        Slot { select, pick: self.pick, matching: self.matching, ..Default::default() }
    }

    /// How many of the app's windows open now a rule `by` would select, whichever the pick: the
    /// count beside each choice, so loosening or tightening shows what it does before it is chosen.
    pub fn count(&self, by: By, rules: &[crate::rules::Rule]) -> usize {
        let slot = Slot { pick: crate::slot::Pick::Any, ..self.slot_by(by) };
        self.look.candidates(&slot, rules, |_| false).len()
    }

    /// How many the chosen rule and pick select now (on display 2: those there).
    pub fn matches(&self, rules: &[crate::rules::Rule]) -> usize {
        self.look.candidates(&self.slot(), rules, |_| false).len()
    }

    /// The app's windows open now, front to back, each ticked if the rule takes it (of several,
    /// the one its pick chooses), with what tells it apart. Titles drawn, never in the state.
    pub fn listed(&self, rules: &[crate::rules::Rule]) -> Vec<Listed<'_>> {
        let found = self.look.candidates(&self.slot(), rules, |_| false);
        let first = found.first().map(|seen| seen.window);
        let listed: Vec<crate::same_title::Listed> = self
            .look
            .windows
            .iter()
            .map(|s| crate::same_title::Listed {
                window: s.window,
                title: &s.title,
                frame: s.frame,
                subrole: &s.subrole,
            })
            .collect();
        let details = crate::same_title::details(&listed, &self.look.displays);
        self.look
            .windows
            .iter()
            .zip(details)
            .map(|(seen, detail)| {
                let size = format!("{}×{}", seen.frame.w.round(), seen.frame.h.round());
                let kind = crate::same_title::role(&seen.subrole).filter(|role| *role != "window");
                let plain = [Some(size), kind.map(String::from)].into_iter().flatten().collect::<Vec<_>>().join(" · ");
                Listed { seen, taken: Some(seen.window) == first, detail: detail.unwrap_or(plain) }
            })
            .collect()
    }

    /// The kind the rule names, in words: `panel`, `dialog`, else as macOS says it.
    pub fn kind_word(&self) -> Option<&str> {
        self.kind.as_deref().map(|kind| crate::same_title::role(kind).unwrap_or(kind))
    }

    /// The rule in one sentence, the dialog's headline: `Waits for any Meetings window, the
    /// largest, and offers it here.`
    pub fn headline(&self, app: &str) -> String {
        let words = self.words.text();
        let titled = |test: Test, words: &str| match test {
            Test::Is => format!("titled “{words}”"),
            test => format!("whose title {} “{words}”", test.label()),
        };
        let which = match self.by {
            By::Any => format!("any {app} window"),
            By::Id => match (self.id().unwrap_or_default(), self.kept_title(By::Id)) {
                (id, _) if id.is_empty() => format!("a {app} window with no window id"),
                (id, None) => format!("the {app} window with id {id}"),
                (id, Some(title)) => {
                    let (test, words) = Test::read(&title);
                    format!("the {app} window with id {id}, {}", titled(test, &words))
                }
            },
            By::Title => format!("a {app} window {}", titled(self.test, words)),
        };
        let kind = self.kind_word().map(|role| format!(" (a {role})")).unwrap_or_default();
        let pick = self.pick.words().map(|pick| format!(", {pick}")).unwrap_or_default();
        let does = match self.matching {
            Matching::Manual => "offers it here",
            Matching::Automatic => "takes it here",
        };
        // By id: that some apps move it, and when other windows have it now.
        let id = self.id().filter(|id| self.by == By::Id && !id.is_empty());
        let sharing = id
            .as_ref()
            .map_or(0, |id| self.look.windows.iter().filter(|seen| seen.identifier.as_ref() == Some(id)).count());
        let note = match (id, sharing) {
            (None, _) => String::new(),
            (Some(_), n) if n > 1 => format!(" {n} windows have this id now; some apps move it between their windows."),
            (Some(_), _) => " Some apps move a window id between their windows.".into(),
        };
        format!("Waits for {which}{kind}{pick}, and {does}.{note}")
    }
}

impl Model {
    /// The slot dialog's title words, while its rule is by title and the picker is not over it.
    pub(crate) fn slot_words(&self) -> Option<&crate::edit::Field> {
        // A bug report over the dialog has its own field.
        let open = |d: &&Dialog| d.by == By::Title && self.picker.is_none() && self.bug.is_none();
        let dialog = self.slot_dialog.as_ref().filter(open)?;
        Some(&dialog.words)
    }

    /// What a slot that waits says of its state, under its rule: its app is not running, its window
    /// is away, or every window it selects is already in another pane (waiting would read as broken).
    pub fn slot_note(&self, pane: PaneId) -> Option<String> {
        let run = self.slots.get(&pane)?;
        let app = &run.name;
        match run.status {
            Status::Absent => return Some(format!("{app} is not running")),
            Status::Away => return Some(format!("{app}'s window is away")),
            Status::Empty => {}
            Status::Offered | Status::Filled => return None,
        }
        let look = self.slot_looks.iter().find(|look| run.of(look))?;
        let held = |window: u32| self.in_pane(window);
        // Every window it selects, one it passed over too: held, not asked for.
        let found = look.candidates(&run.slot, &self.window_rules, |_| false);
        if !found.is_empty() && found.iter().all(|seen| held(seen.window)) {
            return Some("Its window is already in another pane".into());
        }
        // Else why each of the app's windows is not its own: `2 Zoom windows: 1 is in another pane; 1 has no window id`.
        let misses = self.slot_misses(pane);
        let windows: usize = misses.iter().map(|(_, n)| n).sum();
        let plural = if windows == 1 { "" } else { "s" };
        let said: Vec<String> = misses.iter().map(|(miss, n)| format!("{n} {}", miss.words(*n))).collect();
        (!said.is_empty()).then(|| format!("{windows} {app} window{plural}: {}", said.join("; ")))
    }

    /// True if a pane holds `window`.
    fn in_pane(&self, window: u32) -> bool {
        self.panes.iter().any(|p| matches!(p.kind, Kind::App { window: w, .. } if w == window))
    }

    /// The windows the slots of `group` hold.
    fn held_by(&self, group: &[PaneId]) -> Vec<u32> {
        let held = |id: &PaneId| match self.pane(*id).map(|p| &p.kind) {
            Some(Kind::App { window, .. }) => Some(*window),
            _ => None,
        };
        group.iter().filter_map(held).collect()
    }

    /// Why each window of its app's last look is not what waiting slot `pane` offers, counted by
    /// reason, in `Miss` order. Never a title: the shell logs it (`slot_miss`) and draws its words.
    pub fn slot_misses(&self, pane: PaneId) -> Vec<(Miss, usize)> {
        let Some(run) = self.slots.get(&pane).filter(|run| run.status == Status::Empty) else { return Vec::new() };
        let Some(look) = self.slot_looks.iter().find(|look| run.of(look) && look.running) else { return Vec::new() };
        let held = |window: u32| self.in_pane(window);
        let select = &run.slot.select;
        let one = |field: crate::rules::Selector, facts: &crate::rules::Facts| field.selects(facts);
        let mut counted: BTreeMap<Miss, usize> = BTreeMap::new();
        let id = crate::rules::Selector { identifier: select.identifier.clone(), ..Default::default() };
        let title = crate::rules::Selector { title: select.title.clone(), ..Default::default() };
        let kind = crate::rules::Selector { subrole: select.subrole.clone(), ..Default::default() };
        let picks: BTreeSet<u32> =
            look.candidates(&run.slot, &self.window_rules, |_| false).iter().map(|s| s.window).collect();
        for seen in &look.windows {
            let facts = look.facts(seen);
            let by = |selector: &crate::rules::Selector| !one(selector.clone(), &facts);
            let other = self.slots.iter().find(|(id, other)| {
                **id != pane && (other.offered == Some(seen.window) || other.pending == Some(seen.window))
            });
            let miss = if !crate::rules::slot_holds(&self.window_rules, &facts, &seen.shape()) {
                Miss::Rules
            } else if by(&id) {
                if seen.identifier.as_deref().is_none_or(str::is_empty) { Miss::NoId } else { Miss::OtherId }
            } else if by(&title) {
                Miss::OtherTitle
            } else if by(&kind) {
                Miss::OtherKind
            } else if !picks.contains(&seen.window) {
                Miss::OtherDisplay
            } else if held(seen.window) {
                Miss::InPane
            } else if run.declined.contains(&seen.window) {
                Miss::PassedOver
            } else if let Some((_, other)) = other {
                if Self::by_size(run) && Self::by_size(other) { Miss::BiggerPane } else { Miss::OtherSlot }
            } else {
                continue;
            };
            *counted.entry(miss).or_default() += 1;
        }
        counted.into_iter().collect()
    }

    /// The window slot `pane` offers, as its look found it: for the pane to draw, never the state.
    pub fn slot_offer(&self, pane: PaneId) -> Option<&Seen> {
        let run = self.slots.get(&pane)?;
        let look = self.slot_looks.iter().find(|look| run.of(look))?;
        look.windows.iter().find(|seen| Some(seen.window) == run.offered)
    }

    /// The slot ↵ brings a window into: the focused pane, a slot that offers one, while
    /// ottospace has the keyboard in normal mode. The shell sends `SlotAccept` for it.
    pub fn enter_accepts(&self) -> Option<PaneId> {
        let id = self.focused.filter(|_| self.mode() == super::Mode::Normal && !self.keys)?;
        let offers = self.slots.get(&id).is_some_and(|run| run.status == Status::Offered && run.offered.is_some());
        (offers && self.slot_dialog.is_none() && self.features.on(Feature::Slots)).then_some(id)
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
        run.or_else(|| self.slot_dormant.get(&id).map(|(_, _, slot)| slot.clone()))
    }

    /// A restored pane's record names a slot: it starts waiting, never filled by an old window id.
    pub(crate) fn restore_slot(&mut self, id: PaneId, record: &crate::marks::RecordKind) -> bool {
        let crate::marks::RecordKind::App { bundle, name, extra, .. } = record else { return false };
        let Some(value) = extra.get("slot") else { return false };
        let slot = serde_json::from_value::<Slot>(value.clone()).ok();
        // With the flag off, or written by a newer build: a plain app pane that keeps it as it was.
        let Some(slot) = slot.filter(|_| self.features.on(Feature::Slots)) else {
            self.slot_dormant.insert(id, (bundle.clone(), name.clone(), value.clone()));
            return false;
        };
        let Some(pane) = self.panes.iter_mut().find(|pane| pane.id == id) else { return false };
        pane.kind = Kind::Slot { name: name.clone(), bundle: bundle.clone() };
        self.slots.insert(id, Run::new(slot, bundle.clone(), name.clone()));
        true
    }

    pub(crate) fn slot_act(&mut self, action: Action, effects: &mut Vec<Effect>) {
        // The dialog's foot does its act and closes it.
        if let Action::SlotClear { pane } | Action::SlotLookAgain { pane } | Action::SlotGiveBack { pane } = &action
            && self.by_user
            && self.slot_dialog.as_ref().is_some_and(|dialog| dialog.pane == *pane)
        {
            self.slot_dialog = None;
        }
        match action {
            Action::SlotMake { pane, pid, slot } => self.slot_make(pane, pid, slot),
            Action::SlotDialogOpen { pane, look } => self.slot_dialog_open(pane, look),
            Action::SlotDialogSet { choice } => {
                if let Some(dialog) = self.slot_dialog.as_mut() {
                    match choice {
                        // No id to wait for: the choice is not there.
                        Choice::By(By::Id) if !dialog.has_id => {}
                        Choice::By(by) => dialog.by = by,
                        // A hand-written pattern is kept, never chosen.
                        Choice::Test(Test::Pattern) => {}
                        Choice::Test(test) => {
                            // Its words as they read under the test it had: `*a*` stays `a`.
                            if dialog.test == Test::Pattern {
                                dialog.words = crate::edit::Field::new(&Test::read(dialog.words.text()).1, 0);
                            }
                            (dialog.test, dialog.by) = (test, By::Title);
                        }
                        Choice::Pick(pick) => dialog.pick = pick,
                        Choice::Matching(matching) => dialog.matching = matching,
                    }
                }
            }
            Action::SlotDialogClose => self.slot_dialog = None,
            Action::SlotDialogKeep => {
                let Some(dialog) = self.slot_dialog.take() else { return };
                let slot = dialog.slot();
                match (self.slots.get(&dialog.pane), &dialog.window) {
                    // Settings the dialog has no row for (slice 5's) stay.
                    (Some(run), _) => {
                        let slot = Slot { absent: run.slot.absent, closed: run.slot.closed, ..slot };
                        self.slot_make(dialog.pane, run.pid.unwrap_or_default(), slot);
                    }
                    (None, Some(seen)) => self.slot_make(dialog.pane, seen.pid, slot),
                    // An app pane whose window the look did not list: nothing to make it from.
                    (None, None) => {}
                }
            }
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
                if self.live == Some(pane) {
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
            Action::SlotSwap { pane, with } => {
                if self.slot_group(pane).contains(&with) {
                    self.slot_swap(pane, with, effects);
                }
            }
            Action::SlotResort { pane } => self.slot_resort(pane, effects),
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

    /// Each app pane and its window: what a step put in a pane is what is new after it.
    pub(super) fn app_windows(&self) -> BTreeSet<(PaneId, u32)> {
        let window = |pane: &super::Pane| match pane.kind {
            Kind::App { window, .. } => Some((pane.id, window)),
            _ => None,
        };
        self.panes.iter().filter_map(window).collect()
    }

    /// After a step: a window it put in a pane is the user's place (`placed`), or a client's or
    /// the app's own. Panes gone are forgotten.
    pub(super) fn settle_placed(&mut self, held: &BTreeSet<(PaneId, u32)>, placed: bool) {
        for (id, _) in self.app_windows().difference(held) {
            match placed {
                true => self.not_placed.remove(id),
                false => self.not_placed.insert(*id),
            };
        }
        let panes = &self.panes;
        self.not_placed.retain(|id| panes.iter().any(|pane| pane.id == *id));
    }

    /// The slot a user's app pane becomes when its window goes: its app, and the window's own id
    /// (Zoom's `zm.meeting.window.main`: the next meeting's has it too), or none (`""`: a window
    /// with no id), and its kind if a dialog or panel; else any window of the app; the largest. Never
    /// its title: a meeting's changes. None: not the user's, or a window no slot holds (one ignored).
    fn kept_place(&self, pane: PaneId, was: Option<Seen>) -> Option<(i32, Slot)> {
        let placed = self.features.on(Feature::Slots) && !self.not_placed.contains(&pane);
        let Some(Kind::App { window, .. }) = self.pane(pane).map(|p| &p.kind).filter(|_| placed) else { return None };
        // The shell's facts, else what a look listed.
        let listed = self.slot_looks.iter().flat_map(|look| &look.windows).find(|seen| seen.window == *window);
        let was = was.or_else(|| listed.cloned());
        // A window no slot holds (one a rule ignores or floats) is no candidate (rule 4): a place
        // for one would wait for good.
        let Some(Kind::App { bundle, name, .. }) = self.pane(pane).map(|p| &p.kind) else { return None };
        let known = was.as_ref().filter(|seen| !seen.subrole.is_empty());
        let unmanaged = known.is_some_and(|seen| {
            let (title, subrole, identifier) = (&seen.title, &seen.subrole, seen.identifier.as_deref());
            let facts = crate::rules::Facts { app: name, bundle: bundle.as_deref(), title, subrole, identifier };
            !crate::rules::slot_holds(&self.window_rules, &facts, &seen.shape())
        });
        if unmanaged {
            return None;
        }
        // Its kind, a standard one too: a meeting's place is not offered the app's dialogs.
        let subrole = was.as_ref().map(|seen| seen.subrole.clone()).filter(|subrole| !subrole.is_empty());
        let (pid, identifier) = was.map_or((0, None), |seen| (seen.pid, Some(seen.identifier.unwrap_or_default())));
        let select = crate::rules::Selector { identifier, subrole, ..Default::default() };
        // Of several that match, the largest: a meeting's main window over its pop-outs.
        Some((pid, Slot { select, pick: crate::slot::Pick::Largest, ..Default::default() }))
    }

    /// `pane`'s window is gone. A slot waits for it (Away while its app runs), and so does a pane
    /// the user put it in, made a slot (`kept_place`); any other closes, as a `Close` does.
    pub(super) fn window_gone(
        &mut self,
        pane: PaneId,
        app_running: bool,
        at: u64,
        was: Option<Seen>,
        effects: &mut Vec<Effect>,
    ) {
        // The shell's alone: the user's key or click reports no window gone.
        if self.by_user {
            return;
        }
        // A slot that waits already: a late or second report changes nothing.
        if self.slots.contains_key(&pane) && !self.slot_filled(pane) {
            return;
        }
        if !self.slots.contains_key(&pane) {
            match self.kept_place(pane, was) {
                // The user's place: a slot, before any offer (Zoom's chat must not take a meeting's place).
                Some((pid, slot)) => self.slot_make(pane, pid, slot),
                // An app that remade its window: the pane waits, empty, offering the new one.
                None if self.offer_keeps(pane, effects) => return,
                None => return self.act(Action::Close { pane: Some(pane) }, effects),
            }
        }
        // The keyboard comes back to ottospace, as the shell is told (`Effect::Parked`).
        if self.live == Some(pane) {
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

    /// The user picked `kind` into slot `id` that waits, for its pane to take. A window of the
    /// slot's own app fills it, and it stays a slot (Zoom's quitting leaves it waiting); else it ends.
    pub(super) fn slot_picked(&mut self, id: PaneId, kind: &Kind) {
        let Some(run) = self.slots.get(&id) else { return };
        let window = match kind {
            Kind::App { bundle, name, window, .. } if run.of_app((bundle.as_deref(), name)) => *window,
            _ => {
                self.slots.remove(&id);
                return self.slot_emptied(id);
            }
        };
        // A window is in one pane at most: another holds it, and the slot keeps waiting.
        if self.panes.iter().any(|p| matches!(p.kind, Kind::App { window: w, .. } if w == window)) {
            return;
        }
        // Its process, for a return while Away; a window no look listed yet keeps the last, as `slot_fill` does.
        let pid = self.slot_looks.iter().flat_map(|look| &look.windows).find(|seen| seen.window == window);
        let pid = pid.map(|seen| seen.pid);
        let Some(run) = self.slots.get_mut(&id) else { return };
        (run.away, run.offered, run.known, run.pending) = (None, None, None, None);
        run.pid = pid.or(run.pid);
        // The user's own choice: passed over no more.
        run.declined.remove(&window);
        // Empty for `open` to fill; the slot stays.
        if let Some(pane) = self.panes.iter_mut().find(|p| p.id == id && matches!(p.kind, Kind::Slot { .. })) {
            pane.kind = Kind::Empty;
        }
    }

    /// A slot that ended while it waited: its pane is an empty one, and a mark of it stands for none.
    fn slot_emptied(&mut self, id: PaneId) {
        if let Some(pane) = self.panes.iter_mut().find(|p| p.id == id && matches!(p.kind, Kind::Slot { .. })) {
            pane.kind = Kind::Empty;
            self.marks.forget_pane(id);
        }
    }

    /// The slots of `pane`'s app on the tab on show, `pane` among them, in reading order; none
    /// asked a window of the shell. Swap and Re-sort move windows among them.
    pub fn slot_group(&self, pane: PaneId) -> Vec<PaneId> {
        let Some(run) = self.slots.get(&pane).filter(|run| run.pending.is_none()) else { return Vec::new() };
        let app = (run.bundle.as_deref(), run.name.as_str());
        let shown = self.layout.as_ref().map(super::Node::leaves).unwrap_or_default();
        let ours = |id: &PaneId| {
            let other = self.slots.get(id);
            other.is_some_and(|other| other.pending.is_none() && other.of_app(app))
        };
        let group: Vec<PaneId> = shown.into_iter().filter(ours).collect();
        if group.contains(&pane) { group } else { Vec::new() }
    }

    /// Whether Re-sort can pair `pane`'s app's slots: two or more, one holding a window, and the
    /// last look lists each held window with the size its app gave it (seen in no pane).
    pub fn slot_resorts(&self, pane: PaneId) -> bool {
        let group = self.slot_group(pane);
        let held = self.held_by(&group);
        let look = self.slots.get(&pane).and_then(|run| self.slot_looks.iter().find(|look| run.of(look)));
        let known = |w: &u32| {
            self.slot_natural.contains_key(w) && look.is_some_and(|l| l.windows.iter().any(|s| s.window == *w))
        };
        group.len() > 1 && !held.is_empty() && held.iter().all(known)
    }

    /// Slots `a` and `b` exchange what they hold: a window and a window, or a window and none (it
    /// moves, the other waits). The slots, their rules and what they passed over stay.
    fn slot_swap(&mut self, a: PaneId, b: PaneId, effects: &mut Vec<Effect>) {
        let held = |id| matches!(self.pane(id).map(|p| &p.kind), Some(Kind::App { .. }));
        if a == b || !(held(a) || held(b)) {
            return;
        }
        if self.live.is_some_and(|live| live == a || live == b) {
            self.leave(effects);
        }
        // A transcript of one stops: its source pane changes window.
        if self.listen.pane().is_some_and(|pane| pane == a || pane == b) {
            self.listen_end(None, None, effects);
        }
        let kind = |m: &Self, id| m.pane(id).map(|p| p.kind.clone());
        let (Some(ka), Some(kb)) = (kind(self, a), kind(self, b)) else { return };
        for (id, kind) in [(a, kb), (b, ka)] {
            if let Some(pane) = self.panes.iter_mut().find(|p| p.id == id) {
                (pane.kind, pane.fit) = (kind, None);
            }
        }
        let edited = |m: &Self, id| m.edited.contains(&id);
        let (ea, eb) = (edited(self, a), edited(self, b));
        self.edited.retain(|id| *id != a && *id != b);
        self.edited.extend(eb.then_some(a).into_iter().chain(ea.then_some(b)));
        let pid = |m: &Self, id| m.slots.get(&id).and_then(|run| run.pid);
        let (pa, pb) = (pid(self, a), pid(self, b));
        for (id, pid) in [(a, pb), (b, pa)] {
            if let Some(run) = self.slots.get_mut(&id) {
                run.pid = pid;
                run.waits();
                run.away = None;
            }
        }
        effects.push(Effect::SlotSwapped { a, b });
    }

    /// Re-sort by size: the windows `pane`'s app's slots hold, paired again as a look pairs them
    /// (those that name their window first, then the largest pane with the largest window, by the
    /// size the app gave each), and moved there by swaps. A window no slot of them selects stays.
    fn slot_resort(&mut self, pane: PaneId, effects: &mut Vec<Effect>) {
        let group = self.slot_group(pane);
        let Some(run) = self.slots.get(&pane) else { return };
        let held = self.held_by(&group);
        let Some(look) = self.slot_looks.iter().find(|look| run.of(look)) else { return };
        // Only the windows they hold, each free to go to any of them.
        let mut look = look.clone();
        look.windows.retain(|seen| held.contains(&seen.window));
        let held: BTreeSet<u32> = held.into_iter().collect();
        let targets = self.slot_pairs(&look, &group, &held);
        // Each slot in turn gets its window from whichever slot holds it now.
        for (id, window) in targets {
            let holder = group.iter().copied().find(
                |other| matches!(self.pane(*other).map(|p| &p.kind), Some(Kind::App { window: w, .. }) if *w == window),
            );
            if let Some(holder) = holder.filter(|holder| *holder != id) {
                self.slot_swap(id, holder, effects);
            }
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

    /// The dialog on app pane or slot `id`, as the slot is, else from the pane's window: by its id
    /// if it has one, else any window of the app, the largest (a meeting's title changes).
    fn slot_dialog_open(&mut self, id: PaneId, look: Look) {
        let window = match self.pane(id).map(|pane| &pane.kind) {
            Some(Kind::App { window, bundle, name, .. }) => {
                if !same_app((look.bundle.as_deref(), &look.name), (bundle.as_deref(), name)) {
                    return;
                }
                look.windows.iter().find(|seen| seen.window == *window).cloned()
            }
            Some(Kind::Slot { .. }) => None,
            _ => return,
        };
        let run = self.slots.get(&id);
        let kept = run.map(|run| run.slot.select.clone());
        let has_id = window.as_ref().is_some_and(|seen| seen.identifier.is_some())
            || kept.as_ref().is_some_and(|select| select.identifier.is_some());
        let title = window.as_ref().map(|seen| seen.title.clone()).unwrap_or_default();
        let (by, (test, words), kind) = match &kept {
            Some(select) => {
                let by = match (&select.identifier, &select.title) {
                    (Some(_), _) => By::Id,
                    (None, Some(_)) => By::Title,
                    (None, None) => By::Any,
                };
                let words = select.title.as_deref().map_or((Test::Is, title), Test::read);
                (by, words, select.subrole.clone())
            }
            None => {
                let kind = window.as_ref().and_then(|seen| crate::rules::slot_kind(&seen.subrole));
                // Any window of the app (several such slots pair by size): an id is a choice of
                // the user's, as some apps move it between their windows (the owner's Zoom).
                (By::Any, (Test::Is, title), kind)
            }
        };
        let (pick, matching) =
            run.map_or((crate::slot::Pick::Largest, Matching::Manual), |run| (run.slot.pick, run.slot.matching));
        let editing = run.is_some();
        let has_kind = kind.is_some();
        // No limit: a kept pattern is saved again whole.
        let words = crate::edit::Field::new(&words, 0);
        self.slot_dialog = Some(Dialog {
            pane: id,
            look,
            window,
            kept,
            has_id,
            by,
            test,
            words,
            kind,
            has_kind,
            pick,
            matching,
            editing,
        });
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
        // A slot owns no window: the one it holds, and each it holds later, goes back when it closes.
        if let Some(pane) = self.panes.iter_mut().find(|pane| pane.id == id) {
            pane.owned = false;
        }
        self.edited.retain(|edited| *edited != id);
    }

    /// `SlotNext`: the candidate after the one offered, in the pick's order, round again.
    fn slot_next(&mut self, pane: PaneId) {
        let Some(run) = self.slots.get(&pane) else { return };
        let Some(look) = self.slot_looks.iter().find(|look| run.of(look)) else { return };
        let claimed = self.slot_claims(Some(pane), true);
        let passed = |window: u32| claimed.contains(&window) || run.declined.contains(&window);
        let found: Vec<u32> = look.candidates(&run.slot, &self.window_rules, passed).iter().map(|s| s.window).collect();
        let at = found.iter().position(|window| Some(*window) == run.offered);
        let next = at.map_or(found.first(), |at| found.get((at + 1) % found.len())).copied();
        if let Some(run) = self.slots.get_mut(&pane) {
            run.offered = next.or(run.offered);
            run.picked = next.or(run.picked);
        }
    }

    /// Whether a slot pairs by size: it names no window (app only, or a kind), and takes any or
    /// the largest, nor is it Away (its own window comes back to it).
    fn by_size(run: &Run) -> bool {
        let select = &run.slot.select;
        let any = matches!(run.slot.pick, crate::slot::Pick::Any | crate::slot::Pick::Largest);
        select.identifier.is_none() && select.title.is_none() && any && run.away.is_none()
    }

    /// Which window each of `ids` (slots of `look`'s app that wait, on the tab on show, in reading
    /// order) offers or takes (`slot::pair`): those that name their window first (Away ones
    /// first of all), the most served; then the rest by size, the largest pane with the largest
    /// window as its app sized it, ties front to back. A window in a pane or asked for is no one's,
    /// but those `freed` (Re-sort's own).
    fn slot_pairs(&self, look: &Look, ids: &[PaneId], freed: &BTreeSet<u32>) -> BTreeMap<PaneId, u32> {
        let unit = crate::layout::Rect { x: 0., y: 0., w: 1000., h: 1000. };
        let rects = self.layout.as_ref().map(|layout| layout.rects(unit)).unwrap_or_default();
        let area = |id: PaneId| rects.iter().find(|(p, _)| *p == id).map_or(0., |(_, r)| r.w * r.h);
        let natural = |seen: &Seen| self.slot_natural.get(&seen.window).copied().unwrap_or(seen.frame.w * seen.frame.h);
        let at = |window: u32| look.windows.iter().position(|seen| seen.window == window);
        let mut order: Vec<PaneId> = ids.iter().copied().filter(|id| self.slots.contains_key(id)).collect();
        // Named first (Away ones before), in reading order; then by size, the largest pane first.
        order.sort_by(|a, b| {
            let (ra, rb) = (&self.slots[a], &self.slots[b]);
            let key = |run: &Run| (Self::by_size(run), run.away.is_none());
            let size = |id: &PaneId, run: &Run| if Self::by_size(run) { -area(*id) } else { 0. };
            key(ra).cmp(&key(rb)).then(size(a, ra).total_cmp(&size(b, rb)))
        });
        let wants: Vec<crate::slot::Want> = order
            .iter()
            .map(|id| {
                let run = &self.slots[id];
                let claimed = self.slot_claims(Some(*id), false);
                let passed = |window: u32| {
                    !freed.contains(&window) && (claimed.contains(&window) || run.declined.contains(&window))
                };
                let found = look.candidates(&run.slot, &self.window_rules, passed);
                let by_size = Self::by_size(run);
                let mut windows: Vec<&Seen> = found;
                if by_size {
                    windows.sort_by(|a, b| natural(b).total_cmp(&natural(a)).then(at(a.window).cmp(&at(b.window))));
                }
                let mut windows: Vec<u32> = windows.iter().map(|seen| seen.window).collect();
                // What the user's Next chose stays first, and a named slot's offer: an offer does
                // not jump while nothing needs it to.
                let kept = run.picked.or(run.offered.filter(|_| !by_size));
                if let Some(kept) = kept.filter(|w| windows.contains(w)) {
                    windows.retain(|w| *w != kept);
                    windows.insert(0, kept);
                }
                // A named slot: before all, one it would take now (its own back, or a new one for
                // Automatic). A size slot keeps size order: taking only decides host or offer.
                let takes =
                    |w: &u32| look.windows.iter().any(|seen| seen.window == *w && run.takes(seen, look.frontmost));
                if !by_size {
                    windows.sort_by_key(|w| !takes(w));
                }
                crate::slot::Want { windows, by_size }
            })
            .collect();
        let paired = crate::slot::pair(&wants);
        order.into_iter().zip(paired).filter_map(|(id, window)| Some((id, window?))).collect()
    }

    /// The windows no slot may offer: in a pane, asked for by a slot, or (`offers`) offered by
    /// another one (`but`: this one's own offer counts as free).
    fn slot_claims(&self, but: Option<PaneId>, offers: bool) -> BTreeSet<u32> {
        let held = self.panes.iter().filter_map(|pane| match pane.kind {
            Kind::App { window, .. } => Some(window),
            _ => None,
        });
        let asked = self
            .slots
            .iter()
            .filter(|(id, _)| Some(**id) != but)
            .flat_map(|(_, run)| run.pending.into_iter().chain(run.offered.filter(|_| offers)));
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
        // Each window's size as its app made it, while it is in no pane.
        let held: BTreeSet<u32> = self.slot_claims(None, false);
        if let Some(look) =
            self.slot_looks.iter().find(|l| same_app((l.bundle.as_deref(), &l.name), (app.0.as_deref(), &app.1)))
        {
            for seen in look.windows.iter().filter(|seen| !held.contains(&seen.window)) {
                self.slot_natural.insert(seen.window, seen.frame.w * seen.frame.h);
            }
        }
        let shown = self.layout.as_ref().map(super::Node::leaves).unwrap_or_default();
        let ours = |run: &Run| same_app((run.bundle.as_deref(), &run.name), (app.0.as_deref(), &app.1));
        let waiting: Vec<PaneId> = shown
            .iter()
            .copied()
            .filter(|id| !self.slot_filled(*id))
            .filter(|id| self.slots.get(id).is_some_and(|run| run.pending.is_none() && ours(run)))
            .collect();
        let Some(look) = self
            .slot_looks
            .iter()
            .find(|look| same_app((look.bundle.as_deref(), &look.name), (app.0.as_deref(), &app.1)))
            .cloned()
        else {
            return;
        };
        // Which window each offers or takes, the whole app's slots at once (several slots, §3).
        let pairs = if look.running { self.slot_pairs(&look, &waiting, &BTreeSet::new()) } else { BTreeMap::new() };
        for id in waiting {
            let Some(run) = self.slots.get(&id) else { continue };
            // Looked at: a plain pane of its app waits no more, and the app's windows once it
            // starts opened while the slot waited.
            if !look.running {
                self.slots.get_mut(&id).map(|run| run.known.get_or_insert_default());
                continue;
            }
            // A closed window is passed over no more: its id may come back as another window.
            let declined: BTreeSet<u32> = run.declined.intersection(&listed).copied().collect();
            let paired = pairs.get(&id).and_then(|w| look.windows.iter().find(|seen| seen.window == *w));
            let take = paired.filter(|seen| run.takes(seen, look.frontmost)).map(|seen| (seen.pid, seen.window));
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
    /// on show that waits and selects it takes it back (Away's return) or offers it; one adoption,
    /// no race with the slot's look. An Automatic slot takes a new one at its next look, which
    /// says whether the user is in it (`frontmost`). False: no slot selects it, and it is a new pane.
    pub(super) fn slot_adopt(&mut self, kind: &Kind, seen: Seen, effects: &mut Vec<Effect>) -> bool {
        let Kind::App { bundle, name, window, .. } = kind else { return false };
        if *window != seen.window {
            return false;
        }
        let Some((id, take)) = self.slot_choose((bundle.as_deref(), name), &seen) else { return false };
        // Else offered by the settle after this step, and still new to an Automatic slot.
        if take {
            self.slot_fill(id, kind.clone(), Some(seen.pid), effects);
        }
        true
    }

    /// A new window of a hosted app that would float with an offer (`Action::WindowOffered`): the
    /// first slot that selects it takes it back (Away's return: the shell is asked to host it) or
    /// offers it. `None`: no slot selects it, and its app's pane offers it.
    pub(super) fn slot_new_window(
        &mut self,
        app: (Option<&str>, &str),
        seen: &Seen,
        effects: &mut Vec<Effect>,
    ) -> Option<(PaneId, bool)> {
        let (id, take) = self.slot_choose(app, seen)?;
        if let Some(run) = self.slots.get_mut(&id).filter(|_| take) {
            run.pending = Some(seen.window);
            effects.push(Effect::HostSlot { pane: id, pid: seen.pid, window: seen.window });
        }
        Some((id, take))
    }

    /// Which slot on the tab on show, in reading order, a new window of `app` goes to, and whether
    /// it takes it now (Away's return) or offers it. The app's last look gains the window, in front.
    fn slot_choose(&mut self, app: (Option<&str>, &str), seen: &Seen) -> Option<(PaneId, bool)> {
        // The app's last look with this window in front: what the slots choose from.
        let at = self.slot_looks.iter().position(|look| same_app((look.bundle.as_deref(), &look.name), app));
        let mut look = at.map(|at| self.slot_looks[at].clone()).unwrap_or_else(|| Look {
            bundle: app.0.map(String::from),
            name: app.1.to_string(),
            ..Look::default()
        });
        look.running = true;
        look.windows.retain(|old| old.window != seen.window);
        look.windows.insert(0, seen.clone());
        let shown = self.layout.as_ref().map(super::Node::leaves).unwrap_or_default();
        let waiting: Vec<PaneId> = shown
            .into_iter()
            .filter(|id| !self.slot_filled(*id))
            .filter(|id| self.slots.get(id).is_some_and(|run| run.pending.is_none() && run.of(&look)))
            .collect();
        // The slot the pairing gives it, if any: one window, one slot.
        let pairs = self.slot_pairs(&look, &waiting, &BTreeSet::new());
        let id = pairs.iter().find(|(_, w)| **w == seen.window).map(|(id, _)| *id)?;
        // Only a return goes in now: whether the user is in a new window, the next look says.
        let back = self.slots.get(&id).is_some_and(|run| run.away.is_some() && run.takes(seen, look.frontmost));
        let chosen = (id, back);
        match at {
            Some(at) => self.slot_looks[at] = look,
            None => self.slot_looks.push(look),
        }
        Some(chosen)
    }

    /// After every step: each slot's status and offer, from the last looks, in the order of the
    /// tab on show's panes (the first slot that selects a window offers it). A slot on a hidden
    /// tab offers nothing and looks again when its tab shows. Then the apps to look at.
    pub(super) fn settle_slots(&mut self, effects: &mut Vec<Effect>) {
        let panes = &self.panes;
        self.slots.retain(|id, _| panes.iter().any(|pane| pane.id == *id));
        // The dialog goes with its pane (or what it holds: a slot cleared to empty), and with the flag.
        let on = self.features.on(Feature::Slots);
        let dialog_on = |d: &Dialog| {
            let kind = panes.iter().find(|pane| pane.id == d.pane).map(|pane| &pane.kind);
            on && matches!(kind, Some(Kind::App { .. } | Kind::Slot { .. }))
        };
        // A picker or the palette over it takes the keyboard: the dialog closes.
        let over = self.picker.is_some() || self.palette.is_some();
        self.slot_dialog = self.slot_dialog.take().filter(dialog_on).filter(|_| !over);
        // Kept while its pane waits or holds its app: another app put there takes no slot of it.
        self.slot_dormant.retain(|id, (bundle, name, _)| {
            panes.iter().find(|pane| pane.id == *id).is_some_and(|pane| match &pane.kind {
                Kind::Empty => true,
                Kind::App { bundle: b, name: n, .. } => same_app((b.as_deref(), n), (bundle.as_deref(), name)),
                _ => false,
            })
        });
        if !self.features.on(Feature::Slots) && !self.slots.is_empty() {
            for (id, run) in std::mem::take(&mut self.slots) {
                // Each keeps its slot on its record for the flag's return: a filled one stays an app
                // pane, one that waits is a plain app pane waiting, as a restore with the flag off.
                let slot = serde_json::to_value(&run.slot).ok();
                self.slot_dormant.extend(slot.map(|slot| (id, (run.bundle.clone(), run.name.clone(), slot))));
                if !self.slot_filled(id) {
                    let kind = RecordKind::App {
                        bundle: run.bundle,
                        name: run.name,
                        title: String::new(),
                        extra: Extra::new(),
                    };
                    let record = PaneRecord { id, kind, cwd: None, extra: Extra::new() };
                    self.marks.unfilled.push(Unfilled { pane: id, record, why: "not opened yet".into() });
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
        // A window is offered by one slot at a time: each app's waiting slots are paired at once.
        let order: Vec<PaneId> = shown.into_iter().filter(|id| self.slots.contains_key(id)).collect();
        let mut pairs: BTreeMap<PaneId, u32> = BTreeMap::new();
        for look in self.slot_looks.iter().filter(|look| look.running) {
            let waiting: Vec<PaneId> = order
                .iter()
                .copied()
                .filter(|id| !filled.contains(id))
                .filter(|id| self.slots.get(id).is_some_and(|run| run.pending.is_none() && run.of(look)))
                .collect();
            pairs.extend(self.slot_pairs(look, &waiting, &BTreeSet::new()));
        }
        for id in order {
            // What other slots offer is not this one's to count.
            let mut claimed = self.slot_claims(Some(id), false);
            claimed.extend(pairs.iter().filter(|(other, _)| **other != id).map(|(_, w)| *w));
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
                    let offered = pairs.get(&id).copied();
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
        // `N more match` counts what Next can offer: not the windows later slots offer.
        let offering: Vec<PaneId> =
            self.slots.iter().filter(|(_, run)| run.status == Status::Offered).map(|(id, _)| *id).collect();
        for id in offering {
            let claimed = self.slot_claims(Some(id), true);
            let Some(run) = self.slots.get(&id) else { continue };
            let Some(look) = self.slot_looks.iter().find(|look| run.of(look)) else { continue };
            let passed = |window: u32| claimed.contains(&window) || run.declined.contains(&window);
            let matches = look.candidates(&run.slot, &self.window_rules, passed).len();
            if let Some(run) = self.slots.get_mut(&id) {
                run.matches = matches;
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
    use super::Choice;
    use super::*;
    use crate::ax::Frame;
    use crate::layout::Axis::Right;
    use crate::layout::Node;
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
        m.update(system(Action::WindowGone { pane: 2, app_running: true, at: 0, was: None }));
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
        m.update(system(Action::WindowGone { pane: 2, app_running: true, at: 5, was: None }));
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
        m.update(system(Action::WindowGone { pane: 3, app_running: true, at: 6, was: None }));
        assert!(m.pane(3).is_none());
        // The user's pick into it (⌘E Put something else here) ends the slot: the pane is what was picked.
        m.update(Action::Focus { pane: 2 });
        let picker = |m: &mut Model| {
            let items = vec![crate::model::PickItem::Terminal];
            m.update(Action::OpenPicker { axis: Right, new_tab: false, items })
        };
        // A client's pick splits beside it.
        picker(&mut m);
        m.update(Action::PickerChoose { index: Some(0) }.sent_by(Origin::Socket));
        assert!(m.slots.contains_key(&2));
        m.update(Action::Focus { pane: 2 });
        picker(&mut m);
        m.update(Action::PickerChoose { index: Some(0) });
        assert_eq!((m.slots.contains_key(&2), m.pane(2).map(|p| p.kind.clone())), (false, Some(Kind::Terminal)));
    }

    /// The owner's case: Meetings' home window and a meeting window, each put in a pane by the
    /// user (no Keep). The meeting ends, then the app quits: neither pane goes, each waits as a slot
    /// that says what for, and the next meeting's window (another id and title) is offered in the
    /// meeting's place, the home window in the home's.
    #[test]
    fn a_window_the_user_put_in_a_pane_closing_leaves_a_slot_waiting_for_its_kind() {
        let main = |window, title: &str| Seen { identifier: Some("meetings.main".into()), ..seen(7, window, title) };
        let mut m = Model::default();
        m.update(Action::Open { kind: meetings(1, "Home"), axis: Right, beside: None });
        m.update(Action::Open { kind: meetings(2, "SECRET standup"), axis: Right, beside: None });
        m.update(system(look(vec![seen(7, 1, "Home"), main(2, "SECRET standup")])));
        let panes = m.panes.len();
        // The meeting ends: its window goes, Meetings runs on.
        let gone = Action::WindowGone { pane: 2, app_running: true, at: 0, was: Some(main(2, "SECRET standup")) };
        m.update(system(gone));
        let waits = Kind::Slot { name: "Meetings".into(), bundle: Some(BUNDLE.into()) };
        assert_eq!((m.panes.len(), m.pane(2).map(|p| &p.kind)), (panes, Some(&waits)));
        let words = m.slots.get(&2).map(|run| run.slot.words("Meetings"));
        assert_eq!(
            words.as_deref(),
            Some("Meetings · window id meetings.main · the largest"),
            "its kind, not its title"
        );
        m.update(system(Action::SlotTick { at: slot::AWAY_MS }));
        m.update(system(look(vec![seen(7, 1, "Home")])));
        assert_eq!(status(&m, 2), Some(Status::Empty), "the home window is not its kind");
        let state = serde_json::to_string(&m).unwrap();
        assert!(!state.contains("SECRET") && !state.contains("meetings.main"), "{state}");
        // The app quits: the home pane waits too, for a window with no id, and both say Meetings is not running.
        m.update(system(Action::WindowGone { pane: 1, app_running: false, at: 1, was: Some(seen(7, 1, "Home")) }));
        assert_eq!(m.panes.len(), panes);
        assert_eq!((status(&m, 1), status(&m, 2)), (Some(Status::Absent), Some(Status::Absent)));
        // It starts again, the new meeting in front: each place is offered its own kind, never the
        // app's chat (a dialog with no id, larger than Home): the home window was a standard one.
        let chat = Seen { subrole: "AXSystemDialog".into(), ..seen(8, 20, "Meeting chat") };
        m.update(system(look(vec![chat, main(12, "Retro"), seen(8, 11, "Home")])));
        assert_eq!(m.slot_offer(1).map(|s| s.window), Some(11));
        assert_eq!(m.slot_offer(2).map(|s| s.window), Some(12));
        // Only the user's Bring it here fills it.
        let effects = m.update(Action::SlotAccept { pane: 2 });
        assert_eq!(hosts(&effects), [(2, 12)]);
        m.update(system(Action::SlotHosted { pane: 2, kind: Some(meetings(12, "Retro")) }));
        assert_eq!(status(&m, 2), Some(Status::Filled));
        // As easy to be rid of as it came: the user's × closes the waiting one.
        m.update(Action::Close { pane: Some(1) });
        assert!(m.pane(1).is_none() && !m.slots.contains_key(&1));
    }

    /// The user's place wins over an offer in it (the app's chat must not take a meeting's place),
    /// and the user's own split of an offered window (an `Adopt` they sent) is their place too.
    #[test]
    fn the_users_place_is_kept_over_an_offer_and_their_split_of_one_is_theirs() {
        let mut m = Model::default();
        m.update(Action::Open { kind: meetings(1, "a"), axis: Right, beside: None });
        m.update(new_window(1, 7, 9, "chat"));
        m.update(system(Action::WindowGone { pane: 1, app_running: true, at: 0, was: Some(seen(7, 1, "a")) }));
        assert!(matches!(m.pane(1).map(|p| &p.kind), Some(Kind::Slot { .. })), "a slot, not an empty pane offering");
        m.update(Action::Adopt { kind: meetings(9, "chat"), beside: Some(1), seen: None });
        let split = m.panes.iter().find(|p| matches!(p.kind, Kind::App { window: 9, .. })).map(|p| p.id);
        let split = split.unwrap_or_default();
        m.update(system(Action::WindowGone { pane: split, app_running: true, at: 0, was: None }));
        assert!(matches!(m.pane(split).map(|p| &p.kind), Some(Kind::Slot { .. })), "the user's split");
    }

    /// The owner's case: Zoom's meeting chat is an `AXSystemDialog`. A slot kept from it (the
    /// dialog's Keep: any window of the app, its kind named) names that kind, and the next meeting's chat, a new window
    /// of that kind and title, is offered in it; so is a dialog to a slot by title that names no kind.
    #[test]
    fn a_slot_kept_from_a_dialog_matches_the_next_one_of_its_kind() {
        let chat = |window| Seen { subrole: "AXSystemDialog".into(), ..seen(7, window, "Meeting chat") };
        let mut m = Model::default();
        m.update(Action::Open { kind: meetings(1, "Meeting"), axis: Right, beside: None });
        m.update(Action::Open { kind: meetings(2, "Meeting chat"), axis: Right, beside: None });
        let windows = vec![seen(7, 1, "Meeting"), chat(2)];
        let found =
            Look { bundle: Some(BUNDLE.into()), name: "Meetings".into(), running: true, windows, ..Look::default() };
        m.update(Action::SlotDialogOpen { pane: 2, look: found });
        assert_eq!(m.slot_dialog.as_ref().map(|d| (d.by, d.kind_word())), Some((By::Any, Some("dialog"))));
        m.update(Action::SlotDialogKeep);
        let words = m.slots.get(&2).map(|run| run.slot.words("Meetings"));
        assert_eq!(words.as_deref(), Some("Meetings · subrole AXSystemDialog · the largest"));
        m.update(system(Action::WindowGone { pane: 2, app_running: true, at: 0, was: Some(chat(2)) }));
        m.update(system(Action::SlotTick { at: slot::AWAY_MS }));
        // The next meeting's chat, popped out again: a new window, offered in the slot, not in a chip.
        let effects = m.update(system(Action::WindowOffered { pane: 1, seen: chat(5), why: "dialog".into() }));
        assert!(m.slot_offer(2).is_some_and(|s| s.window == 5), "{effects:?}");
        // A slot by title (`*incident*`) is offered a dialog of that title: the picker lists it too.
        let mut plain = kept(Matching::Manual);
        plain.update(system(Action::WindowGone { pane: 2, app_running: true, at: 0, was: None }));
        plain.update(system(Action::SlotTick { at: slot::AWAY_MS }));
        let incident = Seen { subrole: "AXSystemDialog".into(), ..seen(7, 6, "Incident") };
        plain.update(system(look(vec![incident])));
        assert_eq!((status(&plain, 2), plain.slot_offer(2).map(|s| s.window)), (Some(Status::Offered), Some(6)));
        // A user's rule that ignores it keeps it out.
        let ignore = serde_json::from_str(r#"{"title": "Incident", "do": "ignore"}"#).unwrap();
        plain.window_rules.insert(0, ignore);
        plain.update(system(look(vec![Seen { subrole: "AXSystemDialog".into(), ..seen(7, 6, "Incident") }])));
        assert_eq!(status(&plain, 2), Some(Status::Empty));
    }

    /// The owner's two Zoom slots: one holds the meeting window, the other selects that window too.
    /// The second does not just wait: it says its window is already in another pane.
    #[test]
    fn a_slot_whose_window_is_in_another_pane_says_so() {
        let main = || Seen { identifier: Some("meetings.main".into()), ..seen(7, 1, "Weekly") };
        let mut m = waiting(Matching::Manual);
        m.update(Action::Open { kind: meetings(1, "Weekly"), axis: Right, beside: None });
        let by_id = crate::rules::Selector { identifier: Some("meetings.main".into()), ..Default::default() };
        m.update(Action::SlotMake { pane: 2, pid: 7, slot: Slot { select: by_id, ..Default::default() } });
        m.update(system(look(vec![main()])));
        assert_eq!(status(&m, 2), Some(Status::Empty));
        assert_eq!(m.slot_note(2).as_deref(), Some("Its window is already in another pane"));
        // No window it selects at all: it says why the one there is not.
        m.update(system(look(vec![seen(7, 4, "Home")])));
        assert_eq!(m.slot_note(2).as_deref(), Some("1 Meetings window: 1 has no window id"));
        // Its app quits: that is what it says.
        let quit = Look { bundle: Some(BUNDLE.into()), name: "Meetings".into(), ..Look::default() };
        m.update(system(Action::SlotWindows { look: quit }));
        assert_eq!(m.slot_note(2).as_deref(), Some("Meetings is not running"));
    }

    /// Not the user's place: a pane a client opened, or a window the app opened itself, closes with
    /// its window; so does every app pane with `slots` off.
    #[test]
    fn a_window_a_client_or_the_app_put_in_a_pane_closing_closes_it() {
        let gone = |pane, was| system(Action::WindowGone { pane, app_running: true, at: 0, was });
        let mut m = Model::default();
        m.update(Action::Open { kind: meetings(1, "a"), axis: Right, beside: None }.sent_by(Origin::Socket));
        m.update(gone(1, None));
        assert!(m.panes.is_empty(), "a client's");
        m.update(system(Action::Adopt { kind: meetings(2, "b"), beside: None, seen: None }));
        let adopted = m.panes.first().map_or(0, |pane| pane.id);
        assert!(matches!(m.pane(adopted).map(|p| &p.kind), Some(Kind::App { .. })));
        m.update(gone(adopted, None));
        assert!(m.panes.is_empty(), "the app's own");

        let mut m = Model::default();
        m.update(system(Action::SetFeatures { features: crate::features::Features::default() }));
        m.update(Action::Open { kind: meetings(1, "a"), axis: Right, beside: None });
        m.update(gone(m.focused.unwrap_or_default(), None));
        assert!(m.panes.is_empty(), "slots off");
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
        m.update(system(Action::WindowGone { pane: 2, app_running: true, at: 0, was: None }));
        m.update(system(look(Vec::new())));
        // Another process's window that matches is offered; the same process's goes back in.
        let effects = m.update(system(look(vec![seen(8, 5, "Incident 2")])));
        assert!(hosts(&effects).is_empty());
        let effects = m.update(system(look(vec![seen(8, 5, "Incident 2"), seen(7, 6, "Incident review")])));
        assert_eq!(hosts(&effects), [(2, 6)]);
        // The app quit instead: it waits for the app.
        let mut m = kept(Matching::Manual);
        m.update(system(Action::WindowGone { pane: 2, app_running: false, at: 0, was: None }));
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
        m.update(system(Action::WindowGone { pane: 3, app_running: true, at: 0, was: None }));
        m.update(system(Action::SlotTick { at: slot::AWAY_MS }));
        m.update(system(look(vec![seen(7, 2, "Incident review")])));
        assert_eq!((m.slots[&2].offered, m.slots[&3].offered), (Some(2), None));
        m.update(Action::SlotDecline { pane: 2 });
        assert_eq!((m.slots[&2].offered, m.slots[&3].offered), (None, Some(2)));
        // Passed over no more, the first offers it again; swapped, the other is first.
        m.update(Action::SlotLookAgain { pane: 2 });
        assert_eq!((m.slots[&2].offered, m.slots[&3].offered), (Some(2), None));
        m.update(Action::Focus { pane: 2 });
        m.update(Action::SwapDir { dir: crate::layout::Dir::Right });
        assert_eq!(m.layout.as_ref().map(crate::model::Node::leaves), Some(vec![1, 3, 2]));
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
        restore(file.clone(), &mut off);
        assert!(off.slots.is_empty() && off.panes.iter().all(|p| !matches!(p.kind, Kind::Slot { .. })));
        // Hosted as a plain pane, it keeps its slot on the record for the flag's return.
        let kept = off.slot_dormant.keys().next().copied().unwrap_or_default();
        off.update(system(Action::Fill { pane: kept, kind: meetings(1, "Incident review") }));
        let text = crate::layout_file::write(&off.layout_file(&[], &[], 0));
        assert!(text.contains(r#""slot": {"#) && text.contains("*incident*"), "{text}");
        // Another app put in its place takes no slot of Meetings with it.
        let mut other = Model::default();
        other.update(system(Action::SetFeatures { features: crate::features::Features::default() }));
        restore(file.clone(), &mut other);
        let kind = Kind::App { window: 9, title: "x".into(), bundle: Some("com.slack".into()), name: "Slack".into() };
        other.update(Action::Fill { pane: kept, kind });
        let text = crate::layout_file::write(&other.layout_file(&[], &[], 0));
        assert!(!text.contains("*incident*"), "{text}");
        // Turned off mid-run, a filled slot keeps it too, and so does one that waits.
        let mut m = kept_slot_turned_off();
        let text = crate::layout_file::write(&m.layout_file(&[], &[], 0));
        assert!(m.slots.is_empty() && text.contains(r#""slot": {"#), "{text}");
        let mut w = waiting(Matching::Manual);
        w.update(system(Action::SetFeatures { features: crate::features::Features::default() }));
        let text = crate::layout_file::write(&w.layout_file(&[], &[], 0));
        assert!(w.slots.is_empty() && text.contains("*incident*") && text.contains("Meetings"), "{text}");
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
        m.update(system(Action::WindowGone { pane: 2, app_running: true, at: 1, was: None }));
        assert_eq!(status(&m, 2), Some(Status::Empty));
    }

    #[test]
    fn a_window_two_slots_select_goes_to_the_first_even_when_the_second_would_take_it() {
        let mut m = waiting(Matching::Manual);
        m.update(Action::Open { kind: meetings(8, "Incident other"), axis: Right, beside: None });
        m.update(Action::SlotMake { pane: 3, pid: 7, slot: slot("incident*", Matching::Automatic) });
        m.update(system(Action::WindowGone { pane: 3, app_running: true, at: 0, was: None }));
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
        // Automatic, after its first look: offered, and the next look, which says whether the
        // user is in it, takes it.
        let offered = || {
            let mut m = waiting(Matching::Automatic);
            m.update(system(look(Vec::new())));
            assert!(!m.update(adopt(4, 7, "Incident review")).iter().any(|e| matches!(e, Effect::Opened(_))));
            assert_eq!((m.panes.len(), status(&m, 2)), (2, Some(Status::Offered)));
            m
        };
        assert_eq!(hosts(&offered().update(system(look(vec![seen(7, 4, "Incident review")])))), [(2, 4)]);
        let Action::SlotWindows { look: mut front } = look(vec![seen(7, 4, "Incident review")]) else { return };
        front.frontmost = Some(4);
        assert!(hosts(&offered().update(system(Action::SlotWindows { look: front }))).is_empty());
        // Away: its process's window comes back in, Manual too.
        let mut m = kept(Matching::Manual);
        m.update(system(Action::WindowGone { pane: 2, app_running: true, at: 0, was: None }));
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

    /// What the shell sends for a new window of Meetings beside pane `pane`: it would be offered.
    fn new_window(pane: PaneId, pid: i32, window: u32, title: &str) -> crate::model::Envelope {
        system(Action::WindowOffered { pane, seen: seen(pid, window, title), why: "no slot for this window".into() })
    }

    fn placed(effects: &[Effect]) -> Option<crate::model::offers::Placed> {
        effects.iter().find_map(|effect| match effect {
            Effect::Placed { placed, .. } => Some(*placed),
            _ => None,
        })
    }

    #[test]
    fn a_new_window_a_slot_selects_is_the_slots_not_the_chips() {
        use crate::model::offers::Placed;
        // A slot that waits for `*incident*`, beside its app's pane 3.
        let mut m = waiting(Matching::Manual);
        m.update(system(look(Vec::new())));
        m.update(Action::Open { kind: meetings(8, "Lunch"), axis: Right, beside: None });
        let effects = m.update(new_window(3, 7, 4, "Incident review"));
        assert_eq!(placed(&effects), Some(Placed::SlotOffer { slot: 2 }));
        assert!(m.offers.is_empty() && hosts(&effects).is_empty(), "Manual offers in the slot, nothing hosted");
        assert_eq!(m.slot_offer(2).map(|s| s.window), Some(4));
        // One it does not select: the app pane's chip.
        let effects = m.update(new_window(3, 7, 5, "Notes"));
        assert_eq!((placed(&effects), m.offers.len()), (Some(Placed::Offer), 1));
        // Away: its own process's window comes back, hosted at once.
        let mut m = kept(Matching::Manual);
        m.update(Action::Open { kind: meetings(8, "Lunch"), axis: Right, beside: None });
        m.update(system(Action::WindowGone { pane: 2, app_running: true, at: 0, was: None }));
        let effects = m.update(new_window(3, 7, 6, "Incident review"));
        assert_eq!(placed(&effects), Some(Placed::SlotReturn { slot: 2 }));
        assert_eq!(hosts(&effects), [(2, 6)]);
        // With the flag off: the chip, as with no slot.
        let mut m = waiting(Matching::Manual);
        m.update(Action::Open { kind: meetings(8, "Lunch"), axis: Right, beside: None });
        m.update(system(Action::SetFeatures { features: crate::features::Features::default() }));
        assert_eq!(placed(&m.update(new_window(3, 7, 4, "Incident review"))), Some(Placed::Offer));
    }

    /// The owner's call: "biggest pane gets biggest window, next biggest pane gets next biggest
    /// window". Slots that each select any window of the app pair by size: pane 1 is half the tab,
    /// panes 2 and 3 a quarter each (ties in reading order); window 13 is the widest.
    #[test]
    fn several_app_only_slots_take_its_windows_by_size() {
        let mut m = Model::default();
        m.update(Action::Open { kind: meetings(1, "One"), axis: Right, beside: None });
        m.update(Action::Open { kind: meetings(2, "Two"), axis: Right, beside: None });
        m.update(Action::Open { kind: meetings(3, "Three"), axis: Right, beside: None });
        let any = Slot { matching: Matching::Automatic, ..Slot::default() };
        for pane in 1..=3 {
            m.update(Action::SlotMake { pane, pid: 7, slot: any.clone() });
            m.update(system(Action::WindowGone { pane, app_running: true, at: 0, was: None }));
        }
        m.update(system(Action::SlotTick { at: slot::AWAY_MS }));
        m.update(system(look(Vec::new())));
        // Three windows open while they wait: the largest pane takes the largest, and so on.
        let windows = vec![seen(7, 11, "a"), seen(7, 12, "b"), seen(7, 13, "c")];
        let effects = m.update(system(look(windows)));
        assert_eq!(hosts(&effects), [(1, 13), (2, 12), (3, 11)]);
        for (pane, window) in [(1, 13), (2, 12), (3, 11)] {
            m.update(system(Action::SlotHosted { pane, kind: Some(meetings(window, "x")) }));
        }
        assert!(m.slots.values().all(|run| run.status == Status::Filled));
        // Manual: each offers a different one, never two the same.
        let mut m = Model::default();
        m.update(Action::Open { kind: meetings(1, "One"), axis: Right, beside: None });
        m.update(Action::Open { kind: meetings(2, "Two"), axis: Right, beside: None });
        for pane in 1..=2 {
            m.update(Action::SlotMake { pane, pid: 7, slot: Slot::default() });
            m.update(system(Action::WindowGone { pane, app_running: true, at: 0, was: None }));
        }
        m.update(system(Action::SlotTick { at: slot::AWAY_MS }));
        m.update(system(look(vec![seen(7, 11, "a"), seen(7, 12, "b")])));
        assert_eq!((m.slot_offer(1).map(|s| s.window), m.slot_offer(2).map(|s| s.window)), (Some(12), Some(11)));
    }

    /// The dialog's default: any window of the app, the largest (the owner: "a generic zoom window
    /// matcher"; never the whole title: a meeting's changes; an id only if the user picks it). Each choice counts the windows it
    /// would take; a title rule is a test and words, written as the selector's glob; Keep makes
    /// the slot the headline says.
    #[test]
    fn the_dialog_opens_on_the_id_or_any_window_and_keep_makes_the_slot_its_headline_says() {
        use crate::slot::Pick;
        let mut m = Model::default();
        // The pane's title is the shell's (an app pane carries it); the look's is the dialog's.
        m.update(Action::Open { kind: meetings(1, "Pane title"), axis: Right, beside: None });
        let main = Seen { identifier: Some("zm.meeting.window.main".into()), ..seen(7, 1, "Weekly sync") };
        let viewer = seen(7, 2, "Weekly sync");
        let home = seen(7, 3, "Home");
        let Action::SlotWindows { look: mut found } = look(vec![viewer, main, home]) else { return };
        found.windows.iter_mut().for_each(|s| s.frame.w = 900.);
        let open = Action::SlotDialogOpen { pane: 1, look: found.clone() };
        // Only the user opens it.
        assert_eq!(m.refusal(&open, Origin::Socket), Some(slot::USERS_ALONE));
        m.update(open);
        let dialog = m.slot_dialog.clone().unwrap_or_else(|| panic!("open"));
        // Any window of the app, even with an id (some apps move it); the largest of several; the
        // look in its own order, front to back.
        assert_eq!((dialog.has_id, dialog.by, dialog.pick), (true, By::Any, Pick::Largest));
        assert_eq!(dialog.look.windows.iter().map(|s| s.window).collect::<Vec<_>>(), [2, 1, 3]);
        assert_eq!(dialog.headline("Meetings"), "Waits for any Meetings window, the largest, and offers it here.");
        // By id is the user's choice, and says that some apps move it.
        let by_id = Dialog { by: By::Id, ..dialog.clone() };
        assert_eq!(
            by_id.headline("Meetings"),
            "Waits for the Meetings window with id zm.meeting.window.main, the largest, and offers it here. \
             Some apps move a window id between their windows."
        );
        // Each choice says how many it would take.
        let counts = [By::Any, By::Id, By::Title].map(|by| dialog.count(by, &[]));
        assert_eq!(counts, [3, 1, 2]);
        let taken: Vec<(u32, bool)> = dialog.listed(&[]).iter().map(|l| (l.seen.window, l.taken)).collect();
        // Of three as large, the front one.
        assert_eq!(taken, [(2, true), (1, false), (3, false)]);
        // On display 2: none there; the status counts what the pick leaves.
        m.update(Action::SlotDialogSet { choice: Choice::Pick(Pick::Display(2)) });
        assert_eq!(m.slot_dialog.as_ref().map(|d| (d.count(By::Id, &[]), d.matches(&[]))), Some((1, 0)));
        m.update(Action::SlotDialogSet { choice: Choice::Pick(Pick::Largest) });
        // A title test chooses the rule by title: starting with “Weekly”, the selector's glob.
        m.update(Action::SlotDialogSet { choice: Choice::Test(Test::Is) });
        assert_eq!(m.slot_dialog.as_ref().map(|d| d.by), Some(By::Title));
        assert_eq!(m.field().map(|(of, f)| (of, f.text().to_string())), Some(("slot", "Weekly sync".into())));
        let typed = Action::FieldEdit { edit: crate::edit::Edit::SelectAll };
        assert_eq!(m.refusal(&typed, Origin::Socket), Some(slot::USERS_ALONE));
        m.update(typed);
        m.update(Action::FieldEdit { edit: crate::edit::Edit::Insert("Weekly".into()) });
        m.update(Action::SlotDialogSet { choice: Choice::Test(Test::StartsWith) });
        m.update(Action::SlotDialogSet { choice: Choice::Matching(Matching::Automatic) });
        let dialog = m.slot_dialog.clone().unwrap_or_else(|| panic!("open"));
        assert_eq!(dialog.slot().select.title.as_deref(), Some("Weekly*"));
        assert_eq!(dialog.matches(&[]), 2);
        assert_eq!(
            dialog.headline("Meetings"),
            "Waits for a Meetings window whose title starts with “Weekly”, the largest, and takes it here."
        );
        // The state has what is chosen, never the title, the id or the words.
        let state = serde_json::to_string(&m).unwrap_or_default();
        assert!(!state.contains("Weekly") && !state.contains("zm.meeting") && state.contains(r#""by":"title""#));
        m.update(Action::SlotDialogKeep);
        assert!(m.slot_dialog.is_none() && m.field().is_none());
        assert_eq!(
            m.slots.get(&1).map(|run| (run.slot.select.title.clone(), run.status, run.matching)),
            Some((Some("Weekly*".into()), Status::Filled, Matching::Automatic))
        );
        // Opened again on the slot it edits: as the slot is.
        let Action::SlotWindows { look: again } = look(vec![seen(7, 1, "Weekly sync")]) else { return };
        m.update(Action::SlotDialogOpen { pane: 1, look: again });
        let dialog = m.slot_dialog.clone().unwrap_or_else(|| panic!("open"));
        assert_eq!(
            (dialog.editing, dialog.by, dialog.test, dialog.words.text()),
            (true, By::Title, Test::StartsWith, "Weekly")
        );
        // An older slot with an id and a title beside it keeps both while its rule stays by id.
        let both = crate::rules::Selector {
            identifier: Some("main".into()),
            title: Some("*sync*".into()),
            ..Default::default()
        };
        m.update(Action::SlotMake { pane: 1, pid: 7, slot: Slot { select: both.clone(), ..Slot::default() } });
        let Action::SlotWindows { look: again } = look(vec![seen(7, 1, "Weekly sync")]) else { return };
        m.update(Action::SlotDialogOpen { pane: 1, look: again });
        let headline = m.slot_dialog.as_ref().map(|d| d.headline("Meetings"));
        assert_eq!(
            headline.as_deref(),
            Some(
                "Waits for the Meetings window with id main, whose title contains “sync”, and offers it here. \
                 Some apps move a window id between their windows."
            )
        );
        m.update(Action::SlotDialogKeep);
        assert_eq!(m.slots.get(&1).map(|run| run.slot.select.clone()), Some(both));
        // A window with no id: any window of the app, the largest; Window id is no choice.
        let mut m = Model::default();
        m.update(Action::Open { kind: meetings(1, "Pane title"), axis: Right, beside: None });
        let Action::SlotWindows { look: found } = look(vec![seen(7, 1, "Pricing review"), seen(7, 2, "Home")]) else {
            return;
        };
        m.update(Action::SlotDialogOpen { pane: 1, look: found });
        m.update(Action::SlotDialogSet { choice: Choice::By(By::Id) });
        let dialog = m.slot_dialog.clone().unwrap_or_else(|| panic!("open"));
        assert_eq!((dialog.has_id, dialog.by, dialog.pick), (false, By::Any, Pick::Largest));
        assert_eq!(dialog.headline("Meetings"), "Waits for any Meetings window, the largest, and offers it here.");
        m.update(Action::SlotDialogKeep);
        assert_eq!(
            m.slots.get(&1).map(|run| run.slot.words("Meetings")).as_deref(),
            Some("Meetings · any window · the largest")
        );
    }

    #[test]
    fn a_title_glob_reads_back_as_its_test_and_words() {
        assert_eq!(Test::read("*Incident*"), (Test::Contains, "Incident".into()));
        assert_eq!(Test::read("Weekly*"), (Test::StartsWith, "Weekly".into()));
        assert_eq!(Test::read("Weekly sync"), (Test::Is, "Weekly sync".into()));
        assert_eq!(Test::read("*Q*review*"), (Test::Pattern, "*Q*review*".into()));
        for test in [Test::Is, Test::Contains, Test::StartsWith] {
            assert_eq!(Test::read(&test.glob("standup")), (test, "standup".into()));
        }
    }

    /// Review: editing a slot keeps its own pattern and its hand-written `subrole`.
    #[test]
    fn editing_a_slot_keeps_its_pattern_and_the_rows_it_has_none_for() {
        let mut m = kept(Matching::Manual);
        let mut select = crate::rules::Selector { title: Some("*incident*".into()), ..Default::default() };
        select.subrole = Some("AXStandardWindow".into());
        m.update(Action::SlotMake { pane: 2, pid: 7, slot: Slot { select, ..Slot::default() } });
        let Action::SlotWindows { look: found } = look(vec![seen(7, 1, "Incident review 11/18")]) else { return };
        m.update(Action::SlotDialogOpen { pane: 2, look: found });
        m.update(Action::SlotDialogSet { choice: Choice::Matching(Matching::Automatic) });
        let words = m.slot_dialog.as_ref().map(|d| (d.test, d.slot().words("Meetings")));
        let said = "Meetings · title contains “incident”";
        assert_eq!(words, Some((Test::Contains, said.to_string())));
        m.update(Action::SlotDialogKeep);
        let run = &m.slots[&2];
        assert_eq!(
            (run.slot.select.title.as_deref(), run.slot.select.subrole.as_deref()),
            (Some("*incident*"), Some("AXStandardWindow"))
        );
        assert_eq!((run.matching, run.pid), (Matching::Automatic, Some(7)));
    }

    /// Review: a second new window goes to the next slot that waits, and the log says that one.
    #[test]
    fn a_second_new_window_is_the_next_slots_and_placed_says_so() {
        use crate::model::offers::Placed;
        let mut m = Model::default();
        for (window, title) in [(1, "One"), (2, "Two"), (3, "Lunch")] {
            m.update(Action::Open { kind: meetings(window, title), axis: Right, beside: None });
        }
        for pane in 1..=2 {
            m.update(Action::SlotMake { pane, pid: 7, slot: Slot::default() });
            m.update(system(Action::WindowGone { pane, app_running: true, at: 0, was: None }));
        }
        m.update(system(Action::SlotTick { at: slot::AWAY_MS }));
        m.update(system(look(Vec::new())));
        assert_eq!(placed(&m.update(new_window(3, 8, 11, "a"))), Some(Placed::SlotOffer { slot: 1 }));
        // A larger one: the larger pane's offer, and the first moves to the next (offers pair by size).
        assert_eq!(placed(&m.update(new_window(3, 8, 12, "b"))), Some(Placed::SlotOffer { slot: 1 }));
        assert_eq!((m.slot_offer(1).map(|s| s.window), m.slot_offer(2).map(|s| s.window)), (Some(12), Some(11)));
        // Each counts what Next could offer it: not the other's.
        assert_eq!((m.slots[&1].matches, m.slots[&2].matches), (1, 1));
    }

    /// Review: the picker or the palette over the dialog closes it; a pick that opens nothing
    /// (a cancelled file panel) leaves a slot that waits as it was; Keep with no window makes none.
    #[test]
    fn the_dialog_gives_way_to_the_picker_and_a_pick_ends_a_slot_only_once_it_opens() {
        let mut m = waiting(Matching::Manual);
        m.update(Action::SlotDialogOpen { pane: 2, look: Look::default() });
        assert!(m.slot_dialog.is_some());
        let items = vec![crate::model::PickItem::File, crate::model::PickItem::Terminal];
        m.update(Action::OpenPicker { axis: Right, new_tab: false, items: items.clone() });
        assert!(m.slot_dialog.is_none());
        // File: the shell asks which; cancelled, nothing opens and the slot stays.
        m.update(Action::PickerChoose { index: Some(0) });
        assert!(m.slots.contains_key(&2) && matches!(m.pane(2).map(|p| &p.kind), Some(Kind::Slot { .. })));
        // Picked and opened: the slot ends, its pane holds it.
        m.update(Action::Focus { pane: 2 });
        m.update(Action::OpenPicker { axis: Right, new_tab: false, items });
        m.update(Action::PickerChoose { index: Some(0) });
        m.update(Action::Open { kind: Kind::File { path: "/tmp/a.md".into() }, axis: Right, beside: None });
        assert_eq!(m.pane(2).map(|p| p.kind.clone()), Some(Kind::File { path: "/tmp/a.md".into() }));
        assert!(!m.slots.contains_key(&2));
        // An app pane whose window no look listed: Keep makes no slot.
        let mut m = Model::default();
        m.update(Action::Open { kind: meetings(1, "x"), axis: Right, beside: None });
        m.update(Action::SlotDialogOpen {
            pane: 1,
            look: Look { name: "Meetings".into(), bundle: Some(BUNDLE.into()), ..Look::default() },
        });
        m.update(Action::SlotDialogKeep);
        assert!(m.slots.is_empty());
    }

    /// slots.md: `Bring it here ↵`. The user's ↵ on a focused slot that offers a window accepts it.
    #[test]
    fn enter_on_a_focused_slot_that_offers_brings_its_window_in() {
        let mut m = waiting(Matching::Manual);
        assert_eq!(m.enter_accepts(), None, "nothing offered");
        m.update(system(look(vec![seen(7, 4, "Incident review")])));
        m.update(Action::Focus { pane: 2 });
        assert_eq!(m.enter_accepts(), Some(2));
        let effects = m.update(Action::SlotAccept { pane: 2 });
        assert_eq!(hosts(&effects), [(2, 4)]);
        // Not on another pane, nor with the dialog open.
        m.update(Action::Focus { pane: 1 });
        assert_eq!(m.enter_accepts(), None);
        m.update(Action::Focus { pane: 2 });
        m.update(Action::SlotDialogOpen { pane: 2, look: Look::default() });
        assert_eq!(m.enter_accepts(), None);
    }

    /// The owner's report: Zoom's windows picked into its slots (⌘E), then Zoom quit, and the panes
    /// closed: the pick had ended each slot. A window of its own app fills a slot, which waits for
    /// the app (Absent) when it quits, and offers its windows when it starts again.
    #[test]
    fn an_app_picked_into_its_slots_then_quitting_leaves_each_waiting_for_it() {
        let mut m = Model::default();
        for (pane, title) in [(1, "One"), (2, "Two"), (3, "Three")] {
            m.update(Action::Open { kind: meetings(pane, title), axis: Right, beside: None });
            m.update(Action::SlotMake { pane, pid: 7, slot: Slot::default() });
            m.update(system(Action::WindowGone { pane, app_running: true, at: 0, was: None }));
        }
        m.update(system(Action::SlotTick { at: slot::AWAY_MS }));
        m.update(system(look(vec![seen(7, 11, "a"), seen(7, 12, "b"), seen(7, 13, "c")])));
        // Not this one (the largest, 13), then picked anyway: the user's pick wins.
        m.update(Action::SlotDecline { pane: 1 });
        // The user's pick into each: its app's window fills it; it stays a slot.
        for (pane, window) in [(1, 13), (2, 12), (3, 11)] {
            pick_into(&mut m, pane, "Meetings", meetings(window, "x"));
            assert_eq!(m.pane(pane).map(|p| p.kind.clone()), Some(meetings(window, "x")));
            assert_eq!(status(&m, pane), Some(Status::Filled), "slot {pane}");
        }
        assert!(m.slots[&1].declined.is_empty());
        // Its window goes while the app runs, and one of the same process comes back: it goes in.
        m.update(system(Action::WindowGone { pane: 1, app_running: true, at: 0, was: None }));
        let back = vec![seen(7, 12, "b"), seen(7, 11, "c"), seen(7, 14, "a")];
        assert_eq!(hosts(&m.update(system(look(back)))), [(1, 14)]);
        m.update(system(Action::SlotHosted { pane: 1, kind: Some(meetings(14, "a")) }));
        assert_eq!(status(&m, 1), Some(Status::Filled));
        // A window it opened as the meeting ended, offered on a slot's chip.
        m.update(system(Action::WindowOffered { pane: 3, seen: seen(7, 9, "Ended"), why: "x".into() }));
        // The process lingers for the first window's report, not the others'.
        for (pane, running) in [(1, true), (2, false), (3, false)] {
            m.update(system(Action::WindowGone { pane, app_running: running, at: 0, was: None }));
        }
        let gone = Look { bundle: Some(BUNDLE.into()), name: "Meetings".into(), ..Default::default() };
        m.update(system(Action::SlotWindows { look: gone }));
        m.update(system(Action::SlotTick { at: 2 * slot::AWAY_MS }));
        assert_eq!(m.panes.len(), 3);
        for pane in 1..=3 {
            assert_eq!(status(&m, pane), Some(Status::Absent), "slot {pane}");
        }
        // Started again (another process): each offers one of its windows, and takes it on Bring it here.
        m.update(system(look(vec![seen(8, 21, "a"), seen(8, 22, "b"), seen(8, 23, "c")])));
        for (pane, window) in [(1, 23), (2, 22), (3, 21)] {
            assert_eq!(m.slot_offer(pane).map(|s| s.window), Some(window), "slot {pane}");
            m.update(Action::SlotAccept { pane });
            m.update(system(Action::SlotHosted { pane, kind: Some(meetings(window, "x")) }));
            assert_eq!(status(&m, pane), Some(Status::Filled), "slot {pane}");
        }
        // Another app's window picked into a slot still ends it.
        m.update(system(Action::WindowGone { pane: 1, app_running: true, at: 0, was: None }));
        let other =
            Kind::App { window: 50, title: "x".into(), bundle: Some("com.example.other".into()), name: "Other".into() };
        pick_into(&mut m, 1, "Other", other.clone());
        assert_eq!((m.slots.contains_key(&1), m.pane(1).map(|p| p.kind.clone())), (false, Some(other)));
    }

    /// A meeting app's windows as the owner's Zoom had them, front to back: the main meeting window
    /// (880x862, its id), a second window of the same title (875x336, no id), and the chat, a dialog.
    fn meeting_windows() -> Vec<Seen> {
        let sized = |window, title: &str, w, h| Seen { frame: Frame::new(0., 0., w, h), ..seen(7, window, title) };
        let main = Seen { identifier: Some("meetings.main".into()), ..sized(21, "Team Meeting", 880., 862.) };
        let chat = Seen { subrole: "AXSystemDialog".into(), ..sized(23, "Meeting chat", 400., 532.) };
        vec![main, sized(22, "Team Meeting", 875., 336.), chat]
    }

    /// Panes 1, 2 and 3 of Meetings, each a slot of `slots[n]` that waits, laid out with pane 1 the
    /// largest (0.6 of the tab), then 2 (0.28), then 3 (0.12).
    fn three_slots(slots: [Slot; 3]) -> Model {
        let mut m = Model::default();
        for (pane, slot) in (1..=3).zip(slots) {
            m.update(Action::Open { kind: meetings(pane, "x"), axis: Right, beside: None });
            m.update(Action::SlotMake { pane, pid: 7, slot });
            m.update(system(Action::WindowGone { pane, app_running: true, at: 0, was: None }));
        }
        let leaf = |id| Box::new(Node::Leaf(id));
        let right = Node::Split { axis: crate::Axis::Down, ratio: 0.7, a: leaf(2), b: leaf(3) };
        m.layout = Some(Node::Split { axis: crate::Axis::Right, ratio: 0.6, a: leaf(1), b: Box::new(right) });
        m.update(system(Action::SlotTick { at: slot::AWAY_MS }));
        m.update(system(look(Vec::new())));
        m
    }

    fn offers(m: &Model) -> Vec<Option<u32>> {
        (1..=3).map(|pane| m.slot_offer(pane).map(|s| s.window)).collect()
    }

    fn select(json: &str, matching: Matching) -> Slot {
        Slot { matching, ..serde_json::from_str(json).unwrap() }
    }

    /// The owner's three Zoom slots: A by title (both meeting windows, the largest), B by id (only
    /// the main window), C the chat. A specific slot is served first and the most of them: B gets
    /// the main window, A the other of its title, C the chat. Automatic slots take the same.
    #[test]
    fn slots_of_one_app_do_not_starve_each_other() {
        for matching in [Matching::Manual, Matching::Automatic] {
            let a = select(r#"{"select": {"title": "Team Meeting"}, "pick": "largest"}"#, matching);
            let b = select(r#"{"select": {"identifier": "meetings.main"}, "pick": "any"}"#, matching);
            let c = select(r#"{"select": {"title": "Meeting chat"}}"#, matching);
            let mut m = three_slots([a, b, c]);
            let effects = m.update(system(look(meeting_windows())));
            match matching {
                Matching::Manual => assert_eq!(offers(&m), [Some(22), Some(21), Some(23)]),
                Matching::Automatic => assert_eq!(hosts(&effects), [(1, 22), (2, 21), (3, 23)]),
            }
        }
    }

    /// The owner's rule for app-only slots: "biggest pane gets biggest window, next biggest pane
    /// gets next biggest window". The windows' own areas, the dialog too; ties front to back.
    #[test]
    fn app_only_slots_pair_the_largest_pane_with_the_largest_window() {
        let any = || Slot::default();
        let mut m = three_slots([any(), any(), any()]);
        // Listed smallest first: the order is by size, not by place in the list.
        let mut windows = meeting_windows();
        windows.reverse();
        m.update(system(look(windows)));
        assert_eq!(offers(&m), [Some(21), Some(22), Some(23)]);
        // Equal windows: front to back.
        let mut m = three_slots([any(), any(), any()]);
        let same = |window| Seen { frame: Frame::new(0., 0., 500., 500.), ..seen(7, window, "w") };
        m.update(system(look(vec![same(33), same(31), same(32)])));
        assert_eq!(offers(&m), [Some(33), Some(31), Some(32)]);
        // A named slot is served first; the app-only ones share what it leaves, by size.
        let c = select(r#"{"select": {"title": "Team*"}, "pick": "smallest"}"#, Matching::Manual);
        let mut m = three_slots([any(), any(), c]);
        m.update(system(look(meeting_windows())));
        assert_eq!(offers(&m), [Some(21), Some(23), Some(22)]);
    }

    /// Next on a slot that pairs by size: the window it chose stays its offer at the next look.
    #[test]
    fn next_on_an_app_only_slot_keeps_the_window_it_chose() {
        let mut m = three_slots([Slot::default(), Slot::default(), Slot::default()]);
        m.update(Action::SlotClear { pane: 2 });
        m.update(Action::SlotClear { pane: 3 });
        m.update(system(look(meeting_windows())));
        assert_eq!(m.slot_offer(1).map(|s| s.window), Some(21));
        m.update(Action::SlotNext { pane: 1 });
        m.update(system(look(meeting_windows())));
        assert_eq!(m.slot_offer(1).map(|s| s.window), Some(22));
    }

    /// Two windows, then a larger third. Offers pair again by size, but a window in a pane stays.
    #[test]
    fn a_window_in_a_slot_is_not_moved_for_a_larger_one() {
        let any = || Slot::default();
        let mut m = three_slots([any(), any(), any()]);
        let windows = meeting_windows();
        m.update(system(look(windows[1..].to_vec())));
        assert_eq!(offers(&m), [Some(22), Some(23), None]);
        m.update(Action::SlotAccept { pane: 1 });
        m.update(system(Action::SlotHosted { pane: 1, kind: Some(meetings(22, "x")) }));
        // The main window opens, larger: the largest pane keeps its window; the next one offers it.
        m.update(system(look(windows)));
        assert_eq!((status(&m, 1), offers(&m)), (Some(Status::Filled), vec![None, Some(21), Some(23)]));
    }

    /// The owner's B: by id, its window picked into A by the user. It says its window is in another
    /// pane; with that window closed, why each other window is not its own. Never a title.
    #[test]
    fn a_waiting_slot_says_why_each_window_is_not_its_own() {
        let a = select(r#"{"select": {"title": "Team Meeting"}, "pick": "largest"}"#, Matching::Manual);
        let b = select(r#"{"select": {"identifier": "meetings.main"}}"#, Matching::Manual);
        let c = select(r#"{"select": {"title": "Meeting chat"}}"#, Matching::Manual);
        let mut m = three_slots([a, b, c]);
        m.update(system(look(meeting_windows())));
        pick_into(&mut m, 1, "Meetings", meetings(21, "x"));
        m.update(system(look(meeting_windows())));
        assert_eq!(m.slot_note(2).as_deref(), Some("Its window is already in another pane"));
        assert_eq!(m.slot_misses(2), [(Miss::NoId, 2), (Miss::InPane, 1)]);
        // The main window closes: the other two have no id.
        m.update(system(look(meeting_windows()[1..].to_vec())));
        assert_eq!(m.slot_note(2).as_deref(), Some("2 Meetings windows: 2 have no window id"));
        // App-only slots: the smaller pane's window went to the bigger one.
        let mut m = three_slots([Slot::default(), Slot::default(), Slot::default()]);
        m.update(system(look(meeting_windows()[..1].to_vec())));
        assert_eq!(m.slot_note(2).as_deref(), Some("1 Meetings window: 1 went to a bigger pane"));
        // A window the rules leave alone (tiny) is said so.
        let tiny = Seen { frame: Frame::new(0., 0., 40., 40.), ..seen(7, 30, "x") };
        m.update(system(look(vec![tiny])));
        assert_eq!(m.slot_misses(1), [(Miss::Rules, 1)]);
        let state = serde_json::to_string(&m).unwrap();
        assert!(!state.contains("Team Meeting") && !state.contains("meetings.main"), "{state}");
    }

    /// Fill slot `pane` with what it offers, as the user's Bring it here and the shell's hosting.
    fn bring(m: &mut Model, pane: PaneId) {
        let window = m.slot_offer(pane).map_or(0, |seen| seen.window);
        m.update(Action::SlotAccept { pane });
        m.update(system(Action::SlotHosted { pane, kind: Some(meetings(window, "x")) }));
    }

    fn holds(m: &Model) -> Vec<Option<u32>> {
        let held = |pane| match m.pane(pane).map(|p| &p.kind) {
            Some(Kind::App { window, .. }) => Some(*window),
            _ => None,
        };
        (1..=3).map(held).collect()
    }

    /// The owner's call: "an option to rotate or swap slots if they aren't matched correctly". Swap
    /// with… exchanges two slots' windows (one may hold none: it moves, the other waits); the
    /// slots stay. Re-sort by size pairs them again. Only the user's.
    #[test]
    fn swap_exchanges_two_slots_windows_and_resort_pairs_them_by_size() {
        use crate::row_menu::{Item, MenuOf};
        let mut m = three_slots([Slot::default(), Slot::default(), Slot::default()]);
        m.update(system(look(meeting_windows())));
        for pane in 1..=3 {
            bring(&mut m, pane);
        }
        assert_eq!(holds(&m), [Some(21), Some(22), Some(23)]);
        m.update(Action::RowMenu { row: MenuOf::Pane(1), dots: None, keys: false });
        let items = m.row_menu().map(|(_, items)| items).unwrap_or_default();
        let want = [
            Item::SlotEdit,
            Item::SlotGiveBack,
            Item::SlotSwap(2),
            Item::SlotSwap(3),
            Item::SlotResort,
            Item::SlotClear,
        ];
        assert_eq!(items, want);
        assert_eq!(m.menu_label(Item::SlotSwap(2)), "Swap with “x”");
        assert_eq!(m.menu_label(Item::SlotResort), "Re-sort Meetings windows by size");
        m.update(Action::RowMenuClose);
        // A client's swap changes nothing; the user's exchanges the two windows.
        m.update(Action::SlotSwap { pane: 1, with: 2 }.sent_by(Origin::Socket));
        assert_eq!(holds(&m), [Some(21), Some(22), Some(23)]);
        let effects = m.update(Action::SlotSwap { pane: 1, with: 2 });
        assert_eq!(
            (holds(&m), effects.contains(&Effect::SlotSwapped { a: 1, b: 2 })),
            (vec![Some(22), Some(21), Some(23)], true)
        );
        // A window moved stays put when the windows are looked at again.
        m.update(system(look(meeting_windows())));
        assert_eq!(holds(&m), [Some(22), Some(21), Some(23)]);
        // Re-sort: the largest pane gets the largest window again.
        m.update(Action::SlotResort { pane: 3 });
        assert_eq!(holds(&m), [Some(21), Some(22), Some(23)]);
        // With a waiting slot: the window moves, the other waits, and the slots stay as they were.
        m.update(Action::SlotGiveBack { pane: 3 });
        m.update(Action::SlotSwap { pane: 2, with: 3 });
        assert_eq!(holds(&m), [Some(21), None, Some(22)]);
        assert_eq!((status(&m, 2), status(&m, 3)), (Some(Status::Offered), Some(Status::Filled)));
        assert!(m.slots.values().all(|run| run.slot == Slot::default()));
    }

    /// Two slots of the app: one item, `Swap`. The owner's B (by id) waits while A holds its window:
    /// a swap gives B its window and A waits, offered the other of its title.
    #[test]
    fn two_slots_swap_with_one_item_and_the_id_slot_gets_its_window() {
        use crate::row_menu::{Item, MenuOf};
        let a = select(r#"{"select": {"title": "Team Meeting"}, "pick": "largest"}"#, Matching::Manual);
        let b = select(r#"{"select": {"identifier": "meetings.main"}}"#, Matching::Manual);
        let mut m = three_slots([a, b, Slot::default()]);
        m.update(Action::SlotClear { pane: 3 });
        m.update(system(look(meeting_windows()[..2].to_vec())));
        pick_into(&mut m, 1, "Meetings", meetings(21, "x"));
        m.update(system(look(meeting_windows()[..2].to_vec())));
        m.update(Action::RowMenu { row: MenuOf::Pane(2), dots: None, keys: false });
        let items = m.row_menu().map(|(_, items)| items).unwrap_or_default();
        assert_eq!(items, [Item::SlotEdit, Item::SlotSwap(1), Item::SlotResort, Item::SlotClear]);
        assert_eq!(m.menu_label(Item::SlotSwap(1)), "Swap");
        m.update(Action::RowMenuChoose { index: Some(1) });
        assert_eq!((holds(&m)[0], holds(&m)[1]), (None, Some(21)));
        assert_eq!(m.slot_offer(1).map(|s| s.window), Some(22));
    }

    /// The user's ⌘E into slot `pane`: `app`'s window, then the shell's `Open` of what it hosted.
    fn pick_into(m: &mut Model, pane: PaneId, app: &str, kind: Kind) {
        let Kind::App { window, .. } = kind else { return };
        m.update(Action::Focus { pane });
        let pick = crate::model::PickItem::Window {
            app: app.into(),
            window,
            title: "x".into(),
            pane: None,
            frame: Default::default(),
            subrole: String::new(),
        };
        m.update(Action::OpenPicker { axis: Right, new_tab: false, items: vec![pick] });
        m.update(Action::PickerChoose { index: Some(0) });
        m.update(Action::Open { kind, axis: Right, beside: None });
    }

    /// "New window" picked into a slot fills it, but the slot owns no window: given back and
    /// another brought in, closing the pane leaves that window open.
    #[test]
    fn a_new_window_picked_into_a_slot_is_not_owned() {
        let mut m = waiting(Matching::Manual);
        m.update(Action::Focus { pane: 2 });
        let new =
            crate::model::PickItem::NewWindow { app: "Meetings".into(), bundle: Some(BUNDLE.into()), menu_item: true };
        m.update(Action::OpenPicker { axis: Right, new_tab: false, items: vec![new] });
        m.update(Action::PickerChoose { index: Some(0) });
        m.update(Action::NewWindowCame { kind: meetings(40, "Incident new") });
        assert_eq!((status(&m, 2), m.pane(2).map(|p| p.owned)), (Some(Status::Filled), Some(false)));
        // No look listed it yet: the slot keeps the process it had.
        assert_eq!(m.slots[&2].pid, Some(7));
        m.update(Action::SlotGiveBack { pane: 2 });
        m.update(system(look(vec![seen(7, 41, "Incident other")])));
        m.update(Action::SlotAccept { pane: 2 });
        m.update(system(Action::SlotHosted { pane: 2, kind: Some(meetings(41, "Incident other")) }));
        let effects = m.update(Action::Close { pane: Some(2) });
        assert!(!effects.iter().any(|e| matches!(e, Effect::CloseWindow { .. })), "{effects:?}");
    }

    /// Another pane holds the picked window already: the slot keeps waiting, one window one pane.
    #[test]
    fn a_pick_of_a_window_another_pane_holds_leaves_the_slot_waiting() {
        let mut m = waiting(Matching::Manual);
        m.update(Action::Open { kind: meetings(11, "Incident a"), axis: Right, beside: None });
        pick_into(&mut m, 2, "Meetings", meetings(11, "Incident a"));
        assert!(matches!(m.pane(2).map(|p| &p.kind), Some(Kind::Slot { .. })));
    }
}
