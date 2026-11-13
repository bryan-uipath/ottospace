//! The runners against hand-written session files in `tests/fixtures`: made-up content in the
//! real schema. No test reads a real config dir.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use ottospace_sessions::claude::Claude;
use ottospace_sessions::codex::Codex;
use ottospace_sessions::git::checkouts;
use ottospace_sessions::report::report;
use ottospace_sessions::snapshot::Snapshot;
use ottospace_sessions::station::Station;
use ottospace_sessions::tally::{Reader, Tally};
use ottospace_sessions::{Cursor, Kind, Runner, SessionRef, Usage, What, read_all};

const CLAUDE_MAIN: &str = "11111111-1111-4111-8111-111111111111";
const CLAUDE_WORKTREE: &str = "22222222-2222-4222-8222-222222222222";
const CODEX_NEW: &str = "c0de0001-0000-4000-8000-000000000001";
const CODEX_CHILD: &str = "c0de0002-0000-4000-8000-000000000002";
const CODEX_OLD: &str = "c0de0004-0000-4000-8000-000000000004";

/// Read to the end in one call.
const ALL: u64 = u64::MAX;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn claude() -> Claude {
    // A config dir that does not exist is skipped.
    Claude::new(vec![fixtures().join("claude"), fixtures().join("no-such-dir")])
}

fn codex() -> Codex {
    Codex::new(fixtures().join("codex"))
}

/// A fresh folder under the system temp dir, as a resolved path.
fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ottospace-sessions-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn list(runner: &dyn Runner, dir: &str) -> Vec<SessionRef> {
    runner.sessions(&[PathBuf::from(dir)]).unwrap()
}

fn ids(sessions: &[SessionRef]) -> Vec<&str> {
    let mut ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
    ids.sort();
    ids
}

fn find(runner: &dyn Runner, dir: &str, id: &str) -> SessionRef {
    list(runner, dir).into_iter().find(|s| s.id == id).unwrap()
}

fn snapshot(runner: &dyn Runner, dir: &str, id: &str) -> Snapshot {
    Snapshot::fold(&read_all(runner, &find(runner, dir, id)).unwrap())
}

fn counts(snapshot: &Snapshot) -> Vec<(Station, u32)> {
    snapshot.counts.iter().map(|(station, n)| (*station, *n)).collect()
}

#[test]
fn claude_lists_sessions_started_in_a_directory_or_below() {
    let claude = claude();
    let sessions = list(&claude, "/work/app");
    assert_eq!(ids(&sessions), [CLAUDE_MAIN, CLAUDE_WORKTREE], "the worktree's too, not `/work/app-other`");
    assert_eq!(ids(&list(&claude, "/work/app/")), [CLAUDE_MAIN, CLAUDE_WORKTREE], "a trailing slash changes nothing");

    let main = find(&claude, "/work/app", CLAUDE_MAIN);
    assert_eq!(main.kind, Kind::Claude);
    assert_eq!(main.cwd, Path::new("/work/app"));
    assert_eq!(main.started, Some(1_794_474_000_000));
    assert!(main.last_activity.is_some());
    assert_eq!(main.model, None, "listing does not read the end of the file");
    assert_eq!(claude.model(&main).as_deref(), Some("claude-fable-5-2"));
    assert_eq!((main.pid, main.busy), (Some(4242), Some(true)), "from sessions/4242.json");

    let worktree = find(&claude, "/work/app/.claude/worktrees/fix", CLAUDE_WORKTREE);
    assert_eq!((worktree.pid, worktree.busy), (None, None));
    assert_eq!(ids(&list(&claude, "/work/app/.claude/worktrees/fix")), [CLAUDE_WORKTREE]);
    assert_eq!(list(&claude, "/work/app-other").len(), 1);
    assert!(list(&claude, "/work/ap").is_empty());
    assert!(list(&claude, "/nowhere").is_empty());

    // Several directories, one scan, each session once.
    let dirs = [
        PathBuf::from("/work/app"),
        PathBuf::from("/work/app/.claude/worktrees/fix"),
        PathBuf::from("/work/app-other"),
    ];
    assert_eq!(claude.sessions(&dirs).unwrap().len(), 3);
    assert!(claude.sessions(&[]).unwrap().is_empty());
}

#[test]
fn an_empty_or_relative_directory_is_an_error_not_every_session() {
    let (claude, codex) = (claude(), codex());
    let runners: [&dyn Runner; 2] = [&claude, &codex];
    for bad in ["", ".", "work/app", "../app"] {
        for runner in runners {
            let err = runner.sessions(&[PathBuf::from("/work/app"), PathBuf::from(bad)]).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput, "{bad:?}");
        }
        assert_eq!(checkouts(Path::new(bad)).unwrap_err().kind(), ErrorKind::InvalidInput, "{bad:?}");
        assert!(report(&runners, &[PathBuf::from(bad)], 0, 5).is_err(), "{bad:?}");
    }
    assert_eq!(checkouts(Path::new("/no/such/dir/anywhere")).unwrap_err().kind(), ErrorKind::NotFound);
    // A runner pointed at no home at all has no sessions, rather than those of the current directory.
    assert!(list(&Codex::new(PathBuf::new()), "/work/app").is_empty());
    assert!(list(&Claude::new(vec![PathBuf::new(), PathBuf::from("relative")]), "/work/app").is_empty());
}

