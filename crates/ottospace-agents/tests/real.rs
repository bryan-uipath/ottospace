//! Probes of the real CLIs, to settle what `docs/ai/agent-runners.md` marks UNVERIFIED. Each
//! starts one short model session on the user's account, so each is ignored and also needs
//! `OTTOSPACE_REAL_CLI=1`. Run only with the user's approval:
//!
//! `OTTOSPACE_REAL_CLI=1 cargo test -p ottospace-agents --test real -- --ignored --nocapture claude`
//!
//! Each runs in a new empty folder: a turn that asks to run one command (any permission asked is
//! allowed), then a turn interrupted at its first text, then stdin closed. It prints the kinds of
//! events seen, never their text.

use std::process::Command;
use std::time::Duration;

use ottospace_agents::claude::{self, Claude};
use ottospace_agents::codex::{self, Codex};
use ottospace_agents::driver::Driver;
use ottospace_agents::{Answer, Event, Input, Outcome, Runner, Start};

const ASK: &str =
    "Run the shell command `touch ottospace-probe` in the current directory, then reply with one word: done.";
const LONG: &str = "Count from 1 to 200, one number per line.";

fn allowed() -> bool {
    std::env::var("OTTOSPACE_REAL_CLI").is_ok_and(|value| value == "1")
}

fn empty_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ottospace-real-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The event's variant name: what is printed.
fn kind(event: &Event) -> String {
    let debug = format!("{event:?}");
    debug.split([' ', '{', '(']).next().unwrap_or("").to_string()
}

/// Events until `stop`, answering every permission with Allow. Prints each kind.
fn until(driver: &Driver, stop: impl Fn(&Event) -> bool) -> Vec<Event> {
    let mut seen = Vec::new();
    loop {
        let event = driver.events().recv_timeout(Duration::from_secs(180)).expect("an event within 3 minutes");
        println!("{}", kind(&event));
        if let Event::Permission { ticket, .. } = &event {
            driver.send(Input::Answer { ticket: ticket.clone(), answer: Answer::Allow }).unwrap();
        }
        let done = stop(&event) || matches!(event, Event::Exited { .. });
        seen.push(event);
        if done {
            return seen;
        }
    }
}

fn probe(mut driver: Driver) {
    driver.send(Input::Send { text: ASK.into(), images: vec![] }).unwrap();
    let first = until(&driver, |event| matches!(event, Event::TurnFinished { .. }));
    assert!(first.iter().any(|event| matches!(event, Event::Started { session: Some(_), .. })), "{first:#?}");
    assert!(first.iter().any(|event| matches!(event, Event::ToolStart { .. })), "{first:#?}");
    assert!(first.iter().any(|event| matches!(event, Event::Text { .. })), "{first:#?}");
    assert_eq!(first.last(), Some(&Event::TurnFinished { outcome: Outcome::Completed }));

    driver.send(Input::Send { text: LONG.into(), images: vec![] }).unwrap();
    until(&driver, |event| matches!(event, Event::TextDelta { .. }));
    driver.send(Input::Interrupt).unwrap();
    let second = until(&driver, |event| matches!(event, Event::TurnFinished { .. }));
    assert_eq!(second.last(), Some(&Event::TurnFinished { outcome: Outcome::Interrupted }));
    driver.stop();
    until(&driver, |_| false);
}

#[test]
#[ignore = "starts a real Claude Code session: OTTOSPACE_REAL_CLI=1"]
fn claude_real() {
    if !allowed() {
        return;
    }
    let dir = empty_dir("claude");
    // A new id each run: a session id can be used once.
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let session = format!("{:08x}-0000-4000-8000-{:012x}", std::process::id(), nanos & 0xffff_ffff_ffff);
    let argv = claude::argv(Some("haiku"), &Start::New { id: session }).unwrap();
    let mut command = Command::new(argv.program);
    command.args(&argv.args).current_dir(&dir);
    probe(Driver::spawn(command, Claude::new()).unwrap());
}

#[test]
#[ignore = "starts a real Codex session: OTTOSPACE_REAL_CLI=1"]
fn codex_real() {
    if !allowed() {
        return;
    }
    let dir = empty_dir("codex");
    let mut command = Command::new(codex::argv().program);
    command.args(&codex::argv().args).current_dir(&dir);
    probe(Driver::spawn(command, Codex::new(&dir, None, &Start::New { id: String::new() }).unwrap()).unwrap());
}
