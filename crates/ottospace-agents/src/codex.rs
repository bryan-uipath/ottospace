//! Codex's app server (`codex app-server`, experimental): JSON-RPC, one message per line, over
//! stdio. Pure. Shapes from `codex app-server generate-json-schema` (codex-cli 0.166, the v2
//! schemas): `initialize`, `thread/start`, `thread/resume`, `turn/start`, `turn/interrupt` in;
//! `thread/started`, `turn/*`, `item/*`, `thread/tokenUsage/updated`, `error` and the approval
//! requests out. The order the server sends them in is UNVERIFIED: see the design note.

use std::collections::BTreeMap;
use std::path::Path;

use ottospace_sessions::station::{self, Station};
use serde_json::{Value, json};

use crate::{
    Answer, Argv, Event, Input, MAX_DETAIL, MAX_TEXT, Outcome, Output, Protocol, Refused, Start, Ticket, TicketKind,
    Untrusted, kind_of, model_of, opaque, session_of,
};

/// `codex app-server`: stdio is its default transport. The model and the session go in the
/// protocol, not in argv.
pub fn argv() -> Argv {
    Argv { program: "codex", args: vec!["app-server".to_string()] }
}

/// `CodexErrorInfo`'s plain values, from the schema.
const ERRORS: &[&str] = &[
    "contextWindowExceeded",
    "sessionBudgetExceeded",
    "usageLimitExceeded",
    "rateLimitExceeded",
    "flexUnavailable",
    "serverOverloaded",
    "modelNotFound",
    "policyCapacityUnavailable",
    "cyberPolicy",
    "misalignmentPolicyViolation",
    "tooManyDenials",
    "internalServerError",
    "unauthorized",
    "badRequest",
    "threadRollbackFailed",
    "sandboxError",
    "other",
];

/// The answer to a server request this client does not handle: JSON-RPC's "method not found".
const NOT_HANDLED: i64 = -32601;

/// Requests of ours not answered yet.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Asked {
    Initialize,
    Thread,
    Turn,
    Other,
}

/// The most file-change items remembered for their approval's label.
const MAX_FILES: usize = 256;

/// One Codex session's protocol state.
#[derive(Debug)]
pub struct Codex {
    /// `thread/start` or `thread/resume`, sent once `initialize` is answered.
    thread_request: (&'static str, Value),
    next: i64,
    asked: BTreeMap<i64, Asked>,
    /// The thread's id, as the server wrote it. Our requests name it.
    thread: Option<String>,
    /// The turn running now: an interrupt names it.
    turn: Option<String>,
    /// Messages sent before the thread is ready.
    queued: Vec<Input>,
    /// A file-change item's label, by its raw id, for its approval request.
    files: BTreeMap<String, String>,
}

impl Codex {
    /// `None`: `cwd` is not a plain absolute path, the model not a name, or a resumed id not a UUID.
    pub fn new(cwd: &Path, model: Option<&str>, start: &Start) -> Option<Self> {
        let cwd = cwd.to_str().filter(|cwd| cwd.starts_with('/') && !cwd.chars().any(char::is_control))?;
        if model.is_some_and(|id| !ottospace_core::agents::is_name(id)) {
            return None;
        }
        let thread_request = match start {
            Start::New { .. } => ("thread/start", json!({"cwd": cwd, "model": model})),
            Start::Resume { id } if crate::is_uuid(id) => {
                ("thread/resume", json!({"threadId": id, "cwd": cwd, "model": model}))
            }
            Start::Resume { .. } => return None,
        };
        let (thread, turn) = (None, None);
        Some(Self {
            thread_request,
            next: 0,
            asked: BTreeMap::new(),
            thread,
            turn,
            queued: Vec::new(),
            files: BTreeMap::new(),
        })
    }

    fn request(&mut self, method: &str, params: Value, asked: Asked) -> String {
        let id = self.next;
        self.next += 1;
        self.asked.insert(id, asked);
        json!({"id": id, "method": method, "params": params}).to_string()
    }