#[test]
fn claude_snapshot() {
    let s = snapshot(&claude(), "/work/app", CLAUDE_MAIN);
    let running: Vec<_> = s.running.iter().map(|c| (c.tool.as_str(), c.station, c.summary.as_str())).collect();
    assert_eq!(running, [("Bash", Station::Network, "git push")]);
    assert_eq!(s.station(), Some(Station::Network));
    let recent: Vec<_> = s.recent.iter().map(|c| (c.summary.as_str(), c.failed)).collect();
    assert_eq!(recent, [("src/main.rs", false), ("Explore", false), ("cargo test", true), ("agent", false)]);
    assert_eq!(
        counts(&s),
        [
            (Station::You, 1),
            (Station::Model, 3),
            (Station::Files, 1),
            (Station::Shell, 1),
            (Station::Network, 1),
            (Station::Agents, 2)
        ],
        "one model step per message id; the summary, the reminder and the sidechain record count nowhere"
    );
    assert_eq!(s.errors, 2);
    assert_eq!(s.last_error.as_ref().map(|e| e.1.as_str()), Some("rate_limit"));
    let subagents: Vec<_> = s.subagents.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(subagents, ["agent"], "`Explore` finished, by its meta file");
    assert_eq!(s.model.as_deref(), Some("claude-fable-5-2"));
    assert_eq!(
        (s.context_tokens, s.context_window, s.context_percent()),
        (Some(61_000), None, None),
        "under 200k the window may be 200k or 1M: unknown"
    );
    assert_eq!(s.compactions, 1);
}

#[test]
fn claude_events_name_the_model_change() {
    let claude = claude();
    let events = read_all(&claude, &find(&claude, "/work/app", CLAUDE_MAIN)).unwrap();
    let change = events.iter().find_map(|e| match &e.what {
        What::ModelChange { from, to } => Some((from.as_str(), to.as_str())),
        _ => None,
    });
    assert_eq!(change, Some(("claude-opus-5-5", "claude-fable-5-2")));
}

#[test]
fn read_is_incremental_and_waits_for_a_whole_line() {
    let claude = claude();
    let original = find(&claude, "/work/app", CLAUDE_MAIN);
    let text = fs::read_to_string(&original.path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let (first, last) = (lines[..lines.len() - 1].join("\n"), lines[lines.len() - 1]);

    // All lines but the last, then half of the last one, as a runner in the middle of a write leaves it.
    let path = temp("partial").join("session.jsonl");
    let (half, rest) = last.split_at(last.len() / 2);
    fs::write(&path, format!("{first}\n{half}")).unwrap();
    let session = SessionRef { path: path.clone(), ..original };

    let batch = claude.read(&session, Cursor::default(), ALL).unwrap();
    assert_eq!(batch.cursor.offset(), first.len() as u64 + 1, "stops before the half line");
    assert!(!batch.restarted && !batch.more);
    let mut s = Snapshot::fold(&batch.events);
    assert_eq!(s.running.len(), 2, "the last tool result is not in yet");

    let again = claude.read(&session, batch.cursor.clone(), ALL).unwrap();
    assert!(again.events.is_empty());
    assert_eq!(again.cursor, batch.cursor);

    fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(format!("{rest}\n").as_bytes()).unwrap();
    let batch = claude.read(&session, again.cursor, ALL).unwrap();
    assert_eq!(batch.events.len(), 1);
    batch.events.iter().for_each(|e| s.apply(e));
    assert_eq!(s.running.len(), 1);
    assert_eq!(batch.cursor.offset(), text.len() as u64);
}

#[test]
fn read_stops_at_its_byte_budget_and_goes_on_from_there() {
    let claude = claude();
    let session = find(&claude, "/work/app", CLAUDE_MAIN);
    let whole = claude.read(&session, Cursor::default(), ALL).unwrap();

    let (mut events, mut cursor, mut calls) = (Vec::new(), Cursor::default(), 0);
    loop {
        let before = cursor.offset();
        let batch = claude.read(&session, cursor, 1000).unwrap();
        assert!(!batch.restarted);
        // A slice ends at the first line boundary past the budget.
        assert!(batch.cursor.offset() - before < 1000 + 2000, "one line over at most");
        events.extend(batch.events);
        (cursor, calls) = (batch.cursor, calls + 1);
        if !batch.more {
            break;
        }
    }
    assert!(calls > 5, "{calls} slices");
    assert_eq!(events, whole.events, "the slices add up to one whole read");
    assert_eq!(cursor, whole.cursor);
}

#[test]
fn a_truncated_or_replaced_file_restarts_the_cursor() {
    let claude = claude();
    let original = find(&claude, "/work/app", CLAUDE_MAIN);
    let text = fs::read_to_string(&original.path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let dir = temp("replaced");
    let path = dir.join("session.jsonl");
    fs::write(&path, &text).unwrap();
    let session = SessionRef { path: path.clone(), ..original };
    let first = claude.read(&session, Cursor::default(), ALL).unwrap();
    assert!(!first.restarted);

    // Truncated in place to one prompt: everything the cursor knew is void.
    fs::write(&path, format!("{}\n", lines[3])).unwrap();
    let batch = claude.read(&session, first.cursor.clone(), ALL).unwrap();
    assert!(batch.restarted);
    assert_eq!(batch.events.iter().map(|e| &e.what).collect::<Vec<_>>(), [&What::UserMessage]);
    assert_eq!(batch.cursor.offset(), lines[3].len() as u64 + 1);
    let fresh = claude.read(&session, Cursor::default(), ALL).unwrap();
    assert_eq!(batch.cursor, fresh.cursor, "no state of the old file survives");

    // Another, longer file moved into its place: the length does not show it, the inode does.
    let other = dir.join("other.jsonl");
    fs::write(&other, format!("{text}{}\n", lines[3])).unwrap();
    fs::rename(&other, &path).unwrap();
    let batch = claude.read(&session, batch.cursor, ALL).unwrap();
    assert!(batch.restarted);
    assert_eq!(batch.events.len(), first.events.len() + 1, "read from its start");

    let same = claude.read(&session, batch.cursor, ALL).unwrap();
    assert!(!same.restarted && same.events.is_empty());
}

#[test]
fn claude_finds_the_cwd_behind_a_long_head() {
    // 60 settings records and a 2 MB request log come before the first record with a cwd.
    let config = temp("long-head");
    let project = config.join("projects/-work-far");
    fs::create_dir_all(&project).unwrap();
    let mut text = "{\"type\":\"mode\",\"mode\":\"normal\",\"sessionId\":\"s\"}\n".to_string();
    text = text.repeat(60);
    text += &format!("{{\"type\":\"api-request\",\"params\":\"{}\"}}\n", "x".repeat(2 << 20));
    text += r#"{"type":"user","message":{"role":"user","content":"made-up"},"timestamp":"2026-11-12T09:00:00.000Z","cwd":"/work/far"}"#;
    text += "\n";
    fs::write(project.join("44444444-4444-4444-8444-444444444444.jsonl"), text).unwrap();
    let claude = Claude::new(vec![config]);
    let sessions = list(&claude, "/work/far");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].started, Some(1_794_474_000_000));
    assert_eq!(list(&claude, "/work/far").len(), 1, "and again, from what listing kept");
}

