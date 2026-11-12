//! Which station a tool call belongs to, and its short label. Pure.
//!
//! A label for a command is built only from the tables in this file: a known program's name,
//! and a subcommand on that program's list. Nothing else from a command line is ever shown.

use serde::Serialize;

/// Where work goes. `You`, `Model` and `Hooks` are not tools; the rest classify tool calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Station {
    You,
    Model,
    Memory,
    Files,
    Shell,
    Network,
    Browser,
    Agents,
    Skills,
    Hooks,
    Other,
}

use Station::*;

/// The label of a command nothing is known about.
const UNKNOWN: &str = "shell";

/// The station of a tool call. `arg` is the command line of a shell tool or the path of a file tool.
/// e.g. `("Bash", Some("git push"))` → Network, `("Read", Some("/a/b.rs"))` → Files.
pub fn classify(tool: &str, arg: Option<&str>) -> Station {
    // MCP tools are `mcp__<server>__<tool>`: the server's name decides.
    if let Some(rest) = tool.strip_prefix("mcp__") {
        let server = rest.split("__").next().unwrap_or("").to_ascii_lowercase();
        return if server.contains("memory") {
            Memory
        } else if ["playwright", "chrome", "browser", "puppeteer"].iter().any(|b| server.contains(b)) {
            Browser
        } else {
            Network
        };
    }
    match tool {
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" | "Grep" | "Glob" | "LS" | "apply_patch"
        | "view_image" => {
            if arg.is_some_and(is_memory_path) {
                Memory
            } else {
                Files
            }
        }
        "Bash" | "PowerShell" | "Monitor" | "BashOutput" | "KillShell" | "exec_command" | "shell"
        | "local_shell_call" | "exec" | "write_stdin" => arg.map_or(Shell, |line| describe_command(line).0),
        "WebFetch" | "WebSearch" | "web_search" | "web_search_call" => Network,
        "Agent" | "Task" | "SendMessage" | "TaskStop" | "TaskOutput" | "Workflow" | "spawn_agent" | "send_message"
        | "wait_agent" | "close_agent" => Agents,
        "Skill" | "ToolSearch" | "tool_search_call" => Skills,
        "AskUserQuestion" => You,
        _ => Other,
    }
}

/// True for a runner's own memory files, e.g. `~/.claude/projects/<slug>/memory/notes.md`.
fn is_memory_path(path: &str) -> bool {
    path.contains("/.codex/memories/") || (path.contains("/.claude") && path.contains("/memory/"))
}

/// A shell command line's station and label, by the names of the commands in it.
/// Station: Network if any command reaches the network; Files if every one only works on files;
/// else Shell. Label: the first command's, e.g. `cd x && git -C y push -f` → `git push`.
pub fn describe_command(line: &str) -> (Station, String) {
    describe(&parse(line, 0))
}

/// The same for a command already split into words, e.g. `["git", "push"]`. The words are never
/// joined and split again. `["zsh", "-lc", "<line>"]` is read as the line.
pub fn describe_argv(words: &[&str]) -> (Station, String) {
    let words: Vec<String> = words.iter().map(|w| w.to_string()).collect();
    describe(&command(&words, 0))
}

fn describe(commands: &[Cmd]) -> (Station, String) {
    let station = if commands.iter().any(|c| c.station == Network) {
        Network
    } else if !commands.is_empty() && commands.iter().all(|c| c.station == Files) {
        Files
    } else {
        Shell
    };
    // The first command names the call, known or not: never a later one in its place.
    (station, commands.first().map_or(UNKNOWN.to_string(), |c| c.label.clone()))
}

/// A path's label: its last two parts, e.g. `/Users/me/app/src/main.rs` → `src/main.rs`.
pub fn path_label(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    clean(&parts[parts.len().saturating_sub(2)..].join("/"))
}

