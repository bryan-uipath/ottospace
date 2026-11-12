//! Client of the ottobot daemon: one JSON line out, one back, over its Unix socket. ottospace
//! starts, stops and configures nothing there. The wire shapes mirror ottobot's `proto.rs` at
//! `8bbc374`; what they mean for the Routines section is the core's (`ottospace_core::routines`).
//!
//! Trust: every reply is data. Paths, ids and text in it are checked or cleaned in the core
//! before they reach a shell, `open` or the screen. Design: `docs/ai/routines.md`.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use ottospace_core::routines::{self, Alert, Daemon, Level, Quota, Routine, Routines, RunLine, Status, Window};
use ottospace_core::settings::expand_home;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// How long the daemon has to answer one request. ottobot's own clients allow the same.
const DEADLINE: Duration = Duration::from_millis(1500);
/// Between snapshots: with the section open, and with it closed (for the rail's count).
const OPEN_EVERY: Duration = Duration::from_secs(2);
const CLOSED_EVERY: Duration = Duration::from_secs(15);
/// A snapshot holds 200 runs' metadata and the scheduler's log: well under this.
const MAX_REPLY: usize = 32 << 20;

/// The daemon's socket: `$OTTOBOT_SOCKET`, else `routines.socket` in the settings, else
/// `~/.ottobot/otto.sock`. `None` in a test run that names neither: a test never finds the
/// user's own daemon.
pub fn socket_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let named = std::env::var("OTTOBOT_SOCKET").ok().filter(|path| !path.is_empty());
    let set = named.or_else(|| crate::config::get().settings.routines.socket.clone());
    let test_run = std::env::var_os("OTTOSPACE_E2E").is_some();
    let path = set.or_else(|| (!test_run && !home.is_empty()).then(|| "~/.ottobot/otto.sock".to_string()))?;
    Some(PathBuf::from(expand_home(&path, &home)))
}

/// What ottospace asks. ottobot's `Request`, the part used here.
#[derive(Serialize)]
#[serde(tag = "type")]
enum Ask<'a> {
    Snapshot,
    RunNow { skill: &'a str },
    Events { run: &'a str, from: usize },
}

/// Why a request has no usable reply.
#[derive(Debug, PartialEq)]
enum Failure {
    /// Nothing listens, or nothing answered in time.
    Absent(String),
    /// An answer that is not a JSON object with a `type`.
    Unreadable(String),
}

/// Sends one request and reads one reply line: `(type, data)`. Never waits past `DEADLINE`: a
/// daemon that accepts and says nothing counts as absent.
fn request(path: &Path, ask: &Ask) -> Result<(String, Value), Failure> {
    let absent = |err: std::io::Error| Failure::Absent(err.to_string());
    let late = || Failure::Absent(format!("no answer in {} ms", DEADLINE.as_millis()));
    let start = Instant::now();
    let mut stream = UnixStream::connect(path).map_err(absent)?;
    stream.set_write_timeout(Some(DEADLINE)).map_err(absent)?;
    let line = serde_json::to_string(ask).map_err(|err| Failure::Unreadable(err.to_string()))?;
    stream.write_all(format!("{line}\n").as_bytes()).map_err(absent)?;
    let (mut reply, mut chunk) = (Vec::new(), vec![0; 65_536]);
    while !reply.contains(&b'\n') {
        // The time left, not a fresh allowance for each read: a trickle must not hold the thread.
        let left = DEADLINE.checked_sub(start.elapsed()).filter(|left| !left.is_zero()).ok_or_else(late)?;
        stream.set_read_timeout(Some(left)).map_err(absent)?;
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => reply.extend_from_slice(&chunk[..read]),
            Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => return Err(late()),
            Err(err) if err.kind() == ErrorKind::Interrupted => {}
            Err(err) => return Err(absent(err)),
        }
        if reply.len() > MAX_REPLY {
            return Err(Failure::Unreadable("the reply is too large".into()));
        }
    }
    let line = reply.split(|byte| *byte == b'\n').next().unwrap_or_default();
    if line.is_empty() {
        return Err(Failure::Absent("the daemon closed the connection".into()));
    }
    let mut reply: Value =
        serde_json::from_slice(line).map_err(|err| Failure::Unreadable(format!("not JSON: {err}")))?;
    match reply["type"].as_str().map(String::from) {
        Some(kind) => Ok((kind, reply["data"].take())),
        None => Err(Failure::Unreadable("the reply names no type".into())),
    }
}