#[test]
fn listing_rereads_a_file_only_when_it_changed() {
    let home = temp("cache");
    let day = home.join("sessions/2026/11/12");
    fs::create_dir_all(&day).unwrap();
    let path = day.join("rollout-2026-11-12T09-00-00-c0de0009-0000-4000-8000-000000000009.jsonl");
    let meta = |cwd: &str| {
        format!(
            "{{\"timestamp\":\"2026-11-12T09:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"c0de0009-0000-4000-8000-000000000009\",\"cwd\":\"{cwd}\"}}}}\n"
        )
    };
    fs::write(&path, meta("/work/one")).unwrap();
    let codex = Codex::new(home);
    assert_eq!(list(&codex, "/work/one").len(), 1);
    // Rewritten with another cwd of the same length: the new time or length is noticed.
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(&path, meta("/work/two") + "\n").unwrap();
    assert!(list(&codex, "/work/one").is_empty());
    assert_eq!(list(&codex, "/work/two").len(), 1);
    fs::remove_file(&path).unwrap();
    assert!(list(&codex, "/work/two").is_empty());
}

#[test]
fn codex_lists_sessions_started_in_a_directory_or_below() {
    let mut codex = codex();
    let sessions = list(&codex, "/work/app");
    assert_eq!(ids(&sessions), [CODEX_NEW, CODEX_CHILD, CODEX_OLD]);
    assert_eq!(ids(&list(&codex, "/work/app/crates")), [CODEX_NEW, CODEX_CHILD]);
    assert!(list(&codex, "/work/ap").is_empty());

    let new = find(&codex, "/work/app", CODEX_NEW);
    assert_eq!(new.kind, Kind::Codex);
    assert_eq!(new.cwd, Path::new("/work/app/crates"));
    assert_eq!(new.started, Some(1_794_475_800_000));
    assert_eq!(new.model.as_deref(), Some("gpt-6-astra"));
    assert_eq!((new.pid, new.busy, new.parent.as_deref()), (None, None, None));
    assert_eq!(find(&codex, "/work/app", CODEX_CHILD).parent.as_deref(), Some(CODEX_NEW));
    // The model now is the last turn's, not the first's that listing read.
    assert_eq!(codex.model(&new).as_deref(), Some("gpt-6-nova"));
    assert_eq!(codex.model(&find(&codex, "/work/app", CODEX_OLD)).as_deref(), Some("gpt-5.4"));

    // Only the newest rollouts are opened.
    codex.max_files = 2;
    assert_eq!(ids(&list(&codex, "/work/app")), [CODEX_NEW, CODEX_CHILD]);
}

#[test]
fn codex_snapshot() {
    let s = snapshot(&codex(), "/work/app", CODEX_NEW);
    let running: Vec<_> = s.running.iter().map(|c| (c.tool.as_str(), c.station, c.summary.as_str())).collect();
    assert_eq!(running, [("exec", Station::Shell, "exec")]);
    let recent: Vec<_> = s.recent.iter().map(|c| (c.station, c.summary.as_str(), c.failed)).collect();
    assert_eq!(
        recent,
        [
            (Station::Network, "curl", false),
            (Station::Agents, "spawn_agent", false),
            (Station::Shell, "cargo test", true)
        ],
        "`exec` is named by the command it turned out to run"
    );
    assert_eq!(
        counts(&s),
        [(Station::You, 1), (Station::Model, 1), (Station::Shell, 2), (Station::Network, 1), (Station::Agents, 1)]
    );
    assert_eq!(s.errors, 1);
    assert_eq!(s.last_error.as_ref().map(|e| e.1.as_str()), Some("cargo test"));
    assert_eq!(s.subagents.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(), ["reviewer"]);
    assert_eq!(s.model.as_deref(), Some("gpt-6-nova"));
    assert_eq!((s.context_tokens, s.context_window, s.context_percent()), (Some(32_000), Some(400_000), Some(8)));
    assert_eq!(s.compactions, 1);
}

