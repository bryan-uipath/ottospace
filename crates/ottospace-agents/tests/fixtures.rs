//! The runners against hand-written session files in `tests/fixtures`: made-up content in the
//! real schema. No test reads a real config dir.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use ottospace_agents::claude::Claude;
use ottospace_agents::codex::Codex;
use ottospace_agents::git::checkouts;
use ottospace_agents::report::report;
use ottospace_agents::snapshot::Snapshot;
use ottospace_agents::station::Station;
use ottospace_agents::{Cursor, Kind, Runner, SessionRef, What, read_all};

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
    let dir = std::env::temp_dir().join(format!("ottospace-agents-{}-{name}", std::process::id()));
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
    // `cargo test -p ottospace-agents -- --nocapture report_lists` shows it.
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
    fn read(&self, session: &SessionRef, after: Cursor, budget: u64) -> std::io::Result<ottospace_agents::Batch> {
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

/// The marker in every free-text and argument position of `tests/fixtures/adversarial`: prompts,
/// tool output, header values, env prefixes, option values, comments, argv elements, error
/// strings, URL userinfo, path and query, file content, descriptions, free-text names.
const SENTINEL: &str = "ZZSENT";

#[test]
fn nothing_private_comes_out() {
    let root = fixtures().join("adversarial");
    let (claude, codex) = (Claude::new(vec![root.join("claude")]), Codex::new(root.join("codex")));
    let runners: [&dyn Runner; 2] = [&claude, &codex];
    let dirs = [PathBuf::from("/work/adv")];

    // Everything the crate gives out, as text: the JSON, and the Debug form of every public type.
    let mut out = serde_json::to_string(&report(&runners, &dirs, 0, 10).unwrap()).unwrap();
    let mut snapshots = Vec::new();
    for runner in runners {
        let sessions = runner.sessions(&dirs).unwrap();
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        let source = fs::read_to_string(&session.path).unwrap();
        assert!(source.matches(SENTINEL).count() > 50, "the fixture is full of sentinels");
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
    if let Some(at) = out.find(SENTINEL) {
        let from = out[..at].char_indices().rev().nth(80).map_or(0, |(i, _)| i);
        panic!("a sentinel came out: …{}", &out[from..(at + 12).min(out.len())]);
    }

    // The calls were read, not dropped: each is there under a name from the tables.
    let (claude, codex) = (&snapshots[0], &snapshots[1]);
    let labels = |calls: &mut dyn Iterator<Item = &ottospace_agents::snapshot::Call>| {
        calls.map(|c| format!("{:?}:{}", c.station, c.summary)).collect::<Vec<_>>()
    };
    assert_eq!(labels(&mut claude.recent.iter()), ["Network:git push", "Network:kubectl get"]);
    assert_eq!(
        labels(&mut claude.running.iter()),
        [
            "Network:curl",
            "Network:npm publish",
            "Network:gh",
            "Shell:shell",
            "Files:cat",
            "Network:curl",
            "Files:ls",
            "Network:ssh",
            "Files:adv/notes.txt",
            "Files:src/lib.rs",
            "Files:Grep",
            "Network:example.com",
            "Network:x.example",
            "Network:web",
            "Network:WebSearch",
            "Skills:skill",
            "Agents:agent",
            "Network:mcp",
            "Other:tool",
        ]
    );
    assert_eq!(claude.subagents.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(), ["agent"]);
    assert_eq!((claude.errors, claude.last_error.as_ref().map(|e| e.1.as_str())), (2, Some("api_error")));
    assert_eq!(claude.model.as_deref(), Some("claude-opus-5-5"), "a model that is not a name is not a model");

    assert_eq!(
        labels(&mut codex.recent.iter()),
        [
            "Network:curl",
            "Shell:shell",
            "Network:kubectl get",
            "Network:gh",
            "Files:src/lib.rs",
            "Other:tool",
            "Agents:spawn_agent",
            "Network:web_search_call",
        ]
    );
    assert_eq!(labels(&mut codex.running.iter()), ["Shell:shell"]);
    assert_eq!(codex.subagents.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(), ["agent"]);
    assert_eq!((codex.errors, codex.last_error.as_ref().map(|e| e.1.as_str())), (3, Some("aborted")));
    assert_eq!(codex.model.as_deref(), Some("gpt-6-astra"));
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
