//! Codex. Rollouts: `<codex home>/sessions/<yyyy>/<mm>/<dd>/rollout-<time>-<id>.jsonl`.
//! Schema seen in codex-cli 0.166.0 and in rollouts from earlier 2026 versions.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::cache::Cache;
use crate::station::{self, Station};
use crate::{
    Batch, Cursor, Event, Kind, MAX_OPEN, Refined, Runner, SessionRef, Usage, What, jsonl, opaque, recorded_cwd, track,
};

/// The values of an error event's `codex_error_info`: Codex's error kinds, in the snake case it
/// writes. One (`usage_limit_exceeded`) was seen in a rollout; the rest are from Codex's source
/// as remembered, not checked against 0.166. Anything else is shown as `error`.
const ERRORS: &[&str] = &[
    "context_window_exceeded",
    "usage_limit_exceeded",
    "http_connection_failed",
    "response_stream_connection_failed",
    "response_stream_disconnected",
    "response_too_many_failed_attempts",
    "internal_server_error",
    "unauthorized",
    "bad_request",
    "sandbox_error",
    "thread_rollback_failed",
    "other",
];

/// The reasons of an aborted turn. `interrupted` (Esc) is no error. Anything else is shown as `aborted`.
const ABORTS: &[&str] = &["replaced", "review_ended"];

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
        Self::new(crate::config_dirs().codex.unwrap_or_default())
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
                let (pid, busy) = (None, None);
                sessions.push(SessionRef { kind, id, cwd, path, started, last_activity, model, pid, busy, parent });
            }
        }
        self.heads.keep(&rollouts.into_iter().collect::<HashSet<_>>());
        sessions.sort_by_key(|s| std::cmp::Reverse(s.last_activity));
        Ok(sessions)
    }

    fn read(&self, session: &SessionRef, after: Cursor, budget: u64) -> io::Result<Batch> {
        let (file, mut cursor, restarted) = crate::open(&session.path, after)?;
        let mut events = Vec::new();
        let mut skipping = cursor.skipping;
        let (offset, more) = jsonl::lines(file, cursor.offset, budget, &mut skipping, |line| {
            if let Ok(record) = serde_json::from_str::<Value>(line) {
                record_events(&record, &mut cursor, &mut events);
            }
            false
        })?;
        (cursor.offset, cursor.skipping) = (offset, skipping);
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

/// A session id or a model, as written, if it is a name (`station::is_name`).
fn name_of(value: &Value) -> Option<String> {
    value.as_str().filter(|text| station::is_name(text)).map(str::to_string)
}

/// A call or thread id, made opaque. `None` if it is missing or empty.
fn id_of(value: &Value) -> Option<String> {
    value.as_str().and_then(opaque)
}

/// What a rollout says about its session. None if its id is not a name or its cwd not a plain path.
fn head(path: &Path) -> Option<Head> {
    // Line 1 is `session_meta` (long: it holds the base instructions); `turn_context` follows soon.
    let head = jsonl::head(path, 16, 4 << 20).ok()?;
    let meta = &head.iter().find(|r| r["type"] == "session_meta")?["payload"];
    let context = head.iter().find(|r| r["type"] == "turn_context");
    Some(Head {
        id: name_of(&meta["id"])?,
        cwd: recorded_cwd(meta["cwd"].as_str()?)?,
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
                let (input, output, cached) =
                    (count("input_tokens"), count("output_tokens"), count("cached_input_tokens"));
                let window = info["model_context_window"].as_u64();
                emit(What::ModelStep {
                    model: cursor.model.clone(),
                    usage: Some(Usage { input, output, cached }),
                    window,
                });
            }
        }
        // Esc is not an error; any other reason is.
        ("event_msg", "turn_aborted") if payload["reason"] != "interrupted" => {
            emit(What::Error { kind: station::code(payload["reason"].as_str().unwrap_or(""), ABORTS, "aborted") })
        }
        ("event_msg", "error" | "stream_error") => emit(What::Error {
            kind: station::code(payload["codex_error_info"].as_str().unwrap_or(""), ERRORS, "error"),
        }),
        // Earlier rollouts name the call the command belongs to.
        ("event_msg", "exec_command_end") => refine(cursor, id_of(&payload["call_id"]), payload),
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
            let Some(id) = id_of(&payload["call_id"]).or(id_of(&payload["id"])) else { return };
            let (station, summary) = describe(tool, payload);
            emit(What::ToolStart { id: id.clone(), tool: station::tool_label(tool), station, summary });
            if kind == "web_search_call" {
                // A web search is written once, when it is over.
                emit(What::ToolEnd { id, failed: false, station: None, summary: None });
            } else if tool == "exec" {
                track(&mut cursor.open, id);
            }
        }
        (
            "response_item",
            "function_call_output" | "custom_tool_call_output" | "local_shell_call_output" | "tool_search_output",
        ) => {
            let Some(id) = id_of(&payload["call_id"]) else { return };
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
    let Some(id) = id_of(&activity["agent_thread_id"]) else { return };
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
        return (Station::Agents, station::tool_label(tool));
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
        _ => (station::classify(tool, None), station::tool_label(tool)),
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
            (
                json!({"name": "apply_patch", "input": "*** Update File: a private sentence, not a path\n"}),
                Station::Files,
                "file",
            ),
            (json!({"name": "apply_patch", "input": "*** Add File: /tmp/TOKEN=abc/x.txt\n"}), Station::Files, "file"),
            (json!({"name": "apply_patch", "input": "garbage"}), Station::Files, "apply_patch"),
            (
                json!({"name": "spawn_agent", "namespace": "collaboration", "arguments": "{\"message\":\"private\"}"}),
                Station::Agents,
                "spawn_agent",
            ),
            (json!({"name": "private_name", "namespace": "collaboration", "arguments": "{}"}), Station::Agents, "tool"),
            (
                json!({"name": "sleep", "namespace": "clock", "arguments": "{\"duration_ms\":5}"}),
                Station::Other,
                "sleep",
            ),
            (json!({"name": "update_plan", "arguments": "not json"}), Station::Other, "update_plan"),
            (json!({"name": "private_tool", "arguments": "{\"cmd\":\"curl x\"}"}), Station::Other, "tool"),
        ];
        for (payload, station, summary) in cases {
            let tool = payload["name"].as_str().unwrap();
            assert_eq!(describe(tool, &payload), (station, summary.to_string()), "{payload}");
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

    fn fold(records: &[Value]) -> (Cursor, Vec<Event>) {
        let (mut cursor, mut events) = (Cursor::default(), Vec::new());
        records.iter().for_each(|r| record_events(r, &mut cursor, &mut events));
        (cursor, events)
    }

    /// Each ended call as (the call's name in the test, failed, the label its end brought).
    fn ends(records: &[Value]) -> Vec<(String, bool, Option<String>)> {
        let (cursor, events) = fold(records);
        assert!(cursor.open.is_empty() && cursor.refined.is_empty(), "nothing is kept once every call ended");
        let call = |id: &str| ["a", "b"].into_iter().find(|c| opaque(c).as_deref() == Some(id)).unwrap().to_string();
        let ends = events.into_iter().filter_map(|e| match e.what {
            What::ToolEnd { id, failed, summary, .. } => Some((call(&id), failed, summary)),
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

    #[test]
    fn errors_come_from_a_table_and_empty_ids_are_no_ids() {
        let kinds = |records: &[Value]| {
            let kinds = fold(records).1.into_iter().filter_map(|e| match e.what {
                What::Error { kind } => Some(kind),
                _ => None,
            });
            kinds.collect::<Vec<_>>()
        };
        let error =
            |info: &str| record("event_msg", json!({"type": "error", "message": "private", "codex_error_info": info}));
        let aborted = |reason: &str| record("event_msg", json!({"type": "turn_aborted", "reason": reason}));
        let records = [
            error("usage_limit_exceeded"),
            error("zz_private_code"),
            error("Key sk-1 is over its limit"),
            aborted("replaced"),
            aborted("zz_private_reason"),
            aborted("interrupted"),
        ];
        assert_eq!(kinds(&records), ["usage_limit_exceeded", "error", "error", "replaced", "aborted"]);

        let (cursor, events) = fold(&[exec(""), output(""), ran(json!(["ls"]), "completed")]);
        assert!(events.is_empty() && cursor.open.is_empty() && cursor.refined.is_empty(), "{events:?}");
    }

    #[test]
    fn a_cursor_does_not_grow_without_bound() {
        let starts: Vec<Value> = (0..2000).map(|i| exec(&format!("call-{i}"))).collect();
        let (mut cursor, mut events) = fold(&starts);
        assert_eq!(cursor.open.len(), MAX_OPEN);
        let end = |call: usize| {
            let words = json!(["ls"]);
            record("event_msg", json!({"type": "exec_command_end", "call_id": format!("c{call}"), "command": words}))
        };
        (0..2000).for_each(|i| record_events(&end(i), &mut cursor, &mut events));
        assert_eq!(cursor.refined.len(), MAX_OPEN);
    }
}