#[test]
fn codex_snapshot_of_an_older_rollout() {
    let s = snapshot(&codex(), "/work/app", CODEX_OLD);
    assert!(s.running.is_empty(), "a web search starts and ends in one record");
    let recent: Vec<_> = s.recent.iter().map(|c| (c.tool.as_str(), c.station, c.summary.as_str())).collect();
    assert_eq!(
        recent,
        [
            ("exec_command", Station::Files, "rg"),
            ("apply_patch", Station::Files, "src/lib.rs"),
            ("web_search_call", Station::Network, "web_search_call")
        ]
    );
    assert_eq!(counts(&s), [(Station::You, 1), (Station::Model, 1), (Station::Files, 2), (Station::Network, 1)]);
    assert_eq!(
        (s.errors, s.last_error.as_ref().map(|e| e.1.as_str())),
        (1, Some("usage_limit_exceeded")),
        "Esc is no error"
    );
    assert_eq!(s.context_percent(), Some(40));
}

#[test]
fn report_lists_each_session_once() {
    let (claude, codex) = (claude(), codex());
    let dirs = [PathBuf::from("/work/app"), PathBuf::from("/work/app/.claude/worktrees/fix")];
    let newest = claude
        .sessions(&dirs)
        .unwrap()
        .iter()
        .chain(&codex.sessions(&dirs).unwrap())
        .filter_map(|s| s.last_activity)
        .max();
    // The same runner twice: a file is still one session.
    let report = report(&[&claude, &codex, &claude], &dirs, newest.unwrap() + 1000, 10).unwrap();
    // `cargo test -p ottospace-sessions -- --nocapture report_lists` shows it.
    println!("{}", serde_json::to_string_pretty(&report).unwrap());

    let sessions = report["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 5);
    let main = sessions.iter().find(|s| s["id"] == CLAUDE_MAIN).unwrap();
    assert_eq!(main["kind"], "claude");
    assert_eq!(main["busy"], true);
    assert_eq!(main["model"], "claude-fable-5-2");
    assert_eq!(main["snapshot"]["station"], "network");
    assert_eq!(main["snapshot"]["running"][0]["summary"], "git push");
    assert_eq!(main["snapshot"]["context_tokens"], 61_000);
    assert_eq!(main["snapshot"]["counts"]["agents"], 2);
    let new = sessions.iter().find(|s| s["id"] == CODEX_NEW).unwrap();
    assert_eq!(new["snapshot"]["context_percent"], 8);
    assert!(sessions.iter().any(|s| s["active"] == true));
}

#[test]
fn report_snapshots_only_the_newest_sessions() {
    let codex = codex();
    let report = report(&[&codex], &[PathBuf::from("/work/app")], 0, 1).unwrap();
    let with: Vec<bool> = report["sessions"].as_array().unwrap().iter().map(|s| s.get("snapshot").is_some()).collect();
    assert_eq!(with, [true, false, false]);
}

/// Lists sessions, then deletes their files before they are read.
struct Vanishing(Codex);

impl Runner for Vanishing {
    fn kind(&self) -> Kind {
        Kind::Codex
    }
    fn sessions(&self, dirs: &[PathBuf]) -> std::io::Result<Vec<SessionRef>> {
        let sessions = self.0.sessions(dirs)?;
        sessions.iter().for_each(|s| fs::remove_file(&s.path).unwrap());
        Ok(sessions)
    }
    fn read(&self, session: &SessionRef, after: Cursor, budget: u64) -> std::io::Result<ottospace_sessions::Batch> {
        self.0.read(session, after, budget)
    }
}

#[test]
fn report_skips_a_session_whose_file_vanished() {
    let home = temp("vanish");
    let day = home.join("sessions/2026/11/12");
    fs::create_dir_all(&day).unwrap();
    let meta = r#"{"timestamp":"2026-11-12T09:00:00.000Z","type":"session_meta","payload":{"id":"c0de0008-0000-4000-8000-000000000008","cwd":"/work/gone"}}"#;
    fs::write(day.join("rollout-2026-11-12T09-00-00-c0de0008-0000-4000-8000-000000000008.jsonl"), format!("{meta}\n"))
        .unwrap();
    let report = report(&[&Vanishing(Codex::new(home))], &[PathBuf::from("/work/gone")], 0, 5).unwrap();
    assert_eq!(report["sessions"].as_array().unwrap().len(), 0);
}

/// In `tests/fixtures/adversarial`, the marker of a value that must never come out. One is in
/// every free-text and argument position: prompts, tool output, header values, env prefixes,
/// option values (`--opt v`, `--opt=v`, `-ov`), comments, argv elements, `$(…)`, backticks,
/// `<(…)`, `env -S`, `find -exec`, remote commands, nested shells, error strings and codes, URL
/// userinfo, path and query, file content, the leading parts of a path, descriptions, ids, and
/// names that are free text or shaped like a credential.
const HIDDEN: &str = "zzsent";

/// The marker of a value that is shown as written, by design.
const SHOWN: &str = "zzshow";

/// Everything the crate gives out about the adversarial sessions, as text: the report's JSON and
/// the `Debug` form of every public type. With the snapshot of each runner's session.
fn adversarial() -> (String, Snapshot, Snapshot) {
    let root = fixtures().join("adversarial");
    let (claude, codex) = (Claude::new(vec![root.join("claude")]), Codex::new(root.join("codex")));
    let runners: [&dyn Runner; 2] = [&claude, &codex];
    let dirs = [PathBuf::from("/work/ZZSHOWcwd")];

    let mut out = serde_json::to_string(&report(&runners, &dirs, 0, 10).unwrap()).unwrap();
    let mut snapshots = Vec::new();
    for runner in runners {
        // One session each: the files with a name or a cwd that is not one are not listed.
        let sessions = runner.sessions(&dirs).unwrap();
        assert_eq!(sessions.len(), 1, "{sessions:?}");
        let session = &sessions[0];
        let source = fs::read_to_string(&session.path).unwrap().to_lowercase();
        assert!(source.matches(HIDDEN).count() > 80, "the fixture is full of sentinels");
        out += &format!("{session:?} {:?}", runner.model(session));
        // In small slices, so cursors with calls still open are printed too.
        let (mut cursor, mut snapshot) = (Cursor::default(), Snapshot::default());
        loop {
            let batch = runner.read(session, cursor, 300).unwrap();
            batch.events.iter().for_each(|e| snapshot.apply(e));
            out += &format!("{batch:?} {snapshot:?}");
            cursor = batch.cursor.clone();
            if !batch.more {
                break;
            }
        }
        snapshots.push(snapshot);
    }
    let codex = snapshots.pop().unwrap();
    (out, snapshots.pop().unwrap(), codex)
}

fn labels<'a>(calls: impl Iterator<Item = &'a ottospace_sessions::snapshot::Call>) -> Vec<String> {
    calls.map(|c| format!("{:?} {}: {}", c.station, c.tool, c.summary)).collect()
}

