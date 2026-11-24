//! What needs the person, in order (`decision_sources`, design §6): a pure function of the cards
//! and what placed them. Focus first, then taken now, then an agent waiting, then core × urgency
//! (accepted claims only), then the task's priority, then age; never more than three in a row
//! from one source while another has a card in the same tier. Design:
//! `docs/ai/attention/decision-sources.md`.

use crate::filed::{HOUR_MS, Role, Source};

/// A card a run can show: an agent's question (its task and ticket), or a filed decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Card {
    Ask { task: u32, ticket: u64 },
    Filed(u64),
}

/// What placed a card: each input the rule reads, so its line says why it is where it is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Why {
    /// This week's area it is in, by place (0 to 3); `None`: not in focus.
    pub area: Option<usize>,
    /// A focus is set this week: "not in focus" means something.
    pub focus_set: bool,
    /// Taken now (`t`) until then.
    pub taken_now: bool,
    /// An agent waits on it: its own question, or a task it blocks whose agent asks now.
    pub agent_waits: bool,
    /// Core (2) or ancillary (1).
    pub role: Option<Role>,
    /// Counted claims: someone waits, and when it is due.
    pub person_waits: bool,
    pub due: Option<u64>,
}

impl Why {
    /// C: 2 for core, 1 for ancillary (none said: ancillary).
    pub fn c(&self) -> u8 {
        if self.role == Some(Role::Core) { 2 } else { 1 }
    }

    /// U: 1, plus 1 if someone waits, plus 1 if due within a day or 2 within two hours (or overdue).
    pub fn u(&self, now: u64) -> u8 {
        let due = self.due.map_or(0, |due| match due.saturating_sub(now) {
            left if left <= 2 * HOUR_MS => 2,
            left if left <= 24 * HOUR_MS => 1,
            _ => 0,
        });
        1 + u8::from(self.person_waits) + due
    }

    /// C × U: 1 to 8.
    pub fn score(&self, now: u64) -> u8 {
        self.c() * self.u(now)
    }

    /// The focus tier: 0 in an area (or no focus set, all alike), 1 not in focus.
    pub fn tier(&self) -> u8 {
        u8::from(self.focus_set && self.area.is_none())
    }
}

/// A card and what the rule reads of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub card: Card,
    pub source: Source,
    pub why: Why,
    /// Its task's priority (1 first); a card of no task after every task's.
    pub priority: u8,
    /// When it was first seen waiting, then a tie-break that never repeats.
    pub since: u64,
    pub id: u64,
}

/// How many cards of one source may come in a row while another source has one in their tier.
pub const IN_A_ROW: usize = 3;

/// The entries in the order they need the person.
pub fn order(mut entries: Vec<Entry>, now: u64) -> Vec<Entry> {
    entries.sort_by_key(|e| {
        let w = &e.why;
        (w.tier(), !w.taken_now, !w.agent_waits, std::cmp::Reverse(w.score(now)), e.priority, e.since, e.id)
    });
    spaced(entries)
}

/// One pass: a fourth card in a row from one source gives way to the next of another source in
/// its tier. A card an agent waits on is exempt: it is never moved, and it ends a run.
fn spaced(mut left: Vec<Entry>) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::with_capacity(left.len());
    while !left.is_empty() {
        let first = &left[0];
        let run = out
            .iter()
            .rev()
            .take_while(|e| e.why.tier() == first.why.tier() && e.source == first.source && !e.why.agent_waits)
            .count();
        let other = (run >= IN_A_ROW && !first.why.agent_waits)
            .then(|| left.iter().position(|e| e.why.tier() == first.why.tier() && e.source != first.source))
            .flatten();
        out.push(left.remove(other.unwrap_or(0)));
    }
    out
}

/// The line a card shows: what placed it, shortest first. `area`: the area's name, if in one.
pub fn line(why: &Why, area: Option<&str>, now: u64, due: impl Fn(u64) -> String) -> String {
    let mut parts: Vec<String> = Vec::new();
    match area {
        Some(name) => parts.push(name.to_string()),
        None if why.focus_set => parts.push("not in focus".into()),
        None => {}
    }
    if why.taken_now {
        parts.push("taken now".into());
    }
    if why.agent_waits {
        parts.push("an agent waits".into());
    }
    if why.role == Some(Role::Core) && !why.agent_waits {
        parts.push("core".into());
    }
    if why.person_waits {
        parts.push("someone waits".into());
    }
    if let Some(at) = why.due.filter(|_| why.u(now) > 1 + u8::from(why.person_waits)) {
        parts.push(format!("due {}", due(at)));
    }
    parts.join(" · ")
}

