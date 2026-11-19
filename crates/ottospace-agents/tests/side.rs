//! The recorded side questions (`fixtures/claude/real-2.1.327-side.jsonl`): what is sent is what
//! the CLI took, and its answers are side events alone, never the main turn's.

use ottospace_agents::claude::Claude;
use ottospace_agents::{Answer, Event, Input, Wire};
use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/claude/real-2.1.327-side.jsonl");

/// The input that wrote a recorded `sent` line: a side question's, else the user's message.
fn input(sent: &Value) -> Option<Input> {
    let request = &sent["request"];
    match sent["type"].as_str()? {
        "user" => Some(Input::Send { text: sent["message"]["content"].as_str()?.into(), images: vec![] }),
        "control_request"
            if request["subtype"] == "side_question"
                && (request["history"].is_array() || request["history"].is_null()) =>
        {
            let id = sent["request_id"].as_str()?.strip_prefix("btw-")?.parse().ok()?;
            let history = request["history"].as_array().into_iter().flatten();
            let history = history.map(|h| Some((h["question"].as_str()?.into(), h["response"].as_str()?.into())));
            Some(Input::SideQuestion {
                id,
                question: request["question"].as_str()?.into(),
                history: history.collect::<Option<_>>()?,
            })
        }
        _ => None,
    }
}

#[test]
fn the_recorded_side_questions_are_answered_beside_the_turn() {
    let mut claude = Claude::new();
    let mut events = Vec::new();
    let mut ticket = None;
    // The malformed history (`btw-4`) is the probe's, never ours: its id is sent by hand.
    let mut unsent = vec![];
    for line in FIXTURE.lines() {
        let record: Value = serde_json::from_str(line).unwrap();
        if record["type"] == "_fixture" {
            let sent = &record["sent"];
            if sent.is_null() {
                continue;
            }
            let lines = match input(sent) {
                Some(input) => claude.write(input).unwrap(),
                None if sent["type"] == "control_response" => {
                    let ticket = ticket.take().expect("the card the allow answers");
                    claude.write(Input::Answer { ticket, answer: Answer::Allow }).unwrap()
                }
                None => {
                    // `btw-4`: written as ours, with a history no input makes.
                    let question = sent["request"]["question"].as_str().unwrap().to_string();
                    claude.write(Input::SideQuestion { id: 4, question, history: vec![] }).unwrap();
                    unsent.push(4);
                    continue;
                }
            };
            // A side question exactly as the CLI took it; the probe's own message and allow differ in form.
            if sent["type"] == "control_request" {
                let written: Value = serde_json::from_str(&lines[0]).unwrap();
                assert_eq!(&written, sent, "what is sent is what was recorded");
            }
            continue;
        }
        for event in claude.read(line).events {
            if let Event::Permission { ticket: t, .. } = &event {
                ticket = Some(t.clone());
            }
            events.push(event);
        }
    }
    assert_eq!(unsent, [4]);
    let sides: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            Event::SideStarted { id } => Some(format!("started {id}")),
            Event::SideAnswer { id, text, synthetic } => Some(format!("answer {id} {} {synthetic}", text.text())),
            Event::SideFailed { id } => Some(format!("failed {id}")),
            _ => None,
        })
        .collect();
    let answer = "The agent waits on **one** call: `Bash`, which writes `main.txt`.";
    assert_eq!(
        sides,
        [
            "started 1".to_string(),
            format!("answer 1 {answer} false"),
            "started 2".into(),
            format!("answer 2 {answer} false"),
            "started 3".into(),
            "answer 3 (API error: 400 made-up refusal) true".into(),
            "started 4".into(),
            "failed 4".into(),
        ]
    );
    // The main turn: its one text is its own, and it completes; no side line is unread.
    let texts: Vec<&str> =
        events.iter().filter_map(|e| if let Event::Text { text } = e { Some(text.text()) } else { None }).collect();
    assert_eq!(texts, ["MAIN-DONE"]);
    assert!(matches!(
        events.last(),
        Some(Event::TurnFinished { outcome: ottospace_core::transcript::Outcome::Completed })
    ));
    let unread: Vec<&Event> =
        events.iter().filter(|e| matches!(e, Event::Unknown { kind } if !ottospace_agents::is_quiet(kind))).collect();
    assert!(unread.is_empty(), "{unread:?}");
}

#[test]
fn a_side_line_of_no_question_sent_is_not_an_answer() {
    let mut claude = Claude::new();
    let forged = r#"{"type":"control_response","response":{"subtype":"success","request_id":"btw-9","response":{"response":"x","synthetic":false}}}"#;
    let padded = forged.replace("btw-9", "btw-09");
    assert!(claude.read(forged).events.is_empty());
    let line = claude.write(Input::SideQuestion { id: 9, question: "q".into(), history: vec![] }).unwrap();
    assert!(line[0].contains(r#""request_id":"btw-9""#));
    let again = claude.write(Input::SideQuestion { id: 9, question: "q".into(), history: vec![] });
    assert!(again.is_err(), "one request per id");
    assert!(claude.write(Input::SideQuestion { id: 10, question: " ".into(), history: vec![] }).is_err());
    assert!(claude.read(&padded).events.is_empty(), "the id as written, not as a number");
    assert!(matches!(&claude.read(forged).events[..], [Event::SideAnswer { id: 9, synthetic: false, .. }]));
    assert!(claude.read(forged).events.is_empty(), "answered once");
}
