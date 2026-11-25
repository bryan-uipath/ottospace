//! `ottospace city-replay <session.jsonl> [--dir <checkout>]`: a Claude Code session's file
//! replayed through the live chat's path (the runner's `Wire::read`, `event_of`, the model's
//! `city_at`, `placed` on the checkout's plan), printing what the city shows at each call and
//! why. To debug a city that stayed quiet while an agent worked.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use ottospace_agents::Wire;
use ottospace_core::atlas::Atlas;
use ottospace_core::clone::one_line;
use ottospace_core::model::city::{City, Glimpse};
use ottospace_core::transcript::Event;

/// The most paths printed of one list.
const MOST: usize = 6;

const USAGE: &str = "usage: ottospace city-replay <session.jsonl> [--dir <checkout>]";

pub(crate) fn main(args: &[String]) -> ! {
    let Some(file) = args.first() else { fail(USAGE) };
    let given = args.iter().position(|a| a == "--dir").map(|i| args.get(i + 1).unwrap_or_else(|| fail(USAGE)));
    let text = std::fs::read_to_string(file).unwrap_or_else(|e| fail(&format!("{file}: {e}")));
    let records: Vec<serde_json::Value> = text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect();
    // The checkout: given, or where the session ran.
    let cwd = records.iter().find_map(|r| r["cwd"].as_str());
    let Some(dir) = given.map(String::as_str).or(cwd) else { fail("no cwd in the file: pass --dir") };
    let dir: PathBuf = std::fs::canonicalize(dir).unwrap_or_else(|e| fail(&format!("{dir}: {e}")));
    let plan = crate::city::plan_of(&dir);
    let dir = dir.to_string_lossy().into_owned();
    println!("checkout {dir}");
    match &plan {
        Some(plan) => println!("plan     {} places\n", plan.places.len()),
        None => println!("plan     none (not a git checkout, or too large): places unknown\n"),
    }
    let folder = dir.rsplit('/').next().unwrap_or("").to_string();
    let mut run = Run { plan, folder, ..Run::default() };
    // The user's flags, the city's on.
    let features = crate::features::startup().with(ottospace_core::features::Feature::CodeCity, true);
    let mut replay = ottospace_core::city_replay::Replay::new(&dir, features);
    let mut wire = ottospace_agents::claude::Claude::new();
    let mut at = 0;
    for line in text.lines() {
        let record: serde_json::Value = serde_json::from_str(line).unwrap_or_default();
        at = record["timestamp"].as_str().and_then(ottospace_sessions::jsonl::parse_time).unwrap_or(at);
        if at > 0 {
            run.first.get_or_insert(at);
        }
        let events: Vec<Event> =
            wire.read(line).events.into_iter().filter_map(|said| crate::conversation::event_of(said, false)).collect();
        let calls: Vec<Call> = events.iter().filter_map(Call::of).collect();
        if events.is_empty() {
            continue;
        }
        replay.step(events, at);
        let (city, pulse) = (replay.city(), replay.pulse());
        for call in calls {
            run.print(&call, at, city.as_ref(), pulse);
        }
    }
    run.summary();
    std::process::exit(0)
}

fn fail(said: &str) -> ! {
    eprintln!("{said}");
    std::process::exit(2)
}

/// A call's start or end, as the replay prints it.
enum Call {
    Start { id: String, tool: String, detail: String },
    End { id: String, failed: bool },
}

impl Call {
    fn of(event: &Event) -> Option<Call> {
        match event {
            Event::ToolStart { id, tool, detail, .. } => {
                Some(Call::Start { id: id.clone(), tool: tool.clone(), detail: one_line(detail.text()) })
            }
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
    /// Each call's start and tool, for its end's line.
    started: HashMap<String, (u64, String)>,
    /// The glimpses printed, by id, with their found files' count.
    shown: HashMap<String, usize>,
    calls: usize,
    on_file: usize,
    middle: usize,
    missing: usize,
    glimpsed: HashSet<String>,
}

impl Run {
    fn print(&mut self, call: &Call, at: u64, city: Option<&City>, pulse: Option<ottospace_core::activity::Pulse>) {
        let time = format!("+{:.1}s", at.saturating_sub(self.first.unwrap_or(at)) as f64 / 1000.);
        match call {
            Call::Start { id, tool, detail } => {
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
                    Some(agent) => {
                        let placed = crate::city::placed(self.plan.as_ref(), &self.folder, agent.at, &agent.near);
                        let why = match (&placed, agent.at) {
                            (Some(place), Some(_)) => format!("over {place} (its file)"),
                            (Some(place), None) => format!("over {place} (its shell line names it)"),
                            (None, Some(label)) => format!("middle: `{label}` names {}", self.count(label)),
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
                let lit: Vec<String> =
                    city.lit.iter().map(|lit| format!("{} ({})", lit.label, self.count(lit.label))).collect();
                println!("          lit    {}", if lit.is_empty() { "-".to_string() } else { lit.join(", ") });
                let new = city.glimpses.iter().find(|g| g.id.ends_with(&format!(":{id}")));
                match new {
                    Some(glimpse) => {
                        println!("          glimps {}", self.glimpse(glimpse));
                        self.shown.insert(glimpse.id.clone(), glimpse.found.len());
                        self.glimpsed.insert(id.clone());
                    }
                    None => println!("          glimps none: {}", no_glimpse(tool)),
                }
            }
            Call::End { id, failed } => {
                let Some((since, tool)) = self.started.remove(id) else { return };
                let took = at.saturating_sub(since) as f64 / 1000.;
                let failed = if *failed { " failed" } else { "" };
                println!("{time:>8}  {tool:<6} ended after {took:.1}s{failed}");
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
        println!("  no icon            {}", self.missing);
        println!("  glimpsed           {}", self.glimpsed.len());
    }
}

/// ` and n more`, past `MOST`.
fn more(n: usize) -> String {
    if n > MOST { format!(" and {} more", n - MOST) } else { String::new() }
}

/// Why a call is no glimpse, by its tool: only reads and searches are (`ottospace_core::glance`).
fn no_glimpse(tool: &str) -> &'static str {
    match tool {
        "Read" | "Grep" | "Glob" => "its path is outside the checkout",
        "Bash" => "no stage of its line reads or searches inside the checkout",
        _ => "not a read or a search",
    }
}