/// The sum behind the line, for its hover: "in focus (Beacon launch) · core × urgency 3 (someone
/// waits, due 5 pm) = 6".
pub fn sum(why: &Why, area: Option<&str>, now: u64, due: impl Fn(u64) -> String) -> String {
    let focus = match area {
        Some(name) => format!("in focus ({name})"),
        None if why.focus_set => "not in focus".into(),
        None => "no focus set".into(),
    };
    let role = if why.c() == 2 { "core" } else { "ancillary" };
    let mut urgent: Vec<String> = Vec::new();
    if why.person_waits {
        urgent.push("someone waits".into());
    }
    if let Some(at) = why.due.filter(|_| why.u(now) > 1 + u8::from(why.person_waits)) {
        urgent.push(format!("due {}", due(at)));
    }
    let urgent = if urgent.is_empty() { String::new() } else { format!(" ({})", urgent.join(", ")) };
    let mut out = format!("{focus} · {role} × urgency {}{urgent} = {}", why.u(now), why.score(now));
    if why.taken_now {
        out.push_str(" · taken now: first of its tier today");
    }
    if why.agent_waits {
        out.push_str(" · an agent waits: before every filed card");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOON: u64 = 1_000 * HOUR_MS;

    fn entry(card: Card, source: Source, why: Why, priority: u8, since: u64) -> Entry {
        let id = match card {
            Card::Ask { task, .. } => u64::from(task),
            Card::Filed(id) => id,
        };
        Entry { card, source, why, priority, since, id }
    }

    fn ids(entries: &[Entry]) -> Vec<Card> {
        entries.iter().map(|e| e.card).collect()
    }

    /// The design's worked table, Wednesday at noon: A, B, C, then F, E, D.
    #[test]
    fn the_worked_table() {
        let focus = |area: Option<usize>| Why { area, focus_set: true, ..Why::default() };
        let core = Some(Role::Core);
        let a = entry(
            Card::Ask { task: 1, ticket: 1 },
            Source::AgentSession,
            Why { agent_waits: true, role: core, ..focus(Some(0)) },
            3,
            1,
        );
        let b = entry(
            Card::Filed(2),
            Source::Slack,
            Why { role: core, person_waits: true, due: Some(NOON + 5 * HOUR_MS), ..focus(Some(1)) },
            u8::MAX,
            2,
        );
        let c = entry(Card::Filed(3), Source::Routine, Why { role: core, ..focus(Some(3)) }, u8::MAX, 3);
        let d = entry(
            Card::Filed(4),
            Source::Jira,
            Why { role: Some(Role::Ancillary), due: Some(NOON + HOUR_MS), ..focus(None) },
            u8::MAX,
            4,
        );
        let e = entry(
            Card::Ask { task: 5, ticket: 1 },
            Source::AgentSession,
            Why { agent_waits: true, role: core, ..focus(None) },
            1,
            5,
        );
        let f = entry(Card::Filed(6), Source::Slack, Why { taken_now: true, ..focus(None) }, u8::MAX, 6);
        let scores: Vec<u8> = [&a, &b, &c, &d, &e, &f].iter().map(|x| x.why.score(NOON)).collect();
        assert_eq!(scores, [2, 6, 2, 3, 2, 1]);
        let all = vec![d.clone(), f.clone(), e.clone(), c.clone(), b.clone(), a.clone()];
        assert_eq!(ids(&order(all, NOON)), ids(&[a, b, c, f, e, d]));
    }

    #[test]
    fn each_input_alone_moves_a_card() {
        let base = Why { focus_set: true, area: Some(0), ..Why::default() };
        let two = |w: Why| {
            let x = entry(Card::Filed(1), Source::Slack, base.clone(), 5, 1);
            let y = entry(Card::Filed(2), Source::Jira, w, 5, 2);
            ids(&order(vec![x, y], NOON))[0]
        };
        assert_eq!(two(base.clone()), Card::Filed(1), "alike: the older first");
        assert_eq!(two(Why { taken_now: true, ..base.clone() }), Card::Filed(2), "taken now");
        assert_eq!(two(Why { agent_waits: true, ..base.clone() }), Card::Filed(2), "an agent waits");
        assert_eq!(two(Why { role: Some(Role::Core), ..base.clone() }), Card::Filed(2), "core");
        assert_eq!(two(Why { person_waits: true, ..base.clone() }), Card::Filed(2), "someone waits");
        assert_eq!(two(Why { due: Some(NOON + HOUR_MS), ..base.clone() }), Card::Filed(2), "due soon");
        // Out of focus, it goes after.
        let x = entry(Card::Filed(1), Source::Slack, Why { area: None, ..base.clone() }, 1, 1);
        let y = entry(Card::Filed(2), Source::Jira, base.clone(), 9, 2);
        assert_eq!(ids(&order(vec![x, y], NOON))[0], Card::Filed(2), "focus over everything");
        // Priority, when nothing else differs.
        let x = entry(Card::Filed(1), Source::Slack, base.clone(), 4, 1);
        let y = entry(Card::Filed(2), Source::Jira, base.clone(), 2, 2);
        assert_eq!(ids(&order(vec![x, y], NOON))[0], Card::Filed(2), "P2 before P4");
    }

    #[test]
    fn with_no_focus_set_agent_questions_go_by_priority_then_age() {
        let asks = |task: u32, priority: u8, since: u64| {
            let why = Why { agent_waits: true, role: Some(Role::Core), ..Why::default() };
            entry(Card::Ask { task, ticket: 1 }, Source::AgentSession, why, priority, since)
        };
        let got = order(vec![asks(1, 3, 1), asks(2, 1, 5), asks(3, 1, 2)], NOON);
        let tasks: Vec<u64> = got.iter().map(|e| e.id).collect();
        assert_eq!(tasks, [3, 2, 1]);
    }

    #[test]
    fn a_fourth_in_a_row_from_one_source_gives_way_and_an_agent_waiting_is_exempt() {
        let why = Why { focus_set: true, area: Some(0), ..Why::default() };
        let slack = |id: u64| entry(Card::Filed(id), Source::Slack, why.clone(), 5, id);
        let jira = entry(Card::Filed(9), Source::Jira, why.clone(), 5, 9);
        let got = order(vec![slack(1), slack(2), slack(3), slack(4), jira.clone()], NOON);
        assert_eq!(ids(&got), ids(&[slack(1), slack(2), slack(3), jira.clone(), slack(4)]));
        // Not in its tier: the other source does not come forward.
        let out = entry(Card::Filed(9), Source::Jira, Why { area: None, ..why.clone() }, 5, 9);
        let got = order(vec![slack(1), slack(2), slack(3), slack(4), out.clone()], NOON);
        assert_eq!(ids(&got), ids(&[slack(1), slack(2), slack(3), slack(4), out]));
        // A card an agent waits on is never held back.
        let waits = |id: u64| entry(Card::Filed(id), Source::Slack, Why { agent_waits: true, ..why.clone() }, 5, id);
        let got = order(vec![waits(1), waits(2), waits(3), waits(4), jira.clone()], NOON);
        assert_eq!(ids(&got), ids(&[waits(1), waits(2), waits(3), waits(4), jira]));
    }

    #[test]
    fn a_filed_card_never_ranks_above_one_of_its_tier_where_an_agent_waits() {
        let why = Why { focus_set: true, area: Some(0), ..Why::default() };
        let urgent = Why { role: Some(Role::Core), person_waits: true, due: Some(NOON), ..why.clone() };
        let filed = entry(Card::Filed(1), Source::Slack, urgent, 1, 1);
        let ask = entry(Card::Ask { task: 2, ticket: 1 }, Source::AgentSession, Why { agent_waits: true, ..why }, 9, 2);
        assert_eq!(ids(&order(vec![filed, ask.clone()], NOON))[0], ask.card);
    }

    #[test]
    fn the_line_and_its_sum_say_what_placed_it() {
        let at = |_| "5 pm".to_string();
        let b = Why {
            area: Some(1),
            focus_set: true,
            role: Some(Role::Core),
            person_waits: true,
            due: Some(NOON + 5 * HOUR_MS),
            ..Why::default()
        };
        assert_eq!(line(&b, Some("Beacon launch"), NOON, at), "Beacon launch · core · someone waits · due 5 pm");
        assert_eq!(
            sum(&b, Some("Beacon launch"), NOON, at),
            "in focus (Beacon launch) · core × urgency 3 (someone waits, due 5 pm) = 6"
        );
        let e = Why { focus_set: true, agent_waits: true, role: Some(Role::Core), ..Why::default() };
        assert_eq!(line(&e, None, NOON, at), "not in focus · an agent waits");
        let none = Why { agent_waits: true, ..Why::default() };
        assert_eq!(line(&none, None, NOON, at), "an agent waits", "no focus set: no word on it");
    }
}
