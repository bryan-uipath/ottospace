//! Claude Code in print mode with stream-json in and out. Pure.
//!
//! Out (stdout), from the Agent SDK's TypeScript reference: `system/init`, `stream_event` (raw
//! API stream events), `assistant`, `user`, `result`, `system/api_retry`,
//! `system/compact_boundary`. Permission prompts and interrupts go by `control_request` and
//! `control_response` lines, whose shape the docs do not print: UNVERIFIED, see the design note.

use ottospace_sessions::claude::describe;
use ottospace_sessions::station;
use serde_json::{Value, json};

use crate::{
    Answer, Argv, Event, Input, MAX_DETAIL, MAX_TEXT, Outcome, Output, Protocol, Refused, Start, Ticket, TicketKind,
    Untrusted, kind_of, model_of, opaque, session_of,
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
    let (flag, id) = match start {
        Start::New { id } => ("--session-id", id),
        Start::Resume { id } => ("--resume", id),
    };
    if !crate::is_uuid(id) {
        return None;
    }
    let model = model.into_iter().flat_map(|id| ["--model".to_string(), id.to_string()]);
    let args = FLAGS.iter().map(|flag| flag.to_string()).chain(model).chain([flag.to_string(), id.clone()]);
    Some(Argv { program: "claude", args: args.collect() })
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
        match kind {
            Some("system") => match record["subtype"].as_str() {
                Some("init") => emit.push(Event::Started {
                    session: session_of(&record["session_id"]),
                    model: model_of(&record["model"]),
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
                let outcome = if std::mem::take(&mut self.interrupting) {
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
                    emit.push(Event::PermissionClosed { ticket: Ticket { id, kind: TicketKind::Closed } });
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
        let suggestions = request["permission_suggestions"].as_array().cloned().unwrap_or_default();
        out.events.push(Event::Permission {
            tool: station::tool_label(tool),
            station,
            label,
            detail: detail(tool, &input),
            ticket: Ticket { id, kind: TicketKind::Claude { input, suggestions } },
        });
    }
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

impl Protocol for Claude {
    fn opening(&mut self) -> Vec<String> {
        Vec::new()
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
                json!({"type": "user", "message": {"role": "user", "content": content}, "parent_tool_use_id": null, "session_id": ""})
            }
            Input::Answer { ticket, answer } => {
                let TicketKind::Claude { input, suggestions } = ticket.kind else {
                    return Err(Refused("not a Claude Code request"));
                };
                let result = match answer {
                    Answer::Allow => json!({"behavior": "allow", "updatedInput": input}),
                    // The CLI's own suggested rules, kept to this session: never written to settings.
                    Answer::AllowForSession => {
                        let session = suggestions.into_iter().filter(Value::is_object).map(|mut rule| {
                            rule["destination"] = json!("session");
                            rule
                        });
                        json!({"behavior": "allow", "updatedInput": input, "updatedPermissions": session.collect::<Vec<_>>()})
                    }
                    Answer::Deny => json!({"behavior": "deny", "message": DENIED}),
                    Answer::DenyAndStop => {
                        self.interrupting = true;
                        json!({"behavior": "deny", "message": DENIED, "interrupt": true})
                    }
                };
                json!({"type": "control_response",
                    "response": {"subtype": "success", "request_id": ticket.id, "response": result}})
            }
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

    #[test]
    fn always_allow_keeps_the_rules_to_this_session() {
        let mut claude = Claude::new();
        let request = json!({"type": "control_request", "request_id": "r1", "request": {"subtype": "can_use_tool",
            "tool_name": "Bash", "input": {"command": "git push"},
            "permission_suggestions": [{"type": "addRules", "behavior": "allow", "destination": "localSettings",
                "rules": [{"toolName": "Bash", "ruleContent": "git push:*"}]}, "junk"]}});
        let out = claude.read(&request.to_string());
        let Some(Event::Permission { ticket, tool, label, detail, .. }) = out.events.first().cloned() else { panic!() };
        assert_eq!((tool.as_str(), label.as_str(), detail.text()), ("Bash", "git push", "git push"));
        let line = claude.write(Input::Answer { ticket, answer: Answer::AllowForSession }).unwrap();
        let answer: Value = serde_json::from_str(&line[0]).unwrap();
        assert_eq!(answer["response"]["request_id"], "r1");
        let permissions = &answer["response"]["response"]["updatedPermissions"];
        assert_eq!(permissions.as_array().map(Vec::len), Some(1));
        assert_eq!(permissions[0]["destination"], "session");
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
        let line = claude.write(Input::Interrupt).unwrap();
        assert!(line[0].contains(r#""subtype":"interrupt""#));
        let out = claude.read(r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#);
        assert_eq!(out.events.last(), Some(&Event::TurnFinished { outcome: Outcome::Interrupted }));
        let out = claude.read(r#"{"type":"result","subtype":"error_max_turns","is_error":true}"#);
        let failed = Outcome::Failed { kind: "error_max_turns".into() };
        assert_eq!(out.events.last(), Some(&Event::TurnFinished { outcome: failed }));
    }

    #[test]
    fn a_codex_ticket_is_refused() {
        let ticket = Ticket { id: json!(1), kind: TicketKind::Codex };
        assert!(Claude::new().write(Input::Answer { ticket, answer: Answer::Allow }).is_err());
    }

    #[test]
    fn stations_come_from_the_shared_rules() {
        let out = Claude::new().read(
            r#"{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","id":"t","name":"Read","input":{"file_path":"/a/b/c.rs"}}]}}"#,
        );
        let Some(Event::ToolStart { station, label, .. }) = out.events.first() else { panic!("{out:?}") };
        assert_eq!((*station, label.as_str()), (station::Station::Files, "b/c.rs"));
    }
}
