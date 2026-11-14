//! Claude Code in print mode with stream-json in and out. Pure.
//!
//! Out (stdout), from the Agent SDK's TypeScript reference: `system/init`, `stream_event` (raw
//! API stream events), `assistant`, `user`, `result`, `system/api_retry`,
//! `system/compact_boundary`. Permission prompts and interrupts go by `control_request` and
//! `control_response` lines, whose shape the docs do not print: UNVERIFIED, see the design note.

use ottospace_core::runner::{RunnerId, SessionId};
use ottospace_core::transcript::Outcome;
use ottospace_sessions::claude::describe;
use ottospace_sessions::station;
use serde_json::{Value, json};

use crate::payload::Payload;
use crate::{
    Answer, Argv, Event, Input, MAX_DETAIL, MAX_TEXT, Output, Refused, Start, Ticket, TicketKind, Untrusted, Wire,
    kind_of, model_of, opaque, session_of,
};

/// Flags every session gets. Streams in and out, partial text, and permission prompts to us.
/// `--verbose` is required for stream-json output in print mode.
const FLAGS: [&str; 9] = [
    "-p",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--verbose",
    "--include-partial-messages",
    "--permission-prompt-tool",
    "stdio",
];

/// `claude -p … [--model <id>] --session-id <uuid>`, or `--resume <uuid>`. No prompt: it goes on
/// stdin. `None`: the model is not a name or the id not a UUID.
pub fn argv(model: Option<&str>, start: &Start) -> Option<Argv> {
    if model.is_some_and(|id| !ottospace_core::agents::is_name(id)) {
        return None;
    }
    let cli = &RunnerId::Claude.spec().terminal;
    let chosen = match cli.session {
        SessionId::Chosen { flag } => flag,
        SessionId::Found { .. } => return None,
    };
    let (flag, id) = match start {
        Start::New { id } => (chosen, id),
        Start::Resume { id } => (cli.resume, id),
    };
    if !ottospace_core::agents::is_uuid(id) {
        return None;
    }
    let model = model.into_iter().flat_map(|id| [cli.model_flag.to_string(), id.to_string()]);
    let args = FLAGS.iter().map(|flag| flag.to_string()).chain(model).chain([flag.to_string(), id.clone()]);
    Some(Argv { program: cli.program, args: args.collect() })
}

/// Error codes of `system/api_retry` and an assistant message's `error`, from the headless docs.
const ERRORS: &[&str] = &[
    "authentication_failed",
    "oauth_org_not_allowed",
    "account_on_hold",
    "billing_error",
    "rate_limit",
    "overloaded",
    "invalid_request",
    "model_not_found",
    "server_error",
    "max_output_tokens",
    "cloud_credential_error",
];

/// A `result`'s error subtypes, from the Agent SDK's TypeScript reference.
const FAILURES: &[&str] =
    &["error_max_turns", "error_during_execution", "error_max_budget_usd", "error_max_structured_output_retries"];

/// What a denial tells the model. Fixed words.
const DENIED: &str = "The user denied this tool call.";

/// One Claude Code session's protocol state.
#[derive(Debug, Default)]
pub struct Claude {
    /// An interrupt was sent and its turn has not finished: that turn ends interrupted.
    interrupting: bool,
    /// Messages sent whose `result` has not come: with none, there is nothing to interrupt.
    pending: usize,
    /// `TurnStarted` was said for the turn running now.
    announced: bool,
    /// Our own control requests are numbered.
    next: u64,
}

impl Claude {
    pub fn new() -> Self {
        Self::default()
    }

