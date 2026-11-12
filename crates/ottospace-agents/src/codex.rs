//! Codex. Rollouts: `<codex home>/sessions/<yyyy>/<mm>/<dd>/rollout-<time>-<id>.jsonl`.
//! Schema seen in codex-cli 0.166.0 and in rollouts from earlier 2026 versions.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::station::{self, Station};
use crate::{Cursor, Event, Kind, Refined, Runner, SessionRef, Usage, What, jsonl};

pub struct Codex {
    home: PathBuf,
    /// How many rollouts `sessions` opens, newest day first. Rollouts have no index by directory.
    pub max_files: usize,
}

impl Codex {
    pub fn new(home: PathBuf) -> Self {
        Self { home, max_files: 1000 }
    }

    /// `$CODEX_HOME`, or `~/.codex`.
    pub fn from_env() -> Self {
        let home = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".codex"));
        Self::new(std::env::var_os("CODEX_HOME").map(PathBuf::from).or(home).unwrap_or_default())
    }
}

impl Runner for Codex {
    fn kind(&self) -> Kind {
        Kind::Codex
    }

    fn sessions(&self, dir: &Path) -> io::Result<Vec<SessionRef>> {
        let mut rollouts = Vec::new();
        newest_first(&self.home.join("sessions"), 3, self.max_files, &mut rollouts);
        let mut sessions: Vec<SessionRef> = rollouts.iter().filter_map(|path| session(path, dir)).collect();
        sessions.sort_by_key(|s| std::cmp::Reverse(s.last_activity));
        Ok(sessions)
    }