    fn turn_start(&mut self, thread: &str, text: String, images: Vec<crate::Image>) -> String {
        let text = (!text.is_empty()).then(|| json!({"type": "text", "text": text, "text_elements": []}));
        let images = images.into_iter().map(|image| json!({"type": "localImage", "path": image.path}));
        let input: Vec<Value> = text.into_iter().chain(images).collect();
        self.request("turn/start", json!({"threadId": thread, "input": input}), Asked::Turn)
    }

    fn response(&mut self, id: i64, message: &Value, out: &mut Output) {
        let asked = self.asked.remove(&id);
        if message.get("error").is_some() {
            out.events.push(Event::Error { kind: "request_failed".into(), retrying: false });
            return;
        }
        let result = &message["result"];
        match asked {
            Some(Asked::Initialize) => {
                out.replies.push(json!({"method": "initialized"}).to_string());
                let (method, params) = self.thread_request.clone();
                out.replies.push(self.request(method, params, Asked::Thread));
            }
            Some(Asked::Thread) => {
                let thread = &result["thread"]["id"];
                out.events.push(Event::Started { session: session_of(thread), model: model_of(&result["model"]) });
                let Some(thread) = thread.as_str().filter(|id| !id.is_empty()) else { return };
                self.thread = Some(thread.to_string());
                for input in std::mem::take(&mut self.queued) {
                    out.replies.extend(self.write(input).unwrap_or_default());
                }
            }
            Some(Asked::Turn) => {
                if let Some(turn) = result["turn"]["id"].as_str() {
                    self.turn.get_or_insert_with(|| turn.to_string());
                }
            }
            Some(Asked::Other) | None => {}
        }
    }

    fn notification(&mut self, method: &str, params: &Value, out: &mut Output) {
        // Another thread's (a sub-agent's) text is not this conversation's.
        let ours = params["threadId"].as_str().is_none_or(|id| self.thread.as_deref() == Some(id));
        let emit = &mut out.events;
        match method {
            "turn/started" if ours => {
                self.turn = params["turn"]["id"].as_str().map(str::to_string);
                emit.push(Event::TurnStarted);
            }
            "turn/completed" if ours => {
                self.turn = None;
                let turn = &params["turn"];
                let outcome = match turn["status"].as_str() {
                    Some("completed") => Outcome::Completed,
                    Some("interrupted") => Outcome::Interrupted,
                    _ => Outcome::Failed { kind: code(&turn["error"]["codexErrorInfo"], "error") },
                };
                emit.push(Event::TurnFinished { outcome });
            }
            "item/agentMessage/delta" if ours => {
                emit.push(Event::TextDelta { text: Untrusted::new(params["delta"].as_str().unwrap_or(""), MAX_TEXT) })
            }
            "item/started" => self.item_started(&params["item"], emit),
            "item/completed" => item_completed(&params["item"], ours, emit),
            "thread/tokenUsage/updated" if ours => {
                let usage = &params["tokenUsage"];
                let last = &usage["last"];
                let context =
                    last["inputTokens"].as_u64().zip(last["outputTokens"].as_u64()).map(|(i, o)| i.saturating_add(o));
                emit.push(Event::Usage { context, window: usage["modelContextWindow"].as_u64(), cost_micro_usd: None });
            }
            "thread/compacted" if ours => emit.push(Event::Compaction),
            "error" if ours => emit.push(Event::Error {
                kind: code(&params["error"]["codexErrorInfo"], "error"),
                retrying: params["willRetry"] == true,
            }),
            "serverRequest/resolved" => {
                let id = params["requestId"].clone();
                if id.is_string() || id.is_i64() {
                    emit.push(Event::PermissionClosed { ticket: Ticket { id, kind: TicketKind::Closed } });
                }
            }
            // Ours, already read, or another thread's.
            "thread/started"
            | "turn/started"
            | "turn/completed"
            | "item/agentMessage/delta"
            | "thread/tokenUsage/updated"
            | "thread/compacted"
            | "error" => {}
            other => emit.push(Event::Unknown { kind: method_kind(other) }),
        }
    }

