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
use ottospace_agents::{Answer, Event, Input, Output, Process, Refused, Start, Wire};
use ottospace_core::transcript::Outcome;
use serde_json::Value;

const ASK: &str =
    "Run the shell command `touch ottospace-probe` in the current directory, then reply with one word: done.";
const LONG: &str = "Count from 1 to 3000, one number per line, with no other text.";
const PROBE_FILE: &str = "ottospace-probe";

fn allowed() -> bool {
    std::env::var("OTTOSPACE_REAL_CLI").is_ok_and(|value| value == "1")
}

/// Removed when the test ends, pass or fail.
fn empty_dir(name: &str) -> Scratch {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("ottospace-real-{name}-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    Scratch(dir)
}

/// A folder removed on drop. Use it as a `&Path`.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl std::ops::Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.0
    }
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

impl<P: Wire> Wire for Tap<P> {
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

/// Values kept as recorded: protocol names and enums. Every other value is made up.
const KEPT: &[&str] = &[
    "type",
    "subtype",
    "hook_event",
    "outcome",
    "permissionMode",
    "apiKeySource",
    "claude_code_version",
    "output_style",
    "fast_mode_state",
    "status",
    "thinking_display",
    "stop_reason",
    "terminal_reason",
    "role",
    "service_tier",
    "speed",
    "rateLimitType",
    "overageStatus",
    "inference_geo",
];

/// A recorded line with its structure, order and keys as they came, and made-up values: ids,
/// paths, text, hook output, counts and account fields. A model's id is `claude-haiku-5-5`.
fn scrub(value: &Value, key: &str, ids: &mut Vec<String>) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    (if k.starts_with("claude-") { "claude-haiku-5-5".into() } else { k.clone() }, scrub(v, k, ids))
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(|v| scrub(v, key, ids)).collect()),
        Value::Number(n) if n.is_f64() => serde_json::json!(if n.as_f64() == Some(0.0) { 0.0 } else { 0.001 }),
        Value::Number(n) => serde_json::json!(if n.as_u64() == Some(0) { 0 } else { 1 }),
        Value::String(_) if KEPT.contains(&key) => value.clone(),
        Value::String(text) => Value::String(match key {
            "model" => "claude-haiku-5-5".into(),
            "session_id" => "8f14e45f-ceea-467f-a8d3-0242ac130003".into(),
            _ if ottospace_core::agents::is_uuid(text) => {
                let n = ids.iter().position(|id| id == text).unwrap_or_else(|| {
                    ids.push(text.clone());
                    ids.len() - 1
                });
                format!("00000000-0000-4000-8000-{:012}", n + 1)
            }
            "cwd" | "original_cwd" => "/Users/me/scratch".into(),
            "text" | "result" => "ok".into(),
            "output" | "stdout" | "stderr" => String::new(),
            "hook_name" => "SessionStart:startup".into(),
            "timestamp" => "2026-11-14T00:00:00.000Z".into(),
            _ => format!("made-up-{key}"),
        }),
        _ => value.clone(),
    }
}

/// Records one turn of the real CLI as the fixtures the fake `claude` and the parser tests
/// replay: `real-<version>-turn.jsonl` (the whole turn) and `real-<version>-no-reply.jsonl`
/// (its hook lines alone, as a run whose stdin closes first prints), scrubbed by `scrub`.
/// Read them through before committing. See docs/ai/agent-runners.md, Fixtures.
#[test]
#[ignore = "starts a real Claude Code session: OTTOSPACE_REAL_CLI=1"]
fn claude_record() {
    use std::io::{BufRead, Write};
    if !allowed() {
        return;
    }
    let dir = empty_dir("record");
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let session = format!("{:08x}-0000-4000-8000-{:012x}", std::process::id(), nanos & 0xffff_ffff_ffff);
    let argv = claude::argv(Some("haiku"), &Start::New { id: session }).unwrap();
    let mut child = Command::new(argv.program)
        .args(&argv.args)
        .current_dir(&dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let line = serde_json::json!({"type": "user", "message": {"role": "user", "content": [{"type": "text",
        "text": "Reply with the word ok."}]}, "parent_tool_use_id": null, "session_id": ""});
    writeln!(child.stdin.as_mut().unwrap(), "{line}").unwrap();
    let mut lines = Vec::new();
    for line in std::io::BufReader::new(child.stdout.take().unwrap()).lines() {
        let line = line.unwrap();
        let done = line.contains(r#""type":"result""#);
        lines.push(serde_json::from_str::<Value>(&line).unwrap());
        if done {
            break;
        }
    }
    drop(child.stdin.take());
    let _ = child.wait();
    let version = lines.iter().find_map(|v| v["claude_code_version"].as_str()).unwrap_or("unknown").to_string();
    let mut ids = Vec::new();
    let scrubbed: Vec<String> = lines.iter().map(|v| scrub(v, "", &mut ids).to_string()).collect();
    let header = |note: &str| serde_json::json!({"type": "_fixture", "note": note, "cli_version": version}).to_string();
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude");
    let turn = format!(
        "RECORDED from Claude Code {version} (claude -p, stream-json, --model haiku): one turn that answers ok, then scrubbed: every id, path, hook output, count and account field made up; structure, order and field names as recorded. Re-record: docs/ai/agent-runners.md, Fixtures."
    );
    let hooks: Vec<&String> = scrubbed.iter().filter(|line| line.contains(r#""subtype":"hook_"#)).collect();
    let none = format!(
        "The SessionStart hook lines of real-{version}-turn.jsonl: all a run prints when its stdin closes before any message; it then exits 0."
    );
    let write = |name: String, head: String, body: Vec<&String>| {
        let text: String = std::iter::once(&head).chain(body).map(|line| format!("{line}\n")).collect();
        std::fs::write(out.join(name), text).unwrap();
    };
    write(format!("real-{version}-turn.jsonl"), header(&turn), scrubbed.iter().collect());
    write(format!("real-{version}-no-reply.jsonl"), header(&none), hooks);
    println!("recorded {} lines of Claude Code {version}", scrubbed.len());
}
