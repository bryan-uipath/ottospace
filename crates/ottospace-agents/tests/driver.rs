//! The driver against fake `claude` and `codex` scripts (`tests/fakes`) that replay hand-written
//! fixtures and record their argv and stdin: a turn with a permission asked and allowed, then a
//! turn interrupted, then stdin closed. No real CLI runs.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use ottospace_agents::claude::{self, Claude};
use ottospace_agents::codex::{self, Codex};
use ottospace_agents::driver::Driver;
use ottospace_agents::{Answer, Argv, Event, Input, Outcome, Protocol, Runner, Start};

const CLAUDE_SESSION: &str = "8f14e45f-ceea-467f-a8d3-0242ac130003";
const CODEX_THREAD: &str = "019a2b3c-4d5e-7f60-8a9b-0c1d2e3f4a5b";

fn here() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// A fresh folder for what the fake records.
fn record_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ottospace-agents-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The fake in place of `argv.program`, with the table's arguments.
fn spawn(argv: &Argv, dir: &Path, protocol: impl Protocol) -> Driver {
    let mut command = Command::new(here().join("fakes").join(argv.program));
    command.args(&argv.args).current_dir(dir);
    command.env("FAKE_DIR", dir).env("FAKE_FIXTURES", here().join("fixtures").join(argv.program));
    Driver::spawn(command, protocol).unwrap()
}

/// Events up to and including the first that `stop` picks.
fn until(driver: &Driver, stop: impl Fn(&Event) -> bool) -> Vec<Event> {
    let mut seen = Vec::new();
    loop {
        let event = driver.events().recv_timeout(Duration::from_secs(10)).unwrap_or_else(|_| panic!("{seen:#?}"));
        let done = stop(&event);
        seen.push(event);
        if done {
            return seen;
        }
    }
}

fn texts(events: &[Event]) -> Vec<String> {
    let text = |event: &Event| match event {
        Event::TextDelta { text } => Some(format!("+{}", text.text())),
        Event::Text { text } => Some(format!("={}", text.text())),
        _ => None,
    };
    events.iter().filter_map(text).collect()
}

fn finished(outcome: Outcome) -> impl Fn(&Event) -> bool {
    move |event| *event == Event::TurnFinished { outcome: outcome.clone() }
}

/// One session's two turns: a permission round trip, then an interrupt, then stdin closed.
fn two_turns(driver: &mut Driver) -> (Vec<Event>, Vec<Event>) {
    driver.send(Input::Send { text: "Say hi".into(), images: vec![] }).unwrap();
    let mut first = until(driver, |event| matches!(event, Event::Permission { .. }));
    let Some(Event::Permission { ticket, .. }) = first.last().cloned() else { unreachable!() };
    driver.send(Input::Answer { ticket, answer: Answer::Allow }).unwrap();
    first.extend(until(driver, finished(Outcome::Completed)));

    driver.send(Input::Send { text: "Go on".into(), images: vec![] }).unwrap();
    let mut second = until(driver, |event| matches!(event, Event::TextDelta { .. }));
    driver.send(Input::Interrupt).unwrap();
    second.extend(until(driver, finished(Outcome::Interrupted)));
    driver.stop();
    second.extend(until(driver, |event| matches!(event, Event::Exited { .. })));
    (first, second)
}

fn lines(path: &Path) -> Vec<serde_json::Value> {
    let text = std::fs::read_to_string(path).unwrap();
    text.lines().map(|line| serde_json::from_str(line).unwrap()).collect()
}