#[test]
fn nothing_private_comes_out() {
    let (out, claude, codex) = adversarial();
    let lower = out.to_lowercase();
    if let Some(at) = lower.find(HIDDEN) {
        let from = lower[..at].char_indices().rev().nth(80).map_or(0, |(i, _)| i);
        panic!("a sentinel came out: …{}", &lower[from..(at + 12).min(lower.len())]);
    }

    // The calls were read, not dropped: each is there, under a name from the tables or a fixed word.
    assert_eq!(labels(claude.recent.iter()), ["Network Bash: git push", "Files Read: file"]);
    assert_eq!(
        labels(claude.running.iter()),
        [
            "Network Bash: curl",
            "Network Bash: kubectl get",
            "Network Bash: npm publish",
            "Network Bash: gh",
            "Shell Bash: shell",
            "Files Bash: cat",
            "Network Bash: curl",
            "Files Bash: ls",
            "Network Bash: ssh",
            "Network Bash: curl",
            "Shell Bash: shell",
            "Network Bash: diff",
            "Network Bash: curl",
            "Files Bash: grep",
            "Shell Bash: sh",
            "Shell Bash: git",
            "Files Write: ZZSHOWdir/ZZSHOWfile.rs",
            "Files Edit: file",
            "Files Read: file",
            "Files NotebookEdit: file",
            "Files Glob: file",
            "Files Grep: Grep",
            "Network WebFetch: zzshowhost.example",
            "Network WebFetch: web",
            "Network WebFetch: web",
            "Network WebFetch: web",
            "Network WebFetch: web",
            "Network WebFetch: web",
            "Network WebFetch: web",
            "Network WebSearch: WebSearch",
            "Skills Skill: ZZSHOWskill",
            "Skills Skill: skill",
            "Skills Skill: skill",
            "Skills Skill: skill",
            "Skills Skill: skill",
            "Agents Agent: ZZSHOWagent",
            "Agents Agent: agent",
            "Agents Agent: agent",
            "Network ZZSHOWserver/ZZSHOWtool: ZZSHOWserver/ZZSHOWtool",
            "Network mcp: mcp",
            "Network mcp: mcp",
            "Other tool: tool",
            "Other tool: tool",
        ]
    );
    assert_eq!(
        claude.subagents.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(),
        ["ZZSHOWagent", "agent", "agent"]
    );
    // The failed call's label had been rejected: the failure names it `file` too.
    assert_eq!((claude.errors, claude.last_error.as_ref().map(|e| e.1.as_str())), (3, Some("api_error")));
    assert!(claude.recent.iter().any(|c| c.failed && c.summary == "file"));

    assert_eq!(
        labels(codex.recent.iter()),
        [
            "Network exec_command: curl",
            "Shell shell: shell",
            "Network shell: kubectl get",
            "Shell shell: git status",
            "Network shell: curl",
            "Files shell: grep",
            "Network shell: ssh",
            "Network shell: shell",
            "Files shell: ls",
            "Network exec: gh",
            "Files apply_patch: ZZSHOWpdir/ZZSHOWpatch.rs",
            "Files apply_patch: file",
            "Other tool: tool",
            "Other tool: tool",
            "Agents spawn_agent: spawn_agent",
            "Network web_search_call: web_search_call",
        ]
    );
    assert_eq!(labels(codex.running.iter()), ["Shell exec_command: shell"]);
    assert_eq!(
        codex.subagents.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(),
        ["ZZSHOWreviewer", "agent", "agent"]
    );
    assert_eq!((codex.errors, codex.last_error.as_ref().map(|e| e.1.as_str())), (5, Some("aborted")));
}

