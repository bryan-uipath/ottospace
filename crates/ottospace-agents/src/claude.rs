//! Claude Code. Transcripts: `<config dir>/projects/<slugged cwd>/<session id>.jsonl`.
//! Live processes: `<config dir>/sessions/<pid>.json`. Schema seen in Claude Code 2.1.322.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::station::{self, Station};
use crate::{Cursor, Event, Kind, Runner, SessionRef, Usage, What, jsonl};

pub struct Claude {
    config_dirs: Vec<PathBuf>,
}

impl Claude {
    /// One runner over several config dirs, e.g. `~/.claude` and `~/.claude-personal`.
    pub fn new(config_dirs: Vec<PathBuf>) -> Self {
        Self { config_dirs }
    }

    /// `$CLAUDE_CONFIG_DIR` if set, and `~/.claude`.
    pub fn from_env() -> Self {
        let mut dirs: Vec<PathBuf> = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).into_iter().collect();
        if let Some(home) = std::env::var_os("HOME") {
            let default = PathBuf::from(home).join(".claude");
            if !dirs.contains(&default) {
                dirs.push(default);
            }
        }
        Self::new(dirs)
    }
}

/// Claude Code's folder name for a directory: every character but ASCII letters and digits becomes `-`.
/// e.g. `/Users/me/my.app` → `-Users-me-my-app`.
pub fn slug(dir: &Path) -> String {
    dir.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

impl Runner for Claude {
    fn kind(&self) -> Kind {
        Kind::Claude
    }

    fn sessions(&self, dir: &Path) -> io::Result<Vec<SessionRef>> {
        let slug = slug(dir);
        let below = format!("{slug}-");
        let mut sessions = Vec::new();
        for config in &self.config_dirs {
            let live = live(config);
            let Ok(projects) = fs::read_dir(config.join("projects")) else { continue };
            for project in projects.flatten() {
                // The folder of `dir` itself, or of a directory below it. The slug is lossy
                // (`/a/b-c` and `/a/b/c` share one), so `session` checks the recorded cwd too.
                let name = project.file_name().to_string_lossy().into_owned();
                if name != slug && !name.starts_with(&below) {
                    continue;
                }
                let Ok(files) = fs::read_dir(project.path()) else { continue };
                let transcripts =
                    files.flatten().map(|f| f.path()).filter(|p| p.extension().is_some_and(|e| e == "jsonl"));
                sessions.extend(transcripts.filter_map(|path| session(&path, dir, &live)));
            }
        }
        sessions.sort_by_key(|s| std::cmp::Reverse(s.last_activity));
        Ok(sessions)
    }

    fn read(&self, session: &SessionRef, after: Cursor) -> io::Result<(Vec<Event>, Cursor)> {
        let mut cursor = after;
        let mut events = Vec::new();
        let offset = jsonl::lines_after(&session.path, cursor.offset, |line| {
            // Request logs: most of the file's bytes, nothing a station shows.
            if line.starts_with(r#"{"type":"api-request"#) {
                return;
            }
            if let Ok(record) = serde_json::from_str::<Value>(line) {
                record_events(&record, &mut cursor, &mut events);
            }
        })?;
        cursor.offset = offset;
        finished_subagents(&session.path, &mut cursor, &mut events);
        Ok((events, cursor))
    }
}

/// Running Claude Code processes of one config dir: session id → (pid, busy).
fn live(config: &Path) -> HashMap<String, (u32, Option<bool>)> {
    let Ok(files) = fs::read_dir(config.join("sessions")) else { return HashMap::new() };
    let records = files.flatten().filter(|f| f.path().extension().is_some_and(|e| e == "json"));
    records
        .filter_map(|file| {
            let record: Value = serde_json::from_str(&fs::read_to_string(file.path()).ok()?).ok()?;
            let pid = u32::try_from(record["pid"].as_u64()?).ok()?;
            let busy = record["status"].as_str().map(|s| s == "busy");
            Some((record["sessionId"].as_str()?.to_string(), (pid, busy)))
        })
        .collect()
}

/// The session in one transcript, if it started in `dir` or below it.
fn session(path: &Path, dir: &Path, live: &HashMap<String, (u32, Option<bool>)>) -> Option<SessionRef> {
    // The first records are settings without a cwd; message records follow within a few lines.
    let head = jsonl::head(path, 40, 1 << 20).ok()?;
    let cwd = PathBuf::from(head.iter().find_map(|r| r["cwd"].as_str())?);
    if !cwd.starts_with(dir) {
        return None;
    }
    let id = path.file_stem()?.to_string_lossy().into_owned();
    let (pid, busy) = live.get(&id).map_or((None, None), |(pid, busy)| (Some(*pid), *busy));
    Some(SessionRef {
        kind: Kind::Claude,
        cwd,
        path: path.to_path_buf(),
        started: head.iter().map(jsonl::time_of).filter(|t| *t > 0).min(),
        last_activity: jsonl::modified_ms(path),
        model: last_model(path),
        pid,
        busy,
        parent: None,
        id,
    })
}

/// The model of the last model request near the end of a transcript.
fn last_model(path: &Path) -> Option<String> {
    let lines = jsonl::tail(path, 256 << 10).ok()?;
    let assistant = lines.iter().rev().filter(|l| l.contains(r#""type":"assistant""#));
    assistant
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find_map(|r| r["message"]["model"].as_str().filter(|m| *m != "<synthetic>").map(str::to_string))
}

fn record_events(record: &Value, cursor: &mut Cursor, events: &mut Vec<Event>) {
    // A sub-agent's own records live in its own file; any here are not the main loop's.
    if record["isSidechain"] == true {
        return;
    }
    let at = jsonl::time_of(record);
    let mut emit = |what| events.push(Event { at, what });
    match record["type"].as_str() {
        Some("user") => user(record, cursor, &mut emit),
        Some("assistant") => assistant(record, cursor, &mut emit),
        Some("system") if record["subtype"] == "compact_boundary" => {
            emit(What::Compaction { tokens_after: record["compactMetadata"]["postTokens"].as_u64() })
        }
        _ => {}
    }
}

fn user(record: &Value, cursor: &mut Cursor, emit: &mut impl FnMut(What)) {
    let content = &record["message"]["content"];
    let blocks = content.as_array().map(Vec::as_slice).unwrap_or_default();
    let results: Vec<&Value> = blocks.iter().filter(|b| b["type"] == "tool_result").collect();
    for result in &results {
        let Some(id) = result["tool_use_id"].as_str() else { continue };
        emit(What::ToolEnd { id: id.to_string(), failed: result["is_error"] == true, station: None, summary: None });
        // A background agent's call returns at once with `async_launched`; the agent works on.
        if record["toolUseResult"]["status"] != "async_launched" && cursor.agents.remove(id) {
            emit(What::SubAgentEnd { id: id.to_string() });
        }
    }
    // What is left is a prompt, unless Claude Code wrote it itself (`isMeta`, a compaction summary).
    let written = content.is_string() || !blocks.is_empty();
    if results.is_empty() && written && record["isMeta"] != true && record["isCompactSummary"] != true {
        emit(What::UserMessage);
    }
}

fn assistant(record: &Value, cursor: &mut Cursor, emit: &mut impl FnMut(What)) {
    let message = &record["message"];
    // An API failure is written as an assistant record from the model `<synthetic>`.
    if record["isApiErrorMessage"] == true || record["error"].is_string() {
        emit(What::Error { kind: station::clean(record["error"].as_str().unwrap_or("api_error")) });
        return;
    }
    let model = message["model"].as_str().filter(|m| *m != "<synthetic>");
    if let Some(model) = model
        && cursor.model.as_deref() != Some(model)
        && let Some(from) = cursor.model.replace(model.to_string())
    {
        emit(What::ModelChange { from, to: model.to_string() });
    }
    // One request is written as one record per content block, all with the same message id.
    let id = message["id"].as_str();
    if id.is_none() || cursor.step.as_deref() != id {
        cursor.step = id.map(str::to_string);
        let usage = message["usage"].as_object().map(|u| {
            let count = |key: &str| u.get(key).and_then(Value::as_u64).unwrap_or(0);
            let cached = count("cache_read_input_tokens");
            let input = count("input_tokens") + cached + count("cache_creation_input_tokens");
            Usage { input, output: count("output_tokens"), cached }
        });
        let window = usage.map(|u| window(model.unwrap_or(""), u.input + u.output));
        emit(What::ModelStep { model: model.map(str::to_string), usage, window });
        if message["stop_reason"] == "refusal" {
            emit(What::Error { kind: "refusal".to_string() });
        }
    }
    for block in message["content"].as_array().map(Vec::as_slice).unwrap_or_default() {
        let (Some("tool_use"), Some(id), Some(tool)) =
            (block["type"].as_str(), block["id"].as_str(), block["name"].as_str())
        else {
            continue;
        };
        let (station, summary) = describe(tool, &block["input"]);
        emit(What::ToolStart { id: id.to_string(), tool: station::clean(tool), station, summary: summary.clone() });
        if matches!(tool, "Agent" | "Task") && cursor.agents.insert(id.to_string()) {
            emit(What::SubAgentStart { id: id.to_string(), label: summary });
        }
    }
}

/// The context window, estimated: the transcript does not record it. 200k, or 1M once a request exceeds that.
fn window(model: &str, used: u64) -> u64 {
    if model.contains("[1m]") || used > 200_000 { 1_000_000 } else { 200_000 }
}

/// A tool call's station and label, from its name and the names in its input.
fn describe(tool: &str, input: &Value) -> (Station, String) {
    let text = |key: &str| input[key].as_str();
    let path = text("file_path").or(text("notebook_path")).or(text("path"));
    let arg = text("command").or(path);
    let station = station::classify(tool, arg);
    let summary = if let Some(rest) = tool.strip_prefix("mcp__") {
        station::clean(&rest.replacen("__", "/", 1))
    } else if let Some(command) = text("command") {
        station::command_label(command)
    } else if let Some(path) = path.filter(|_| matches!(station, Station::Files | Station::Memory)) {
        station::path_label(path)
    } else {
        match tool {
            "WebFetch" => station::host_label(text("url").unwrap_or("")),
            "Skill" => station::clean(text("skill").unwrap_or(tool)),
            // The agent's type, not its description: that is free text the model wrote.
            "Agent" | "Task" => station::clean(text("subagent_type").unwrap_or("agent")),
            _ => station::clean(tool),
        }
    };
    (station, summary)
}

/// Ends sub-agents whose `<session>/subagents/agent-*.meta.json` says they finished or were stopped.
fn finished_subagents(transcript: &Path, cursor: &mut Cursor, events: &mut Vec<Event>) {
    if cursor.agents.is_empty() {
        return;
    }
    let Ok(files) = fs::read_dir(transcript.with_extension("").join("subagents")) else { return };
    for path in files.flatten().map(|f| f.path()).filter(|p| p.to_string_lossy().ends_with(".meta.json")) {
        let Some(meta) = fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else {
            continue;
        };
        let done = meta["finished"] == true || meta["stoppedByUser"] == true;
        if let Some(id) = meta["toolUseId"].as_str().filter(|id| done && cursor.agents.remove(*id)) {
            let at = jsonl::modified_ms(&path).unwrap_or(0);
            events.push(Event { at, what: What::SubAgentEnd { id: id.to_string() } });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_like_claude_code() {
        assert_eq!(slug(Path::new("/Users/me/code/app")), "-Users-me-code-app");
        assert_eq!(slug(Path::new("/Users/me.name/.cfg/my_app")), "-Users-me-name--cfg-my-app");
    }

    #[test]
    fn describes_calls_by_name_only() {
        let cases = [
            (
                "Bash",
                r#"{"command":"cd app && git push -f origin secret-branch","description":"Push"}"#,
                Station::Network,
                "git push",
            ),
            ("Read", r#"{"file_path":"/Users/me/app/src/main.rs"}"#, Station::Files, "src/main.rs"),
            ("Write", r#"{"file_path":"/Users/me/app/.env","content":"TOKEN=abc"}"#, Station::Files, "app/.env"),
            ("Grep", r#"{"pattern":"password","path":"/Users/me/app/src"}"#, Station::Files, "app/src"),
            ("Grep", r#"{"pattern":"password"}"#, Station::Files, "Grep"),
            (
                "WebFetch",
                r#"{"url":"https://example.com/a?key=abc","prompt":"read it"}"#,
                Station::Network,
                "example.com",
            ),
            ("WebSearch", r#"{"query":"something private"}"#, Station::Network, "WebSearch"),
            ("Skill", r#"{"skill":"review","args":"private"}"#, Station::Skills, "review"),
            (
                "Agent",
                r#"{"description":"Look into it","prompt":"long","subagent_type":"Explore"}"#,
                Station::Agents,
                "Explore",
            ),
            ("Agent", r#"{"description":"Look into it","prompt":"long"}"#, Station::Agents, "agent"),
            ("mcp__playwright__browser_click", r#"{"ref":"e12"}"#, Station::Browser, "playwright/browser_click"),
            ("TodoWrite", r#"{"todos":[]}"#, Station::Other, "TodoWrite"),
            ("Bash", "null", Station::Shell, "Bash"),
        ];
        for (tool, input, station, summary) in cases {
            let input: Value = serde_json::from_str(input).unwrap();
            assert_eq!(describe(tool, &input), (station, summary.to_string()), "{tool}");
        }
    }

    #[test]
    fn window_is_an_estimate() {
        assert_eq!(window("claude-opus-5-5", 150_000), 200_000);
        assert_eq!(window("claude-opus-5-5", 250_000), 1_000_000);
        assert_eq!(window("claude-opus-5-5[1m]", 10), 1_000_000);
    }
}