/// A web address's label: the host alone, e.g. `https://me:pw@example.com:8080/a?b` → `example.com`.
/// Anything but letters, digits, `.` and `-` in the host (an IPv6 address too) gives `web`.
pub fn host_label(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    // Browsers read `\` as `/`, so `https://a.example\@b/` is the host `a.example`.
    let authority = rest.split(['/', '\\', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("").split(':').next().unwrap_or("");
    let plain = host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if host.is_empty() || host.len() > 60 || !plain { "web".to_string() } else { host.to_string() }
}

/// A label fit to show: printable ASCII only (no control, bidi or zero-width characters), at most 60 characters.
pub fn clean(label: &str) -> String {
    label.chars().filter(|c| (' '..='~').contains(c)).take(60).collect()
}

/// `text` if it has the shape of a name: 1 to 64 ASCII letters, digits or `_ . : -`. Else `fallback`.
/// e.g. `("code-review", "skill")` → `code-review`, `("review my secret", "skill")` → `skill`.
pub fn name(text: &str, fallback: &str) -> String {
    let shaped =
        (1..=64).contains(&text.len()) && text.chars().all(|c| c.is_ascii_alphanumeric() || "_.:-".contains(c));
    if shaped { text } else { fallback }.to_string()
}

/// `text` if it has the shape of an error code: 1 to 40 of `a-z 0-9 _`. Else `fallback`.
/// e.g. `("rate_limit", "error")` → `rate_limit`, `("Rate limit for key sk-1", "error")` → `error`.
pub fn code(text: &str, fallback: &str) -> String {
    let shaped = (1..=40).contains(&text.len())
        && text.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if shaped { text } else { fallback }.to_string()
}

/// One command of a line: its station, and a label made of table entries.
#[derive(Clone, Debug, PartialEq)]
struct Cmd {
    station: Station,
    label: String,
}

/// A shell inside a shell is followed this deep, e.g. `bash -c "sh -c 'curl x'"`.
const MAX_DEPTH: u8 = 3;

/// Every command in a line, in order. `cd`, `echo` and the like are left out.
fn parse(line: &str, depth: u8) -> Vec<Cmd> {
    split(line).iter().flat_map(|words| command(words, depth)).collect()
}

/// Splits a line into commands, each a list of words. Separators: `; | & ( )`, a new line, a backtick,
/// and `$(`, also inside double quotes. Quotes are honoured. Dropped: `# comments`, heredoc bodies
/// and array values (`A=(x y)`), which are data, not commands.
fn split(line: &str) -> Vec<Vec<String>> {
    let chars: Vec<char> = line.chars().collect();
    let mut commands: Vec<Vec<String>> = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    // Inside double quotes; inside backticks that opened inside double quotes.
    let (mut double, mut ticked) = (false, false);
    // For each open `$(`: whether it began inside double quotes.
    let mut subs: Vec<bool> = Vec::new();
    // Delimiters of heredocs whose bodies start at the next new line.
    let mut heredocs: Vec<String> = Vec::new();
    let end_word = |word: &mut String, words: &mut Vec<String>| {
        if !word.is_empty() {
            words.push(std::mem::take(word));
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        let next = chars.get(i).copied();
        if c == '\\' {
            // A backslash before a new line joins the lines; else the next character is literal.
            if let Some(n) = next.filter(|n| *n != '\n') {
                word.push(n);
            }
            i += 1;
            continue;
        }
        if c == '$' && next == Some('(') {
            i += 1;
            subs.push(double);
            double = false;
            end_word(&mut word, &mut words);
            commands.push(std::mem::take(&mut words));
            continue;
        }
        if double {
            match c {
                '"' => double = false,
                '`' => {
                    (double, ticked) = (false, true);
                    end_word(&mut word, &mut words);
                    commands.push(std::mem::take(&mut words));
                }
                _ => word.push(c),
            }
            continue;
        }
        match c {
            '\'' => {
                let end = chars[i..].iter().position(|c| *c == '\'').map_or(chars.len(), |p| i + p);
                word.extend(&chars[i..end]);
                i = end + 1;
            }
            '"' => double = true,
            ' ' | '\t' => end_word(&mut word, &mut words),
            // A comment runs to the end of its line.
            '#' if word.is_empty() && (i < 2 || matches!(chars[i - 2], ' ' | '\t' | '\n' | ';' | '|' | '&' | '(')) => {
                i = chars[i..].iter().position(|c| *c == '\n').map_or(chars.len(), |p| i + p);
            }
            // `A=(x y)`: an array's values are not commands.
            '(' if is_assignment(&word) && word.ends_with('=') => {
                word.clear();
                i = chars[i..].iter().position(|c| *c == ')').map_or(chars.len(), |p| i + p + 1);
            }
            // `<<EOF`, `<<-'EOF'`, `<<"E OF"`: note the delimiter. `<<<` is a here-string, left alone.
            '<' if next == Some('<') && chars.get(i + 1) != Some(&'<') => {
                i += 1;
                if chars.get(i) == Some(&'-') {
                    i += 1;
                }
                while matches!(chars.get(i), Some(' ' | '\t')) {
                    i += 1;
                }
                let mut delimiter = String::new();
                let mut quote: Option<char> = None;
                while let Some(&d) = chars.get(i) {
                    match (quote, d) {
                        (Some(q), _) if d == q => quote = None,
                        (Some(_), _) => delimiter.push(d),
                        (None, '\'' | '"') => quote = Some(d),
                        (None, '\\') => {}
                        (None, ' ' | '\t' | '\n' | ';' | '|' | '&' | '(' | ')' | '<' | '>') => break,
                        (None, _) => delimiter.push(d),
                    }
                    i += 1;
                }
                heredocs.push(delimiter);
            }
            // `2>&1` and `&>` are redirections, not separators.
            '&' if word.ends_with('>') || next == Some('>') => word.push(c),
            ')' if !subs.is_empty() => {
                end_word(&mut word, &mut words);
                commands.push(std::mem::take(&mut words));
                double = subs.pop().unwrap_or(false);
            }
            '`' if ticked => {
                end_word(&mut word, &mut words);
                commands.push(std::mem::take(&mut words));
                (double, ticked) = (true, false);
            }
            ';' | '|' | '&' | '(' | ')' | '\n' | '`' => {
                end_word(&mut word, &mut words);
                commands.push(std::mem::take(&mut words));
                // Skip each pending heredoc's body: up to the line that is its delimiter, or the end.
                let bodies = if c == '\n' { std::mem::take(&mut heredocs) } else { Vec::new() };
                for delimiter in bodies {
                    loop {
                        let end =
                            chars[i.min(chars.len())..].iter().position(|c| *c == '\n').map_or(chars.len(), |p| i + p);
                        let body: String = chars[i.min(chars.len())..end].iter().collect();
                        i = end + 1;
                        if end >= chars.len() || body.trim() == delimiter {
                            break;
                        }
                    }
                }
            }
            _ => word.push(c),
        }
    }
    end_word(&mut word, &mut words);
    commands.push(words);
    commands
}

/// A program the labels may name. `subs`: the subcommands a label may add. `network`: the
/// subcommands that reach the network. `values`: options that take the next word as their value,
/// so `git -C dir push` finds `push`.
struct Program {
    name: &'static str,
    station: Station,
    subs: &'static [&'static str],
    network: &'static [&'static str],
    values: &'static [&'static str],
}

