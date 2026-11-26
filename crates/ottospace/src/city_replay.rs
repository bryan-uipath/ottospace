//! `ottospace city-replay <session.jsonl> [--dir <checkout>]`: a Claude Code session's file
//! replayed through the live chat's path (the runner's `Wire::read`, `event_of`, the model's
//! `city_at`, `placed` on the checkout's plan), printing what the city shows at each call and
//! why. To debug a city that stayed quiet while an agent worked.

use std::collections::{HashMap, HashSet};

use ottospace_agents::Wire;
use ottospace_core::activity::{Pulse, Station};
use ottospace_core::atlas::Atlas;
use ottospace_core::clone::one_line;
use ottospace_core::model::city::{City, Glimpse};
use ottospace_core::transcript::{Event, Outcome, Text};

/// The most paths printed of one list.
const MOST: usize = 6;

const USAGE: &str = "usage: ottospace city-replay <session.jsonl> [--dir <checkout>]";

pub(crate) fn main(args: &[String]) -> ! {
    // Piped to `head`: end quietly when the reader goes, as other commands do.
    // SAFETY: no thread has started yet.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let flag = args.iter().position(|a| a == "--dir");
    let given = flag.map(|i| args.get(i + 1).unwrap_or_else(|| fail(USAGE)));
    let file = (args.iter().enumerate()).find(|(i, a)| !a.starts_with("--") && flag.is_none_or(|f| *i != f + 1));
    let Some((_, file)) = file else { fail(USAGE) };
    let text = std::fs::read_to_string(file).unwrap_or_else(|e| fail(&format!("{}: {e}", one_line(file))));
    // Where the session ran: its absolute paths name it. The plan is read from `--dir`, else there.
    let cwd: Option<String> = text.lines().find_map(|line| {
        let record: serde_json::Value = serde_json::from_str(line).ok()?;
        record["cwd"].as_str().map(str::to_string)
    });
    let Some(read_at) = given.or(cwd.as_ref()) else { fail("no cwd in the file: pass --dir") };
    let read_at = std::fs::canonicalize(read_at).unwrap_or_else(|e| fail(&format!("{}: {e}", one_line(read_at))));
    let plan = crate::city::plan_of(&read_at);
    let read_at = read_at.to_string_lossy().into_owned();
    let ran = cwd.map(|cwd| std::fs::canonicalize(&cwd).map_or(cwd, |p| p.to_string_lossy().into_owned()));
    let ran = ran.unwrap_or_else(|| read_at.clone());
    println!("checkout {}", one_line(&read_at));
    if ran != read_at {
        println!("ran in   {}", one_line(&ran));
    }
    match &plan {
        Some(plan) => println!("plan     {} places\n", plan.places.len()),
        None => println!("plan     none (not a git checkout, or too large): places unknown\n"),
    }
    let folder = ran.rsplit('/').next().unwrap_or("").to_string();
    let mut run = Run { plan, folder, ..Run::default() };
    // The user's flags, the city's on.
    let features = crate::features::startup().with(ottospace_core::features::Feature::CodeCity, true);
    let mut replay = ottospace_core::city_replay::Replay::new(&ran, features);
    let mut wire = ottospace_agents::claude::Claude::new();
    let (mut at, mut turn) = (0, false);
    for line in text.lines() {
        let record: serde_json::Value = serde_json::from_str(line).unwrap_or_default();
        let stamp = Some(ottospace_sessions::jsonl::time_of(&record)).filter(|t| *t > 0);
        at = stamp.unwrap_or(at);
        if let Some(stamp) = stamp {
            run.first.get_or_insert(stamp);
        }
        // The user's message: live, the last turn's `result` came first, and the send clears an error.
        if prompt(&record) {
            if std::mem::take(&mut turn) {
                replay.step(vec![Event::TurnFinished { outcome: Outcome::Completed }], at);
            }
            wire = ottospace_agents::claude::Claude::new();
            replay.step(vec![Event::UserText { text: Text::default() }], at);
            continue;
        }
        let events = wire.read(line).events.into_iter().filter_map(|said| crate::conversation::event_of(said, false));
        // One at a time: each call printed with the city right after it.
        for event in events {
            turn |= matches!(event, Event::TurnStarted);
            let call = Call::of(&event);
            replay.step(vec![event], at);
            if let Some(call) = call {
                run.print(&call, stamp, replay.city().as_ref(), replay.pulse());
            }
        }
    }
    run.summary();
    std::process::exit(0)
}