#[test]
fn claude_asks_is_allowed_is_interrupted_and_exits() {
    let dir = record_dir("claude");
    let argv = claude::argv(Some("claude-opus-5-5"), &Start::New { id: CLAUDE_SESSION.into() }).unwrap();
    let mut driver = spawn(&argv, &dir, Claude::new());
    let (first, second) = two_turns(&mut driver);

    let started = Event::Started { session: Some(CLAUDE_SESSION.into()), model: Some("claude-opus-5-5".into()) };
    assert_eq!(first[0], started);
    let (call, permission) = (first.iter().find_map(tool_start), first.iter().find_map(permission));
    assert_eq!(call.as_deref(), Some("Bash git status"));
    assert_eq!(permission.as_deref(), Some("Bash git status: command: git status --short"));
    assert!(first.iter().any(|event| matches!(event, Event::ToolEnd { failed: false, .. })));
    assert_eq!(
        texts(&first),
        [
            "+Let me look ",
            "+at the **status**.",
            "=Let me look at the **status**.",
            "=One file changed: `src/main.rs`. See [the docs](https://example.com/docs)."
        ]
    );
    let usage = Event::Usage { context: None, window: Some(1_000_000), cost_micro_usd: Some(12_300) };
    assert!(first.contains(&usage), "{first:#?}");
    assert!(first.contains(&Event::Usage { context: Some(2322), window: None, cost_micro_usd: None }));
    let Some(Event::Exited { code, stderr }) = second.last() else { panic!() };
    assert_eq!((*code, stderr.text()), (Some(3), "fake claude: stdin closed\n"));

    // argv is the table's, and no prompt is in it.
    let recorded = std::fs::read_to_string(dir.join("argv")).unwrap();
    assert_eq!(recorded.lines().collect::<Vec<_>>(), argv.args);
    assert!(!recorded.contains("Say hi"));
    // stdin: the message, the answer to the CLI's own request id, the next message, the interrupt.
    let sent = lines(&dir.join("stdin"));
    assert_eq!(sent.len(), 4, "{sent:#?}");
    assert_eq!(sent[0]["message"]["content"][0]["text"], "Say hi");
    assert_eq!(sent[1]["response"]["request_id"], "req-fixture-1");
    assert_eq!(sent[1]["response"]["response"]["behavior"], "allow");
    assert_eq!(sent[1]["response"]["response"]["updatedInput"]["command"], "git status --short");
    assert_eq!(sent[3]["request"]["subtype"], "interrupt");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn codex_asks_is_allowed_is_interrupted_and_exits() {
    let dir = record_dir("codex");
    let codex = Codex::new(&dir, Some("gpt-6-astra"), &Start::New { id: String::new() }).unwrap();
    let mut driver = spawn(&codex::argv(), &dir, codex);
    let (first, second) = two_turns(&mut driver);

    let started = Event::Started { session: Some(CODEX_THREAD.into()), model: Some("gpt-6-astra".into()) };
    assert_eq!(first[0], started, "{first:#?}");
    assert_eq!(first.iter().find_map(tool_start).as_deref(), Some("exec cargo test"));
    assert_eq!(
        first.iter().find_map(permission).as_deref(),
        Some("exec cargo test: command: cargo test\ncwd: /work/app")
    );
    assert!(first.iter().any(|event| matches!(event, Event::PermissionClosed { .. })));
    assert!(first.iter().any(|event| matches!(event, Event::ToolEnd { failed: false, .. })));
    assert_eq!(texts(&first), ["+Checking ", "+the tests.", "=Checking the tests."]);
    assert!(first.contains(&Event::Usage { context: Some(12_300), window: Some(272_000), cost_micro_usd: None }));
    let Some(Event::Exited { code, stderr }) = second.last() else { panic!() };
    assert_eq!((*code, stderr.text()), (Some(4), "fake codex: stdin closed\n"));

    assert_eq!(std::fs::read_to_string(dir.join("argv")).unwrap(), "app-server\n");
    let sent = lines(&dir.join("stdin"));
    let methods: Vec<&str> = sent.iter().map(|line| line["method"].as_str().unwrap_or("(answer)")).collect();
    assert_eq!(
        methods,
        ["initialize", "initialized", "thread/start", "turn/start", "(answer)", "turn/start", "turn/interrupt"]
    );
    assert_eq!(sent[2]["params"]["cwd"], dir.to_str().unwrap());
    assert_eq!(sent[4], serde_json::json!({"id": "srv-fixture-1", "result": {"decision": "accept"}}));
    assert_eq!(sent[6]["params"], serde_json::json!({"threadId": CODEX_THREAD, "turnId": "turn-fixture-2"}));
    std::fs::remove_dir_all(dir).unwrap();
}

/// A CLI that is not there: spawning fails, nothing hangs.
#[test]
fn a_missing_cli_is_an_error() {
    let command = Command::new("/nonexistent/claude");
    assert!(Driver::spawn(command, Claude::new()).is_err());
}

/// What the CLI started (a background command holding its stdout) ends with it, whether the CLI
/// is killed or exits by itself, and `Exited` still comes.
#[test]
fn what_the_cli_started_ends_with_it() {
    for (name, script, killed) in [("kill", "exec cat", true), ("exit", "exit 5", false)] {
        let dir = record_dir(name);
        let mut command = Command::new("/bin/sh");
        let script = format!("sleep 1000 & echo $! >\"$FAKE_DIR/pid\"; {script}");
        command.args(["-c", &script]).env("FAKE_DIR", &dir);
        let mut driver = Driver::spawn(command, Claude::new()).unwrap();
        let pid_file = dir.join("pid");
        while std::fs::read_to_string(&pid_file).map_or(true, |pid| !pid.ends_with('\n')) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let pid = std::fs::read_to_string(&pid_file).unwrap().trim().to_string();
        if killed {
            driver.kill().unwrap();
        }
        let events = until(&driver, |event| matches!(event, Event::Exited { .. }));
        let Some(Event::Exited { code, .. }) = events.last() else { unreachable!() };
        assert_eq!(*code, if killed { None } else { Some(5) });
        let probe = Command::new("/bin/kill").args(["-0", &pid]).stderr(std::process::Stdio::null()).status();
        assert!(!probe.unwrap().success(), "{name}: the background command {pid} still runs");
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// A permission request too long to read cannot be answered: the session ends, not hangs.
#[test]
fn an_unreadable_request_ends_the_session() {
    let driver = Driver::spawn(Command::new(here().join("fakes/huge-request")), Claude::new()).unwrap();
    let events = until(&driver, |event| matches!(event, Event::Exited { .. }));
    let error = Event::Error { kind: "oversized_request".into(), retrying: false };
    assert!(events.contains(&error), "{events:#?}");
    assert!(matches!(events.last(), Some(Event::Exited { code: None, .. })), "killed: {events:#?}");
}

/// A CLI that does not read its stdin never blocks a send: writes queue on their own thread.
#[test]
fn a_send_never_waits_on_the_cli() {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "exec sleep 1000"]);
    let mut driver = Driver::spawn(command, Claude::new()).unwrap();
    let big = "x".repeat(1 << 20);
    for _ in 0..8 {
        driver.send(Input::Send { text: big.clone(), images: vec![] }).unwrap();
    }
    driver.kill().unwrap();
    until(&driver, |event| matches!(event, Event::Exited { .. }));
}

fn tool_start(event: &Event) -> Option<String> {
    match event {
        Event::ToolStart { tool, label, .. } => Some(format!("{tool} {label}")),
        _ => None,
    }
}

fn permission(event: &Event) -> Option<String> {
    match event {
        Event::Permission { tool, label, detail, .. } => Some(format!("{tool} {label}: {}", detail.text())),
        _ => None,
    }
}