const fn plain(name: &'static str, station: Station) -> Program {
    Program { name, station, subs: &[], network: &[], values: &[] }
}

const NODE_SUBS: &[&str] = &[
    "install",
    "i",
    "ci",
    "add",
    "remove",
    "uninstall",
    "update",
    "upgrade",
    "run",
    "test",
    "start",
    "build",
    "exec",
    "publish",
    "pack",
    "link",
    "init",
    "create",
    "dlx",
    "audit",
    "outdated",
    "ls",
    "list",
    "why",
    "version",
    "lint",
    "dev",
    "format",
    "typecheck",
];
const NODE_NETWORK: &[&str] =
    &["install", "i", "ci", "add", "update", "upgrade", "publish", "create", "dlx", "audit", "outdated"];
const NODE_VALUES: &[&str] = &[
    "--otp",
    "--registry",
    "--prefix",
    "-C",
    "--dir",
    "--cwd",
    "-w",
    "--workspace",
    "--filter",
    "--tag",
    "--userconfig",
    "--cache",
];
const CONTAINER_SUBS: &[&str] = &[
    "build", "run", "exec", "ps", "images", "pull", "push", "login", "logout", "compose", "stop", "start", "restart",
    "rm", "rmi", "logs", "tag", "inspect", "network", "volume", "system", "buildx", "cp", "kill",
];
const CONTAINER_VALUES: &[&str] = &["-H", "--host", "--context", "-c", "--config", "-l", "--log-level"];
const PIP_SUBS: &[&str] = &["install", "uninstall", "download", "list", "show", "freeze", "check"];