/// A message the user wrote (not a tool's result, nor the CLI's own record).
fn prompt(record: &serde_json::Value) -> bool {
    if record["type"] != "user" || record["isMeta"] == true || record["isSidechain"] == true {
        return false;
    }
    match &record["message"]["content"] {
        serde_json::Value::String(_) => true,
        serde_json::Value::Array(blocks) => {
            blocks.iter().any(|b| b["type"] == "text") && !blocks.iter().any(|b| b["type"] == "tool_result")
        }
        _ => false,
    }
}

fn fail(said: &str) -> ! {
    eprintln!("{said}");
    std::process::exit(2)
}

/// A call's start or end, as the replay prints it.
enum Call {
    Start { id: String, tool: String, station: Station, detail: String },
    End { id: String, failed: bool },
}

impl Call {
    fn of(event: &Event) -> Option<Call> {
        match event {
            Event::ToolStart { id, tool, station, detail, .. } => Some(Call::Start {
                id: id.clone(),
                tool: tool.clone(),
                station: *station,
                detail: one_line(detail.text()),
            }),
            Event::ToolEnd { id, failed, .. } => Some(Call::End { id: id.clone(), failed: *failed }),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Run {
    plan: Option<Atlas>,
    folder: String,
    first: Option<u64>,
    /// Each call's start (none: its record had no time) and tool, for its end's line.
    started: HashMap<String, (Option<u64>, String)>,
    /// The glimpses printed, by id, with their found files' count.
    shown: HashMap<String, usize>,
    calls: usize,
    on_file: usize,
    middle: usize,
    unplaced: usize,
    missing: usize,
    glimpsed: HashSet<String>,
}

impl Run {
    fn print(&mut self, call: &Call, at: Option<u64>, city: Option<&City>, pulse: Option<Pulse>) {
        let time = match (at, self.first) {
            (Some(at), Some(first)) => format!("+{:.1}s", at.saturating_sub(first) as f64 / 1000.),
            _ => "?".to_string(),
        };
        match call {
            Call::Start { id, tool, station, detail } => {
                self.calls += 1;
                self.started.insert(id.clone(), (at, tool.clone()));
                println!("{time:>8}  {tool:<6} {detail}");
                let Some(city) = city else {
                    self.missing += 1;
                    println!("          icon   none: no city");
                    return;
                };
                match city.agents.first() {
                    None => {
                        self.missing += 1;
                        println!("          icon   none: the chat is {pulse:?}, not Running");
                    }
                    Some(_) if self.plan.is_none() => {
                        self.unplaced += 1;
                        println!("          icon   unplaced: no plan, the lattice shows");
                    }
                    Some(agent) => {
                        let placed = crate::city::placed(self.plan.as_ref(), &self.folder, agent.at, &agent.near);
                        let why = match (&placed, agent.at) {
                            (Some(place), Some(_)) => format!("over {} (its file)", one_line(place)),
                            (Some(place), None) => format!("over {} (its shell line names it)", one_line(place)),
                            (None, Some(label)) => format!("middle: `{}` names {}", one_line(label), self.count(label)),
                            (None, None) if agent.near.is_empty() => {
                                "middle: no file, and its shell line names no path here".to_string()
                            }
                            (None, None) => {
                                let named: Vec<String> = agent.near.iter().take(MOST).map(|p| one_line(p)).collect();
                                format!(
                                    "middle: its line names {}{}, none a place",
                                    named.join(", "),
                                    more(agent.near.len())
                                )
                            }
                        };
                        if placed.is_some() {
                            self.on_file += 1
                        } else {
                            self.middle += 1
                        }
                        println!("          icon   {why}");
                    }
                }
                // Each lit label with the buildings it lights, as the renderer lights them all.
                let lit: Vec<String> = city.lit.iter().map(|lit| self.lights(lit.label)).collect();
                println!("          lit    {}", if lit.is_empty() { "-".to_string() } else { lit.join("; ") });
                let new = city.glimpses.iter().find(|g| g.id.ends_with(&format!(":{id}")));
                match new {
                    Some(glimpse) => {
                        println!("          glimps {}", self.glimpse(glimpse));
                        self.shown.insert(glimpse.id.clone(), glimpse.found.len());
                        self.glimpsed.insert(id.clone());
                    }
                    None if pulse != Some(Pulse::Running) => {
                        println!("          glimps none: the chat is {pulse:?}, glimpses play only while it runs")
                    }
                    None => println!("          glimps none: {}", no_glimpse(tool, *station)),
                }
            }
            Call::End { id, failed } => {
                let Some((since, tool)) = self.started.remove(id) else { return };
                let took = match (since, at) {
                    (Some(since), Some(at)) => format!("{:.1}s", at.saturating_sub(since) as f64 / 1000.),
                    _ => "an unknown time".to_string(),
                };
                let failed = if *failed { " failed" } else { "" };
                println!("{time:>8}  {tool:<6} ended after {took}{failed}");
                // A search's result: the files it found, once known.
                let found = city.into_iter().flat_map(|c| &c.glimpses).find(|g| g.id.ends_with(&format!(":{id}")));
                if let Some(glimpse) = found.filter(|g| self.shown.get(&g.id).is_some_and(|n| *n != g.found.len())) {
                    let places: Vec<String> = glimpse.found.iter().take(MOST).map(|f| self.mark(f)).collect();
                    println!("          found  {}{}", places.join(", "), more(glimpse.found.len()));
                    self.shown.insert(glimpse.id.clone(), glimpse.found.len());
                }
            }
        }
    }

    /// How many buildings a label names: one is placed, none or more is not.
    fn count(&self, label: &str) -> String {
        match &self.plan {
            None => "plan unknown".to_string(),
            Some(plan) => match plan.labelled(label, &self.folder).count() {
                0 => "no building".to_string(),
                1 => "1 building".to_string(),
                n => format!("{n} buildings"),
            },
        }
    }

    /// A lit label and the buildings it lights: `src/lib.rs → a/src/lib.rs, b/src/lib.rs`.
    fn lights(&self, label: &str) -> String {
        let Some(plan) = &self.plan else { return format!("{} (plan unknown)", one_line(label)) };
        let ids: Vec<&str> = plan.labelled(label, &self.folder).map(|place| place.id.as_str()).collect();
        if ids.is_empty() {
            return format!("{} → no building", one_line(label));
        }
        let shown: Vec<String> = ids.iter().take(MOST).map(|id| one_line(id)).collect();
        format!("{} → {}{}", one_line(label), shown.join(", "), more(ids.len()))
    }

    /// A path with whether it is a place of the plan.
    fn mark(&self, path: &str) -> String {
        let known = self.plan.as_ref().map(|plan| plan.places.iter().any(|place| place.id == path));
        let mark = match known {
            Some(true) => "✓",
            Some(false) => "✗ no place",
            None => "?",
        };
        format!("{} {mark}", one_line(path))
    }

    fn glimpse(&self, glimpse: &Glimpse) -> String {
        let kind = if glimpse.search { "search" } else { "read" };
        let scope: Vec<String> = glimpse.scope.iter().map(|p| self.mark(p)).collect();
        let scope = if scope.is_empty() { "the whole checkout".to_string() } else { scope.join(", ") };
        format!("{kind} {scope}")
    }

    fn summary(&self) {
        println!("\n{} calls", self.calls);
        println!("  icon over a place  {}", self.on_file);
        println!("  icon mid-city      {}", self.middle);
        println!("  icon unplaced      {}", self.unplaced);
        println!("  no icon            {}", self.missing);
        println!("  glimpsed           {}", self.glimpsed.len());
    }
}

/// ` and n more`, past `MOST`.
fn more(n: usize) -> String {
    if n > MOST { format!(" and {} more", n - MOST) } else { String::new() }
}

/// Why a call is no glimpse, sorted as `Glances::start` sorts calls.
fn no_glimpse(tool: &str, station: Station) -> &'static str {
    match (tool, station) {
        ("Read" | "Grep" | "Glob", _) => "its path is no place in the checkout (outside it, or with `..`)",
        (_, Station::Shell) => "no stage of its line reads or searches in the checkout",
        _ => "not a read or a search",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The user's own words are a prompt; a tool's result, a meta record or a sub-agent's are not.
    #[test]
    fn only_the_users_words_are_a_prompt() {
        assert!(prompt(&json!({"type": "user", "message": {"content": "go"}})));
        assert!(prompt(&json!({"type": "user", "message": {"content": [{"type": "text", "text": "go"}]}})));
        let result = json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t"}]}});
        assert!(!prompt(&result));
        assert!(!prompt(&json!({"type": "user", "isMeta": true, "message": {"content": "x"}})));
        assert!(!prompt(&json!({"type": "user", "isSidechain": true, "message": {"content": "x"}})));
        assert!(!prompt(&json!({"type": "assistant", "message": {"content": "x"}})));
    }
}
