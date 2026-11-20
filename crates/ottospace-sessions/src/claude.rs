//! Claude Code. Transcripts: `<config dir>/projects/<slugged cwd>/<session id>.jsonl`.
//! Live processes: `<config dir>/sessions/<pid>.json`. Schema seen in Claude Code 2.1.322.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use ottospace_core::runner::RunnerId;
use ottospace_core::sessions::WaitsFor;
use serde_json::Value;

use crate::cache::Cache;
use crate::station;
use crate::{Batch, Cursor, Event, SessionReader, SessionRef, Usage, What, id_of, jsonl, recorded_cwd, track};
use ottospace_core::activity::Station;

/// The most bytes a process or sub-agent record (`*.json`) is read with.
const RECORD_MOST: u64 = 65_536;

/// The values of an API-error record's `error` field. Seen in transcripts: the first five.
/// The rest are the API's other error types. Anything else is shown as `api_error`.
const ERRORS: &[&str] = &[
    "authentication_failed",
    "invalid_request",
    "model_not_found",
    "rate_limit",
    "server_error",
    "billing_error",
    "overloaded",
    "permission_denied",
    "not_found",
    "request_too_large",
    "timeout",
    "max_output_tokens",
    "unknown",
];

pub struct Claude {
    config_dirs: Vec<PathBuf>,
    /// Every dir as given, relative ones too: where a known session's file is looked for (`path_of`).
    named: Vec<PathBuf>,
    heads: Cache<Head>,
    tails: Cache<Tail>,
}

/// What a transcript's last records say: read again only when the file changed.
#[derive(Clone, Default)]
struct Tail {
    model: Option<String>,
    title: Option<String>,
}

/// What a transcript's first records say. They do not change as the file grows.
#[derive(Clone)]
struct Head {
    cwd: PathBuf,
    started: Option<u64>,
}

impl Claude {
    /// One runner over several config dirs, e.g. `~/.claude` and `~/.claude-personal`.
    /// A relative one is ignored.
    pub fn new(config_dirs: Vec<PathBuf>) -> Self {
        let named = config_dirs.clone();
        let config_dirs = config_dirs.into_iter().filter(|d| d.is_absolute()).collect();
        Self { config_dirs, named, heads: Cache::default(), tails: Cache::default() }
    }
}

impl Claude {
    fn tail(&self, session: &SessionRef) -> Tail {
        self.tails.get(&session.path, || tail(&session.path)).map(|(tail, _)| tail).unwrap_or_default()
    }
}

/// The model and the title in the last 256 KiB of a transcript. Only whole records of the
/// right type count: the same words inside a prompt or a tool's output are another record's text.
fn tail(path: &Path) -> Option<Tail> {
    let lines = jsonl::tail(path, 256 << 10).ok()?;
    let (mut model, mut custom, mut ai) = (None, None, None);
    let text = |line: &str, kind: &str, key: &str| {
        let record = serde_json::from_str::<Value>(line).ok().filter(|r| r["type"] == kind)?;
        Some(record[key].as_str().and_then(crate::title_text))
    };
    // Newest first: the first of each kind found is the last written.
    for line in lines.iter().rev() {
        if model.is_none() && line.contains(r#""type":"assistant""#) {
            model = serde_json::from_str::<Value>(line).ok().and_then(|r| model_of(&r["message"]));
        } else if custom.is_none() && line.contains(r#""type":"custom-title""#) {
            custom = text(line, "custom-title", "customTitle");
        } else if ai.is_none() && line.contains(r#""type":"ai-title""#) {
            ai = text(line, "ai-title", "aiTitle");
        }
    }
    // An empty custom title is a name taken back: the model's title shows again.
    Some(Tail { model, title: custom.flatten().or(ai.flatten()) })
}

/// Claude Code's folder name for a directory: `-` before each part of the path, and every
/// character but ASCII letters and digits becomes `-`. e.g. `/Users/me/my.app/` → `-Users-me-my-app`.
pub fn slug(dir: &Path) -> String {
    let parts = dir.components().filter_map(|c| match c {
        Component::Normal(part) => Some(part.to_string_lossy()),
        _ => None,
    });
    parts.flat_map(|part| format!("/{part}").chars().collect::<Vec<_>>()).map(dash).collect()
}