    fn record(&mut self, record: &Value, out: &mut Output) {
        let kind = record["type"].as_str();
        let main = record["parent_tool_use_id"].is_null();
        let emit = &mut out.events;
        // Claude names no turn start: the first of its output after a `result` is one.
        if matches!(kind, Some("stream_event" | "assistant")) && !std::mem::replace(&mut self.announced, true) {
            emit.push(Event::TurnStarted);
        }
        match kind {
            Some("system") => match record["subtype"].as_str() {
                Some("init") => emit.push(Event::Started {
                    session: session_of(&record["session_id"]),
                    model: model_of(&record["model"]),
                    mode: record["permissionMode"].as_str().filter(|mode| station::is_name(mode)).map(str::to_string),
                }),
                Some("compact_boundary") => emit.push(Event::Compaction),
                Some("api_retry") => {
                    emit.push(Event::Error { kind: code(&record["error"], "api_error"), retrying: true })
                }
                other => emit.push(Event::Unknown { kind: format!("system.{}", kind_of(other)) }),
            },
            // Raw API stream events. Only text deltas are read: the whole message follows.
            Some("stream_event") => {
                let event = &record["event"];
                let delta = &event["delta"];
                if main && event["type"] == "content_block_delta" && delta["type"] == "text_delta" {
                    let text = delta["text"].as_str().unwrap_or("");
                    emit.push(Event::TextDelta { text: Untrusted::new(text, MAX_TEXT) });
                }
            }
            Some("assistant") => assistant(record, main, emit),
            Some("user") => {
                for block in record["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] == "tool_result"
                        && let Some(id) = opaque(&block["tool_use_id"])
                    {
                        emit.push(Event::ToolEnd { id, failed: block["is_error"] == true });
                    }
                }
            }
            Some("result") => {
                let window = record["modelUsage"].as_object().into_iter().flatten();
                let window = window.filter_map(|(_, usage)| usage["contextWindow"].as_u64()).max();
                let cost = record["total_cost_usd"].as_f64().filter(|usd| usd.is_finite() && *usd >= 0.0);
                emit.push(Event::Usage { context: None, window, cost_micro_usd: cost.map(|usd| (usd * 1e6) as u64) });
                self.pending = self.pending.saturating_sub(1);
                self.announced = false;
                // The CLI says a turn was cut short: `terminal_reason: aborted_streaming` (seen
                // after an interrupt). Our own interrupt counts only if the turn did not succeed:
                // one that arrives as the turn ends leaves it completed.
                let aborted = record["terminal_reason"].as_str().is_some_and(|reason| reason.starts_with("aborted"));
                let interrupted = std::mem::take(&mut self.interrupting) && record["subtype"] != "success";
                let outcome = if aborted || interrupted {
                    Outcome::Interrupted
                } else if record["subtype"] == "success" && record["is_error"] != true {
                    Outcome::Completed
                } else {
                    let subtype = record["subtype"].as_str().unwrap_or("");
                    Outcome::Failed { kind: station::code(subtype, FAILURES, "error") }
                };
                emit.push(Event::TurnFinished { outcome });
            }
            Some("control_request") => self.control(record, out),
            Some("control_cancel_request") => {
                if let Some(id) = request_id(record) {
                    emit.push(Event::PermissionClosed { ticket: Ticket::new(id, TicketKind::Closed) });
                }
            }
            // The answer to our interrupt.
            Some("control_response") => {}
            other => emit.push(Event::Unknown { kind: kind_of(other) }),
        }
    }

    /// A request from the CLI. A tool's permission is the user's to answer; anything else is
    /// answered at once with an error, so the CLI never waits on what nobody will answer.
    fn control(&mut self, record: &Value, out: &mut Output) {
        let Some(id) = request_id(record) else { return };
        let request = &record["request"];
        if request["subtype"] != "can_use_tool" {
            let error = json!({"type": "control_response",
                "response": {"subtype": "error", "request_id": id, "error": "not supported by this client"}});
            out.replies.push(error.to_string());
            out.events.push(Event::Unknown { kind: format!("control.{}", kind_of(request["subtype"].as_str())) });
            return;
        }
        let tool = request["tool_name"].as_str().unwrap_or("");
        let input = request["input"].clone();
        let (station, label) = describe(tool, &input);
        // The CLI's `permission_suggestions` are never read: an answer grants this call alone.
        let (payload, claim) = payload(tool, &input, &request["blocked_path"]);
        let tool_label = station::tool_label(tool);
        let unreadable = !request["blocked_path"].is_null() && !request["blocked_path"].is_string();
        let ask = crate::Ask { tool: tool_label, station, label, payload, claim: &claim };
        let event = match unreadable {
            true => ask.denied(UNREADABLE),
            false => ask.event(Ticket::new(id.clone(), TicketKind::Claude { input })),
        };
        if let Event::AutoDenied { reason, .. } = &event {
            let deny = json!({"behavior": "deny", "message": format!("ottospace denied this tool call: {reason}.")});
            let reply = json!({"type": "control_response",
                    "response": {"subtype": "success", "request_id": id, "response": deny}});
            out.replies.push(reply.to_string());
        }
        out.events.push(event);
    }
}

