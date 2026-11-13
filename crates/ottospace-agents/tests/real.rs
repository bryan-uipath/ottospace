//! Probes of the real CLIs, to settle what `docs/ai/agent-runners.md` marks UNVERIFIED. Each
//! starts one short model session on the user's account, so each is ignored and also needs
//! `OTTOSPACE_REAL_CLI=1`. Run only with the user's approval:
//!
//! `OTTOSPACE_REAL_CLI=1 cargo test -p ottospace-agents --test real -- --ignored --nocapture claude`
//!
//! Each runs in a new empty folder: a turn that asks to run one command (a permission asked is
//! checked against the folder, then allowed), then a turn interrupted at its first text, then
//! stdin closed. It prints event kinds and the shape of each line a rule did not read (protocol
//! names and key names), never model or tool text. `OTTOSPACE_PROBE_ISOLATED=1` adds
//! `--setting-sources project` for Claude: the user's own settings are not loaded, in an empty
//! folder that has none, so nothing they allow answers a prompt first.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use ottospace_agents::claude::{self, Claude};
use ottospace_agents::codex::{self, Codex};
use ottospace_agents::driver::Driver;
use ottospace_agents::{Answer, Event, Input, Outcome, Output, Protocol, Refused, Runner, Start};
use serde_json::Value;

const ASK: &str =
    "Run the shell command `touch ottospace-probe` in the current directory, then reply with one word: done.";
const LONG: &str = "Count from 1 to 3000, one number per line, with no other text.";
const PROBE_FILE: &str = "ottospace-probe";

fn allowed() -> bool {
    std::env::var("OTTOSPACE_REAL_CLI").is_ok_and(|value| value == "1")
}