/// Programs with subcommands.
const PROGRAMS: &[Program] = &[
    Program {
        name: "git",
        station: Shell,
        subs: &[
            "add",
            "am",
            "apply",
            "bisect",
            "blame",
            "branch",
            "checkout",
            "cherry-pick",
            "clean",
            "clone",
            "commit",
            "config",
            "describe",
            "diff",
            "fetch",
            "grep",
            "init",
            "log",
            "ls-files",
            "ls-remote",
            "merge",
            "mv",
            "pull",
            "push",
            "rebase",
            "reflog",
            "remote",
            "reset",
            "restore",
            "rev-parse",
            "revert",
            "rm",
            "show",
            "stash",
            "status",
            "submodule",
            "switch",
            "tag",
            "worktree",
        ],
        network: &["push", "pull", "fetch", "clone", "ls-remote"],
        values: &["-C", "-c", "--git-dir", "--work-tree", "--namespace", "--exec-path", "--config-env"],
    },
    Program {
        name: "gh",
        station: Network,
        subs: &[
            "pr",
            "issue",
            "repo",
            "api",
            "run",
            "workflow",
            "release",
            "auth",
            "gist",
            "search",
            "status",
            "browse",
            "label",
            "secret",
            "variable",
            "codespace",
            "extension",
            "cache",
            "project",
            "ruleset",
        ],
        network: &[],
        values: &["-R", "--repo", "--hostname"],
    },
    Program {
        name: "cargo",
        station: Shell,
        subs: &[
            "build", "check", "test", "run", "clippy", "fmt", "doc", "bench", "clean", "update", "install", "add",
            "remove", "publish", "fetch", "search", "tree", "metadata", "new", "init", "e2e", "e2e-apps", "nextest",
            "expand", "fix", "vendor",
        ],
        network: &["install", "fetch", "publish", "update", "add", "search"],
        values: &["-C", "--config", "--color", "-Z", "--manifest-path"],
    },
    Program { name: "npm", station: Shell, subs: NODE_SUBS, network: NODE_NETWORK, values: NODE_VALUES },
    Program { name: "pnpm", station: Shell, subs: NODE_SUBS, network: NODE_NETWORK, values: NODE_VALUES },
    Program { name: "yarn", station: Shell, subs: NODE_SUBS, network: NODE_NETWORK, values: NODE_VALUES },
    Program { name: "bun", station: Shell, subs: NODE_SUBS, network: NODE_NETWORK, values: NODE_VALUES },
    Program {
        name: "docker",
        station: Shell,
        subs: CONTAINER_SUBS,
        network: &["pull", "push", "login"],
        values: CONTAINER_VALUES,
    },
    Program {
        name: "podman",
        station: Shell,
        subs: CONTAINER_SUBS,
        network: &["pull", "push", "login"],
        values: CONTAINER_VALUES,
    },
    Program {
        name: "kubectl",
        station: Network,
        subs: &[
            "get",
            "describe",
            "apply",
            "delete",
            "logs",
            "exec",
            "create",
            "edit",
            "rollout",
            "scale",
            "port-forward",
            "config",
            "top",
            "patch",
            "diff",
            "explain",
            "version",
            "cp",
            "label",
            "annotate",
            "wait",
        ],
        network: &[],
        values: &[
            "--token",
            "-n",
            "--namespace",
            "--context",
            "--kubeconfig",
            "-s",
            "--server",
            "--user",
            "--cluster",
            "--as",
            "--as-group",
            "--certificate-authority",
            "--client-certificate",
            "--client-key",
            "--password",
            "--username",
            "--request-timeout",
            "--cache-dir",
            "--tls-server-name",
        ],
    },
    Program {
        name: "helm",
        station: Network,
        subs: &[
            "install",
            "upgrade",
            "uninstall",
            "list",
            "repo",
            "template",
            "lint",
            "status",
            "rollback",
            "pull",
            "push",
        ],
        network: &[],
        values: &["-n", "--namespace", "--kube-context", "--kubeconfig", "--kube-token", "--kube-apiserver"],
    },
    Program {
        name: "brew",
        station: Shell,
        subs: &[
            "install",
            "uninstall",
            "update",
            "upgrade",
            "list",
            "info",
            "search",
            "tap",
            "services",
            "fetch",
            "doctor",
        ],
        network: &["install", "update", "upgrade", "fetch", "tap", "search"],
        values: &[],
    },
    Program {
        name: "go",
        station: Shell,
        subs: &[
            "build", "test", "run", "get", "install", "mod", "fmt", "vet", "generate", "work", "clean", "list", "env",
        ],
        network: &["get", "install"],
        values: &["-C"],
    },
    Program {
        name: "make",
        station: Shell,
        subs: &[
            "all", "build", "test", "check", "clean", "install", "lint", "fmt", "run", "dev", "release", "dist", "deps",
        ],
        network: &[],
        values: &["-C", "-f", "-j", "-I", "-o", "-W", "--directory", "--file", "--jobs"],
    },
    Program {
        name: "just",
        station: Shell,
        subs: &["build", "test", "check", "clean", "install", "lint", "fmt", "run", "dev", "release"],
        network: &[],
        values: &["-f", "--justfile", "-d", "--working-directory", "--set"],
    },
    Program { name: "pip", station: Shell, subs: PIP_SUBS, network: &["install", "download"], values: &[] },
    Program { name: "pip3", station: Shell, subs: PIP_SUBS, network: &["install", "download"], values: &[] },
    Program {
        name: "uv",
        station: Shell,
        subs: &["add", "remove", "sync", "lock", "run", "pip", "venv", "tool", "python", "build", "publish", "init"],
        network: &["add", "sync", "pip", "publish", "lock"],
        values: &["--directory", "--project", "--config-file"],
    },
    Program {
        name: "gem",
        station: Shell,
        subs: &["install", "uninstall", "list", "build", "push"],
        network: &["install", "push"],
        values: &[],
    },
    Program {
        name: "terraform",
        station: Shell,
        subs: &["init", "plan", "apply", "destroy", "fmt", "validate", "output", "state", "import", "workspace"],
        network: &["init", "plan", "apply", "destroy", "import"],
        values: &["-chdir"],
    },
    Program {
        name: "dotnet",
        station: Shell,
        subs: &["build", "test", "run", "restore", "publish", "pack", "clean", "new", "add", "format", "tool"],
        network: &["restore", "publish"],
        values: &[],
    },
    Program {
        name: "rustup",
        station: Network,
        subs: &["update", "install", "default", "show", "toolchain", "target", "component", "run", "which"],
        network: &[],
        values: &[],
    },
    // Programs that only read, search, move, delete or edit files.
    plain("cat", Files),
    plain("head", Files),
    plain("tail", Files),
    plain("less", Files),
    plain("more", Files),
    plain("nl", Files),
    plain("ls", Files),
    plain("grep", Files),
    plain("egrep", Files),
    plain("fgrep", Files),
    plain("rg", Files),
    plain("ag", Files),
    plain("find", Files),
    plain("fd", Files),
    plain("mv", Files),
    plain("cp", Files),
    plain("rm", Files),
    plain("rmdir", Files),
    plain("mkdir", Files),
    plain("touch", Files),
    plain("sed", Files),
    plain("awk", Files),
    plain("tee", Files),
    plain("wc", Files),
    plain("sort", Files),
    plain("uniq", Files),
    plain("cut", Files),
    plain("tr", Files),
    plain("tree", Files),
    plain("stat", Files),
    plain("file", Files),
    plain("diff", Files),
    plain("ln", Files),
    plain("chmod", Files),
    plain("chown", Files),
    plain("du", Files),
    plain("df", Files),
    plain("basename", Files),
    plain("dirname", Files),
    plain("realpath", Files),
    plain("readlink", Files),
    plain("jq", Files),
    plain("yq", Files),
    plain("xxd", Files),
    plain("od", Files),
    plain("strings", Files),
    plain("tar", Files),
    plain("zip", Files),
    plain("unzip", Files),
    plain("gzip", Files),
    plain("gunzip", Files),
    plain("bat", Files),
    plain("patch", Files),
    plain("cmp", Files),
    plain("comm", Files),
    plain("column", Files),
    plain("shasum", Files),
    plain("md5", Files),
    plain("plutil", Files),
    // Programs that reach the network whatever their arguments.
    plain("curl", Network),
    plain("wget", Network),
    plain("ssh", Network),
    plain("scp", Network),
    plain("sftp", Network),
    plain("rsync", Network),
    plain("nc", Network),
    plain("ncat", Network),
    plain("telnet", Network),
    plain("ftp", Network),
    plain("ping", Network),
    plain("dig", Network),
    plain("nslookup", Network),
    plain("host", Network),
    plain("http", Network),
    plain("https", Network),
    plain("aws", Network),
    plain("az", Network),
    plain("gcloud", Network),
    plain("psql", Network),
    plain("mysql", Network),
    plain("redis-cli", Network),
    // Other tools a label may name.
    plain("npx", Shell),
    plain("bunx", Shell),
    plain("node", Shell),
    plain("deno", Shell),
    plain("tsc", Shell),
    plain("tsx", Shell),
    plain("eslint", Shell),
    plain("prettier", Shell),
    plain("jest", Shell),
    plain("vitest", Shell),
    plain("playwright", Shell),
    plain("python", Shell),
    plain("python3", Shell),
    plain("pytest", Shell),
    plain("ruff", Shell),
    plain("mypy", Shell),
    plain("ruby", Shell),
    plain("bundle", Shell),
    plain("rake", Shell),
    plain("rustc", Shell),
    plain("rustfmt", Shell),
    plain("gcc", Shell),
    plain("cc", Shell),
    plain("clang", Shell),
    plain("cmake", Shell),
    plain("ninja", Shell),
    plain("swift", Shell),
    plain("xcodebuild", Shell),
    plain("xcrun", Shell),
    plain("codesign", Shell),
    plain("java", Shell),
    plain("javac", Shell),
    plain("mvn", Shell),
    plain("gradle", Shell),
    plain("bash", Shell),
    plain("sh", Shell),
    plain("zsh", Shell),
    plain("fish", Shell),
    plain("pwsh", Shell),
    plain("open", Shell),
    plain("osascript", Shell),
    plain("date", Shell),
    plain("which", Shell),
    plain("ps", Shell),
    plain("kill", Shell),
    plain("pkill", Shell),
    plain("pgrep", Shell),
    plain("lsof", Shell),
    plain("top", Shell),
    plain("uname", Shell),
    plain("whoami", Shell),
    plain("sqlite3", Shell),
    plain("tmux", Shell),
    plain("vim", Shell),
    plain("nvim", Shell),
    plain("code", Shell),
    plain("claude", Shell),
    plain("codex", Shell),
    plain("ottospace", Shell),
    plain("osi", Shell),
    plain("rtk", Shell),
    plain("defaults", Shell),
    plain("launchctl", Shell),
    plain("hdiutil", Shell),
    plain("ditto", Shell),
    plain("say", Shell),
    plain("pbcopy", Shell),
    plain("pbpaste", Shell),
    plain("perl", Shell),
    plain("php", Shell),
    plain("lua", Shell),
    plain("shellcheck", Shell),
    plain("actionlint", Shell),
    plain("hyperfine", Shell),
];

