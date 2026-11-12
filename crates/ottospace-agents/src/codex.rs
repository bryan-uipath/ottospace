//! Codex. Rollouts: `<codex home>/sessions/<yyyy>/<mm>/<dd>/rollout-<time>-<id>.jsonl`.
//! Schema seen in codex-cli 0.166.0 and in rollouts from earlier 2026 versions.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::cache::Cache;
use crate::station::{self, Station};
use crate::{Batch, Cursor, Event, Kind, Refined, Runner, SessionRef, Usage, What, jsonl};

/// Calls a cursor tracks at once. Past it the oldest state is dropped: a file cannot grow the cursor without bound.
const MAX_OPEN: usize = 256;

pub struct Codex {
    home: PathBuf,
    /// How many rollouts `sessions` looks at, newest day first. Rollouts have no index by directory.
    pub max_files: usize,
    heads: Cache<Head>,
}

/// What a rollout's first records say. They do not change as the file grows.
#[derive(Clone)]
struct Head {
    id: String,
    cwd: PathBuf,
    started: Option<u64>,
    model: Option<String>,
    parent: Option<String>,
}

impl Codex {
    /// A relative or empty `home` has no sessions.
    pub fn new(home: PathBuf) -> Self {
        Self { home, max_files: 1000, heads: Cache::default() }
    }

    /// `$CODEX_HOME` if set and not empty, or `~/.codex`.
    pub fn from_env() -> Self {
        let set = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        Self::new(set("CODEX_HOME").or(set("HOME").map(|h| h.join(".codex"))).unwrap_or_default())
    }
}

impl Runner for Codex {
    fn kind(&self) -> Kind {
        Kind::Codex
    }

    fn sessions(&self, dirs: &[PathBuf]) -> io::Result<Vec<SessionRef>> {
        crate::absolute(dirs)?;
        let mut rollouts = Vec::new();
        if self.home.is_absolute() {
            newest_first(&self.home.join("sessions"), 3, self.max_files, &mut rollouts);
        }
        let mut sessions = Vec::new();
        for path in &rollouts {
            let Some((head, modified)) = self.heads.get(path, || head(path)) else { continue };
            if dirs.iter().any(|dir| head.cwd.starts_with(dir)) {
                let (kind, path, last_activity) = (Kind::Codex, path.clone(), Some(modified));
                let Head { id, cwd, started, model, parent } = head;
                sessions.push(SessionRef {
                    kind,
                    id,
                    cwd,
                    path,
                    started,
                    last_activity,
                    model,
                    pid: None,
                    busy: None,
                    parent,
                });
            }
        }
        self.heads.keep(&rollouts.into_iter().collect::<HashSet<_>>());
        sessions.sort_by_key(|s| std::cmp::Reverse(s.last_activity));
        Ok(sessions)
    }

    fn read(&self, session: &SessionRef, after: Cursor, budget: u64) -> io::Result<Batch> {
        let (file, mut cursor, restarted) = crate::open(&session.path, after)?;
        let mut events = Vec::new();
        let (offset, more) = jsonl::lines(file, cursor.offset, budget, |line| {
            if let Ok(record) = serde_json::from_str::<Value>(line) {
                record_events(&record, &mut cursor, &mut events);
            }
            false
        })?;
        cursor.offset = offset;
        Ok(Batch { events, cursor, restarted, more })
    }
}

/// Collects up to `max` rollout files, walking `depth` levels of date folders with the newest names first.
fn newest_first(dir: &Path, depth: usize, max: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort_by(|a, b| b.cmp(a));
    for path in paths {
        if out.len() >= max {
            return;
        }
        if depth > 0 {
            newest_first(&path, depth - 1, max, out);
        } else if path.extension().is_some_and(|e| e == "jsonl") {
            out.push(path);
        }
    }
}

/// An id or a model, if it has a name's shape.
fn name_of(value: &Value) -> Option<String> {
    value.as_str().filter(|text| station::name(text, "") == *text).map(str::to_string)
}