/// A time on the wire (RFC 3339, with its offset), as milliseconds since 1970.
#[derive(Clone, Copy)]
struct Time(u64);

impl<'de> Deserialize<'de> for Time {
    fn deserialize<D: Deserializer<'de>>(from: D) -> Result<Self, D::Error> {
        let text = String::deserialize(from)?;
        let time = routines::parse_time(&text).map(Time);
        time.ok_or_else(|| serde::de::Error::custom(format!("`{text}` is not an RFC 3339 time")))
    }
}

/// For a field the section can do without: a value of another shape reads as the default,
/// e.g. an older daemon's `runner_usage`, or a verdict that is not one (unassessed).
fn lenient<'de, D: Deserializer<'de>, T: serde::de::DeserializeOwned + Default>(from: D) -> Result<T, D::Error> {
    Ok(serde_json::from_value(Value::deserialize(from)?).unwrap_or_default())
}

/// ottobot's `Snapshot`, the fields read here. Unknown fields are ignored, and a field an older
/// daemon lacks takes its default; `root`, `store_dir`, `skills` and `runs` must be there.
#[derive(Deserialize)]
struct Snapshot {
    root: String,
    store_dir: String,
    skills: Vec<Skill>,
    runs: Vec<Run>,
    #[serde(default)]
    skill_errors: Vec<(String, String)>,
    #[serde(default, deserialize_with = "lenient")]
    runner_health: Vec<(String, Result<String, String>)>,
    #[serde(default, deserialize_with = "lenient")]
    runner_usage: Vec<(String, Result<Vec<UsageWindow>, String>)>,
    #[serde(default)]
    daemon_pid: Option<u32>,
    #[serde(default)]
    alerts: Vec<AlertSnap>,
    #[serde(default)]
    commit: String,
    #[serde(default)]
    dismissed_failures: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Skill {
    name: String,
    #[serde(default)]
    description: Option<String>,
    trigger: Result<String, String>,
    #[serde(default)]
    next_fire: Option<Time>,
    #[serde(default)]
    last_fired: Option<Time>,
    #[serde(default)]
    active_run: Option<String>,
    #[serde(default)]
    note: String,
}

#[derive(Deserialize)]
enum RunStatus {
    Running,
    Succeeded,
    Failed(String),
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum LevelSnap {
    Normal,
    Critical,
    Immediate,
}

#[derive(Deserialize)]
struct Verdict {
    level: LevelSnap,
    #[serde(default, deserialize_with = "lenient")]
    headline: String,
}

#[derive(Deserialize)]
struct Run {
    id: String,
    skill: String,
    #[serde(default)]
    runner: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    effort: Option<String>,
    started_at: Time,
    #[serde(default)]
    finished_at: Option<Time>,
    status: RunStatus,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    report: Option<String>,
    /// Missing or not a verdict: unassessed.
    #[serde(default, deserialize_with = "lenient")]
    verdict: Option<Verdict>,
}

#[derive(Deserialize)]
struct AlertSnap {
    skill: String,
    key: String,
    level: LevelSnap,
    #[serde(default)]
    headline: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    report: Option<String>,
    #[serde(default)]
    run_id: String,
    first_seen: Time,
}

#[derive(Deserialize)]
struct UsageWindow {
    label: String,
    used_percent: f64,
}

fn level(level: LevelSnap) -> Level {
    match level {
        LevelSnap::Normal => Level::Normal,
        LevelSnap::Critical => Level::Critical,
        LevelSnap::Immediate => Level::Immediate,
    }
}

impl Snapshot {
    fn routines(self) -> Routines {
        let run = |run: Run| RunLine {
            agent: format!(
                "{}{}{}",
                run.runner,
                run.model.map(|model| format!("/{model}")).unwrap_or_default(),
                run.effort.map(|effort| format!("@{effort}")).unwrap_or_default()
            ),
            id: run.id,
            routine: run.skill,
            status: match run.status {
                RunStatus::Running => Status::Running,
                RunStatus::Succeeded => Status::Succeeded,
                RunStatus::Failed(why) => Status::Failed(why),
            },
            started: run.started_at.0,
            finished: run.finished_at.map(|at| at.0),
            level: run.verdict.as_ref().map(|verdict| level(verdict.level)),
            line: run.verdict.map(|verdict| verdict.headline).filter(|headline| !headline.is_empty()).or(run.summary),
            report: run.report,
        };
        // One line a runner: its health, then its quota windows.
        let mut quota: Vec<Quota> = Vec::new();
        let mut of = |runner: String| match quota.iter().position(|q| q.runner == runner) {
            Some(at) => at,
            None => {
                quota.push(Quota { runner, health: Ok(String::new()), windows: Ok(Vec::new()) });
                quota.len() - 1
            }
        };
        let health: Vec<(usize, _)> =
            self.runner_health.into_iter().map(|(runner, health)| (of(runner), health)).collect();
        let usage: Vec<(usize, _)> = self.runner_usage.into_iter().map(|(runner, usage)| (of(runner), usage)).collect();
        for (at, health) in health {
            quota[at].health = health;
        }
        for (at, usage) in usage {
            let left =
                |w: UsageWindow| Window { label: w.label, left_percent: (100. - w.used_percent).clamp(0., 100.) as u8 };
            quota[at].windows = usage.map(|windows| windows.into_iter().map(left).collect());
        }
        Routines {
            daemon: Daemon::Up { pid: self.daemon_pid, root: self.root, commit: self.commit },
            store_dir: self.store_dir,
            list: self.skills.into_iter().map(Skill::routine).collect(),
            runs: self.runs.into_iter().map(run).collect(),
            alerts: self.alerts.into_iter().map(AlertSnap::alert).collect(),
            quota,
            errors: self.skill_errors,
            dismissed_failures: self.dismissed_failures.into_iter().collect(),
        }
    }
}

impl Skill {
    fn routine(self) -> Routine {
        Routine {
            name: self.name,
            description: self.description,
            trigger: self.trigger,
            next_fire: self.next_fire.map(|at| at.0),
            last_fired: self.last_fired.map(|at| at.0),
            active_run: self.active_run,
            note: self.note,
        }
    }
}

impl AlertSnap {
    fn alert(self) -> Alert {
        Alert {
            routine: self.skill,
            key: self.key,
            level: level(self.level),
            headline: self.headline,
            summary: self.summary,
            report: self.report,
            run: self.run_id,
            first_seen: self.first_seen.0,
        }
    }
}

/// What a reply to `Snapshot` says: the routines, or why there are none to show.
fn read_snapshot(reply: Result<(String, Value), Failure>) -> Routines {
    let incompatible = |reason: String| Routines { daemon: Daemon::Incompatible { reason }, ..Routines::default() };
    match reply {
        Err(Failure::Absent(_)) => Routines::default(),
        Err(Failure::Unreadable(why)) => incompatible(why),
        Ok((kind, data)) if kind == "Snapshot" => match serde_json::from_value::<Snapshot>(data) {
            Ok(snapshot) => snapshot.routines(),
            Err(err) => incompatible(format!("its snapshot is not one this build reads: {err}")),
        },
        Ok((kind, data)) if kind == "Error" => {
            incompatible(format!("it refused a snapshot: {}", data.as_str().unwrap_or("?")))
        }
        Ok((kind, _)) => incompatible(format!("it answered a snapshot request with `{kind}`")),
    }
}

fn snapshot(path: &Path) -> Routines {
    read_snapshot(request(path, &Ask::Snapshot))
}

/// Asks the daemon to run a routine now, once. A request that got no answer may have arrived:
/// it is never sent again.
fn run_now(path: &Path, name: &str) -> Result<(), String> {
    match request(path, &Ask::RunNow { skill: name }) {
        Ok((kind, _)) if kind == "Ok" => Ok(()),
        Ok((kind, data)) if kind == "Error" => Err(data.as_str().unwrap_or("the daemon refused").to_string()),
        Ok((kind, _)) => Err(format!("the daemon answered `{kind}`")),
        Err(Failure::Absent(why) | Failure::Unreadable(why)) => {
            Err(format!("no usable answer ({why}); not sent again"))
        }
    }
}

/// What the poll thread reports.
pub enum Event {
    Snapshot(Routines),
    /// The daemon's answer to a run request. `None`: it took it.
    Ran {
        name: String,
        error: Option<String>,
    },
}

enum Order {
    /// The section opened or closed.
    Watch(bool),
    /// A snapshot now, not at the next turn.
    Poll,
    Run(String),
}

/// The thread that talks to the daemon, so a slow or silent one never holds up the window.
pub struct Poller {
    orders: Sender<Order>,
    pub events: Receiver<Event>,
}

impl Poller {
    pub fn start(path: PathBuf) -> Self {
        let (orders, inbox) = channel();
        let (outbox, events) = channel();
        thread::spawn(move || {
            let (mut open, mut last) = (false, None::<Instant>);
            loop {
                let every = if open { OPEN_EVERY } else { CLOSED_EVERY };
                let wait = last.map_or(Duration::ZERO, |at| every.saturating_sub(at.elapsed()));
                match inbox.recv_timeout(wait) {
                    Ok(Order::Watch(now)) => {
                        open = now;
                        continue;
                    }
                    // Not more than twice a second, however often a socket client asks.
                    Ok(Order::Poll) if last.is_some_and(|at| at.elapsed() < Duration::from_millis(500)) => continue,
                    Ok(Order::Poll) | Err(RecvTimeoutError::Timeout) => {}
                    Ok(Order::Run(name)) => {
                        let error = run_now(&path, &name).err();
                        if outbox.send(Event::Ran { name, error }).is_err() {
                            return;
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                }
                last = Some(Instant::now());
                if outbox.send(Event::Snapshot(snapshot(&path))).is_err() {
                    return;
                }
            }
        });
        Self { orders, events }
    }

    pub fn watch(&self, open: bool) {
        let _ = self.orders.send(Order::Watch(open));
    }

    pub fn poll(&self) {
        let _ = self.orders.send(Order::Poll);
    }

    pub fn run(&self, name: String) {
        let _ = self.orders.send(Order::Run(name));
    }
}

/// One event of a run (ottobot's `RunEvent`, e.g. `{"Text":"hi"}`) as lines to print, and
/// whether it ends the run. An event of a kind not known here prints nothing.
fn event_lines(event: &Value) -> (Vec<String>, bool) {
    let text = |value: &Value| value.as_str().unwrap_or_default().to_string();
    let Some((kind, body)) = event.as_object().and_then(|object| object.iter().next()) else {
        return (Vec::new(), false);
    };
    let shown = match kind.as_str() {
        "Init" => format!("session {} · {}", text(&body["session"]), body["model"].as_str().unwrap_or("default model")),
        "Text" | "Raw" => text(body),
        "ToolUse" => format!("▸ {} {}", text(&body["name"]), text(&body["summary"])),
        "ToolResult" if body["is_error"] == true => format!("  ✗ {}", text(&body["summary"])),
        "ToolResult" => format!("  ← {}", text(&body["summary"])),
        "Stderr" => format!("stderr: {}", text(body)),
        "Done" => format!("{} · {}", if body["ok"] == true { "done" } else { "failed" }, text(&body["summary"])),
        _ => String::new(),
    };
    (routines::output_lines(&shown), kind == "Done")
}

/// `ottospace routine-tail <run>`: prints a run's output as it comes, until the run ends. A
/// viewer: it reads `Events`, sends the run nothing, and masks what looks like a credential.
pub fn tail(args: &[String]) -> ! {
    const USAGE: &str = "usage: ottospace routine-tail <run>\n\
        Prints the output of an ottobot run (an id from the Routines section) until it ends.";
    fn fail(code: i32, message: String) -> ! {
        eprintln!("{message}");
        std::process::exit(code);
    }
    let Some(run) = args.first().filter(|run| routines::safe_id(run)) else { fail(2, USAGE.into()) };
    let Some(path) = socket_path() else { fail(1, "no ottobot socket: set OTTOBOT_SOCKET".into()) };
    // An id the daemon does not know answers with no events, for ever: look it up first.
    let status = |routines: &Routines| {
        routines.runs.iter().find(|r| r.id == *run).map(|r| (r.status.clone(), r.routine.clone(), r.agent.clone()))
    };
    let known = snapshot(&path);
    match (&known.daemon, status(&known)) {
        (Daemon::Up { .. }, Some((_, routine, agent))) => {
            println!("{}", routines::plain(&format!("{routine} · {run} · {agent}")));
        }
        (Daemon::Up { .. }, None) => fail(1, format!("no run {run} among the daemon's last runs")),
        (Daemon::Absent, _) => fail(1, format!("ottobot daemon not running at {}", path.display())),
        (Daemon::Incompatible { reason }, _) => {
            fail(1, format!("incompatible ottobot daemon: {}", routines::plain(reason)))
        }
    }
    let (mut from, mut silent, mut misses) = (0, 0u32, 0u32);
    loop {
        match request(&path, &Ask::Events { run, from }) {
            Ok((kind, Value::Array(events))) if kind == "Events" => {
                misses = 0;
                silent = if events.is_empty() { silent + 1 } else { 0 };
                from += events.len();
                for event in &events {
                    let (lines, done) = event_lines(event);
                    lines.iter().for_each(|line| println!("{line}"));
                    if done {
                        std::process::exit(0);
                    }
                }
                // Quiet for a while: the run may have ended with no last event (a daemon restart).
                if silent > 0 && silent % 6 == 0 {
                    match status(&snapshot(&path)) {
                        Some((Status::Running, ..)) => {}
                        Some((Status::Succeeded, ..)) => fail(0, "the run ended: succeeded".into()),
                        Some((Status::Failed(why), ..)) => {
                            fail(0, format!("the run ended: failed: {}", routines::plain(&why)))
                        }
                        None => fail(1, "the daemon no longer lists this run".into()),
                    }
                }
            }
            Ok((kind, _)) => fail(1, format!("the daemon answered `{}`, not events", routines::plain(&kind))),
            // About ten seconds of no daemon.
            Err(_) if misses >= 20 => fail(1, "ottobot daemon gone".into()),
            Err(_) => misses += 1,
        }
        thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;

    use ottospace_core::routines::{Group, Tone};

    use super::*;

    /// A snapshot as ottobot `8bbc374` writes it, with made-up routines.
    const FIXTURE: &str = include_str!("../tests/fixtures/ottobot-snapshot.json");
    /// 2026-11-12T18:00:00Z, a little after the fixture's last run.
    const NOW: u64 = 1_794_506_400_000;

    fn reply(text: &str) -> Result<(String, Value), Failure> {
        let mut reply: Value = serde_json::from_str(text).expect("fixture is JSON");
        Ok((reply["type"].as_str().expect("type").to_string(), reply["data"].take()))
    }

    #[test]
    fn a_snapshot_from_ottobot_is_read() {
        let routines = read_snapshot(reply(FIXTURE));
        let daemon = Daemon::Up { pid: Some(4242), root: "/Users/me/.ottobot/skills".into(), commit: "8bbc374".into() };
        assert_eq!(routines.daemon, daemon);
        assert_eq!(routines.store_dir, "/Users/me/.ottobot/projects/-Users-me--ottobot-skills");
        let names: Vec<&str> = routines.list.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["morning-brief", "pr-watch", "disk-space", "broken-trigger"]);
        assert_eq!(routines.list[0].trigger.as_deref(), Ok("cron 0 8 * * 1-5"));
        assert!(routines.list[3].trigger.is_err());
        // A local time with its offset: 09:45 at -08:00 is 17:45 UTC.
        assert_eq!(routines.runs[0].started, NOW - 15 * 60_000);
        assert_eq!(
            (routines.runs[0].status.clone(), routines.runs[0].agent.as_str()),
            (Status::Running, "codex/gpt-6-astra@medium")
        );
        // A verdict gives the level and the line; a run without one is unassessed.
        assert_eq!(
            (routines.runs[1].level, routines.runs[1].line.as_deref()),
            (Some(Level::Normal), Some("Nothing urgent overnight"))
        );
        assert_eq!((routines.runs[3].level, routines.runs[3].line.as_deref()), (None, Some("Brief written, 3 items")));
        assert_eq!(routines.runs[4].status, Status::Failed("timeout after 1h".into()));
        assert_eq!(
            (routines.alerts.len(), routines.alerts[0].level, routines.alerts[0].run.as_str()),
            (1, Level::Critical, "20261112-080500-disk-space")
        );
        assert_eq!(
            routines.errors,
            [("/Users/me/.ottobot/skills/draft.md".to_string(), "no trigger in frontmatter".to_string())]
        );
        assert_eq!(routines.quota_lines()[0], ("claude · 5h 63% left · 7d 93% left".to_string(), Tone::Plain));
        assert_eq!(routines.quota_lines()[1].0, "codex · not usable: codex not found on PATH");
        // What the section lists.
        let rows: Vec<(Group, String)> = routines.rows(NOW).into_iter().map(|row| (row.group, row.title)).collect();
        let row = |group, title: &str| (group, title.to_string());
        assert_eq!(
            rows,
            [
                row(Group::Alerts, "disk-space"),
                row(Group::Now, "pr-watch"),
                row(Group::Attention, "flaky-sync"),
                row(Group::Attention, "draft.md"),
                row(Group::Next, "pr-watch"),
                row(Group::Next, "disk-space"),
                row(Group::Next, "morning-brief"),
                row(Group::Next, "broken-trigger"),
            ]
        );
    }

    #[test]
    fn a_newer_or_an_older_daemon_is_read_through_the_defaults() {
        // Newer: fields this build does not know, at every level.
        let mut newer: Value = serde_json::from_str(FIXTURE).unwrap();
        newer["data"]["protocol"] = serde_json::json!(3);
        newer["data"]["skills"][0]["permission_mode"] = serde_json::json!("plan");
        newer["data"]["runs"][0]["executor"] = serde_json::json!({ "pane": 4 });
        newer["data"]["alerts"][0]["muted"] = serde_json::json!(false);
        assert_eq!(read_snapshot(reply(&newer.to_string())), read_snapshot(reply(FIXTURE)));
        // Older: no alerts, commit or dismissed failures; quota as one summary string a runner.
        let mut older: Value = serde_json::from_str(FIXTURE).unwrap();
        let data = older["data"].as_object_mut().unwrap();
        for key in ["alerts", "commit", "dismissed_failures"] {
            data.remove(key);
        }
        data.insert("runner_usage".into(), serde_json::json!([["claude", { "Ok": "5h 3%" }]]));
        let read = read_snapshot(reply(&older.to_string()));
        assert_eq!(
            read.daemon,
            Daemon::Up { pid: Some(4242), root: "/Users/me/.ottobot/skills".into(), commit: String::new() }
        );
        assert_eq!((read.alerts.len(), read.list.len(), read.runs.len()), (0, 4, 5));
        assert_eq!(read.quota_lines()[0].0, "claude · no usage data");
        // A verdict that is not one is no verdict.
        let mut odd: Value = serde_json::from_str(FIXTURE).unwrap();
        odd["data"]["runs"][1]["verdict"] = serde_json::json!({ "level": "fine" });
        assert_eq!(read_snapshot(reply(&odd.to_string())).runs[1].level, None);
    }

    #[test]
    fn a_reply_that_is_no_snapshot_is_incompatible_not_a_crash() {
        let reason = |routines: Routines| match routines.daemon {
            Daemon::Incompatible { reason } => reason,
            other => panic!("{other:?}"),
        };
        assert!(reason(read_snapshot(reply(r#"{"type":"Ok"}"#))).contains("`Ok`"));
        assert!(
            reason(read_snapshot(reply(r#"{"type":"Error","data":"scheduler stopped"}"#)))
                .contains("scheduler stopped")
        );
        assert!(reason(read_snapshot(reply(r#"{"type":"Snapshot","data":{"skills":[]}}"#))).contains("root"));
        assert!(reason(read_snapshot(reply(r#"{"type":"Snapshot","data":[1]}"#))).contains("snapshot"));
        // A status or a time this build does not know: said, not guessed.
        let mut odd: Value = serde_json::from_str(FIXTURE).unwrap();
        odd["data"]["runs"][0]["status"] = serde_json::json!("Paused");
        assert!(reason(read_snapshot(reply(&odd.to_string()))).contains("Paused"));
        odd["data"]["runs"][0]["status"] = serde_json::json!("Running");
        odd["data"]["runs"][0]["started_at"] = serde_json::json!("this morning");
        assert!(reason(read_snapshot(reply(&odd.to_string()))).contains("RFC 3339"));
        assert!(reason(read_snapshot(Err(Failure::Unreadable("not JSON: x".into())))).contains("not JSON"));
        assert_eq!(read_snapshot(Err(Failure::Absent("refused".into()))).daemon, Daemon::Absent);
    }

    /// A daemon for one test: answers each connection's first line with `answer(line)`, or says
    /// nothing for `None`. Lives in the per-user temp dir under a short name (a socket path is
    /// at most 104 bytes).
    fn daemon(name: &str, answer: fn(&str) -> Option<String>) -> PathBuf {
        let path = std::env::temp_dir().join(format!("otto-ut-{name}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind");
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                thread::spawn(move || {
                    let mut line = String::new();
                    let _ = BufReader::new(&stream).read_line(&mut line);
                    match answer(line.trim_end()) {
                        Some(reply) => drop((&stream).write_all(format!("{reply}\n").as_bytes())),
                        None => thread::sleep(Duration::from_secs(10)),
                    }
                });
            }
        });
        path
    }

    #[test]
    fn requests_go_out_as_ottobot_reads_them_and_a_silent_daemon_is_absent() {
        // Echoes the request back as an error, so the test sees the line as it was sent.
        let echo = daemon("echo", |line| Some(serde_json::json!({ "type": "Error", "data": line }).to_string()));
        assert_eq!(run_now(&echo, "morning-brief").unwrap_err(), r#"{"type":"RunNow","skill":"morning-brief"}"#);
        let events = request(&echo, &Ask::Events { run: "r1", from: 3 }).unwrap().1;
        assert_eq!(events, r#"{"type":"Events","run":"r1","from":3}"#);
        assert_eq!(request(&echo, &Ask::Snapshot).unwrap().1, r#"{"type":"Snapshot"}"#);
        let ok = daemon("ok", |_| Some(r#"{"type":"Ok"}"#.into()));
        assert_eq!(run_now(&ok, "morning-brief"), Ok(()));

        // Accepts and never answers: absent within the deadline, and the run request says so.
        let silent = daemon("silent", |_| None);
        let start = Instant::now();
        assert_eq!(snapshot(&silent).daemon, Daemon::Absent);
        assert!(start.elapsed() < DEADLINE + Duration::from_secs(1), "{:?}", start.elapsed());
        assert!(run_now(&silent, "morning-brief").unwrap_err().contains("not sent again"));

        // Nothing there, a reply that is not JSON, a connection closed with no reply.
        let nowhere = std::env::temp_dir().join("otto-ut-nowhere.sock");
        assert!(matches!(request(&nowhere, &Ask::Snapshot), Err(Failure::Absent(_))));
        let noise = daemon("noise", |_| Some("hello".into()));
        assert!(matches!(snapshot(&noise).daemon, Daemon::Incompatible { .. }));
        let closes = daemon("closes", |_| Some(String::new()));
        assert!(matches!(snapshot(&closes).daemon, Daemon::Incompatible { .. } | Daemon::Absent));
        for path in [echo, ok, silent, noise, closes] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn a_runs_events_print_as_plain_lines() {
        let lines = |event: Value| event_lines(&event);
        assert_eq!(
            lines(serde_json::json!({ "Text": "one\ntwo" })),
            (vec!["one".to_string(), "two".to_string()], false)
        );
        let tool = serde_json::json!({ "ToolUse": { "name": "Bash", "summary": "gh pr list" } });
        assert_eq!(lines(tool).0, ["▸ Bash gh pr list"]);
        let failed = serde_json::json!({ "ToolResult": { "summary": "exit 1", "is_error": true } });
        assert_eq!(lines(failed).0, ["  ✗ exit 1"]);
        // Raw stderr: a credential is masked, an escape sequence loses its escape.
        let stderr =
            serde_json::json!({ "Stderr": "curl -H 'Authorization: Bearer abcdefgh12345678' \u{1b}[31mfailed" });
        assert_eq!(lines(stderr).0, ["stderr: curl -H 'Authorization: Bearer <redacted> [31mfailed"]);
        assert_eq!(
            lines(serde_json::json!({ "Usage": { "input": 1, "output": 2, "cost_usd": null } })).0,
            [] as [&str; 0]
        );
        assert_eq!(lines(serde_json::json!({ "Later": { "x": 1 } })), (vec![], false));
        assert_eq!(lines(serde_json::json!("text")), (vec![], false));
        let done = serde_json::json!({ "Done": { "ok": true, "summary": "All quiet" } });
        assert_eq!(lines(done), (vec!["done · All quiet".to_string()], true));
    }
}
