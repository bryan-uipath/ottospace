//! Claude Code in print mode with stream-json in and out. Pure.
//!
//! Out (stdout), from the Agent SDK's TypeScript reference: `system/init`, `stream_event` (raw
//! API stream events), `assistant`, `user`, `result`, `system/api_retry`,
//! `system/compact_boundary`. Permission prompts and interrupts go by `control_request` and
//! `control_response` lines, whose shape the docs do not print: UNVERIFIED, see the design note.

use ottospace_core::ask::{self, Choice, Chosen, Question};
use ottospace_core::runner::{RunnerId, SessionId};
use ottospace_core::transcript::{By, Outcome, Status, Text};
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
    /// The error code an assistant message of this turn carried: a failed `result` names it.
    error: Option<String>,
    /// Side questions sent and not answered yet, by our id: only their lines are read as side events.
    sides: Vec<u64>,
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
                // Seen in 2.1.326's own code: `task_started` {task_id, tool_use_id, description,
                // task_type, …}, `task_notification` {task_id, tool_use_id, status, summary, …},
                // `permission_denied` {tool_name, tool_use_id, decision_reason_type, …},
                // `notification` {key, text, priority, color}.
                Some("task_started") => emit.push(Event::Subtask {
                    id: opaque(&record["tool_use_id"]),
                    description: Untrusted::new(record["description"].as_str().unwrap_or(""), MAX_DETAIL),
                }),
                Some("task_notification") => emit.push(Event::SubtaskEnded {
                    status: Status::named(record["status"].as_str()),
                    summary: Untrusted::new(record["summary"].as_str().unwrap_or(""), MAX_DETAIL),
                }),
                Some("permission_denied") => {
                    if let Some(by) = By::claude(record["decision_reason_type"].as_str()) {
                        let tool = station::tool_label(record["tool_name"].as_str().unwrap_or(""));
                        emit.push(Event::DeniedBy { tool, by });
                    }
                }
                Some("notification") if record["text"].is_string() => {
                    emit.push(Event::Note { text: Untrusted::new(record["text"].as_str().unwrap_or(""), MAX_DETAIL) })
                }
                Some("model_fallback" | "model_refusal_fallback" | "model_consent_fallback") => {
                    let name = |a: &str, b: &str| model_of(&record[a]).or_else(|| model_of(&record[b]));
                    match name("fallbackModel", "fallback_model") {
                        Some(to) => {
                            emit.push(Event::ModelFallback { from: name("originalModel", "original_model"), to })
                        }
                        None => emit
                            .push(Event::Unknown { kind: format!("system.{}", kind_of(record["subtype"].as_str())) }),
                    }
                }
                // A side question's progress: `started`, or `api_retry` (the CLI retries by itself).
                Some("control_request_progress") if let Some(id) = self.side_of(record) => {
                    if record["status"] == "started" {
                        emit.push(Event::SideStarted { id });
                    }
                }
                other => emit.push(unread(record, format!("system.{}", kind_of(other)))),
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
            Some("assistant") => {
                if let Some(kind) = assistant(record, main, emit) {
                    self.error = Some(kind);
                }
            }
            Some("user") => {
                for block in record["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] == "tool_result"
                        && let Some(id) = opaque(&block["tool_use_id"])
                    {
                        let output = ottospace_core::gist::output(&block["content"]);
                        let output = Untrusted::new(output.text(), ottospace_core::gist::MAX_OUTPUT);
                        emit.push(Event::ToolEnd { id, failed: block["is_error"] == true, output });
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
                let error = self.error.take();
                let subtype = record["subtype"].as_str().unwrap_or("");
                let outcome = if aborted || interrupted {
                    Outcome::Interrupted
                } else if subtype == "success" && record["is_error"] != true {
                    Outcome::Completed
                } else if FAILURES.contains(&subtype) {
                    Outcome::Failed { kind: subtype.to_string() }
                } else if subtype == "success" {
                    // `is_error` on a "success": the CLI's own failure (an API key, say), named by
                    // the assistant message's `error` if it had one.
                    Outcome::Failed { kind: error.unwrap_or_else(|| "error".into()) }
                } else {
                    // A subtype no table has: its name, if it is one, so it is never silent.
                    Outcome::Failed { kind: kind_of(Some(subtype)) }
                };
                emit.push(Event::TurnFinished { outcome });
            }
            Some("control_request") => self.control(record, out),
            Some("control_cancel_request") => {
                if let Some(id) = request_id(record) {
                    emit.push(Event::PermissionClosed { ticket: Ticket::new(id, TicketKind::Closed) });
                }
            }
            Some("control_response") => self.side_answer(&record["response"], emit),
            other => emit.push(unread(record, kind_of(other))),
        }
    }

    /// The side question a line names by its `request_id` (`btw-<id>`), if it is one waiting.
    fn side_of(&self, record: &Value) -> Option<u64> {
        let named = record["request_id"].as_str()?;
        // The whole id as we wrote it: `btw-01` is not `btw-1`.
        self.sides.iter().copied().find(|id| named == format!("{SIDE}{id}"))
    }

    /// The answer to a request of ours: a side question's, else our interrupt's (nothing to read).
    /// Seen in 2.1.327: `{subtype: success, response: {response, synthetic}}` or `{subtype: error, error}`.
    fn side_answer(&mut self, response: &Value, emit: &mut Vec<Event>) {
        let Some(id) = self.side_of(response) else { return };
        self.sides.retain(|of| *of != id);
        let answer = &response["response"];
        emit.push(match (response["subtype"].as_str(), answer["response"].as_str()) {
            (Some("success"), Some(text)) => {
                Event::SideAnswer { id, text: Untrusted::new(text, MAX_TEXT), synthetic: answer["synthetic"] == true }
            }
            _ => Event::SideFailed { id },
        });
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
        let asks = RunnerId::Claude.spec().structured.asks == Some(tool);
        let shown = asks.then(|| questions(&input)).flatten();
        let mut event = match unreadable {
            true => ask.denied(UNREADABLE),
            false => ask.event(Ticket::new(id.clone(), TicketKind::Claude { input })),
        };
        // The agent asks the user: a question card. One a card cannot show is a permission card.
        if let (Event::Permission { questions, .. }, Some(shown)) = (&mut event, shown) {
            *questions = shown;
        }
        if let Event::AutoDenied { reason, .. } = &event {
            let deny = json!({"behavior": "deny", "message": format!("ottospace denied this tool call: {reason}.")});
            let reply = json!({"type": "control_response",
                    "response": {"subtype": "success", "request_id": id, "response": deny}});
            out.replies.push(reply.to_string());
        }
        out.events.push(event);
    }
}

