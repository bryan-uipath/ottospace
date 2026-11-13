//! The parsers are total: random bytes, cut lines and fixture lines with any value swapped for
//! another type never panic. A sentinel in any value never reaches a label: only `Untrusted`
//! carries text, and its `Debug` shows a length. `OTTOSPACE_PROP_SEED=<n>` runs one seed.

use std::path::{Path, PathBuf};

use ottospace_agents::claude::Claude;
use ottospace_agents::codex::Codex;
use ottospace_agents::lines::{Line, Lines};
use ottospace_agents::{Event, Protocol, Start};
use serde_json::{Value, json};

const SENTINEL: &str = "x SENTINEL $(y) ../../z";

/// splitmix64, as the core's property tests use.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn seeds() -> Vec<u64> {
    match std::env::var("OTTOSPACE_PROP_SEED").ok().and_then(|seed| seed.parse().ok()) {
        Some(seed) => vec![seed],
        None => (1..=64).collect(),
    }
}

fn fixtures(runner: &str) -> Vec<String> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(runner);
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().path()).collect();
    files.sort();
    files
        .iter()
        .flat_map(|file| std::fs::read_to_string(file).unwrap().lines().map(str::to_string).collect::<Vec<_>>())
        .collect()
}

/// A Codex past its handshake, with its thread known: every rule that needs one is reachable.
fn codex() -> Codex {
    let mut codex = Codex::new(Path::new("/work/app"), None, &Start::New { id: String::new() }).unwrap();
    codex.opening();
    codex.read(r#"{"id":0,"result":{}}"#);
    codex.read(&fixtures("codex").into_iter().find(|line| line.starts_with(r#"{"id":1"#)).unwrap());
    codex
}

/// Makes a fresh protocol of one runner.
type Make = Box<dyn Fn() -> Box<dyn Protocol>>;

fn protocols() -> Vec<(&'static str, Make)> {
    vec![("claude", Box::new(|| Box::new(Claude::new()))), ("codex", Box::new(|| Box::new(codex())))]
}

/// Labels never carry the sentinel; whatever the line, the parser returns.
fn check(protocol: &mut dyn Protocol, line: &str) {
    let out = protocol.read(line);
    let shown = format!("{:?}", out.events);
    assert!(!shown.contains("SENTINEL"), "{line}\n{shown}");
}

/// Every string, number, bool and null inside `value`, as a path of keys and indexes.
fn leaves(value: &Value, path: &mut Vec<Value>, out: &mut Vec<Vec<Value>>) {
    match value {
        Value::Object(map) => map.iter().for_each(|(key, child)| {
            path.push(json!(key));
            leaves(child, path, out);
            path.pop();
        }),
        Value::Array(items) => items.iter().enumerate().for_each(|(at, child)| {
            path.push(json!(at));
            leaves(child, path, out);
            path.pop();
        }),
        _ => out.push(path.clone()),
    }
}

fn at<'a>(value: &'a mut Value, path: &[Value]) -> &'a mut Value {
    path.iter().fold(value, |value, step| match step {
        Value::String(key) => &mut value[key.as_str()],
        _ => &mut value[step.as_u64().unwrap_or(0) as usize],
    })
}

#[test]
fn any_value_swapped_for_another_type_is_read_and_never_shown() {
    let swaps = [
        json!(null),
        json!(0),
        json!(-1),
        json!(1.5e308),
        json!(true),
        json!(""),
        json!([]),
        json!({}),
        json!(SENTINEL),
        json!([SENTINEL]),
        json!({"id": SENTINEL, "type": SENTINEL}),
        json!(format!("mcp__{SENTINEL}__tool")),
        json!(format!("/a/b/{SENTINEL}")),
        json!(format!("https://user:pw@{SENTINEL}")),
        json!("x".repeat(70_000)),
    ];
    for (runner, make) in protocols() {
        let mut swapped = 0;
        for line in fixtures(runner) {
            let record: Value = serde_json::from_str(&line).unwrap();
            let mut paths = Vec::new();
            leaves(&record, &mut Vec::new(), &mut paths);
            for path in paths {
                for swap in &swaps {
                    let mut changed = record.clone();
                    *at(&mut changed, &path) = swap.clone();
                    check(make().as_mut(), &changed.to_string());
                    swapped += 1;
                }
            }
        }
        assert!(swapped > 1000, "{runner}: {swapped}");
    }
}

#[test]
fn every_value_a_sentinel_at_once() {
    for (runner, make) in protocols() {
        for line in fixtures(runner) {
            let mut record: Value = serde_json::from_str(&line).unwrap();
            let mut paths = Vec::new();
            leaves(&record, &mut Vec::new(), &mut paths);
            for path in &paths {
                // Dispatch keys stay, so the record still reaches its rule.
                let keep =
                    matches!(path.last().and_then(Value::as_str), Some("type" | "subtype" | "method" | "status"));
                if !keep && at(&mut record, path).is_string() {
                    *at(&mut record, path) = json!(SENTINEL);
                }
            }
            check(make().as_mut(), &record.to_string());
        }
    }
}

#[test]
fn cut_lines_and_random_bytes_are_read() {
    for (runner, make) in protocols() {
        for line in fixtures(runner) {
            let mut protocol = make();
            for end in (0..line.len()).filter(|end| line.is_char_boundary(*end)) {
                protocol.read(&line[..end]);
            }
        }
        for seed in seeds() {
            let mut rng = Rng(seed);
            let mut protocol = make();
            let mut lines = Lines::new(1 << 10);
            let pieces: &[&[u8]] = &[
                b"{",
                b"}",
                b"\"",
                b":",
                b",",
                b"\n",
                b"[",
                b"]",
                b"\"type\"",
                b"\"id\"",
                b"\"method\"",
                b"null",
                b"\xff",
                b"7",
            ];
            for _ in 0..2000 {
                let bytes: Vec<u8> =
                    (0..rng.below(64)).flat_map(|_| pieces[rng.below(pieces.len())].to_vec()).collect();
                for line in lines.push(&bytes) {
                    if let Line::Text(text) = line {
                        check(protocol.as_mut(), &text);
                    }
                }
            }
        }
    }
}

#[test]
fn unknown_records_are_kept_as_unknown() {
    let unknown = |kind: &str| vec![Event::Unknown { kind: kind.into() }];
    assert_eq!(Claude::new().read(r#"{"type":"brand_new","x":1}"#).events, unknown("brand_new"));
    assert_eq!(Claude::new().read(r#"{"type":"system","subtype":"later"}"#).events, unknown("system.later"));
    assert_eq!(Claude::new().read("not json at all").events, unknown("not_json"));
    assert_eq!(Claude::new().read(&format!(r#"{{"type":"{SENTINEL}"}}"#)).events, unknown("unknown"));
    assert_eq!(codex().read(r#"{"method":"brand/new","params":{}}"#).events, unknown("brand.new"));
    assert_eq!(codex().read("[1,2]").events, unknown("unknown"));
}