/// What a rollout says about its session.
fn head(path: &Path) -> Option<Head> {
    // Line 1 is `session_meta` (long: it holds the base instructions); `turn_context` follows soon.
    let head = jsonl::head(path, 16, 4 << 20).ok()?;
    let meta = &head.iter().find(|r| r["type"] == "session_meta")?["payload"];
    let context = head.iter().find(|r| r["type"] == "turn_context");
    Some(Head {
        id: name_of(&meta["id"])?,
        cwd: PathBuf::from(meta["cwd"].as_str()?),
        started: meta["timestamp"].as_str().and_then(jsonl::parse_time),
        model: context.and_then(|r| name_of(&r["payload"]["model"])),
        parent: name_of(&meta["parent_thread_id"]),
    })
}

fn record_events(record: &Value, cursor: &mut Cursor, events: &mut Vec<Event>) {
    let at = jsonl::time_of(record);
    let mut emit = |what| events.push(Event { at, what });
    let payload = &record["payload"];
    match (record["type"].as_str().unwrap_or(""), payload["type"].as_str().unwrap_or("")) {
        ("turn_context", _) => {
            if let Some(model) = name_of(&payload["model"])
                && cursor.model.as_ref() != Some(&model)
                && let Some(from) = cursor.model.replace(model.clone())
            {
                emit(What::ModelChange { from, to: model });
            }
        }
        ("compacted", _) => emit(What::Compaction { tokens_after: None }),
        ("event_msg", "user_message") => emit(What::UserMessage),
        ("event_msg", "token_count") => {
            // `info` is null on a record that only carries rate limits.
            let info = &payload["info"];
            if let Some(last) = info["last_token_usage"].as_object() {
                let count = |key: &str| last.get(key).and_then(Value::as_u64).unwrap_or(0);
                let usage = Usage {
                    input: count("input_tokens"),
                    output: count("output_tokens"),
                    cached: count("cached_input_tokens"),
                };
                let window = info["model_context_window"].as_u64();
                emit(What::ModelStep { model: cursor.model.clone(), usage: Some(usage), window });
            }
        }
        // Esc is not an error; any other reason is.
        ("event_msg", "turn_aborted") if payload["reason"] != "interrupted" => {
            emit(What::Error { kind: station::code(payload["reason"].as_str().unwrap_or(""), "aborted") })
        }
        ("event_msg", "error" | "stream_error") => {
            emit(What::Error { kind: station::code(payload["codex_error_info"].as_str().unwrap_or(""), "error") })
        }
        // Earlier rollouts name the call the command belongs to.
        ("event_msg", "exec_command_end") => refine(cursor, name_of(&payload["call_id"]), payload),
        ("event_msg", "sub_agent_activity") => subagent(payload, &mut emit),
        ("event_msg", "item_completed") => {
            let item = &payload["item"];
            match item["type"].as_str() {
                Some("UserMessage") => emit(What::UserMessage),
                // No call id here: the command is the open `exec` call's, if exactly one is open.
                Some("CommandExecution") => {
                    let only = (cursor.open.len() == 1).then(|| cursor.open.first().cloned()).flatten();
                    refine(cursor, only, item)
                }
                Some("SubAgentActivity") => subagent(item, &mut emit),
                _ => {}
            }
        }
        (
            "response_item",
            "function_call" | "custom_tool_call" | "local_shell_call" | "tool_search_call" | "web_search_call",
        ) => {
            let kind = payload["type"].as_str().unwrap_or("");
            let tool = payload["name"].as_str().unwrap_or(kind);
            let Some(id) = name_of(&payload["call_id"]).or(name_of(&payload["id"])) else { return };
            let (station, summary) = describe(tool, payload);
            emit(What::ToolStart { id: id.clone(), tool: station::name(tool, "tool"), station, summary });
            if kind == "web_search_call" {
                // A web search is written once, when it is over.
                emit(What::ToolEnd { id, failed: false, station: None, summary: None });
            } else if tool == "exec" {
                if cursor.open.len() >= MAX_OPEN {
                    cursor.open.pop_first();
                }
                cursor.open.insert(id);
            }
        }
        (
            "response_item",
            "function_call_output" | "custom_tool_call_output" | "local_shell_call_output" | "tool_search_output",
        ) => {
            let Some(id) = name_of(&payload["call_id"]) else { return };
            cursor.open.remove(&id);
            // What the call's command turned out to be, if a record said.
            let refined = cursor.refined.remove(&id);
            let (failed, station, summary) =
                refined.map_or((false, None, None), |r| (r.failed, Some(r.station), Some(r.summary)));
            emit(What::ToolEnd { id, failed, station, summary });
        }
        _ => {}
    }
}