/// The start of a side question's `request_id`: `btw-1`.
const SIDE: &str = "btw-";

/// Why a request is denied when a field that says what it does cannot be read.
const UNREADABLE: &str = "a field of it cannot be read";

/// An `AskUserQuestion` input's questions (`question`, `header`, `options` of `label` and
/// `description`, `multiSelect`), as a card shows them. `None`: not one a card can show: no
/// question or too many, too many options, a question or label that is not text or too long
/// to show whole, or two questions alike (its answers are keyed by the question's text). Core
/// `Text` from here, as `Untrusted` is: drawn, never in the state.
fn questions(input: &Value) -> Option<Vec<Question>> {
    let all = input["questions"].as_array().filter(|all| (1..=ask::MAX_QUESTIONS).contains(&all.len()))?;
    // Shown whole or not at all: two labels cut alike could not be told apart.
    let text = |value: &Value| value.as_str().filter(|text| text.len() <= MAX_DETAIL).map(Text::new);
    let read = |q: &Value| {
        let options = q["options"].as_array().map_or(&[][..], Vec::as_slice);
        let choice = |o: &Value| {
            let description = text(&o["description"]).unwrap_or_default();
            Some(Choice { label: text(&o["label"]).filter(|label| !label.is_empty())?, description })
        };
        let options = options.iter().map(choice).collect::<Option<Vec<Choice>>>()?;
        let (header, question) = (text(&q["header"]).unwrap_or_default(), text(&q["question"])?);
        (options.len() <= ask::MAX_OPTIONS).then_some(Question {
            header,
            question,
            options,
            multi: q["multiSelect"] == true,
        })
    };
    let read: Vec<Question> = all.iter().map(read).collect::<Option<_>>()?;
    let alike = read.iter().enumerate().any(|(i, q)| read[..i].iter().any(|other| other.question == q.question));
    (!alike).then_some(read)
}