fn dash(c: char) -> char {
    if c.is_ascii_alphanumeric() { c } else { '-' }
}

impl SessionReader for Claude {
    fn runner(&self) -> RunnerId {
        RunnerId::Claude
    }

    fn sessions(&self, dirs: &[PathBuf]) -> io::Result<Vec<SessionRef>> {
        crate::absolute(dirs)?;
        let slugs: Vec<String> = dirs.iter().map(|d| slug(d)).collect();
        let mut sessions = Vec::new();
        for config in &self.config_dirs {
            let live = live(config);
            let Ok(projects) = fs::read_dir(config.join("projects")) else { continue };
            for project in projects.flatten() {
                // The folder of a directory itself, or of one below it. The slug is lossy
                // (`/a/b-c` and `/a/b/c` share one), so the recorded cwd is checked too.
                let name = project.file_name().to_string_lossy().into_owned();
                let below = |slug: &String| {
                    name.strip_prefix(slug.as_str()).is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
                };
                if !slugs.iter().any(below) {
                    continue;
                }
                let Ok(files) = fs::read_dir(project.path()) else { continue };
                for path in files.flatten().map(|f| f.path()).filter(|p| p.extension().is_some_and(|e| e == "jsonl")) {
                    // The id is the file's name. One that is not a name (see `station::is_name`) is not listed.
                    let Some(id) = path.file_stem().and_then(|s| s.to_str()).filter(|s| station::is_name(s)) else {
                        continue;
                    };
                    let Some((head, modified)) = self.heads.get(&path, || head(&path)) else { continue };
                    if !dirs.iter().any(|dir| head.cwd.starts_with(dir)) {
                        continue;
                    }
                    let holders = live.get(id).cloned().unwrap_or_default();
                    let last = holders.iter().max_by_key(|(holder, _)| holder.recorded);
                    let (pid, busy) = last.map_or((None, None), |(holder, busy)| (Some(holder.pid), *busy));
                    let holders = holders.into_iter().map(|(holder, _)| holder).collect();
                    let (runner, id, last_activity) = (RunnerId::Claude, id.to_string(), Some(modified));
                    let (cwd, started) = (head.cwd, head.started);
                    let (model, parent) = (None, None);
                    sessions.push(SessionRef {
                        runner,
                        id,
                        cwd,
                        path,
                        started,
                        last_activity,
                        model,
                        pid,
                        busy,
                        holders,
                        parent,
                    });
                }
            }
        }
        sessions.sort_by_key(|s| std::cmp::Reverse(s.last_activity));
        Ok(sessions)
    }

    /// The model of the last model request near the end of the transcript.
    fn model(&self, session: &SessionRef) -> Option<String> {
        self.tail(session).model
    }

    fn waits(&self) -> Vec<crate::Waiting> {
        let records = self.config_dirs.iter().flat_map(|config| records(config));
        let wait = |(id, holder, record): (String, crate::Holder, Value)| {
            let since = record["statusUpdatedAt"].as_u64();
            Some(crate::Waiting { id, holder, waits_for: waits_for(&record)?, since })
        };
        records.filter_map(wait).collect()
    }

    /// The name the user gave the session (`custom-title`), else the one Claude Code's own model
    /// gave it (`ai-title`): the last of each near the end of the transcript, where both are repeated.
    fn title(&self, session: &SessionRef) -> Option<String> {
        self.tail(session).title
    }

    /// `<config dir>/projects/<slug of cwd, links resolved>/<id>.jsonl`, in the first config dir that has it.
    fn path_of(&self, cwd: &Path, id: &str) -> Option<PathBuf> {
        let (folder, file) = (slug(&cwd.canonicalize().ok()?), format!("{id}.jsonl"));
        self.named.iter().map(|dir| dir.join("projects").join(&folder).join(&file)).find(|path| path.is_file())
    }