/// Notes what call `id`'s command was: its station and label, and whether it failed.
/// A call that ran several commands keeps the last one's name, and failed if any did.
fn refine(cursor: &mut Cursor, id: Option<String>, item: &Value) {
    let (Some(id), Some((station, summary))) = (id, command(&item["command"])) else { return };
    let failed = item["status"] == "failed" || item["exit_code"].as_i64().is_some_and(|code| code != 0);
    let failed = failed || cursor.refined.get(&id).is_some_and(|r| r.failed);
    if cursor.refined.len() >= MAX_OPEN {
        cursor.refined.pop_first();
    }
    cursor.refined.insert(id, Refined { station, summary, failed });
}

/// A command's station and label: a string is a command line; an array is its words, used as they are.
fn command(command: &Value) -> Option<(Station, String)> {
    if let Some(line) = command.as_str() {
        return Some(station::describe_command(line));
    }
    let words: Vec<&str> = command.as_array()?.iter().filter_map(Value::as_str).collect();
    Some(station::describe_argv(&words))
}

fn subagent(activity: &Value, emit: &mut impl FnMut(What)) {
    let Some(id) = name_of(&activity["agent_thread_id"]) else { return };
    match activity["kind"].as_str() {
        Some("started") => {
            // The last part of e.g. `root/reviewer`.
            let last = activity["agent_path"].as_str().and_then(|p| p.rsplit('/').next()).unwrap_or("");
            emit(What::SubAgentStart { id, label: station::name(last, "agent") })
        }
        Some("completed" | "interrupted" | "failed") => emit(What::SubAgentEnd { id }),
        _ => {}
    }
}