/// `updatedInput` for an answered question request: its input as it came, and `answers`: each
/// question's text → the label picked, or the user's words; in a multi-select question each
/// joined by `", "`, one holding `", "` or `"` quoted as JSON, as the CLI joins and splits them.
/// A question with no answer is left out: the CLI then reports only the others, and "The
/// user did not answer the questions." with none (read in 2.1.327's code, seen in a probe).
fn answered(input: &Value, chosen: &[Chosen]) -> Result<Value, Refused> {
    let all = input["questions"].as_array().filter(|all| all.len() == chosen.len());
    let all = all.ok_or(Refused("not one answer per question"))?;
    let mut answers = serde_json::Map::new();
    for (q, chosen) in all.iter().zip(chosen) {
        let label = |at: &usize| q["options"][*at]["label"].as_str().map(str::to_string);
        let labels = chosen.options.iter().map(label).collect::<Option<Vec<String>>>();
        let mut parts = labels.ok_or(Refused("no such option"))?;
        parts.extend(chosen.other.clone());
        let quoted = |part: &String| match part.contains(", ") || part.contains('"') {
            true => Value::from(part.as_str()).to_string(),
            false => part.clone(),
        };
        let value = match (q["multiSelect"] == true, &parts[..]) {
            (_, []) => continue,
            (false, [one]) => one.clone(),
            (false, _) => return Err(Refused("one answer to a single-select question")),
            (true, _) => parts.iter().map(quoted).collect::<Vec<String>>().join(", "),
        };
        let question = q["question"].as_str().ok_or(Refused("a question that is not text"))?;
        answers.insert(question.to_string(), Value::from(value));
    }
    let mut updated = input.clone();
    updated["answers"] = Value::Object(answers);
    Ok(updated)
}

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

/// A record no rule reads: an `Error` if it looks like one (`is_error`, an `error` field, or
/// "error" in its kind), so a failure is never silent; else `Unknown`.
fn unread(record: &Value, kind: String) -> Event {
    let erring = record["is_error"] == true || !record["error"].is_null() || kind.contains("error");
    match erring {
        true => Event::Error { kind, retrying: false },
        false => Event::Unknown { kind },
    }
}