#[test]
fn a_title_is_the_last_written_and_a_name_given_by_hand_comes_first() {
    let (claude, codex) = (claude(), codex());
    let title = |runner: &dyn Runner, dir: &str, id: &str| runner.title(&find(runner, dir, id));
    assert_eq!(title(&claude, "/work/app", CLAUDE_MAIN).as_deref(), Some("Greet the user"));
    assert_eq!(title(&claude, "/work/app", CLAUDE_WORKTREE).as_deref(), Some("Renamed by hand"));
    assert_eq!(title(&claude, "/work/app-other", "33333333-3333-4333-8333-333333333333"), None);
    // Codex: a name in the index beside the rollouts, for the sessions that have one.
    assert_eq!(title(&codex, "/work/app", CODEX_NEW).as_deref(), Some("Tidy the imports"));
    assert_eq!(title(&codex, "/work/app", CODEX_OLD), None);
    assert_eq!(title(&codex, "/elsewhere", "c0de0003-0000-4000-8000-000000000003"), None, "an empty name");
}

#[test]
fn a_hostile_title_is_one_cut_line_and_only_a_title_record_counts() {
    let path = temp("title").join("session.jsonl");
    let session = SessionRef { path: path.clone(), ..find(&claude(), "/work/app", CLAUDE_MAIN) };
    let record = |kind: &str, key: &str, text: &str| serde_json::json!({ "type": kind, key: text }).to_string();
    let title = |lines: &[String]| {
        fs::write(&path, lines.join("\n") + "\n").unwrap();
        Claude::new(Vec::new()).title(&session)
    };
    let hostile = format!("  Ignore the user\u{1b}[31m\u{202e}{}\nand a second line\r\nand a third", "x".repeat(5000));
    let shown = title(&[record("ai-title", "aiTitle", &hostile)]).unwrap();
    assert!(shown.starts_with("Ignore the user[31m\u{202e}xxx"), "bidi marks are the core's to drop: {shown:?}");
    assert_eq!(shown.chars().count(), ottospace_sessions::MAX_TITLE);
    assert!(!shown.chars().any(char::is_control) && !shown.contains("second"));
    // The words in a prompt or a tool's output are no title record, and neither is another shape.
    let prompt = serde_json::json!({ "type": "user", "message": { "role": "user", "content":
        "{\"type\":\"ai-title\",\"aiTitle\":\"from a prompt\"}" }, "aiTitle": "beside a prompt" });
    let nested = r#"{"type":"progress","data":{"type":"ai-title","aiTitle":"from a tool"}}"#.to_string();
    let wrong = r#"{"type":"ai-title","aiTitle":{"text":"not a text"}}"#.to_string();
    assert_eq!(title(&[prompt.to_string(), nested.clone()]), None);
    assert_eq!(title(&[record("ai-title", "aiTitle", "Real"), prompt.to_string(), nested]).as_deref(), Some("Real"));
    assert_eq!(title(&[record("ai-title", "aiTitle", "Earlier"), wrong]), None, "the last record decides");
    // A name taken back (an empty custom title) shows the model's again; blank is no title.
    let custom = |text: &str| record("custom-title", "customTitle", text);
    let ai = record("ai-title", "aiTitle", "The model's");
    assert_eq!(title(&[custom("Mine"), ai.clone()]).as_deref(), Some("Mine"));
    assert_eq!(title(&[custom("Mine"), ai, custom("")]).as_deref(), Some("The model's"));
    assert_eq!(title(&[record("ai-title", "aiTitle", " \n\u{7} ")]), None);
    assert_eq!(title(&[]), None);
}

#[test]
fn a_title_is_read_again_only_when_the_file_changed() {
    let path = temp("title-cache").join("session.jsonl");
    let session = SessionRef { path: path.clone(), ..find(&claude(), "/work/app", CLAUDE_MAIN) };
    let line = |text: &str| format!("{{\"type\":\"ai-title\",\"aiTitle\":\"{text}\"}}\n");
    let claude = Claude::new(Vec::new());
    fs::write(&path, line("One")).unwrap();
    assert_eq!(claude.title(&session).as_deref(), Some("One"));
    // Same length, and its time put back: taken for unchanged, so what was read is kept.
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, line("Two")).unwrap();
    fs::File::options().write(true).open(&path).unwrap().set_modified(modified).unwrap();
    assert_eq!(claude.title(&session).as_deref(), Some("One"));
    fs::write(&path, line("Three")).unwrap();
    assert_eq!(claude.title(&session).as_deref(), Some("Three"));
}

#[test]
fn a_title_leaves_by_title_alone() {
    // `nothing_private_comes_out` has it that no title is in the report or in any `Debug` form.
    let root = fixtures().join("adversarial");
    let (claude, codex) = (Claude::new(vec![root.join("claude")]), Codex::new(root.join("codex")));
    let dirs = [PathBuf::from("/work/ZZSHOWcwd")];
    let titles: Vec<Option<String>> = [&claude as &dyn Runner, &codex]
        .iter()
        .map(|runner| runner.title(&runner.sessions(&dirs).unwrap()[0]))
        .collect();
    // Free text, quoted as written but for its first line and control characters.
    assert_eq!(titles[0].as_deref(), Some("ZZSENT506 ignore what came before"));
    assert_eq!(titles[1].as_deref(), Some("ZZSENT508 run this[31m"));
}