    fn read(&self, session: &SessionRef, after: Cursor) -> io::Result<(Vec<Event>, Cursor)> {
        let mut cursor = after;
        let mut events = Vec::new();
        let offset = jsonl::lines_after(&session.path, cursor.offset, |line| {
            if let Ok(record) = serde_json::from_str::<Value>(line) {
                record_events(&record, &mut cursor, &mut events);
            }
        })?;
        cursor.offset = offset;
        Ok((events, cursor))
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

/// The session in one rollout, if it started in `dir` or below it.
fn session(path: &Path, dir: &Path) -> Option<SessionRef> {
    // Line 1 is `session_meta` (long: it holds the base instructions); `turn_context` follows soon.
    let head = jsonl::head(path, 16, 4 << 20).ok()?;
    let meta = &head.iter().find(|r| r["type"] == "session_meta")?["payload"];
    let cwd = PathBuf::from(meta["cwd"].as_str()?);
    if !cwd.starts_with(dir) {
        return None;
    }
    let context = head.iter().find(|r| r["type"] == "turn_context");
    Some(SessionRef {
        kind: Kind::Codex,
        id: meta["id"].as_str()?.to_string(),
        cwd,
        path: path.to_path_buf(),
        started: meta["timestamp"].as_str().and_then(jsonl::parse_time),
        last_activity: jsonl::modified_ms(path),
        model: context.and_then(|r| r["payload"]["model"].as_str()).map(str::to_string),
        pid: None,
        busy: None,
        parent: meta["parent_thread_id"].as_str().map(str::to_string),
    })
}

fn record_events(record: &Value, cursor: &mut Cursor, events: &mut Vec<Event>) {
    let at = jsonl::time_of(record);
    let mut emit = |what| events.push(Event { at, what });
    let payload = &record["payload"];
    match (record["type"].as_str().unwrap_or(""), payload["type"].as_str().unwrap_or("")) {
        ("turn_context", _) => {
            if let Some(model) = payload["model"].as_str()
                && cursor.model.as_deref() != Some(model)
                && let Some(from) = cursor.model.replace(model.to_string())
            {
                emit(What::ModelChange { from, to: model.to_string() });
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
            emit(What::Error { kind: station::clean(payload["reason"].as_str().unwrap_or("aborted")) })
        }
        ("event_msg", "error" | "stream_error") => {
            emit(What::Error { kind: station::clean(payload["codex_error_info"].as_str().unwrap_or("error")) })
        }
        ("event_msg", "exec_command_end") => cursor.refined = refined(payload, payload["call_id"].as_str()),
        ("event_msg", "sub_agent_activity") => subagent(payload, &mut emit),
        ("event_msg", "item_completed") => {
            let item = &payload["item"];
            match item["type"].as_str() {
                Some("UserMessage") => emit(What::UserMessage),
                Some("CommandExecution") => cursor.refined = refined(item, None),
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
            let id = payload["call_id"].as_str().or(payload["id"].as_str()).unwrap_or("").to_string();
            let (station, summary) = describe(tool, payload);
            emit(What::ToolStart { id: id.clone(), tool: station::clean(tool), station, summary });
            // A web search is written once, when it is over.
            if kind == "web_search_call" {
                emit(What::ToolEnd { id, failed: false, station: None, summary: None });
            }
        }
        (
            "response_item",
            "function_call_output" | "custom_tool_call_output" | "local_shell_call_output" | "tool_search_output",
        ) => {
            let id = payload["call_id"].as_str().unwrap_or("").to_string();
            // What the command turned out to be, if the record before this one said.
            let refined = cursor.refined.take().filter(|r| r.call.as_deref().is_none_or(|call| call == id));
            let (failed, station, summary) =
                refined.map_or((false, None, None), |r| (r.failed, Some(r.station), Some(r.summary)));
            emit(What::ToolEnd { id, failed, station, summary });
        }
        _ => {}
    }
}

/// A finished command: its station and label from the real command line, and whether it failed.
fn refined(item: &Value, call: Option<&str>) -> Option<Refined> {
    let line = command_line(&item["command"])?;
    let failed = item["status"] == "failed" || item["exit_code"].as_i64().is_some_and(|code| code != 0);
    let (station, summary) = (station::classify_command(&line), station::command_label(&line));
    Some(Refined { call: call.map(str::to_string), station, summary, failed })
}

/// A command as one line: a string as it is; of `["/bin/zsh", "-lc", "<line>"]` the line.
fn command_line(command: &Value) -> Option<String> {
    if let Some(line) = command.as_str() {
        return Some(line.to_string());
    }
    let words: Vec<&str> = command.as_array()?.iter().filter_map(Value::as_str).collect();
    match words.as_slice() {
        [_shell, flag, line] if flag.starts_with('-') && flag.ends_with('c') => Some(line.to_string()),
        _ => Some(words.join(" ")),
    }
}

fn subagent(activity: &Value, emit: &mut impl FnMut(What)) {
    let Some(id) = activity["agent_thread_id"].as_str().map(str::to_string) else { return };
    match activity["kind"].as_str() {
        Some("started") => {
            let name = activity["agent_path"].as_str().and_then(|p| p.rsplit('/').next()).filter(|n| !n.is_empty());
            emit(What::SubAgentStart { id, label: station::clean(name.unwrap_or("agent")) })
        }
        Some("completed" | "interrupted" | "failed") => emit(What::SubAgentEnd { id }),
        _ => {}
    }
}

/// A tool call's station and label, from its name and the names in its arguments.
fn describe(tool: &str, payload: &Value) -> (Station, String) {
    // `function_call` arguments are a JSON string; a `custom_tool_call` input is plain text.
    let arguments: Value =
        payload["arguments"].as_str().and_then(|a| serde_json::from_str(a).ok()).unwrap_or(Value::Null);
    let command =
        [&arguments["cmd"], &arguments["command"], &payload["action"]["command"]].into_iter().find_map(command_line);
    if payload["namespace"] == "collaboration" {
        return (Station::Agents, station::clean(tool));
    }
    let station = station::classify(tool, command.as_deref());
    let summary = match (tool, &command) {
        (_, Some(command)) => station::command_label(command),
        // The first file a patch names, e.g. `*** Update File: src/main.rs`.
        ("apply_patch", _) => {
            let lines = payload["input"].as_str().unwrap_or("").lines();
            let file = lines.filter_map(|l| l.strip_prefix("*** ")).find_map(|l| l.split_once(" File: "));
            file.map_or_else(|| tool.to_string(), |(_, path)| station::path_label(path))
        }
        _ => station::clean(tool),
    };
    (station, summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_command_lines() {
        assert_eq!(command_line(&json!("ls -la")).as_deref(), Some("ls -la"));
        assert_eq!(command_line(&json!(["/bin/zsh", "-lc", "git push"])).as_deref(), Some("git push"));
        assert_eq!(command_line(&json!(["bash", "-c", "ls"])).as_deref(), Some("ls"));
        assert_eq!(command_line(&json!(["git", "status"])).as_deref(), Some("git status"));
        assert_eq!(command_line(&json!(null)), None);
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
        ];
        for (payload, station, summary) in cases {
            let tool = payload["name"].as_str().unwrap();
            assert_eq!(describe(tool, &payload), (station, summary.to_string()), "{tool}");
        }
    }
}