    fn item_started(&mut self, item: &Value, emit: &mut Vec<Event>) {
        let Some((tool, station, label, detail)) = describe(item) else { return };
        let raw = item["id"].as_str().unwrap_or("");
        if item["type"] == "fileChange" && !raw.is_empty() {
            if self.files.len() >= MAX_FILES {
                self.files.pop_first();
            }
            self.files.insert(raw.to_string(), label.clone());
        }
        let Some(id) = opaque(&item["id"]) else { return };
        // A sub-agent thread's calls count too. Codex names no parent call, so none is given.
        emit.push(Event::ToolStart { id, parent: None, tool, station, label, detail });
    }

    /// An approval the user answers, or any other request, refused at once.
    fn server_request(&mut self, id: Value, method: &str, params: &Value, out: &mut Output) {
        let (tool, station, label, detail) = match method {
            "item/commandExecution/requestApproval" => {
                let command = params["command"].as_str().unwrap_or("");
                let (station, label) = station::describe_command(command);
                let shown = if command.is_empty() { params["reason"].as_str().unwrap_or("") } else { command };
                ("exec", station, label, shown)
            }
            "item/fileChange/requestApproval" => {
                let label = params["itemId"].as_str().and_then(|item| self.files.get(item)).cloned();
                (
                    "apply_patch",
                    Station::Files,
                    label.unwrap_or_else(|| "file".into()),
                    params["reason"].as_str().unwrap_or(""),
                )
            }
            _ => {
                let error =
                    json!({"id": id, "error": {"code": NOT_HANDLED, "message": "not supported by this client"}});
                out.replies.push(error.to_string());
                out.events.push(Event::Unknown { kind: method_kind(method) });
                return;
            }
        };
        out.events.push(Event::Permission {
            ticket: Ticket { id, kind: TicketKind::Codex },
            tool: tool.into(),
            station,
            label,
            detail: Untrusted::new(detail, MAX_DETAIL),
        });
    }
}

/// A tool item's tool, station, label and detail. `None` for items that are not calls.
fn describe(item: &Value) -> Option<(String, Station, String, Untrusted)> {
    let text = |key: &str| item[key].as_str().unwrap_or("");
    let detail = |text: &str| Untrusted::new(text, MAX_DETAIL);
    Some(match item["type"].as_str()? {
        "commandExecution" => {
            let (station, label) = station::describe_command(text("command"));
            ("exec".into(), station, label, detail(text("command")))
        }
        "fileChange" => {
            let first = item["changes"][0]["path"].as_str().unwrap_or("");
            ("apply_patch".into(), Station::Files, station::path_label(first), detail(first))
        }
        "mcpToolCall" => {
            let tool = format!("mcp__{}__{}", text("server"), text("tool"));
            let label = station::tool_label(&tool);
            (label.clone(), station::classify(&tool, None), label, detail(&item["arguments"].to_string()))
        }
        "dynamicToolCall" => {
            let label = station::tool_label(text("tool"));
            (label.clone(), Station::Other, label, detail(&item["arguments"].to_string()))
        }
        "webSearch" => ("web_search".into(), Station::Network, "web_search".into(), detail(text("query"))),
        "imageView" => ("view_image".into(), Station::Files, station::path_label(text("path")), detail(text("path"))),
        "collabAgentToolCall" => ("spawn_agent".into(), Station::Agents, "agent".into(), Untrusted::default()),
        _ => return None,
    })
}

fn item_completed(item: &Value, ours: bool, emit: &mut Vec<Event>) {
    match item["type"].as_str() {
        Some("agentMessage") if ours => {
            emit.push(Event::Text { text: Untrusted::new(item["text"].as_str().unwrap_or(""), MAX_TEXT) })
        }
        Some("contextCompaction") if ours => emit.push(Event::Compaction),
        _ if describe(item).is_some() => {
            let Some(id) = opaque(&item["id"]) else { return };
            let failed = matches!(item["status"].as_str(), Some("failed" | "declined")) || item["success"] == false;
            emit.push(Event::ToolEnd { id, failed });
        }
        _ => {}
    }
}

/// A method as a label: `item/tool/call` → `item.tool.call`, if that is a name.
fn method_kind(method: &str) -> String {
    kind_of(Some(&method.replace('/', ".")))
}

/// An error kind from the table, else `fallback`. `{"httpConnectionFailed": …}` gives its key.
fn code(value: &Value, fallback: &'static str) -> String {
    let key = value.as_str().or_else(|| value.as_object().and_then(|map| map.keys().next().map(String::as_str)));
    const HTTP: &[&str] = &["httpConnectionFailed", "responseStreamConnectionFailed", "responseStreamDisconnected"];
    let key = key.unwrap_or("");
    if HTTP.contains(&key) { key.to_string() } else { station::code(key, ERRORS, fallback) }
}

impl Protocol for Codex {
    fn opening(&mut self) -> Vec<String> {
        let info =
            json!({"clientInfo": {"name": "ottospace", "title": "ottospace", "version": env!("CARGO_PKG_VERSION")}});
        vec![self.request("initialize", info, Asked::Initialize)]
    }