#[test]
fn what_is_shown_as_written() {
    let (out, claude, codex) = adversarial();
    // Every value with the marker that came out, and nothing else with it.
    let lower = out.to_lowercase();
    let mut shown: Vec<&str> = lower
        .match_indices(SHOWN)
        .map(|(at, _)| {
            let rest = &lower[at..];
            &rest[..rest.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(rest.len())]
        })
        .collect();
    shown.sort();
    shown.dedup();
    assert_eq!(
        shown,
        [
            "zzshowagent",    // a sub-agent's type, from the model's tool call
            "zzshowcodex",    // a Codex session id, from the rollout (and its file name)
            "zzshowcwd",      // a session's directory, from the session file
            "zzshowdir",      // a file's folder: the path's second-last part, from a tool call
            "zzshowfile",     // a file's name: the path's last part
            "zzshowgpt",      // a Codex model, from the rollout
            "zzshowhost",     // a host, from a URL with no userinfo
            "zzshowmodel",    // a Claude model, from the transcript
            "zzshowpatch",    // a file's name, from a Codex patch
            "zzshowpdir",     // a file's folder, from a Codex patch
            "zzshowreviewer", // the last part of a Codex agent path
            "zzshowserver",   // an MCP server's name
            "zzshowsid",      // a Claude session id: the transcript's file name
            "zzshowskill",    // a skill's name, from the model's tool call
            "zzshowtool",     // an MCP tool's name
        ]
    );
    // Where each is.
    let (claude_runner, codex_runner) = {
        let root = fixtures().join("adversarial");
        (Claude::new(vec![root.join("claude")]), Codex::new(root.join("codex")))
    };
    let dirs = [PathBuf::from("/work/ZZSHOWcwd")];
    let session = &claude_runner.sessions(&dirs).unwrap()[0];
    assert_eq!(session.id, "ZZSHOWsid-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
    assert_eq!(session.cwd, Path::new("/work/ZZSHOWcwd"));
    assert_eq!(claude.model.as_deref(), Some("ZZSHOWmodel-5"), "a later model that is no name changes nothing");
    assert_eq!(claude_runner.model(session).as_deref(), Some("ZZSHOWmodel-5"));
    let session = &codex_runner.sessions(&dirs).unwrap()[0];
    assert_eq!(session.id, "ZZSHOWcodex-0000-4000-8000-0000000000ad");
    assert_eq!(session.model.as_deref(), Some("ZZSHOWgpt-6"));
    assert_eq!(codex.model.as_deref(), Some("ZZSHOWgpt-6"));
    // Known tools are named from a table; ids are hashes.
    assert!(claude.running.iter().all(|c| c.id.len() == 16 && c.id.chars().all(|c| c.is_ascii_hexdigit())));
}

#[test]
fn a_fifo_named_like_a_session_does_not_block() {
    // Opening a FIFO with no writer never returns. Both runners must pass it by.
    let root = temp("fifo");
    let (project, day) = (root.join("claude/projects/-work-pipe"), root.join("codex/sessions/2026/11/12"));
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&day).unwrap();
    let pipes = [
        project.join("55555555-5555-4555-8555-555555555555.jsonl"),
        day.join("rollout-2026-11-12T09-00-00-c0de0007-0000-4000-8000-000000000007.jsonl"),
    ];
    for pipe in &pipes {
        assert!(std::process::Command::new("mkfifo").arg(pipe).status().unwrap().success());
    }
    let (claude, codex) = (Claude::new(vec![root.join("claude")]), Codex::new(root.join("codex")));
    assert!(list(&claude, "/work/pipe").is_empty());
    assert!(list(&codex, "/work/pipe").is_empty());
    // And a session whose file became one is an error to read, not a hang.
    let session = SessionRef { path: pipes[0].clone(), ..find(&self::claude(), "/work/app", CLAUDE_MAIN) };
    assert_eq!(claude.read(&session, Cursor::default(), ALL).unwrap_err().kind(), ErrorKind::InvalidInput);
    assert_eq!(claude.model(&session), None);
}