    /// `<session>/subagents/agent-*.jsonl`: each sub-agent's own transcript.
    fn subagents(&self, session: &SessionRef) -> Vec<SessionRef> {
        let Ok(files) = fs::read_dir(session.path.with_extension("").join("subagents")) else { return Vec::new() };
        let mut subs = Vec::new();
        for path in files.flatten().map(|f| f.path()).filter(|p| p.extension().is_some_and(|e| e == "jsonl")) {
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).filter(|s| station::is_name(s)) else { continue };
            let modified = path.metadata().and_then(|m| m.modified()).ok();
            let last_activity = modified.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
            subs.push(SessionRef {
                runner: RunnerId::Claude,
                id: id.to_string(),
                cwd: session.cwd.clone(),
                path,
                started: None,
                last_activity: last_activity.map(|d| d.as_millis() as u64),
                model: None,
                pid: None,
                busy: None,
                holders: Vec::new(),
                parent: Some(session.id.clone()),
            });
        }
        subs.sort_by(|a, b| a.path.cmp(&b.path));
        subs
    }

    fn read(&self, session: &SessionRef, after: Cursor, budget: u64) -> io::Result<Batch> {
        let (file, mut cursor, restarted) = crate::open(&session.path, after)?;
        let mut events = Vec::new();
        let mut skipping = cursor.skipping;
        // A sub-agent's own file: its records are its main loop.
        let sub = session.parent.is_some();
        let (offset, more) = jsonl::lines(file, cursor.offset, budget, &mut skipping, |line| {
            // Request logs: most of the file's bytes, nothing a station shows.
            if !line.starts_with(r#"{"type":"api-request"#)
                && let Ok(record) = serde_json::from_str::<Value>(line)
            {
                record_events(&record, sub, &mut cursor, &mut events);
            }
            false
        })?;
        (cursor.offset, cursor.skipping) = (offset, skipping);
        if !more {
            finished_subagents(&session.path, &mut cursor, &mut events);
        }
        Ok(Batch { events, cursor, restarted, more })
    }
}

/// Claude Code's process records of one config dir: session id → each (process, busy). Two
/// records may name one session: every one is kept, so one of a dead process hides no live one.
fn live(config: &Path) -> HashMap<String, Vec<(crate::Holder, Option<bool>)>> {
    let mut live: HashMap<String, Vec<_>> = HashMap::new();
    for (id, holder, record) in records(config) {
        let busy = record["status"].as_str().map(|s| s == "busy");
        live.entry(id).or_default().push((holder, busy));
    }
    live
}

/// Each process record of one config dir (`sessions/<pid>.json`), as (session id, holder, record).
fn records(config: &Path) -> Vec<(String, crate::Holder, Value)> {
    let Ok(files) = fs::read_dir(config.join("sessions")) else { return Vec::new() };
    let records = files.flatten().filter(|f| f.path().extension().is_some_and(|e| e == "json"));
    let mut out = Vec::new();
    for file in records {
        let record = jsonl::small(&file.path(), RECORD_MOST).and_then(|text| serde_json::from_str::<Value>(&text).ok());
        let Some(record) = record else { continue };
        let pid = record["pid"].as_u64().and_then(|pid| u32::try_from(pid).ok());
        let (Some(pid), Some(id)) = (pid, record["sessionId"].as_str().filter(|id| station::is_name(id))) else {
            continue;
        };
        let holder = crate::Holder { pid, recorded: record["updatedAt"].as_u64() };
        out.push((id.to_string(), holder, record));
    }
    out
}

/// What a record's `status: "waiting"` waits for, by its `waitingFor` (Claude Code 2.1.329). A
/// dialog the user opened (`dialog open`) waits on nobody. Another word: some input.
fn waits_for(record: &Value) -> Option<WaitsFor> {
    if record["status"] != "waiting" {
        return None;
    }
    match record["waitingFor"].as_str() {
        Some("dialog open") => None,
        Some("permission prompt" | "sandbox request" | "worker request" | "goal proposal") => {
            Some(WaitsFor::Permission)
        }
        Some("input needed") => Some(WaitsFor::Question),
        _ => Some(WaitsFor::Input),
    }
}