/// Why a request is denied when a field that says what it does cannot be read.
const UNREADABLE: &str = "a field of it cannot be read";

/// What a `can_use_tool` request asks to approve, a line per field, and the model's own
/// description of it. A shell command and every option besides its description (`option: key =
/// value`); any other tool, its whole input (`input: {…}`); the path outside what the session may
/// touch that made the CLI ask (`blocked path:`).
fn payload(tool: &str, input: &Value, blocked: &Value) -> (Payload, String) {
    let mut payload = Payload::new();
    let claim = match (tool, input["command"].as_str(), input.as_object()) {
        ("Bash" | "PowerShell", Some(command), Some(fields)) => {
            payload.field("command", command);
            for (key, value) in fields.iter().filter(|(key, _)| !matches!(key.as_str(), "command" | "description")) {
                payload.field("option", &format!("{key} = {value}"));
            }
            input["description"].as_str().unwrap_or("").to_string()
        }
        _ => {
            payload.field("input", &input.to_string());
            String::new()
        }
    };
    if let Some(path) = blocked.as_str() {
        payload.field("blocked path", path);
    }
    (payload, claim)
}

fn assistant(record: &Value, main: bool, emit: &mut Vec<Event>) {
    let message = &record["message"];
    let parent = opaque(&record["parent_tool_use_id"]);
    for block in message["content"].as_array().into_iter().flatten() {
        match block["type"].as_str() {
            Some("text") if main => {
                emit.push(Event::Text { text: Untrusted::new(block["text"].as_str().unwrap_or(""), MAX_TEXT) })
            }
            Some("tool_use") => {
                let Some(id) = opaque(&block["id"]) else { continue };
                let tool = block["name"].as_str().unwrap_or("");
                let (station, label) = describe(tool, &block["input"]);
                let detail = detail(tool, &block["input"]);
                emit.push(Event::ToolStart {
                    id,
                    parent: parent.clone(),
                    tool: station::tool_label(tool),
                    station,
                    label,
                    detail,
                });
            }
            _ => {}
        }
    }
    if let Some(error) = record.get("error").filter(|error| !error.is_null()) {
        emit.push(Event::Error { kind: code(error, "api_error"), retrying: false });
    }
    let usage = &message["usage"];
    let parts = ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens", "output_tokens"];
    if main && usage.is_object() {
        let context = parts.iter().filter_map(|part| usage[*part].as_u64()).fold(0u64, u64::saturating_add);
        emit.push(Event::Usage { context: Some(context), window: None, cost_micro_usd: None });
    }
}

/// A control request's id: a non-empty string.
fn request_id(record: &Value) -> Option<Value> {
    Some(record["request_id"].clone()).filter(|id| id.as_str().is_some_and(|id| !id.is_empty()))
}

/// An error code from the table, else `fallback`.
fn code(value: &Value, fallback: &'static str) -> String {
    station::code(value.as_str().unwrap_or(""), ERRORS, fallback)
}

/// What the user reads to judge a call: the command, the path or the address; else the input
/// as JSON. Untrusted, and cut.
fn detail(tool: &str, input: &Value) -> Untrusted {
    let text = |key: &str| input[key].as_str();
    let shown = match tool {
        "Bash" | "PowerShell" => text("command"),
        "WebFetch" => text("url"),
        _ => text("file_path").or(text("notebook_path")).or(text("path")),
    };
    match shown {
        Some(text) => Untrusted::new(text, MAX_DETAIL),
        None => Untrusted::new(&input.to_string(), MAX_DETAIL),
    }
}