#[test]
fn checkouts_are_the_main_checkout_and_its_worktrees() {
    let root = temp("git");
    let (main, linked, moved) = (root.join("app"), root.join("elsewhere/app-fix"), root.join("app-rel"));
    for dir in [&main, &linked, &moved] {
        fs::create_dir_all(dir.join("src")).unwrap();
    }
    // One worktree with absolute pointers, as git writes them.
    let admin = main.join(".git/worktrees/app-fix");
    fs::create_dir_all(&admin).unwrap();
    fs::write(admin.join("gitdir"), format!("{}\n", linked.join(".git").display())).unwrap();
    fs::write(admin.join("commondir"), "../..\n").unwrap();
    fs::write(linked.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
    // One with relative pointers (`worktree.useRelativePaths`): each relative to where it is written.
    let admin = main.join(".git/worktrees/app-rel");
    fs::create_dir_all(&admin).unwrap();
    fs::write(admin.join("gitdir"), "../../../../app-rel/.git\n").unwrap();
    fs::write(admin.join("commondir"), "../..\n").unwrap();
    fs::write(moved.join(".git"), "gitdir: ../app/.git/worktrees/app-rel\n").unwrap();
    // One deleted and not pruned, and one whose pointer is not a small file.
    fs::create_dir_all(main.join(".git/worktrees/gone")).unwrap();
    fs::write(main.join(".git/worktrees/gone/gitdir"), format!("{}\n", root.join("gone/.git").display())).unwrap();
    fs::create_dir_all(main.join(".git/worktrees/huge")).unwrap();
    fs::write(main.join(".git/worktrees/huge/gitdir"), "x".repeat(5000)).unwrap();

    let all = [main.clone(), moved.clone(), linked.clone()];
    assert_eq!(checkouts(&main).unwrap(), all);
    assert_eq!(checkouts(&main.join("src")).unwrap(), all);
    assert_eq!(checkouts(&linked.join("src")).unwrap(), all, "from a worktree's subdirectory");
    assert_eq!(checkouts(&moved).unwrap(), all, "through relative pointers");
    assert_eq!(checkouts(&main.join("src/../src")).unwrap(), all, "the directory is resolved first");
    let plain = root.join("plain");
    fs::create_dir_all(&plain).unwrap();
    assert_eq!(checkouts(&plain).unwrap(), std::slice::from_ref(&plain), "not a repository");
}

/// (model, sub-agent's, input, output, cached), as a tally sums them.
fn tokens(tally: &Tally) -> Vec<(&str, bool, u64, u64, u64)> {
    tally
        .models
        .iter()
        .map(|((model, sub), u): (&(String, bool), &Usage)| (model.as_str(), *sub, u.input, u.output, u.cached))
        .collect()
}

#[test]
fn claude_tally_sums_tokens_by_model_with_sub_agents_apart() {
    let claude = claude();
    let session = find(&claude, "/work/app", CLAUDE_MAIN);
    let mut reader = Reader::default();
    let read = reader.advance(&claude, &session, ALL).unwrap();
    assert!(read.caught_up && !read.restarted);
    // One request per message id; the inline sidechain record is not the main loop's, and the
    // sub-agent's own file is counted as a sub-agent's.
    assert_eq!(
        tokens(&reader.tally),
        [
            ("claude-fable-5-2", false, 60_500, 500, 60_000),
            ("claude-haiku-5", true, 4_250, 100, 4_100),
            ("claude-opus-5-5", false, 84_800, 240, 82_000),
        ]
    );
    assert_eq!(reader.tally.context, Some(61_000), "after the main loop's last request");
    // Activity from every file, oldest first: the sub-agent's at 09:00:12 among the main loop's.
    assert!(read.times.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(read.times.contains(&1_794_474_012_000), "{:?}", read.times);
    // Nothing new: nothing more is counted.
    let again = reader.advance(&claude, &session, ALL).unwrap();
    assert!(again.times.is_empty() && again.caught_up);
    assert_eq!(tokens(&reader.tally)[2], ("claude-opus-5-5", false, 84_800, 240, 82_000));
}

#[test]
fn codex_tally_counts_a_child_rollout_as_a_sub_agent() {
    let codex = codex();
    let session = find(&codex, "/work/app", CODEX_NEW);
    assert_eq!(ids(&codex.subagents(&session)), [CODEX_CHILD]);
    let mut reader = Reader::default();
    assert!(reader.advance(&codex, &session, ALL).unwrap().caught_up);
    assert_eq!(
        tokens(&reader.tally),
        [("gpt-6-astra", false, 30_000, 2_000, 20_000), ("gpt-6-astra", true, 8_000, 300, 6_000)]
    );
    assert_eq!((reader.tally.context, reader.tally.window), (Some(32_000), Some(400_000)));
}

#[test]
fn a_tally_goes_on_from_its_cursor_and_starts_again_with_a_new_file() {
    let dir = temp("tally");
    let path = dir.join("s.jsonl");
    let step = |id: &str, at: &str, input: u64| {
        format!(
            r#"{{"type":"assistant","message":{{"id":"{id}","model":"claude-opus-5-5","content":[],"usage":{{"input_tokens":{input},"output_tokens":1}}}},"timestamp":"2026-11-12T09:00:{at}.000Z","cwd":"/w"}}"#
        )
    };
    fs::write(&path, step("m1", "01", 10) + "\n").unwrap();
    let session = SessionRef {
        kind: Kind::Claude,
        id: "s".into(),
        cwd: "/w".into(),
        path: path.clone(),
        started: None,
        last_activity: None,
        model: None,
        pid: None,
        busy: None,
        parent: None,
    };
    let claude = claude();
    let mut reader = Reader::default();
    reader.advance(&claude, &session, ALL).unwrap();
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(file, "{}", step("m2", "02", 20)).unwrap();
    let read = reader.advance(&claude, &session, ALL).unwrap();
    assert_eq!((read.times.len(), tokens(&reader.tally)[0].2), (1, 30), "only the new line");
    // Gone for one read: the cursor stays, so nothing is counted twice when it is back.
    fs::rename(&path, path.with_extension("away")).unwrap();
    assert!(reader.advance(&claude, &session, ALL).is_err());
    fs::rename(path.with_extension("away"), &path).unwrap();
    reader.advance(&claude, &session, ALL).unwrap();
    assert_eq!(tokens(&reader.tally)[0].2, 30);
    // Another file at the path: counted from its start alone.
    fs::remove_file(&path).unwrap();
    fs::write(&path, step("m3", "03", 5) + "\n").unwrap();
    let read = reader.advance(&claude, &session, ALL).unwrap();
    assert!(read.restarted);
    assert_eq!(tokens(&reader.tally)[0].2, 5);
    let _ = fs::remove_dir_all(dir);
}