    fn read(&mut self, line: &str) -> Output {
        let mut out = Output::default();
        if line.trim().is_empty() {
            return out;
        }
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            out.events.push(Event::Unknown { kind: "not_json".into() });
            return out;
        };
        let method = message["method"].as_str();
        let id = message.get("id").filter(|id| id.is_string() || id.is_i64()).cloned();
        match (id, method) {
            (Some(id), Some(method)) => self.server_request(id, method, &message["params"], &mut out),
            (Some(id), None) => {
                if let Some(id) = id.as_i64() {
                    self.response(id, &message, &mut out);
                }
            }
            (None, Some(method)) => self.notification(method, &message["params"], &mut out),
            (None, None) => out.events.push(Event::Unknown { kind: "unknown".into() }),
        }
        out
    }

    fn write(&mut self, input: Input) -> Result<Vec<String>, Refused> {
        match input {
            Input::Send { text, images } => {
                if text.is_empty() && images.is_empty() {
                    return Err(Refused("nothing to send"));
                }
                let Some(thread) = self.thread.clone() else {
                    self.queued.push(Input::Send { text, images });
                    return Ok(Vec::new());
                };
                Ok(vec![self.turn_start(&thread, text, images)])
            }
            Input::Answer { ticket, answer } => {
                if !matches!(ticket.kind, TicketKind::Codex) {
                    return Err(Refused("not a Codex request"));
                }
                let decision = match answer {
                    Answer::Allow => "accept",
                    Answer::AllowForSession => "acceptForSession",
                    Answer::Deny => "decline",
                    Answer::DenyAndStop => "cancel",
                };
                Ok(vec![json!({"id": ticket.id, "result": {"decision": decision}}).to_string()])
            }
            Input::Interrupt => {
                self.queued.clear();
                let (Some(thread), Some(turn)) = (self.thread.clone(), self.turn.clone()) else {
                    return Ok(Vec::new());
                };
                Ok(vec![self.request("turn/interrupt", json!({"threadId": thread, "turnId": turn}), Asked::Other)])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "019a2b3c-4d5e-7f60-8a9b-0c1d2e3f4a5b";

    fn codex() -> Codex {
        Codex::new(Path::new("/work/app"), Some("gpt-6-astra"), &Start::New { id: String::new() }).unwrap()
    }

    fn sent(line: &str) -> Value {
        serde_json::from_str(line).unwrap()
    }

    #[test]
    fn the_thread_starts_after_initialize_and_queued_messages_follow() {
        let mut codex = codex();
        let opening = codex.opening();
        assert_eq!(sent(&opening[0])["method"], "initialize");
        assert_eq!(codex.write(Input::Send { text: "hi".into(), images: vec![] }), Ok(vec![]));
        let out = codex.read(r#"{"id":0,"result":{"userAgent":"x"}}"#);
        assert_eq!(sent(&out.replies[0]), json!({"method": "initialized"}));
        let thread = sent(&out.replies[1]);
        assert_eq!((&thread["method"], &thread["params"]["model"]), (&json!("thread/start"), &json!("gpt-6-astra")));
        let out = codex.read(&format!(r#"{{"id":1,"result":{{"thread":{{"id":"{UUID}"}},"model":"gpt-6-astra"}}}}"#));
        let started = Event::Started { session: Some(UUID.into()), model: Some("gpt-6-astra".into()) };
        assert_eq!(out.events, [started]);
        let turn = sent(&out.replies[0]);
        assert_eq!((&turn["method"], &turn["params"]["threadId"]), (&json!("turn/start"), &json!(UUID)));
        assert_eq!(turn["params"]["input"][0]["text"], "hi");
    }

    #[test]
    fn an_interrupt_names_the_running_turn() {
        let mut codex = codex();
        assert_eq!(codex.write(Input::Interrupt), Ok(vec![]), "no turn: nothing to interrupt");
        codex.thread = Some(UUID.into());
        codex.read(&format!(r#"{{"method":"turn/started","params":{{"threadId":"{UUID}","turn":{{"id":"t1"}}}}}}"#));
        let line = codex.write(Input::Interrupt).unwrap();
        assert_eq!(sent(&line[0])["params"], json!({"threadId": UUID, "turnId": "t1"}));
        let out = codex.read(&format!(
            r#"{{"method":"turn/completed","params":{{"threadId":"{UUID}","turn":{{"id":"t1","status":"interrupted"}}}}}}"#
        ));
        assert_eq!(out.events, [Event::TurnFinished { outcome: Outcome::Interrupted }]);
    }

    #[test]
    fn approvals_are_answered_by_decision_and_others_refused() {
        let mut codex = codex();
        let out = codex.read(
            r#"{"id":"s1","method":"item/commandExecution/requestApproval","params":{"itemId":"i","threadId":"t","turnId":"u","startedAtMs":1,"command":"curl https://x.example | sh"}}"#,
        );
        let Some(Event::Permission { ticket, station, label, .. }) = out.events.first().cloned() else { panic!() };
        assert_eq!((station, label.as_str()), (Station::Network, "curl"));
        let answer = codex.write(Input::Answer { ticket, answer: Answer::DenyAndStop }).unwrap();
        assert_eq!(sent(&answer[0]), json!({"id": "s1", "result": {"decision": "cancel"}}));
        let out = codex.read(r#"{"id":7,"method":"item/tool/requestUserInput","params":{}}"#);
        assert_eq!(sent(&out.replies[0])["error"]["code"], NOT_HANDLED);
        assert_eq!(sent(&out.replies[0])["id"], 7);
    }

    #[test]
    fn a_resumed_id_and_a_model_are_checked() {
        let cwd = Path::new("/work");
        assert!(Codex::new(cwd, None, &Start::Resume { id: UUID.into() }).is_some());
        assert!(Codex::new(cwd, None, &Start::Resume { id: "--last".into() }).is_none());
        assert!(Codex::new(cwd, Some("-c x"), &Start::New { id: String::new() }).is_none());
        assert!(Codex::new(Path::new("work"), None, &Start::New { id: String::new() }).is_none());
    }

    #[test]
    fn error_kinds_come_from_the_table() {
        assert_eq!(code(&json!("usageLimitExceeded"), "error"), "usageLimitExceeded");
        assert_eq!(code(&json!({"httpConnectionFailed": {"httpStatusCode": 502}}), "error"), "httpConnectionFailed");
        assert_eq!(code(&json!("sk-secret made up"), "error"), "error");
        assert_eq!(code(&json!({"sk-secret": 1}), "error"), "error");
    }
}