/// Words in front of a command that run it, as (name, options that take a value, words to skip after the options).
/// e.g. `sudo -u deploy <cmd>`, `timeout -s KILL 30 <cmd>`, `xargs -I {} <cmd>`, `if <cmd>`.
const WRAPPERS: &[(&str, &[&str], usize)] = &[
    ("sudo", &["-u", "-g", "-h", "-p", "-C", "-D", "-R", "-T", "-r", "-t", "-U"], 0),
    ("env", &["-u", "-C", "-S", "-P"], 0),
    ("timeout", &["-s", "-k", "--signal", "--kill-after"], 1),
    ("xargs", &["-I", "-n", "-P", "-L", "-s", "-E", "-d", "-a", "-J", "-R"], 0),
    ("nice", &["-n"], 0),
    ("nohup", &[], 0),
    ("time", &[], 0),
    ("command", &[], 0),
    ("exec", &[], 0),
    ("if", &[], 0),
    ("then", &[], 0),
    ("else", &[], 0),
    ("elif", &[], 0),
    ("do", &[], 0),
    ("while", &[], 0),
    ("until", &[], 0),
    ("!", &[], 0),
    ("{", &[], 0),
];

/// Shell words that do nothing a station shows.
const NEUTRAL: &[&str] = &[
    "cd", "pushd", "popd", "export", "set", "unset", "local", "true", "false", ":", "echo", "printf", "fi", "done",
    "}", "for", "case", "esac", "wait", "sleep", "source", ".", "[", "[[", "test", "pwd", "exit", "return",
];