/// A tool call's station and label, from its name and the names in its arguments.
fn describe(tool: &str, payload: &Value) -> (Station, String) {
    if payload["namespace"] == "collaboration" {
        return (Station::Agents, station::name(tool, "tool"));
    }
    // `function_call` arguments are a JSON string; a `custom_tool_call` input is plain text.
    let arguments: Value =
        payload["arguments"].as_str().and_then(|a| serde_json::from_str(a).ok()).unwrap_or(Value::Null);
    let ran = [&arguments["cmd"], &arguments["command"], &payload["action"]["command"]].into_iter().find_map(command);
    let shell = station::classify(tool, None) == Station::Shell;
    match (ran, tool) {
        (Some(ran), _) if shell => ran,
        // The first file a patch names, e.g. `*** Update File: src/main.rs`.
        (_, "apply_patch") => {
            let lines = payload["input"].as_str().unwrap_or("").lines();
            let file = lines.filter_map(|l| l.strip_prefix("*** ")).find_map(|l| l.split_once(" File: "));
            (Station::Files, file.map_or_else(|| tool.to_string(), |(_, path)| station::path_label(path)))
        }
        _ => (station::classify(tool, None), station::name(tool, "tool")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn commands_are_named_from_their_words() {
        let cases = [
            (json!("ls -la"), Station::Files, "ls"),
            (json!(["/bin/zsh", "-lc", "git -C client push"]), Station::Network, "git push"),
            (json!(["bash", "-c", "ls"]), Station::Files, "ls"),
            (json!(["git", "status"]), Station::Shell, "git status"),
            (json!(["git", "private-thing"]), Station::Shell, "git"),
            (json!(["private tool", "git", "push"]), Station::Shell, "shell"),
            (json!(["ls", "a; curl private"]), Station::Files, "ls"),
        ];
        for (words, station, label) in cases {
            assert_eq!(command(&words), Some((station, label.to_string())), "{words}");
        }
        assert_eq!(command(&json!(null)), None);
    }

    #[test]
    fn describes_calls_by_name_only() {
        let cases = [
            (
                json!({"name": "exec_command", "arguments": "{\"cmd\":\"rg secret src\",\"workdir\":\"/w\"}"}),
                Station::Files,
                "rg",
            ),
            (
                json!({"name": "shell", "arguments": "{\"command\":[\"bash\",\"-lc\",\"curl example.com\"]}"}),
                Station::Network,
                "curl",
            ),
            (
                json!({"name": "shell", "arguments": "{\"command\":[\"private-tool\",\"--key\",\"abc\"]}"}),
                Station::Shell,
                "shell",
            ),
            (json!({"name": "exec", "input": "text(await run('anything'))"}), Station::Shell, "exec"),
            (
                json!({"name": "apply_patch", "input": "*** Begin Patch\n*** Update File: /w/src/main.rs\n+secret\n"}),
                Station::Files,
                "src/main.rs",
            ),
            (json!({"name": "apply_patch", "input": "garbage"}), Station::Files, "apply_patch"),
            (
                json!({"name": "spawn_agent", "namespace": "collaboration", "arguments": "{\"message\":\"private\"}"}),
                Station::Agents,
                "spawn_agent",
            ),
            (
                json!({"name": "sleep", "namespace": "clock", "arguments": "{\"duration_ms\":5}"}),
                Station::Other,
                "sleep",
            ),
            (json!({"name": "update_plan", "arguments": "not json"}), Station::Other, "update_plan"),
            (json!({"name": "a private tool", "arguments": "{\"cmd\":\"curl x\"}"}), Station::Other, "tool"),
        ];
        for (payload, station, summary) in cases {
            let tool = payload["name"].as_str().unwrap();
            assert_eq!(describe(tool, &payload), (station, summary.to_string()), "{tool}");
        }
    }

    fn record(kind: &str, payload: Value) -> Value {
        json!({"timestamp": "2026-11-12T09:00:00.000Z", "type": kind, "payload": payload})
    }

    fn exec(call: &str) -> Value {
        record("response_item", json!({"type": "custom_tool_call", "call_id": call, "name": "exec", "input": "x"}))
    }

    fn ran(words: Value, status: &str) -> Value {
        let item = json!({"type": "CommandExecution", "id": "i", "command": words, "status": status});
        record("event_msg", json!({"type": "item_completed", "item": item}))
    }

    fn output(call: &str) -> Value {
        record("response_item", json!({"type": "custom_tool_call_output", "call_id": call, "output": []}))
    }

    fn ends(records: &[Value]) -> Vec<(String, bool, Option<String>)> {
        let (mut cursor, mut events) = (Cursor::default(), Vec::new());
        records.iter().for_each(|r| record_events(r, &mut cursor, &mut events));
        assert!(cursor.open.is_empty() && cursor.refined.is_empty(), "nothing is kept once every call ended");
        let ends = events.into_iter().filter_map(|e| match e.what {
            What::ToolEnd { id, failed, summary, .. } => Some((id, failed, summary)),
            _ => None,
        });
        ends.collect()
    }

    #[test]
    fn a_command_belongs_to_the_one_open_call() {
        let records =
            [exec("a"), ran(json!(["ls"]), "completed"), ran(json!(["cargo", "test"]), "failed"), output("a")];
        assert_eq!(ends(&records), [("a".to_string(), true, Some("cargo test".to_string()))]);
    }

    #[test]
    fn parallel_calls_do_not_take_each_others_commands() {
        // Two calls open: nothing says whose command it was, so neither is renamed.
        let records = [exec("a"), exec("b"), ran(json!(["curl", "x"]), "failed"), output("b"), output("a")];
        assert_eq!(ends(&records), [("b".to_string(), false, None), ("a".to_string(), false, None)]);
        // Earlier rollouts name the call: each gets its own, whatever the order.
        let end = |call: &str, words: Value, code: i64| {
            record(
                "event_msg",
                json!({"type": "exec_command_end", "call_id": call, "command": words, "exit_code": code}),
            )
        };
        let call = |id: &str| {
            record(
                "response_item",
                json!({"type": "function_call", "call_id": id, "name": "exec_command", "arguments": "{}"}),
            )
        };
        let done =
            |id: &str| record("response_item", json!({"type": "function_call_output", "call_id": id, "output": ""}));
        let records =
            [call("a"), call("b"), end("b", json!(["curl", "x"]), 0), end("a", json!(["ls"]), 2), done("a"), done("b")];
        let expected =
            [("a".to_string(), true, Some("ls".to_string())), ("b".to_string(), false, Some("curl".to_string()))];
        assert_eq!(ends(&records), expected);
    }
}