/// Where and when a transcript's session started.
fn head(path: &Path) -> Option<Head> {
    // The first records are settings without a cwd; message records follow within a few lines.
    let head = jsonl::head(path, 40, 1 << 20).ok()?;
    let started = head.iter().map(jsonl::time_of).filter(|t| *t > 0).min();
    if let Some(cwd) = head.iter().find_map(|r| r["cwd"].as_str()) {
        return Some(Head { cwd: recorded_cwd(cwd)?, started });
    }
    // Not there, e.g. behind a long request log: look through the whole file, parsing only lines that may have it.
    let mut found = None;
    jsonl::lines(jsonl::open(path).ok()?, 0, u64::MAX, &mut false, |line| {
        if line.contains(r#""cwd":""#)
            && !line.starts_with(r#"{"type":"api-request"#)
            && let Ok(record) = serde_json::from_str::<Value>(line)
            && let Some(cwd) = record["cwd"].as_str()
        {
            found = Some((cwd.to_string(), jsonl::time_of(&record)));
        }
        found.is_some()
    })
    .ok()?;
    let (cwd, at) = found?;
    Some(Head { cwd: recorded_cwd(&cwd)?, started: started.or(Some(at).filter(|t| *t > 0)) })
}

/// A message's model, if it is a real one with a name's shape.
fn model_of(message: &Value) -> Option<String> {
    let model = message["model"].as_str().filter(|m| *m != "<synthetic>")?;
    // `[1m]` marks a 1M-token window, e.g. `claude-opus-5-5[1m]`. Alone it names no model.
    let bare = model.strip_suffix("[1m]").unwrap_or(model);
    station::is_name(bare).then(|| model.to_string())
}

fn record_events(record: &Value, sub: bool, cursor: &mut Cursor, events: &mut Vec<Event>) {
    // A sub-agent's own records live in its own file; any in its parent's are not the main loop's.
    if record["isSidechain"] == true && !sub {
        return;
    }
    let at = jsonl::time_of(record);
    let mut emit = |what| events.push(Event { at, what });
    // Where the main loop works: each record says. A sub-agent's file says its own.
    if let Some(cwd) =
        record["cwd"].as_str().and_then(recorded_cwd).filter(|cwd| !sub && cursor.cwd.as_ref() != Some(cwd))
    {
        cursor.cwd = Some(cwd.clone());
        emit(What::Cwd { cwd });
    }
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
        let Some(id) = id_of(&result["tool_use_id"]) else { continue };
        // A background agent's call returns at once with `async_launched`; the agent works on.
        let ended = record["toolUseResult"]["status"] != "async_launched" && cursor.agents.remove(&id);
        let output = cursor.edits.then(|| ottospace_core::gist::output(&result["content"]));
        let failed = result["is_error"] == true;
        emit(What::ToolEnd { id: id.clone(), failed, station: None, summary: None, output });
        if ended {
            emit(What::SubAgentEnd { id });
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
        emit(What::Error { kind: station::code(record["error"].as_str().unwrap_or(""), ERRORS, "api_error") });
        return;
    }
    let model = model_of(message);
    if let Some(model) = &model
        && cursor.model.as_ref() != Some(model)
        && let Some(from) = cursor.model.replace(model.clone())
    {
        emit(What::ModelChange { from, to: model.clone() });
    }
    // One request is written as one record per content block, all with the same message id.
    let id = id_of(&message["id"]);
    if id.is_none() || cursor.step != id {
        cursor.step = id;
        let usage = message["usage"].as_object().map(|u| {
            let count = |key: &str| u.get(key).and_then(Value::as_u64).unwrap_or(0);
            let cached = count("cache_read_input_tokens");
            let input =
                count("input_tokens").saturating_add(cached).saturating_add(count("cache_creation_input_tokens"));
            Usage { input, output: count("output_tokens"), cached }
        });
        if let Some(usage) = usage {
            cursor.peak = cursor.peak.max(usage.input.saturating_add(usage.output));
        }
        let window = usage.and_then(|_| window(model.as_deref().unwrap_or(""), cursor.peak));
        emit(What::ModelStep { model, usage, window });
        if message["stop_reason"] == "refusal" {
            emit(What::Error { kind: "refusal".to_string() });
        }
    }
    for block in message["content"].as_array().map(Vec::as_slice).unwrap_or_default() {
        let (Some("tool_use"), Some(id), Some(tool)) =
            (block["type"].as_str(), id_of(&block["id"]), block["name"].as_str())
        else {
            continue;
        };
        let (station, summary) = describe(tool, &block["input"]);
        let diff = cursor.edits.then(|| ottospace_core::diff::claude(tool, &block["input"])).flatten();
        let gist = cursor.edits.then(|| ottospace_core::gist::claude(tool, &block["input"])).flatten();
        let label = station::tool_label(tool);
        emit(What::ToolStart { id: id.clone(), tool: label, station, summary: summary.clone(), diff, gist });
        if matches!(tool, "Agent" | "Task") && track(&mut cursor.agents, id.clone()) {
            emit(What::SubAgentStart { id, label: summary });
        }
    }
}

/// The context window, estimated: the transcript does not record it. `peak` is the most tokens
/// one request of the session used. 1M once a request exceeded 200k, and it stays 1M after a
/// compaction. Below that: 200k for a model with no 1M window, unknown for the others.
fn window(model: &str, peak: u64) -> Option<u64> {
    if model.ends_with("[1m]") || peak > 200_000 {
        Some(1_000_000)
    } else if model.contains("haiku") {
        Some(200_000)
    } else {
        None
    }
}

/// A tool call's station and label, from its name and the names in its input.
/// `ottospace-agents` labels a live call's start and permission request with it.
pub fn describe(tool: &str, input: &Value) -> (Station, String) {
    let text = |key: &str| input[key].as_str();
    let path = text("file_path").or(text("notebook_path")).or(text("path"));
    let station = station::classify(tool, text("command").or(path));
    let summary = if tool.starts_with("mcp__") {
        station::tool_label(tool)
    } else if let Some(command) = text("command") {
        station::command_label(command)
    } else if let Some(path) = path.filter(|_| matches!(station, Station::Files | Station::Memory)) {
        station::path_label(path)
    } else {
        match tool {
            "WebFetch" => station::host_label(text("url").unwrap_or("")),
            "Skill" => station::name(text("skill").unwrap_or(""), "skill"),
            // The agent's type, not its description: that is free text the model wrote.
            "Agent" | "Task" => station::name(text("subagent_type").unwrap_or(""), "agent"),
            _ => station::tool_label(tool),
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
    for file in files.flatten().filter(|f| f.file_name().to_string_lossy().ends_with(".meta.json")) {
        let Some(record) = jsonl::small(&file.path(), RECORD_MOST).and_then(|t| serde_json::from_str::<Value>(&t).ok())
        else {
            continue;
        };
        let done = record["finished"] == true || record["stoppedByUser"] == true;
        if let Some(id) = id_of(&record["toolUseId"]).filter(|id| done && cursor.agents.remove(id)) {
            let modified = file.metadata().ok().and_then(|m| m.modified().ok());
            let at =
                modified.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64);
            events.push(Event { at, what: What::SubAgentEnd { id } });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_like_claude_code() {
        assert_eq!(slug(Path::new("/Users/me/code/app")), "-Users-me-code-app");
        assert_eq!(slug(Path::new("/Users/me/code/app/")), "-Users-me-code-app", "a trailing slash changes nothing");
        assert_eq!(slug(Path::new("/Users/me.name/.cfg/my_app")), "-Users-me-name--cfg-my-app");
        assert_eq!(slug(Path::new("/")), "");
    }

    #[test]
    fn describes_calls_by_name_only() {
        let cases = [
            (
                "Bash",
                r#"{"command":"cd app && git push -f origin secret-branch","description":"Push"}"#,
                Station::Shell,
                "git push",
            ),
            ("Bash", r#"{"command":"./run-secret.sh --token abc"}"#, Station::Shell, "shell"),
            ("Read", r#"{"file_path":"/Users/me/app/src/main.rs"}"#, Station::Files, "src/main.rs"),
            ("Read", r#"{"file_path":"please read the private plan"}"#, Station::Files, "file"),
            ("Write", r#"{"file_path":"/tmp/API_KEY=sk-abc.txt","content":"x"}"#, Station::Files, "file"),
            ("Edit", r#"{"file_path":"https://me:pw@example.com/a/b.txt"}"#, Station::Files, "file"),
            ("NotebookEdit", r#"{"notebook_path":"C:\\Users\\me\\private.ipynb"}"#, Station::Files, "file"),
            ("Write", r#"{"file_path":"/Users/me/app/.env","content":"TOKEN=abc"}"#, Station::Files, "app/.env"),
            ("Grep", r#"{"pattern":"password","path":"/Users/me/app/src"}"#, Station::Files, "app/src"),
            ("Glob", r#"{"pattern":"**/*.rs","path":"a private note, not a path"}"#, Station::Files, "file"),
            ("Grep", r#"{"pattern":"password"}"#, Station::Files, "Grep"),
            (
                "WebFetch",
                r#"{"url":"https://example.com/a?key=abc","prompt":"read it"}"#,
                Station::Network,
                "example.com",
            ),
            ("WebFetch", r#"{"url":"https://me:pa/ss@example.com/"}"#, Station::Network, "web"),
            ("WebSearch", r#"{"query":"something private"}"#, Station::Network, "WebSearch"),
            ("Skill", r#"{"skill":"review","args":"private"}"#, Station::Skills, "review"),
            ("Skill", r#"{"skill":"review my private notes"}"#, Station::Skills, "skill"),
            ("Skill", r#"{"skill":"ghp_abcdefghijklmnopqrstuvwxyz"}"#, Station::Skills, "skill"),
            (
                "Agent",
                r#"{"description":"Look into it","prompt":"long","subagent_type":"Explore"}"#,
                Station::Agents,
                "Explore",
            ),
            (
                "Agent",
                r#"{"description":"Look into it","prompt":"long","subagent_type":"a private type"}"#,
                Station::Agents,
                "agent",
            ),
            (
                "Agent",
                r#"{"description":"Look into it","prompt":"long","subagent_type":"sk-ant-abc"}"#,
                Station::Agents,
                "agent",
            ),
            ("Agent", r#"{"description":"Look into it","prompt":"long"}"#, Station::Agents, "agent"),
            ("mcp__playwright__browser_click", r#"{"ref":"e12"}"#, Station::Mcp, "playwright/browser_click"),
            ("mcp__my server__do a private thing", "{}", Station::Mcp, "mcp"),
            ("TodoWrite", r#"{"todos":[]}"#, Station::Other, "TodoWrite"),
            ("private_tool_name", "{}", Station::Other, "tool"),
            ("A private tool name", "{}", Station::Other, "tool"),
            ("Bash", "null", Station::Shell, "Bash"),
        ];
        for (tool, input, station, summary) in cases {
            let input: Value = serde_json::from_str(input).unwrap();
            assert_eq!(describe(tool, &input), (station, summary.to_string()), "{tool} {input}");
        }
    }

    #[test]
    fn window_is_an_estimate_that_sticks() {
        assert_eq!(window("claude-opus-5-5", 150_000), None, "may be 200k or 1M: unknown");
        assert_eq!(window("claude-haiku-5", 150_000), Some(200_000));
        assert_eq!(window("claude-opus-5-5", 250_000), Some(1_000_000));
        assert_eq!(window("claude-opus-5-5[1m]", 10), Some(1_000_000));
    }

    #[test]
    fn the_window_stays_1m_after_a_compaction() {
        let step = |id: &str, input: u64| {
            let usage = serde_json::json!({ "input_tokens": input, "output_tokens": 0 });
            let message = serde_json::json!({ "id": id, "model": "claude-opus-5-5", "usage": usage, "content": [] });
            serde_json::json!({ "type": "assistant", "message": message })
        };
        let (mut cursor, mut events) = (Cursor::default(), Vec::new());
        for record in [step("m1", 150_000), step("m2", 250_000), step("m3", 40_000), step("m4", u64::MAX)] {
            record_events(&record, false, &mut cursor, &mut events);
        }
        let windows = events.iter().filter_map(|e| match &e.what {
            What::ModelStep { window, .. } => Some(*window),
            _ => None,
        });
        let m = Some(1_000_000);
        assert_eq!(windows.collect::<Vec<_>>(), [None, m, m, m], "and a huge count does not overflow");
    }

    /// A file's contents leave the crate only for a cursor that asks: an edit's change, as a
    /// `Diff` whose `Debug` prints lengths.
    #[test]
    fn an_edit_carries_its_change_only_for_a_cursor_that_asks() {
        let input =
            serde_json::json!({ "file_path": "/w/a.rs", "old_string": "secret old", "new_string": "secret new" });
        let block = serde_json::json!({ "type": "tool_use", "id": "t", "name": "Edit", "input": input });
        let record = serde_json::json!({ "type": "assistant", "message": { "content": [block] } });
        let diff = |mut cursor: Cursor| {
            let mut events = Vec::new();
            record_events(&record, false, &mut cursor, &mut events);
            let debug = format!("{events:?}");
            assert!(!debug.contains("secret"), "{debug}");
            events.into_iter().find_map(|e| match e.what {
                What::ToolStart { diff, .. } => Some(diff),
                _ => None,
            })
        };
        assert_eq!(diff(Cursor::default()), Some(None));
        let shown = diff(Cursor::with_edits()).flatten().expect("a change");
        assert_eq!((shown.files[0].added, shown.files[0].removed), (1, 1));
    }

    /// A command and what it printed leave the crate only for the watch view's cursor.
    #[test]
    fn a_calls_gist_and_output_only_for_a_cursor_that_asks() {
        let block =
            serde_json::json!({ "type": "tool_use", "id": "t", "name": "Bash", "input": { "command": "echo secret" } });
        let call = serde_json::json!({ "type": "assistant", "message": { "content": [block] } });
        let result = serde_json::json!({ "type": "tool_result", "tool_use_id": "t", "content": "secret out" });
        let end = serde_json::json!({ "type": "user", "message": { "content": [result] } });
        let read = |mut cursor: Cursor| {
            let mut events = Vec::new();
            record_events(&call, false, &mut cursor, &mut events);
            record_events(&end, false, &mut cursor, &mut events);
            assert!(!format!("{events:?}").contains("secret"));
            let gist = events.iter().find_map(|e| match &e.what {
                What::ToolStart { gist, .. } => Some(gist.as_ref().map(|gist| gist.line(None))),
                _ => None,
            });
            let output = events.iter().find_map(|e| match &e.what {
                What::ToolEnd { output, .. } => Some(output.as_ref().map(|o| o.text().to_string())),
                _ => None,
            });
            (gist.flatten(), output.flatten())
        };
        assert_eq!(read(Cursor::default()), (None, None));
        assert_eq!(read(Cursor::with_edits()), (Some("echo secret".into()), Some("secret out".into())));
    }

    #[test]
    fn models_ids_and_errors() {
        let message = |model: &str| serde_json::json!({ "model": model });
        assert_eq!(model_of(&message("claude-opus-5-5")).as_deref(), Some("claude-opus-5-5"));
        assert_eq!(model_of(&message("claude-opus-5-5[1m]")).as_deref(), Some("claude-opus-5-5[1m]"));
        for bad in ["<synthetic>", "a private note", "[1m]", "", "sk-ant-abc", "eyJhbGciOi"] {
            assert_eq!(model_of(&message(bad)), None, "{bad}");
        }
        assert_eq!(id_of(&serde_json::json!("")), None);
        assert_eq!(id_of(&serde_json::json!(7)), None);
        let id = id_of(&serde_json::json!("toolu private text")).unwrap();
        assert!(!id.contains("private") && id.len() == 16);

        let kind = |error: Value| {
            let (mut cursor, mut events) = (Cursor::default(), Vec::new());
            let record =
                serde_json::json!({ "type": "assistant", "error": error, "isApiErrorMessage": true, "message": {} });
            record_events(&record, false, &mut cursor, &mut events);
            match &events[0].what {
                What::Error { kind } => kind.clone(),
                other => panic!("{other:?}"),
            }
        };
        assert_eq!(kind(serde_json::json!("rate_limit")), "rate_limit");
        assert_eq!(kind(serde_json::json!("zz_private_code")), "api_error", "a code's shape is not enough");
        assert_eq!(kind(serde_json::json!("Key sk-1 is over its limit")), "api_error");
        assert_eq!(kind(Value::Null), "api_error");
    }
}