/// Shells whose `-c <line>` is read as the line.
const SHELLS: &[&str] = &["bash", "sh", "zsh", "dash", "fish"];

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.starts_with(|c: char| c.is_ascii_digit())
    })
}

fn is_redirection(word: &str) -> bool {
    word.trim_start_matches(|c: char| c.is_ascii_digit() || c == '&').starts_with(['>', '<'])
}

/// The first word that is not an option or an option's value. e.g. `["-C", "dir", "--no-pager", "push"]` → `push`.
fn first_plain<'a>(words: &[&'a str], values: &[&str]) -> Option<&'a str> {
    let mut words = words.iter();
    while let Some(word) = words.next() {
        if !word.starts_with('-') {
            return Some(word);
        }
        if values.contains(word) {
            words.next();
        }
    }
    None
}

/// The commands one list of words runs: usually one; none for `cd` and the like; several for `bash -c '<line>'`.
/// e.g. `["sudo", "-u", "deploy", "git", "-C", "x", "push"]` → [(Network, "git push")].
fn command(words: &[String], depth: u8) -> Vec<Cmd> {
    let mut rest: Vec<&str> = words.iter().map(String::as_str).filter(|w| !is_redirection(w)).collect();
    // Skip `A=b` and wrappers, with each wrapper's own options.
    loop {
        let Some(first) = rest.first().copied() else { return Vec::new() };
        if is_assignment(first) {
            rest.remove(0);
        } else if let Some((_, values, skip)) = WRAPPERS.iter().find(|w| w.0 == first) {
            rest.remove(0);
            while rest.first().is_some_and(|w| w.starts_with('-')) {
                let option = rest.remove(0);
                if values.contains(&option) && !rest.is_empty() {
                    rest.remove(0);
                }
            }
            rest.drain(..(*skip).min(rest.len()));
        } else {
            break;
        }
    }
    // A program run by its path goes by its name, e.g. `/usr/bin/curl` and `CURL.EXE` are `curl`.
    let program = rest[0].rsplit(['/', '\\']).next().unwrap_or("").to_ascii_lowercase();
    let program = program.strip_suffix(".exe").unwrap_or(&program);
    if NEUTRAL.contains(&program) || program.is_empty() {
        return Vec::new();
    }
    // `bash -lc '<line>'` is read as the line.
    if SHELLS.contains(&program)
        && depth < MAX_DEPTH
        && let Some(i) = rest.iter().position(|w| w.starts_with('-') && !w.starts_with("--") && w.ends_with('c'))
        && let Some(line) = rest.get(i + 1)
    {
        return parse(line, depth + 1);
    }
    // `find … -exec rm {} +` is read as the command it runs.
    if program == "find"
        && depth < MAX_DEPTH
        && let Some(i) = rest.iter().position(|w| matches!(*w, "-exec" | "-execdir" | "-ok"))
    {
        let inner: Vec<String> = rest[i + 1..].iter().map(|w| w.to_string()).collect();
        let inner = command(&inner, depth + 1);
        return if inner.is_empty() { vec![Cmd { station: Files, label: "find".to_string() }] } else { inner };
    }
    let Some(known) = PROGRAMS.iter().find(|p| p.name == program) else {
        return vec![Cmd { station: Shell, label: UNKNOWN.to_string() }];
    };
    let sub = first_plain(&rest[1..], known.values).filter(|sub| known.subs.contains(sub));
    let network = sub.is_some_and(|sub| known.network.contains(&sub))
        || (known.name == "npx" && rest.iter().any(|w| matches!(*w, "-y" | "--yes")));
    let station = if network { Network } else { known.station };
    // Both parts come from the tables, not from the command line.
    let sub = sub.and_then(|sub| known.subs.iter().find(|s| **s == sub));
    let label = sub.map_or(known.name.to_string(), |sub| format!("{} {sub}", known.name));
    vec![Cmd { station, label }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_go_to_their_stations() {
        let cases: &[(&str, Option<&str>, Station)] = &[
            ("Read", Some("/Users/me/app/src/main.rs"), Files),
            ("Edit", Some("/Users/me/app/src/main.rs"), Files),
            ("Grep", None, Files),
            ("apply_patch", None, Files),
            ("Read", Some("/Users/me/.claude/projects/-Users-me-app/memory/notes.md"), Memory),
            ("Write", Some("/Users/me/.codex/memories/app.md"), Memory),
            ("Bash", None, Shell),
            ("Bash", Some("cargo test"), Shell),
            ("Bash", Some("cat Cargo.toml"), Files),
            ("Bash", Some("git push origin main"), Network),
            ("exec_command", Some("rg TODO src"), Files),
            ("exec", None, Shell),
            ("WebFetch", None, Network),
            ("WebSearch", None, Network),
            ("web_search_call", None, Network),
            ("mcp__github__create_issue", None, Network),
            ("mcp__memory__search", None, Memory),
            ("mcp__playwright__browser_click", None, Browser),
            ("mcp__chrome-devtools__take_screenshot", None, Browser),
            ("Agent", None, Agents),
            ("Task", None, Agents),
            ("SendMessage", None, Agents),
            ("spawn_agent", None, Agents),
            ("wait_agent", None, Agents),
            ("Skill", None, Skills),
            ("ToolSearch", None, Skills),
            ("AskUserQuestion", None, You),
            ("update_plan", None, Other),
            ("SomethingNew", None, Other),
        ];
        for (tool, arg, want) in cases {
            assert_eq!(classify(tool, *arg), *want, "{tool} {arg:?}");
        }
    }

    /// Every word of a label is a name from the tables.
    fn from_tables(label: &str) -> bool {
        let mut words = label.split(' ');
        let program = words.next().unwrap_or("");
        let known = PROGRAMS.iter().find(|p| p.name == program);
        let sub_ok = words.all(|w| known.is_some_and(|p| p.subs.contains(&w)));
        label == UNKNOWN || (known.is_some() && sub_ok && label.split(' ').count() <= 2)
    }

    #[test]
    fn commands_go_by_name() {
        let cases: &[(&str, Station, &str)] = &[
            ("cat a.txt", Files, "cat"),
            ("grep -rn foo src | head -5", Files, "grep"),
            ("rm -rf target && mkdir out", Files, "rm"),
            ("sed -i '' 's/a;b/c|d/' f.txt", Files, "sed"),
            ("find . -name '*.rs' -delete", Files, "find"),
            ("find . -name '*.o' -exec rm {} +", Files, "rm"),
            ("find . -exec curl -T {} https://example.com \\;", Network, "curl"),
            ("cargo test --workspace", Shell, "cargo test"),
            ("cd crates && cargo build", Shell, "cargo build"),
            ("ls && cargo build", Shell, "ls"),
            ("python -c \"import urllib\"", Shell, "python"),
            ("make test", Shell, "make test"),
            ("make -C sub deploy-prod", Shell, "make"),
            ("curl -s https://example.com/a?token=secret", Network, "curl"),
            ("gh pr create --draft --title 'secret title'", Network, "gh pr"),
            ("git push -f origin main", Network, "git push"),
            ("git status", Shell, "git status"),
            ("git commit -m 'curl is mentioned; rm too'", Shell, "git commit"),
            ("npm install left-pad", Network, "npm install"),
            ("npm test", Shell, "npm test"),
            ("cargo install ripgrep", Network, "cargo install"),
            ("docker pull alpine", Network, "docker pull"),
            ("npx -y cowsay hi", Network, "npx"),
            ("npx tsc", Shell, "npx"),
            ("sudo curl example.com", Network, "curl"),
            ("FOO=1 BAR=2 env -i timeout 30 ssh host uptime", Network, "ssh"),
            ("ls | xargs rm", Files, "ls"),
            ("ls | xargs -n1 curl", Network, "ls"),
            ("nohup /usr/local/bin/wget http://example.com &", Network, "wget"),
            ("CURL.EXE http://example.com", Network, "curl"),
            ("echo \"$(curl -s example.com)\"", Network, "curl"),
            ("echo \"`curl -s example.com` done\"", Network, "curl"),
            ("x=`git fetch`", Network, "git fetch"),
            ("cargo build 2>&1 | tail -5", Shell, "cargo build"),
            ("cat a.txt > b.txt 2>/dev/null", Files, "cat"),
            ("if grep -q x f; then rm f; fi", Files, "grep"),
            ("cat > notes.md <<'EOF'\ncurl is only text here\nrm -rf /\nEOF", Files, "cat"),
            ("cat <<EOF | tee out.txt\nssh host\nEOF\ncargo test", Shell, "cat"),
            ("cargo \\\n  test", Shell, "cargo test"),
            ("./scripts/deploy.sh prod && git push", Network, "shell"),
            ("cd /tmp", Shell, "shell"),
            ("", Shell, "shell"),
            ("echo 'unterminated", Shell, "shell"),
        ];
        for (line, station, label) in cases {
            assert_eq!(describe_command(line), (*station, label.to_string()), "{line}");
            assert!(from_tables(label), "{label}");
        }
    }

    #[test]
    fn nothing_from_a_command_line_reaches_a_label() {
        // (line, station, label, what must not show)
        let cases: &[(&str, Station, &str, &str)] = &[
            ("kubectl --token s3cr3tt0k3n get pods", Network, "kubectl get", "s3cr3t"),
            ("kubectl --made-up-option s3cr3t get pods", Network, "kubectl", "s3cr3t"),
            ("npm --otp 123456 publish", Network, "npm publish", "123456"),
            ("git -C clientacme push", Network, "git push", "clientacme"),
            ("git -c http.extraheader=s3cr3t fetch", Network, "git fetch", "s3cr3t"),
            ("git s3cr3t-branch", Shell, "git", "s3cr3t"),
            ("gh SECRET123", Network, "gh", "SECRET123"),
            ("SECRET123 --flag", Shell, "shell", "SECRET123"),
            ("/opt/acme-client/bin/deploy now", Shell, "shell", "acme"),
            ("ls -la # password is hunter2; hunter2", Files, "ls", "hunter2"),
            ("cat notes.txt # token | s3cr3t", Files, "cat", "s3cr3t"),
            ("# s3cr3t\nls", Files, "ls", "s3cr3t"),
            ("TOKENS=(abc123 def456); ls", Files, "ls", "abc123"),
            ("grep foo url?a=1&s3cr3t", Shell, "grep", "s3cr3t"),
            ("TOKEN=s3cr3t curl https://x", Network, "curl", "s3cr3t"),
            ("curl -H 'Authorization: Bearer s3cr3t' https://x", Network, "curl", "s3cr3t"),
            ("cat <<\"E OF\"\ns3cr3t\nE OF\nls", Files, "cat", "s3cr3t"),
            ("cat <<$DELIM\ncurl s3cr3t\n$DELIM", Files, "cat", "s3cr3t"),
            ("cat <<-'END-1'\n\ts3cr3t; curl x\n\tEND-1\n", Files, "cat", "s3cr3t"),
            ("cat <<EOF\ns3cr3t; curl x\n", Files, "cat", "s3cr3t"),
            ("sudo -u deploy curl https://x", Network, "curl", "deploy"),
            ("timeout -s KILL 30 curl x", Network, "curl", "KILL"),
            ("xargs -I {} curl {}", Network, "curl", "{}"),
            ("bash -c 'curl https://x/s3cr3t'", Network, "curl", "s3cr3t"),
            ("bash -lc \"cd s3cr3t && git -C s3cr3t push\"", Network, "git push", "s3cr3t"),
            ("zsh -c 's3cr3t --run'", Shell, "shell", "s3cr3t"),
            ("bash s3cr3t.sh", Shell, "bash", "s3cr3t"),
            ("echo s3cr3t; s3cr3t", Shell, "shell", "s3cr3t"),
            ("s3\u{202e}cr3t\u{200b} x", Shell, "shell", "cr3t"),
        ];
        for (line, station, label, secret) in cases {
            let got = describe_command(line);
            assert_eq!(got, (*station, label.to_string()), "{line}");
            assert!(from_tables(&got.1) && !got.1.contains(secret), "{line} → {}", got.1);
        }
    }

    #[test]
    fn argv_is_not_joined_and_split_again() {
        let cases: &[(&[&str], Station, &str)] = &[
            (&["git", "push"], Network, "git push"),
            (&["git", "-C", "client acme", "push"], Network, "git push"),
            (&["/bin/zsh", "-lc", "curl -s https://x | jq ."], Network, "curl"),
            (&["bash", "-c", "ls"], Files, "ls"),
            // One word with a separator in it is one argument, not two commands.
            (&["ls", "a; curl s3cr3t"], Files, "ls"),
            (&["s3cr3t tool", "--x"], Shell, "shell"),
            (&["gh", "s3cr3t"], Network, "gh"),
            (&[], Shell, "shell"),
        ];
        for (words, station, label) in cases {
            assert_eq!(describe_argv(words), (*station, label.to_string()), "{words:?}");
        }
    }

    #[test]
    fn labels_are_names_only() {
        assert_eq!(path_label("/Users/me/app/src/main.rs"), "src/main.rs");
        assert_eq!(path_label("main.rs"), "main.rs");
        assert_eq!(path_label(""), "");
        assert_eq!(path_label("/a/b\u{202e}c/d\u{200b}e\n.rs"), "bc/de.rs");
        let hosts = [
            ("https://me:pw@example.com:8080/a/b?q=secret#f", "example.com"),
            ("example.com/path", "example.com"),
            ("https://x.example\\@evil.example/", "x.example"),
            ("https://user@evil.example\\@x.example/", "evil.example"),
            ("https://s3cr3t:pw@/path", "web"),
            ("http://[::1]:8080/a", "web"),
            ("https://exa mple.com/", "web"),
            ("https://ex\u{202e}ample.com/", "web"),
            ("https://éxample.com/", "web"),
            ("", "web"),
        ];
        for (url, host) in hosts {
            assert_eq!(host_label(url), host, "{url}");
        }
        assert_eq!(clean("a\u{1b}[31mb\nc\u{202e}d\u{200b}é"), "a[31mbcd");
        assert_eq!(clean(&"x".repeat(200)).len(), 60);
        assert_eq!(name("code-review", "skill"), "code-review");
        assert_eq!(name("ns:tool_1.2", "skill"), "ns:tool_1.2");
        for bad in ["", "review my secret", "a/b", "é", "a\u{200b}b", &"x".repeat(65)] {
            assert_eq!(name(bad, "skill"), "skill", "{bad}");
        }
        assert_eq!(code("rate_limit", "error"), "rate_limit");
        for bad in ["", "Rate limit", "rate-limit", "key sk-1 is over its limit", &"x".repeat(41)] {
            assert_eq!(code(bad, "error"), "error", "{bad}");
        }
    }
}