/// Standard base64 with padding, for an image's bytes.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            let shown = i <= chunk.len();
            out.push(if shown { ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

impl Wire for Claude {
    fn opening(&mut self) -> Vec<String> {
        Vec::new()
    }

    fn awaits_answer(&self, head: &str) -> bool {
        head.contains("\"control_request\"")
    }

    fn read(&mut self, line: &str) -> Output {
        let mut out = Output::default();
        if line.trim().is_empty() {
            return out;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(record) if record.is_object() => self.record(&record, &mut out),
            _ => out.events.push(Event::Unknown { kind: "not_json".into() }),
        }
        out
    }

    fn write(&mut self, input: Input) -> Result<Vec<String>, Refused> {
        let line = match input {
            Input::Send { text, images } => {
                if text.is_empty() && images.is_empty() {
                    return Err(Refused("nothing to send"));
                }
                let images = images.iter().map(|image| {
                    let source =
                        json!({"type": "base64", "media_type": image.media.mime(), "data": base64(&image.bytes)});
                    json!({"type": "image", "source": source})
                });
                let text = (!text.is_empty()).then(|| json!({"type": "text", "text": text}));
                let content: Vec<Value> = text.into_iter().chain(images).collect();
                self.pending += 1;
                json!({"type": "user", "message": {"role": "user", "content": content}, "parent_tool_use_id": null, "session_id": ""})
            }
            Input::Answer { ticket, answer } => {
                let TicketKind::Claude { input } = ticket.kind else {
                    return Err(Refused("not a Claude Code request"));
                };
                // This call, its input as it came: never `updatedPermissions`, never a mode.
                let result = match answer {
                    Answer::Allow => json!({"behavior": "allow", "updatedInput": input}),
                    Answer::Deny { message } => {
                        json!({"behavior": "deny", "message": message.unwrap_or_else(|| DENIED.to_string())})
                    }
                };
                json!({"type": "control_response",
                    "response": {"subtype": "success", "request_id": ticket.id, "response": result}})
            }
            // Idle: nothing to interrupt, and no flag left to mark the next turn interrupted.
            Input::Interrupt if self.pending == 0 => return Ok(Vec::new()),
            Input::Interrupt => {
                self.interrupting = true;
                self.next += 1;
                let id = format!("ottospace-{}", self.next);
                json!({"type": "control_request", "request_id": id, "request": {"subtype": "interrupt"}})
            }
        };
        Ok(vec![line.to_string()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "8f14e45f-ceea-467f-a8d3-0242ac130003";

    #[test]
    fn argv_is_the_table_a_model_and_a_session() {
        let new = Start::New { id: UUID.into() };
        let args = argv(Some("claude-opus-5-5"), &new).unwrap().args.join(" ");
        assert_eq!(
            args,
            format!(
                "-p --input-format stream-json --output-format stream-json --verbose --include-partial-messages \
                 --permission-prompt-tool stdio --model claude-opus-5-5 --session-id {UUID}"
            )
        );
        let resume = argv(None, &Start::Resume { id: UUID.into() }).unwrap();
        assert_eq!(resume.args[FLAGS.len()..], ["--resume".to_string(), UUID.to_string()]);
        assert_eq!(argv(Some("--dangerously-skip-permissions"), &new), None);
        assert_eq!(argv(None, &Start::Resume { id: "--continue".into() }), None);
    }

    #[test]
    fn base64_matches_the_rfc_vectors() {
        let vectors = [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foobar", "Zm9vYmFy")];
        for (plain, coded) in vectors {
            assert_eq!(base64(plain.as_bytes()), coded);
        }
        assert_eq!(base64(&[0xff, 0xfe]), "//4=");
    }

    #[test]
    fn a_message_goes_on_stdin_with_its_images() {
        let image = crate::Image { path: "/i.png".into(), media: crate::Media::Png, bytes: b"\x89PNG".to_vec() };
        let lines = Claude::new().write(Input::Send { text: "hi".into(), images: vec![image] }).unwrap();
        let sent: Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(sent["message"]["content"][0], json!({"type": "text", "text": "hi"}));
        assert_eq!(sent["message"]["content"][1]["source"]["data"], "iVBORw==");
        assert_eq!(
            Claude::new().write(Input::Send { text: String::new(), images: vec![] }),
            Err(Refused("nothing to send"))
        );
    }

    fn ask(tool: &str, input: Value, suggestions: Value) -> Output {
        let request = json!({"type": "control_request", "request_id": "r1", "request": {"subtype": "can_use_tool",
            "tool_name": tool, "input": input, "permission_suggestions": suggestions}});
        Claude::new().read(&request.to_string())
    }

    fn answered(claude: &mut Claude, out: &Output, answer: Answer) -> Result<Value, Refused> {
        let Some(Event::Permission { ticket, .. }) = out.events.first().cloned() else { panic!("{out:?}") };
        let line = claude.write(Input::Answer { ticket, answer })?;
        Ok(serde_json::from_str(&line[0]).unwrap())
    }

    /// An answer grants this one call, as shown, and nothing more: the CLI's suggestions (a
    /// mode, directories, rules) are never sent back, nor any rule of ours.
    #[test]
    fn an_answer_grants_this_call_alone() {
        let hostile = json!([
            {"type": "setMode", "mode": "bypassPermissions", "destination": "session"},
            {"type": "addDirectories", "directories": ["/"], "destination": "session"},
            {"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": "*"}], "behavior": "allow", "destination": "session"}
        ]);
        let input = json!({"command": "git status --short", "description": "Show changes"});
        let out = ask("Bash", input.clone(), hostile);
        let Some(Event::Permission { detail, claim, cut, .. }) = out.events.first().cloned() else { panic!() };
        assert_eq!((detail.text(), claim.text(), cut), ("command: git status --short", "Show changes", false));
        let allowed = answered(&mut Claude::new(), &out, Answer::Allow).unwrap();
        assert_eq!(allowed["response"]["response"], json!({"behavior": "allow", "updatedInput": input}));
        let message = Some("Use git diff".to_string());
        let denied = answered(&mut Claude::new(), &out, Answer::Deny { message }).unwrap();
        assert_eq!(denied["response"]["response"], json!({"behavior": "deny", "message": "Use git diff"}));
        let denied = answered(&mut Claude::new(), &out, Answer::Deny { message: None }).unwrap();
        assert_eq!(denied["response"]["response"]["message"], DENIED);
    }

    /// What is approved is shown whole, or marked cut with its full text on the ticket; past the
    /// hard cap it is denied without asking.
    #[test]
    fn a_long_command_is_marked_cut_and_a_huge_one_denied() {
        let long = format!("echo {}", "a".repeat(MAX_DETAIL * 2));
        let out = ask("Bash", json!({"command": long, "run_in_background": true}), json!([]));
        let Some(Event::Permission { ticket, detail, len, cut, .. }) = out.events.first().cloned() else { panic!() };
        let whole = format!("command: {long}\noption: run_in_background = true");
        assert!(cut && len == whole.len() && ticket.full().text() == whole);
        assert!(detail.text().ends_with(&format!("… [{} more bytes]", whole.len() - MAX_DETAIL)), "{detail:?}");
        // Another tool: its whole input, so a Write's content is part of what is approved.
        let out = ask("Write", json!({"file_path": "/w/a", "content": "x\u{1b}[2J\u{202e}y"}), json!([]));
        let Some(Event::Permission { detail, .. }) = out.events.first() else { panic!() };
        assert!(
            detail.text().starts_with("input: {") && detail.text().contains("x\\\\u001b[2J\\u{202e}y"),
            "{}",
            detail.text()
        );
        // Past the hard cap: denied at once, nothing to answer.
        let out = ask("Bash", json!({"command": "a".repeat(crate::MAX_APPROVABLE + 1)}), json!([]));
        assert!(matches!(&out.events[..], [Event::AutoDenied { reason: "too long to show in full", .. }]));
        let reply: Value = serde_json::from_str(&out.replies[0]).unwrap();
        assert_eq!(
            (&reply["response"]["request_id"], &reply["response"]["response"]["behavior"]),
            (&json!("r1"), &json!("deny"))
        );
    }

    #[test]
    fn requests_nobody_answers_get_an_error_at_once() {
        let out =
            Claude::new().read(r#"{"type":"control_request","request_id":"h1","request":{"subtype":"hook_callback"}}"#);
        assert_eq!(out.events, [Event::Unknown { kind: "control.hook_callback".into() }]);
        let reply: Value = serde_json::from_str(&out.replies[0]).unwrap();
        assert_eq!((&reply["response"]["subtype"], &reply["response"]["request_id"]), (&json!("error"), &json!("h1")));
    }

    #[test]
    fn an_interrupted_turn_ends_interrupted() {
        let mut claude = Claude::new();
        // Idle: nothing is sent, and the next turn is not marked.
        assert_eq!(claude.write(Input::Interrupt), Ok(vec![]));
        let send = || Input::Send { text: "go".into(), images: vec![] };
        claude.write(send()).unwrap();
        let out = claude.read(r#"{"type":"stream_event","parent_tool_use_id":null,"event":{"type":"message_start"}}"#);
        assert_eq!(out.events, [Event::TurnStarted]);
        let line = claude.write(Input::Interrupt).unwrap();
        assert!(line[0].contains(r#""subtype":"interrupt""#));
        let out = claude.read(r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#);
        assert_eq!(out.events.last(), Some(&Event::TurnFinished { outcome: Outcome::Interrupted }));
        assert_eq!(claude.write(Input::Interrupt), Ok(vec![]), "the turn is over");
        claude.write(send()).unwrap();
        let out = claude.read(r#"{"type":"result","subtype":"error_max_turns","is_error":true}"#);
        let failed = Outcome::Failed { kind: "error_max_turns".into() };
        assert_eq!(out.events.last(), Some(&Event::TurnFinished { outcome: failed }));
        // An interrupt that reaches a turn as it ends leaves it completed.
        claude.write(send()).unwrap();
        claude.write(Input::Interrupt).unwrap();
        let done = r#"{"type":"result","subtype":"success","is_error":false,"terminal_reason":"completed"}"#;
        assert_eq!(claude.read(done).events.last(), Some(&Event::TurnFinished { outcome: Outcome::Completed }));
        // The CLI's own word for a cut turn, as 2.1.323 sends it after an interrupt (seen in a probe).
        let cut = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"terminal_reason":"aborted_streaming"}"#;
        assert_eq!(Claude::new().read(cut).events.last(), Some(&Event::TurnFinished { outcome: Outcome::Interrupted }));
    }

    /// The path that made the CLI ask is part of what is allowed: shown with the command.
    #[test]
    fn a_blocked_path_is_shown() {
        let request = json!({"type": "control_request", "request_id": "r", "request": {"subtype": "can_use_tool",
            "tool_name": "Bash", "input": {"command": "cat ~/.ssh/id_rsa"}, "blocked_path": "/Users/me/.ssh/id_rsa"}});
        let out = Claude::new().read(&request.to_string());
        let Some(Event::Permission { detail, .. }) = out.events.first() else { panic!("{out:?}") };
        assert_eq!(detail.text(), "command: cat ~/.ssh/id_rsa\nblocked path: /Users/me/.ssh/id_rsa");
    }

    /// Whatever the strings in a request, what is approved is one line per field, each starting
    /// with its fixed label: `command`, an `option` per other input field, `blocked path`.
    #[test]
    fn any_request_is_one_labelled_line_per_field() {
        let mut text = crate::payload::testing::hostile(5);
        for _ in 0..2000 {
            let mut input = json!({"command": text(), "description": text()});
            let options = (text().len() % 3) + usize::from(text().is_empty());
            for i in 0..options {
                input[format!("{}{i}", text())] = json!(text());
            }
            let keys = input.as_object().map_or(0, |map| map.len()) - 2;
            let blocked = json!(text());
            let (payload, _) = payload("Bash", &input, &blocked);
            let labels: Vec<&str> =
                std::iter::once("command").chain(std::iter::repeat_n("option", keys)).chain(["blocked path"]).collect();
            crate::payload::testing::assert_lines(payload.text(), &labels);
            let (payload, _) = payload_of_other(&input, &blocked);
            crate::payload::testing::assert_lines(payload.text(), &["input", "blocked path"]);
        }
    }

    /// `payload` the function, which the test's `payload` values shadow.
    fn payload_of_other(input: &Value, blocked: &Value) -> (Payload, String) {
        payload("Write", input, blocked)
    }

    /// A `blocked_path` that is not text cannot be shown: the request is denied, not asked.
    #[test]
    fn an_unreadable_blocked_path_is_denied() {
        let request = json!({"type": "control_request", "request_id": "r", "request": {"subtype": "can_use_tool",
            "tool_name": "Bash", "input": {"command": "ls"}, "blocked_path": {"path": "/x\nharmless"}}});
        let out = Claude::new().read(&request.to_string());
        assert!(matches!(&out.events[..], [Event::AutoDenied { reason: UNREADABLE, .. }]), "{out:?}");
        let reply: Value = serde_json::from_str(&out.replies[0]).unwrap();
        assert_eq!(reply["response"]["response"]["behavior"], "deny");
    }

    #[test]
    fn a_codex_ticket_is_refused() {
        let ticket = Ticket::new(json!(1), TicketKind::Codex);
        assert!(Claude::new().write(Input::Answer { ticket, answer: Answer::Allow }).is_err());
    }

    #[test]
    fn stations_come_from_the_shared_rules() {
        let out = Claude::new().read(
            r#"{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","id":"t","name":"Read","input":{"file_path":"/a/b/c.rs"}}]}}"#,
        );
        let Some(Event::ToolStart { station, label, .. }) = out.events.get(1) else { panic!("{out:?}") };
        assert_eq!((*station, label.as_str()), (station::Station::Files, "b/c.rs"));
    }
}