fn empty_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("ottospace-real-{name}-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A protocol name worth printing: name-shaped (`station::is_name`, plus `/`), else `?`.
fn name(value: &Value) -> String {
    let text = value.as_str().unwrap_or("?");
    let plain = text.len() <= 64 && text.chars().all(|c| c.is_ascii_alphanumeric() || "_.:/-".contains(c));
    if plain { text } else { "?" }.to_string()
}

/// Key names; a key whose value is null says so (`key=null`).
fn keys(value: &Value) -> String {
    let named = |(key, value): (&String, &Value)| if value.is_null() { format!("{key}=null") } else { key.clone() };
    value.as_object().map_or(String::new(), |map| map.iter().map(named).collect::<Vec<_>>().join(","))
}

/// A line's shape: its protocol names (type, subtype, method, enums) and key names, no values.
fn shape(line: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(line) else { return "not json".into() };
    let mut parts = Vec::new();
    for key in ["type", "subtype", "method"] {
        if value.get(key).is_some() {
            parts.push(format!("{key}={}", name(&value[key])));
        }
    }
    parts.push(format!("keys=[{}]", keys(&value)));
    // Enum-like values that say what happened, never text.
    let enums = [
        ("event.type", &value["event"]["type"]),
        ("request.subtype", &value["request"]["subtype"]),
        ("response.subtype", &value["response"]["subtype"]),
        ("permissionMode", &value["permissionMode"]),
        ("stop_reason", &value["stop_reason"]),
        ("terminal_reason", &value["terminal_reason"]),
        ("is_error", &value["is_error"]),
        ("turn.status", &value["params"]["turn"]["status"]),
        ("item.type", &value["params"]["item"]["type"]),
        ("item.status", &value["params"]["item"]["status"]),
    ];
    for (label, found) in enums {
        match found {
            Value::Null => {}
            Value::Bool(flag) => parts.push(format!("{label}={flag}")),
            other => parts.push(format!("{label}={}", name(other))),
        }
    }
    for nested in ["params", "request", "response", "result"] {
        if value[nested].is_object() {
            parts.push(format!("{nested}.keys=[{}]", keys(&value[nested])));
        }
    }
    if value["type"] == "system" && value["subtype"] == "init" {
        let tools = value["tools"].as_array().map_or(0, Vec::len);
        parts.push(format!(
            "tools={tools} capabilities=[{}]",
            value["capabilities"].as_array().map_or(String::new(), |c| c
                .iter()
                .map(name)
                .collect::<Vec<_>>()
                .join(","))
        ));
    }
    parts.join(" ")
}

/// The protocol, with the shape of each line printed when what it gave needs looking at.
struct Tap<P>(P);

impl<P: Protocol> Protocol for Tap<P> {
    fn opening(&mut self) -> Vec<String> {
        self.0.opening()
    }

    fn read(&mut self, line: &str) -> Output {
        let out = self.0.read(line);
        let telling = |event: &Event| {
            matches!(
                event,
                Event::Unknown { .. }
                    | Event::AutoDenied { .. }
                    | Event::Started { .. }
                    | Event::Permission { .. }
                    | Event::PermissionClosed { .. }
                    | Event::TurnFinished { .. }
                    | Event::Error { .. }
            )
        };
        if out.events.iter().any(telling) || line.contains("control_") {
            println!("    < {}", shape(line));
        }
        for event in &out.events {
            if let Event::AutoDenied { reason, .. } = event {
                println!("    auto-denied: {reason}");
            }
        }
        for reply in &out.replies {
            println!("    > reply {}", shape(reply));
        }
        out
    }

    fn write(&mut self, input: Input) -> Result<Vec<String>, Refused> {
        let lines = self.0.write(input)?;
        for line in &lines {
            println!("    > {}", shape(line));
        }
        Ok(lines)
    }

    fn awaits_answer(&self, head: &str) -> bool {
        self.0.awaits_answer(head)
    }
}

/// The event's variant name: what is printed.
fn kind(event: &Event) -> String {
    let debug = format!("{event:?}");
    debug.split([' ', '{', '(']).next().unwrap_or("").to_string()
}

/// Events until `stop`. A permission is checked against `dir` (nothing made yet), then allowed.
fn until(driver: &Driver, dir: &Path, stop: impl Fn(&Event) -> bool) -> Vec<Event> {
    let mut seen = Vec::new();
    loop {
        let event = driver.events().recv_timeout(Duration::from_secs(180)).expect("an event within 3 minutes");
        println!("{}", kind(&event));
        if let Event::Permission { ticket, tool, label, cut, .. } = &event {
            let made = dir.join(PROBE_FILE).exists();
            println!("    permission: tool={tool} label={label} cut={cut} file_before_allow={made}");
            assert!(!made, "the command ran before it was allowed");
            driver.send(Input::Answer { ticket: ticket.clone(), answer: Answer::Allow }).unwrap();
        }
        let done = stop(&event) || matches!(event, Event::Exited { .. });
        seen.push(event);
        if done {
            return seen;
        }
    }
}

fn probe(mut driver: Driver, dir: &Path) {
    driver.send(Input::Send { text: ASK.into(), images: vec![] }).unwrap();
    let first = until(&driver, dir, |event| matches!(event, Event::TurnFinished { .. }));
    let asked = first.iter().any(|event| matches!(event, Event::Permission { .. }));
    println!("== first turn: permission asked={asked} file_after={}", dir.join(PROBE_FILE).exists());
    assert!(first.iter().any(|event| matches!(event, Event::Started { session: Some(_), .. })), "{first:#?}");
    assert_eq!(first.last(), Some(&Event::TurnFinished { outcome: Outcome::Completed }));

    driver.send(Input::Send { text: LONG.into(), images: vec![] }).unwrap();
    let deltas = until(&driver, dir, |event| matches!(event, Event::TextDelta { .. }));
    driver.send(Input::Interrupt).unwrap();
    let second = until(&driver, dir, |event| matches!(event, Event::TurnFinished { .. }));
    let after = second.iter().filter(|event| matches!(event, Event::TextDelta { .. })).count();
    println!("== second turn: deltas before interrupt={} after={after} end={:?}", deltas.len(), second.last());
    driver.stop();
    until(&driver, dir, |_| false);
}

#[test]
#[ignore = "starts a real Claude Code session: OTTOSPACE_REAL_CLI=1"]
fn claude_real() {
    if !allowed() {
        return;
    }
    let dir = empty_dir("claude");
    // A new id each run: a session id can be used once.
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let session = format!("{:08x}-0000-4000-8000-{:012x}", std::process::id(), nanos & 0xffff_ffff_ffff);
    let argv = claude::argv(Some("haiku"), &Start::New { id: session }).unwrap();
    let mut command = Command::new(argv.program);
    command.args(&argv.args).current_dir(&dir);
    if std::env::var("OTTOSPACE_PROBE_ISOLATED").is_ok_and(|value| value == "1") {
        command.args(["--setting-sources", "project"]);
    }
    probe(Driver::spawn(command, Tap(Claude::new())).unwrap(), &dir);
}

#[test]
#[ignore = "starts a real Codex session: OTTOSPACE_REAL_CLI=1"]
fn codex_real() {
    if !allowed() {
        return;
    }
    let dir = empty_dir("codex");
    let mut command = Command::new(codex::argv().program);
    command.args(&codex::argv().args).current_dir(&dir);
    let codex = Codex::new(&dir, None, &Start::New { id: String::new() }).unwrap();
    probe(Driver::spawn(command, Tap(codex)).unwrap(), &dir);
}