/// An assistant message's events. Returns its `error` code, if it had one.
fn assistant(record: &Value, main: bool, emit: &mut Vec<Event>) -> Option<String> {
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
                    diff: ottospace_core::diff::claude(tool, &block["input"]),
                    gist: ottospace_core::gist::claude(tool, &block["input"]),
                });
            }
            _ => {}
        }
    }
    let error = record.get("error").filter(|error| !error.is_null()).map(|error| code(error, "api_error"));
    if let Some(kind) = &error {
        emit.push(Event::Error { kind: kind.clone(), retrying: false });
    }
    let usage = &message["usage"];
    let parts = ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens", "output_tokens"];
    if main && usage.is_object() {
        let context = parts.iter().filter_map(|part| usage[*part].as_u64()).fold(0u64, u64::saturating_add);
        emit.push(Event::Usage { context: Some(context), window: None, cost_micro_usd: None });
    }
    error
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
/// as JSON, pretty-printed (an expanded row wraps it). Untrusted, and cut.
fn detail(tool: &str, input: &Value) -> Untrusted {
    let text = |key: &str| input[key].as_str();
    let shown = match tool {
        "Bash" | "PowerShell" => text("command"),
        "WebFetch" => text("url"),
        _ => text("file_path").or(text("notebook_path")).or(text("path")),
    };
    match shown {
        Some(text) => Untrusted::new(text, MAX_DETAIL),
        None => Untrusted::json(input, MAX_DETAIL),
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
                    Answer::Questions(chosen) => {
                        json!({"behavior": "allow", "updatedInput": answered(&input, &chosen)?})
                    }
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
            Input::SideQuestion { .. } if !RunnerId::Claude.spec().structured.side_question => {
                return Err(Refused("no side questions"));
            }
            Input::SideQuestion { id, question, .. } if question.trim().is_empty() || self.sides.contains(&id) => {
                return Err(Refused("nothing to ask"));
            }
            Input::SideQuestion { id, question, history } => {
                self.sides.push(id);
                let mut request = json!({"subtype": "side_question", "question": question});
                if !history.is_empty() {
                    let history = history.into_iter().map(|(q, a)| json!({"question": q, "response": a}));
                    request["history"] = Value::Array(history.collect());
                }
                json!({"type": "control_request", "request_id": format!("{SIDE}{id}"), "request": request})
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

    /// The request as 2.1.327 sent it in a probe (`docs/ai/agent-runners.md`), labels made up.
    fn asked() -> Value {
        json!({"questions": [
            {"question": "Which colour?", "header": "Colour", "multiSelect": false,
             "options": [{"label": "Red", "description": "warm"}, {"label": "Blue", "description": "cool"}]},
            {"question": "Which sizes?", "header": "Sizes", "multiSelect": true,
             "options": [{"label": "Small", "description": "s"}, {"label": "Large, wide", "description": "l"}]}]})
    }

    fn chosen(options: &[usize], other: Option<&str>) -> Chosen {
        Chosen { options: options.to_vec(), other: other.map(str::to_string) }
    }

    /// The runner's asking tool is a question card; its answers go back as `updatedInput.answers`,
    /// the input otherwise as it came: question → label, several joined as the CLI joins them
    /// (one holding `", "` quoted), or the user's words. The CLI's reply to exactly these, in
    /// the probe: "Which colour?"="Blue", "Which sizes?"="Small, \"Large, wide\"".
    #[test]
    fn a_question_request_is_answered_with_the_picks() {
        let out = ask("AskUserQuestion", asked(), json!([]));
        let Some(Event::Permission { questions, .. }) = out.events.first() else { panic!("{out:?}") };
        let shown: Vec<(&str, &str, usize, bool)> =
            questions.iter().map(|q| (q.header.text(), q.question.text(), q.options.len(), q.multi)).collect();
        assert_eq!(shown, [("Colour", "Which colour?", 2, false), ("Sizes", "Which sizes?", 2, true)]);
        assert_eq!(questions[1].options[1].label.text(), "Large, wide");
        let answers = |chosen: Vec<Chosen>| answered(&mut Claude::new(), &out, Answer::Questions(chosen));
        let allowed = answers(vec![chosen(&[1], None), chosen(&[0, 1], None)]).unwrap();
        let mut input = asked();
        input["answers"] = json!({"Which colour?": "Blue", "Which sizes?": "Small, \"Large, wide\""});
        assert_eq!(allowed["response"]["response"], json!({"behavior": "allow", "updatedInput": input}));
        let words = answers(vec![chosen(&[], Some("Green please")), chosen(&[], Some("huge"))]).unwrap();
        let said = &words["response"]["response"]["updatedInput"]["answers"];
        assert_eq!(said, &json!({"Which colour?": "Green please", "Which sizes?": "huge"}));
        // Words beside picks in a multi-select question; a quote in them quoted too.
        let both = answers(vec![chosen(&[0], None), chosen(&[0], Some(r#"a "b""#))]).unwrap();
        let said = &both["response"]["response"]["updatedInput"]["answers"]["Which sizes?"];
        assert_eq!(said, &json!(r#"Small, "a \"b\"""#));
        // A question left unanswered is left out.
        let skipped = answers(vec![chosen(&[0], None), chosen(&[], None)]).unwrap();
        let said = &skipped["response"]["response"]["updatedInput"]["answers"];
        assert_eq!(said, &json!({"Which colour?": "Red"}));
        let wrong = [
            vec![chosen(&[0], None)],
            vec![chosen(&[5], None), chosen(&[0], None)],
            vec![chosen(&[0, 1], None), chosen(&[0], None)],
        ];
        for wrong in wrong {
            assert!(answers(wrong).is_err());
        }
    }

    /// A question request a card cannot show is an ordinary permission card; another tool's
    /// `questions` field is only its input.
    #[test]
    fn a_question_request_a_card_cannot_show_is_a_permission() {
        let questions = |tool: &str, input: Value| match ask(tool, input, json!([])).events.first() {
            Some(Event::Permission { questions, .. }) => questions.len(),
            other => panic!("{other:?}"),
        };
        let mut bad = asked();
        bad["questions"][0]["options"][0]["label"] = json!(7);
        let mut alike = asked();
        alike["questions"][1]["question"] = json!("Which colour?");
        let many = json!({"questions": vec![asked()["questions"][0].clone(); ask::MAX_QUESTIONS + 1]});
        let mut long = asked();
        long["questions"][0]["options"][0]["label"] = json!("a".repeat(MAX_DETAIL + 1));
        for input in [bad, alike, many, long, json!({"questions": []}), json!({"questions": "pick"})] {
            assert_eq!(questions("AskUserQuestion", input.clone()), 0, "{input}");
        }
        assert_eq!(questions("Write", asked()), 0);
        // No options: "Other…" alone takes the answer.
        assert_eq!(questions("AskUserQuestion", json!({"questions": [{"question": "Why?"}]})), 1);
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

    /// Each line of a fixture file read by one session, `_fixture` headers left out.
    fn replay(fixture: &str) -> Vec<Event> {
        let mut claude = Claude::new();
        claude.write(Input::Send { text: "go".into(), images: vec![] }).unwrap();
        fixture
            .lines()
            .filter(|line| !line.contains(r#""_fixture""#))
            .flat_map(|line| claude.read(line).events)
            .collect()
    }

    fn loud(events: &[Event]) -> Vec<&Event> {
        events.iter().filter(|e| !matches!(e, Event::Unknown { kind } if crate::is_quiet(kind))).collect()
    }

    /// The recorded 2.1.325 turn: hooks and status are quiet; the reply, once, as a delta then
    /// whole (the `assistant` comes before `content_block_stop`); the turn completes.
    #[test]
    fn a_recorded_turn_reads_whole() {
        let events = replay(include_str!("../tests/fixtures/claude/real-2.1.325-turn.jsonl"));
        let shown = loud(&events);
        let kinds: Vec<String> =
            shown.iter().map(|e| format!("{e:?}").split([' ', '{', '(']).next().unwrap_or("").to_string()).collect();
        assert_eq!(
            kinds,
            ["Started", "TurnStarted", "TextDelta", "Text", "Usage", "Usage", "TurnFinished"],
            "{shown:#?}"
        );
        let text: Vec<&str> =
            shown.iter().filter_map(|e| if let Event::Text { text } = e { Some(text.text()) } else { None }).collect();
        assert_eq!(text, ["ok"]);
        assert_eq!(events.last(), Some(&Event::TurnFinished { outcome: Outcome::Completed }));
        // A stdin closed before any message: hook lines alone, all quiet.
        assert!(loud(&replay(include_str!("../tests/fixtures/claude/real-2.1.325-no-reply.jsonl"))).is_empty());
    }

    /// The CLI's own failures are never silent: an API key's failure names its code, a retry and
    /// a failed turn say so, and a result or record no table has still says its kind.
    #[test]
    fn every_error_shape_is_shown() {
        let events = replay(include_str!("../tests/fixtures/claude/synthetic-auth-failure.jsonl"));
        let failed = Outcome::Failed { kind: "authentication_failed".into() };
        assert!(
            events.contains(&Event::Error { kind: "authentication_failed".into(), retrying: false }),
            "{events:#?}"
        );
        assert_eq!(events.last(), Some(&Event::TurnFinished { outcome: failed }));
        let events = replay(include_str!("../tests/fixtures/claude/synthetic-error-result.jsonl"));
        let outcomes: Vec<&Event> = events.iter().filter(|e| !matches!(e, Event::Usage { .. })).collect();
        let failed = |kind: &str| Event::TurnFinished { outcome: Outcome::Failed { kind: kind.into() } };
        assert_eq!(
            outcomes,
            [
                &Event::Error { kind: "overloaded".into(), retrying: true },
                &failed("error_during_execution"),
                &failed("error_made_up_kind"),
                &Event::Error { kind: "made_up_error".into(), retrying: false },
            ]
        );
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
        assert_eq!((*station, label.as_str()), (ottospace_core::activity::Station::Files, "b/c.rs"));
    }

    #[test]
    fn a_calls_json_input_is_pretty_printed() {
        let block =
            json!({"type": "tool_use", "id": "t", "name": "AskUserQuestion", "input": {"questions": [{"q": "a"}]}});
        let line = json!({"type": "assistant", "parent_tool_use_id": null, "message": {"content": [block]}});
        let out = Claude::new().read(&line.to_string());
        let Some(Event::ToolStart { detail, .. }) = out.events.get(1) else { panic!("{out:?}") };
        assert_eq!(detail.text(), "{\n  \"questions\": [\n    {\n      \"q\": \"a\"\n    }\n  ]\n}");
        // Too long pretty, it stays compact: the indentation cuts nothing that would have fit.
        let input = json!({"todos": vec!["x".repeat(30); (MAX_DETAIL - 200) / 33]});
        assert!(serde_json::to_string_pretty(&input).unwrap().len() > MAX_DETAIL);
        assert_eq!(Untrusted::json(&input, MAX_DETAIL).text(), input.to_string());
    }

    #[test]
    fn an_edit_call_carries_its_change() {
        let line = |name: &str, input: Value| {
            let block = json!({"type": "tool_use", "id": "t", "name": name, "input": input});
            json!({"type": "assistant", "parent_tool_use_id": null, "message": {"content": [block]}}).to_string()
        };
        let edit = line("Edit", json!({"file_path": "/a/b.rs", "old_string": "x", "new_string": "y"}));
        let out = Claude::new().read(&edit);
        let Some(Event::ToolStart { diff: Some(diff), .. }) = out.events.get(1) else { panic!("{out:?}") };
        assert_eq!((diff.files[0].added, diff.files[0].removed), (1, 1));
        let out = Claude::new().read(&line("Read", json!({"file_path": "/a/b.rs"})));
        assert!(matches!(out.events.get(1), Some(Event::ToolStart { diff: None, .. })), "{out:?}");
    }
    /// Rows for what the CLI says besides replies and calls: a subtask's start (by its call),
    /// a sub-agent's call under it, its end, a denial by settings, a notification and a model
    /// fallback. A permission check and a subtask's progress say nothing; nothing is unread.
    #[test]
    fn subtasks_denials_notes_and_fallbacks_are_events() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/claude/synthetic-2.1.326-rows.jsonl");
        let mut claude = Claude::new();
        let events: Vec<Event> = std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .skip(1)
            .flat_map(|line| claude.read(line).events)
            .filter(|event| !matches!(event, Event::TurnStarted))
            .filter(|event| !matches!(event, Event::Unknown { kind } if crate::is_quiet(kind)))
            .collect();
        let agent = opaque(&json!("toolu_agent"));
        let shown: Vec<String> = events
            .iter()
            .map(|event| match event {
                Event::ToolStart { tool, parent, gist, .. } => {
                    let line = gist.as_ref().map(|gist| gist.line(Some("/made/up"))).unwrap_or_default();
                    format!("call {tool} {line} under:{}", *parent == agent)
                }
                Event::Subtask { id, description } => format!("subtask {} {}", *id == agent, description.text()),
                Event::SubtaskEnded { status, summary } => format!("ended {status:?} {}", summary.text()),
                Event::DeniedBy { tool, by } => format!("denied {tool} {by:?}"),
                Event::Note { text } => format!("note {}", text.text()),
                Event::ModelFallback { from, to } => format!("fallback {from:?} {to}"),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(
            shown,
            [
                "call Agent Find the redirect under:false",
                "subtask true Find the redirect",
                "call Grep redirect in src under:true",
                "ended Completed Found it in auth.rs",
                "denied Bash Settings",
                "note Fast mode is off for now",
                r#"fallback Some("claude-opus-5-5") claude-sonnet-5"#,
            ]
        );
        // The user's own answer through this client is the card's notice: no second row.
        let ours = r#"{"type":"system","subtype":"permission_denied","tool_name":"Bash","decision_reason_type":"permissionPromptTool"}"#;
        assert_eq!(Claude::new().read(ours).events, []);
    }

    #[test]
    fn a_results_text_ends_its_call() {
        let line = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":[{"type":"text","text":"M src/a.rs"}],"is_error":false}]}}"#;
        let Some(Event::ToolEnd { output, failed: false, .. }) = Claude::new().read(line).events.pop() else {
            panic!()
        };
        assert_eq!(output.text(), "M src/a.rs");
    }
}
