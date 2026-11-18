//! The recorded `AskUserQuestion` turn (`fixtures/claude/real-2.1.327-ask.jsonl`): its request
//! is a question card, and the answers sent back are the ones the CLI said it read.

use ottospace_agents::claude::Claude;
use ottospace_agents::{Answer, Event, Input, Wire};
use ottospace_core::ask::Chosen;
use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/claude/real-2.1.327-ask.jsonl");

fn records() -> Vec<Value> {
    let lines = FIXTURE.lines().filter(|line| !line.contains(r#""_fixture""#));
    lines.map(|line| serde_json::from_str(line).unwrap()).collect()
}

#[test]
fn the_recorded_question_is_answered_as_the_cli_read_it() {
    let mut claude = Claude::new();
    claude.write(Input::Send { text: "ask me".into(), images: vec![] }).unwrap();
    let events: Vec<Event> = FIXTURE.lines().flat_map(|line| claude.read(line).events).collect();
    let card = events.iter().find_map(|event| match event {
        Event::Permission { ticket, questions, .. } if !questions.is_empty() => Some((ticket.clone(), questions)),
        _ => None,
    });
    let Some((ticket, questions)) = card else { panic!("no question card: {events:?}") };
    let shown: Vec<(&str, usize, bool)> =
        questions.iter().map(|q| (q.question.text(), q.options.len(), q.multi)).collect();
    assert_eq!(shown, [("Which colour?", 2, false), ("Which sizes?", 2, true)]);

    // Blue; Small and "Large, wide": what the recording answered.
    let chosen = vec![Chosen { options: vec![1], other: None }, Chosen { options: vec![0, 1], other: None }];
    let lines = claude.write(Input::Answer { ticket, answer: Answer::Questions(chosen) }).unwrap();
    let sent: Value = serde_json::from_str(&lines[0]).unwrap();
    let records = records();
    let asked = records.iter().find(|r| r["type"] == "control_request").unwrap();
    assert_eq!(sent["response"]["request_id"], asked["request_id"]);
    let reply = &sent["response"]["response"];
    assert_eq!(reply["behavior"], "allow");
    // The input as it came, and the answers.
    let mut input = reply["updatedInput"].clone();
    let answers = input.as_object_mut().and_then(|input| input.remove("answers")).unwrap();
    assert_eq!(input, asked["request"]["input"]);

    // The CLI's own tool result for them, as it writes it: `"question"="answer"`, in order.
    let said: Vec<String> = questions
        .iter()
        .map(|q| format!("\"{}\"=\"{}\"", q.question.text(), answers[q.question.text()].as_str().unwrap()))
        .collect();
    let result = records.iter().find(|r| r["type"] == "user").unwrap();
    let content = result["message"]["content"][0]["content"].as_str().unwrap();
    assert_eq!(
        content,
        format!(
            "Your questions have been answered: {}. You can now continue with these answers in mind.",
            said.join(", ")
        )
    );
}
