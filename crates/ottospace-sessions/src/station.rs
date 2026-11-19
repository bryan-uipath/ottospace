//! Which station a tool call belongs to, and its short label. Pure.
//!
//! A label for a command is built only from the tables in this file: a known program's name,
//! and a subcommand on that program's list. Nothing else from a command line is ever shown.

use ottospace_core::activity::Station;

use Station::*;

/// The label of a command nothing is known about.
const UNKNOWN: &str = "shell";

/// The station of a tool call. `arg` is the command line of a shell tool or the path of a file tool.
/// e.g. `("Bash", Some("git push"))` → Shell, `("Read", Some("/a/b.rs"))` → Files.
pub fn classify(tool: &str, arg: Option<&str>) -> Station {
    // MCP tools are `mcp__<server>__<tool>`: MCP whatever the server, never guessed from its name.
    if tool.starts_with("mcp__") {
        return Mcp;
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
        | "local_shell_call" | "exec" | "write_stdin" => Shell,
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

/// A shell command line's label: its first command's name, e.g. `cd x && git -C y push -f` →
/// `git push`. A command always lights Shell: what it did is never guessed from its line.
pub fn command_label(line: &str) -> String {
    label(&parse(line, 0))
}

/// The same for a command already split into words, e.g. `["git", "push"]`. The words are never
/// joined and split again. `["zsh", "-lc", "<line>"]` is read as the line.
pub fn argv_label(words: &[&str]) -> String {
    let words: Vec<String> = words.iter().map(|w| w.to_string()).collect();
    label(&command(&words, 0))
}

/// The first command names the call, known or not: never a later one in its place.
fn label(commands: &[String]) -> String {
    commands.first().map_or(UNKNOWN.to_string(), String::clone)
}

/// A file's label: the last two parts of its path, e.g. `/Users/me/app/src/main.rs` → `src/main.rs`.
/// Only for something with a path's shape: it has a `/` and no `://`, and each part shown is at
/// most 40 of `A-Z a-z 0-9 space . _ + @ , ( ) -`. A string with no `/` is shown if it is a file's
/// name: 1 to 64 of `A-Z a-z 0-9 _ . -`. Anything else gives `file`.
pub fn path_label(path: &str) -> String {
    const FILE: &str = "file";
    if path.contains("://") {
        return FILE.to_string();
    }
    if !path.contains('/') {
        let named =
            (1..=64).contains(&path.len()) && path.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c));
        return if named { path } else { FILE }.to_string();
    }
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let shown = &parts[parts.len().saturating_sub(2)..];
    let plain =
        |part: &&str| part.len() <= 40 && part.chars().all(|c| c.is_ascii_alphanumeric() || " ._+@,()-".contains(c));
    if shown.is_empty() || !shown.iter().all(plain) { FILE.to_string() } else { shown.join("/") }
}

/// A web address's label: the host alone, e.g. `https://example.com:8080/a?b` → `example.com`.
/// Only from `scheme://host`, where the host is letters, digits, `.` and `-` with a dot in it.
/// An address with an `@` anywhere after the scheme gives `web`: where userinfo ends cannot be
/// told safely (`https://user:pa/ss@host`). So do an IPv6 address and anything else.
pub fn host_label(url: &str) -> String {
    const WEB: &str = "web";
    let Some((scheme, rest)) = url.split_once("://") else { return WEB.to_string() };
    let schemed = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c));
    if !schemed || rest.contains('@') {
        return WEB.to_string();
    }
    // Browsers read `\` as `/`.
    let authority = rest.split(['/', '\\', '?', '#']).next().unwrap_or("");
    let (host, port) = authority.split_once(':').unwrap_or((authority, ""));
    let plain = host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && port.chars().all(|c| c.is_ascii_digit());
    let shown = plain && host.contains('.') && host.len() <= 60 && !looks_secret(host);
    if shown { host } else { WEB }.to_string()
}

/// True for a value shaped like a credential: a well-known token prefix, or a `:` with 8 or more
/// characters after it (`user:password`).
fn looks_secret(text: &str) -> bool {
    const PREFIXES: &[&str] = &["ghp_", "gho_", "github_pat_", "sk-", "xox", "AKIA", "eyJ"];
    PREFIXES.iter().any(|p| text.starts_with(p)) || text.split_once(':').is_some_and(|(_, after)| after.len() >= 8)
}

/// True if `text` has the shape of a name and not of a credential: 1 to 40 ASCII letters, digits
/// or `_ . : -`, e.g. `code-review`. Not `review my notes`, `ghp_abc`, `me:hunter2hunter2`.
pub fn is_name(text: &str) -> bool {
    (1..=40).contains(&text.len())
        && text.chars().all(|c| c.is_ascii_alphanumeric() || "_.:-".contains(c))
        && !looks_secret(text)
}

/// `text` if it is a name (`is_name`), else `fallback`. The text is still what a model or a file
/// said: a name-shaped value is shown as written.
pub fn name(text: &str, fallback: &str) -> String {
    if is_name(text) { text } else { fallback }.to_string()
}

/// `text` if it is one of the `known` error codes, else `fallback`. e.g. `rate_limit` is known;
/// `rate limit for key sk-1` and `zz_made_up_code` are not.
pub fn code(text: &str, known: &[&'static str], fallback: &'static str) -> String {
    known.iter().find(|k| **k == text).copied().unwrap_or(fallback).to_string()
}

/// The runners' built-in tools. Any other tool that is not an MCP tool is shown as `tool`.
const TOOLS: &[&str] = &[
    // Claude Code
    "Read",
    "Write",
    "Edit",
    "MultiEdit",
    "NotebookEdit",
    "Grep",
    "Glob",
    "LS",
    "Bash",
    "BashOutput",
    "KillShell",
    "KillBash",
    "PowerShell",
    "Monitor",
    "WebFetch",
    "WebSearch",
    "Agent",
    "Task",
    "SendMessage",
    "TaskStop",
    "TaskOutput",
    "TaskCreate",
    "TaskUpdate",
    "TaskGet",
    "TaskList",
    "Workflow",
    "Skill",
    "ToolSearch",
    "AskUserQuestion",
    "TodoWrite",
    "EnterPlanMode",
    "ExitPlanMode",
    "EnterWorktree",
    "ExitWorktree",
    "Artifact",
    "ArtifactComments",
    "ArtifactData",
    "StructuredOutput",
    "CronCreate",
    "CronDelete",
    "CronList",
    "LSP",
    "ListMcpResourcesTool",
    "ReadMcpResourceTool",
    // Codex
    "exec",
    "exec_command",
    "shell",
    "local_shell_call",
    "write_stdin",
    "apply_patch",
    "view_image",
    "update_plan",
    "web_search",
    "web_search_call",
    "tool_search_call",
    "spawn_agent",
    "send_message",
    "wait_agent",
    "close_agent",
    "sleep",
];

/// A tool's label: a built-in tool's name from the table; `server/tool` for an MCP tool whose
/// two names are names (else `mcp`); `tool` for anything else.
pub fn tool_label(tool: &str) -> String {
    if let Some(rest) = tool.strip_prefix("mcp__") {
        return match rest.split_once("__") {
            Some((server, tool)) if is_name(server) && is_name(tool) => format!("{server}/{tool}"),
            _ => "mcp".to_string(),
        };
    }
    TOOLS.iter().find(|t| **t == tool).copied().unwrap_or("tool").to_string()
}

/// A longer command line is read by its first part only. The label is the first command's anyway.
const MAX_CHARS: usize = 256 << 10;

/// A shell inside a shell is followed this deep, e.g. `bash -c "sh -c 'curl x'"`.
const MAX_DEPTH: u8 = 3;

/// Every command in a line, in order, by its label. `cd`, `echo` and the like are left out.
fn parse(line: &str, depth: u8) -> Vec<String> {
    split(line).iter().flat_map(|words| command(words, depth)).collect()
}

/// Splits a line into commands, each a list of words. Separators: `; | & ( )`, a new line, a backtick,
/// and `$(`, also inside double quotes. Quotes are honoured. Dropped: `# comments`, heredoc bodies
/// and array values (`A=(x y)`), which are data, not commands.
fn split(line: &str) -> Vec<Vec<String>> {
    let chars: Vec<char> = line.chars().take(MAX_CHARS).collect();
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

/// A program the labels may name. `subs`: the subcommands a label may add. `values`: options that
/// take the next word as their value, so `git -C dir push` finds `push`.
struct Program {
    name: &'static str,
    subs: &'static [&'static str],
    values: &'static [&'static str],
}

const fn plain(name: &'static str) -> Program {
    Program { name, subs: &[], values: &[] }
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
        values: &["-C", "-c", "--git-dir", "--work-tree", "--namespace", "--exec-path", "--config-env"],
    },
    Program {
        name: "gh",
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
        values: &["-R", "--repo", "--hostname"],
    },
    Program {
        name: "cargo",
        subs: &[
            "build", "check", "test", "run", "clippy", "fmt", "doc", "bench", "clean", "update", "install", "add",
            "remove", "publish", "fetch", "search", "tree", "metadata", "new", "init", "e2e", "e2e-apps", "nextest",
            "expand", "fix", "vendor",
        ],
        values: &["-C", "--config", "--color", "-Z", "--manifest-path"],
    },
    Program { name: "npm", subs: NODE_SUBS, values: NODE_VALUES },
    Program { name: "pnpm", subs: NODE_SUBS, values: NODE_VALUES },
    Program { name: "yarn", subs: NODE_SUBS, values: NODE_VALUES },
    Program { name: "bun", subs: NODE_SUBS, values: NODE_VALUES },
    Program { name: "docker", subs: CONTAINER_SUBS, values: CONTAINER_VALUES },
    Program { name: "podman", subs: CONTAINER_SUBS, values: CONTAINER_VALUES },
    Program {
        name: "kubectl",
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
        values: &["-n", "--namespace", "--kube-context", "--kubeconfig", "--kube-token", "--kube-apiserver"],
    },
    Program {
        name: "brew",
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
        values: &[],
    },
    Program {
        name: "go",
        subs: &[
            "build", "test", "run", "get", "install", "mod", "fmt", "vet", "generate", "work", "clean", "list", "env",
        ],
        values: &["-C"],
    },
    Program {
        name: "make",
        subs: &[
            "all", "build", "test", "check", "clean", "install", "lint", "fmt", "run", "dev", "release", "dist", "deps",
        ],
        values: &["-C", "-f", "-j", "-I", "-o", "-W", "--directory", "--file", "--jobs"],
    },
    Program {
        name: "just",
        subs: &["build", "test", "check", "clean", "install", "lint", "fmt", "run", "dev", "release"],
        values: &["-f", "--justfile", "-d", "--working-directory", "--set"],
    },
    Program { name: "pip", subs: PIP_SUBS, values: &[] },
    Program { name: "pip3", subs: PIP_SUBS, values: &[] },
    Program {
        name: "uv",
        subs: &["add", "remove", "sync", "lock", "run", "pip", "venv", "tool", "python", "build", "publish", "init"],
        values: &["--directory", "--project", "--config-file"],
    },
    Program { name: "gem", subs: &["install", "uninstall", "list", "build", "push"], values: &[] },
    Program {
        name: "terraform",
        subs: &["init", "plan", "apply", "destroy", "fmt", "validate", "output", "state", "import", "workspace"],
        values: &["-chdir"],
    },
    Program {
        name: "dotnet",
        subs: &["build", "test", "run", "restore", "publish", "pack", "clean", "new", "add", "format", "tool"],
        values: &[],
    },
    Program {
        name: "rustup",
        subs: &["update", "install", "default", "show", "toolchain", "target", "component", "run", "which"],
        values: &[],
    },
    // Programs that work on files.
    plain("cat"),
    plain("head"),
    plain("tail"),
    plain("less"),
    plain("more"),
    plain("nl"),
    plain("ls"),
    plain("grep"),
    plain("egrep"),
    plain("fgrep"),
    plain("rg"),
    plain("ag"),
    plain("find"),
    plain("fd"),
    plain("mv"),
    plain("cp"),
    plain("rm"),
    plain("rmdir"),
    plain("mkdir"),
    plain("touch"),
    plain("sed"),
    plain("awk"),
    plain("tee"),
    plain("wc"),
    plain("sort"),
    plain("uniq"),
    plain("cut"),
    plain("tr"),
    plain("tree"),
    plain("stat"),
    plain("file"),
    plain("diff"),
    plain("ln"),
    plain("chmod"),
    plain("chown"),
    plain("du"),
    plain("df"),
    plain("basename"),
    plain("dirname"),
    plain("realpath"),
    plain("readlink"),
    plain("jq"),
    plain("yq"),
    plain("xxd"),
    plain("od"),
    plain("strings"),
    plain("tar"),
    plain("zip"),
    plain("unzip"),
    plain("gzip"),
    plain("gunzip"),
    plain("bat"),
    plain("patch"),
    plain("cmp"),
    plain("comm"),
    plain("column"),
    plain("shasum"),
    plain("md5"),
    plain("plutil"),
    // Network clients.
    plain("curl"),
    plain("wget"),
    plain("ssh"),
    plain("scp"),
    plain("sftp"),
    plain("rsync"),
    plain("nc"),
    plain("ncat"),
    plain("telnet"),
    plain("ftp"),
    plain("ping"),
    plain("dig"),
    plain("nslookup"),
    plain("host"),
    plain("http"),
    plain("https"),
    plain("aws"),
    plain("az"),
    plain("gcloud"),
    plain("psql"),
    plain("mysql"),
    plain("redis-cli"),
    // Other tools a label may name.
    plain("npx"),
    plain("bunx"),
    plain("node"),
    plain("deno"),
    plain("tsc"),
    plain("tsx"),
    plain("eslint"),
    plain("prettier"),
    plain("jest"),
    plain("vitest"),
    plain("playwright"),
    plain("python"),
    plain("python3"),
    plain("pytest"),
    plain("ruff"),
    plain("mypy"),
    plain("ruby"),
    plain("bundle"),
    plain("rake"),
    plain("rustc"),
    plain("rustfmt"),
    plain("gcc"),
    plain("cc"),
    plain("clang"),
    plain("cmake"),
    plain("ninja"),
    plain("swift"),
    plain("xcodebuild"),
    plain("xcrun"),
    plain("codesign"),
    plain("java"),
    plain("javac"),
    plain("mvn"),
    plain("gradle"),
    plain("bash"),
    plain("sh"),
    plain("zsh"),
    plain("fish"),
    plain("pwsh"),
    plain("open"),
    plain("osascript"),
    plain("date"),
    plain("which"),
    plain("ps"),
    plain("kill"),
    plain("pkill"),
    plain("pgrep"),
    plain("lsof"),
    plain("top"),
    plain("uname"),
    plain("whoami"),
    plain("sqlite3"),
    plain("tmux"),
    plain("vim"),
    plain("nvim"),
    plain("code"),
    plain("claude"),
    plain("codex"),
    plain("ottospace"),
    plain("osi"),
    plain("rtk"),
    plain("defaults"),
    plain("launchctl"),
    plain("hdiutil"),
    plain("ditto"),
    plain("say"),
    plain("pbcopy"),
    plain("pbpaste"),
    plain("perl"),
    plain("php"),
    plain("lua"),
    plain("shellcheck"),
    plain("actionlint"),
    plain("hyperfine"),
];

/// Words in front of a command that run it, as (name, options that take a value, words to skip after the options).
/// e.g. `sudo -u deploy <cmd>`, `timeout -s KILL 30 <cmd>`, `xargs -I {} <cmd>`, `if <cmd>`.
const WRAPPERS: &[(&str, &[&str], usize)] = &[
    ("sudo", &["-u", "-g", "-h", "-p", "-C", "-D", "-R", "-T", "-r", "-t", "-U"], 0),
    ("env", &["-u", "-C", "-S", "--split-string", "-P"], 0),
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
/// e.g. `["sudo", "-u", "deploy", "git", "-C", "x", "push"]` → ["git push"].
fn command(words: &[String], depth: u8) -> Vec<String> {
    let all: Vec<&str> = words.iter().map(String::as_str).filter(|w| !is_redirection(w)).collect();
    // Skip `A=b` and wrappers, with each wrapper's own options. By index: a line may hold very many of them.
    let mut at = 0;
    loop {
        let Some(first) = all.get(at).copied() else { return Vec::new() };
        if is_assignment(first) {
            at += 1;
        } else if let Some((wrapper, values, skip)) = WRAPPERS.iter().find(|w| w.0 == first) {
            at += 1;
            while let Some(option) = all.get(at).copied().filter(|w| w.starts_with('-')) {
                at += 1;
                if !values.contains(&option) {
                    continue;
                }
                // `env -S '<line>'` runs the line.
                if *wrapper == "env" && matches!(option, "-S" | "--split-string") {
                    return all.get(at).filter(|_| depth < MAX_DEPTH).map_or(Vec::new(), |line| parse(line, depth + 1));
                }
                at += 1;
            }
            at += skip;
        } else {
            break;
        }
    }
    let rest = &all[at..];
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
        return if inner.is_empty() { vec!["find".to_string()] } else { inner };
    }
    let Some(known) = PROGRAMS.iter().find(|p| p.name == program) else {
        return vec![UNKNOWN.to_string()];
    };
    let sub = first_plain(&rest[1..], known.values).filter(|sub| known.subs.contains(sub));
    // Both parts come from the tables, not from the command line.
    let sub = sub.and_then(|sub| known.subs.iter().find(|s| **s == sub));
    let label = sub.map_or(known.name.to_string(), |sub| format!("{} {sub}", known.name));
    vec![label]
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
            // A terminal command lights Shell, never Files: only the file tools above do.
            ("Bash", Some("ls -la"), Shell),
            ("Bash", Some("cat Cargo.toml"), Shell),
            ("exec_command", Some("rg TODO src"), Shell),
            ("exec", None, Shell),
            ("WebFetch", None, Network),
            ("WebSearch", None, Network),
            ("web_search_call", None, Network),
            ("mcp__github__create_issue", None, Mcp),
            ("mcp__memory__search", None, Mcp),
            ("mcp__playwright__browser_click", None, Mcp),
            ("mcp__chrome-devtools__take_screenshot", None, Mcp),
            ("mcp__browser__open", None, Mcp),
            ("mcp__puppeteer__goto", None, Mcp),
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

    /// A command lights Shell whatever it runs: whether it reached the network is not guessed.
    #[test]
    fn a_shell_command_always_lights_shell() {
        let lines = [
            "curl -s https://example.com",
            "git push origin main",
            "ssh host uptime",
            "npm install left-pad",
            "npx -y cowsay hi",
            "gh pr view 1",
            "kubectl get pods",
            "ls | xargs -n1 curl",
            "cargo build && git fetch | tee log; ssh host 'curl x'",
            "bash -lc 'wget http://example.com'",
        ];
        for tool in ["Bash", "PowerShell", "exec_command", "shell", "local_shell_call", "exec", "write_stdin"] {
            for line in lines {
                assert_eq!(classify(tool, Some(line)), Shell, "{tool} {line}");
            }
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
        let cases: &[(&str, &str)] = &[
            ("cat a.txt", "cat"),
            ("grep -rn foo src | head -5", "grep"),
            ("rm -rf target && mkdir out", "rm"),
            ("sed -i '' 's/a;b/c|d/' f.txt", "sed"),
            ("find . -name '*.rs' -delete", "find"),
            ("find . -name '*.o' -exec rm {} +", "rm"),
            ("find . -exec curl -T {} https://example.com \\;", "curl"),
            ("cargo test --workspace", "cargo test"),
            ("cd crates && cargo build", "cargo build"),
            ("ls && cargo build", "ls"),
            ("python -c \"import urllib\"", "python"),
            ("make test", "make test"),
            ("make -C sub deploy-prod", "make"),
            ("curl -s https://example.com/a?token=secret", "curl"),
            ("gh pr create --draft --title 'secret title'", "gh pr"),
            ("git push -f origin main", "git push"),
            ("git status", "git status"),
            ("git commit -m 'curl is mentioned; rm too'", "git commit"),
            ("npm install left-pad", "npm install"),
            ("npm test", "npm test"),
            ("cargo install ripgrep", "cargo install"),
            ("docker pull alpine", "docker pull"),
            ("npx -y cowsay hi", "npx"),
            ("npx tsc", "npx"),
            ("sudo curl example.com", "curl"),
            ("FOO=1 BAR=2 env -i timeout 30 ssh host uptime", "ssh"),
            ("ls | xargs rm", "ls"),
            ("ls | xargs -n1 curl", "ls"),
            ("nohup /usr/local/bin/wget http://example.com &", "wget"),
            ("CURL.EXE http://example.com", "curl"),
            ("echo \"$(curl -s example.com)\"", "curl"),
            ("echo \"`curl -s example.com` done\"", "curl"),
            ("x=`git fetch`", "git fetch"),
            ("cargo build 2>&1 | tail -5", "cargo build"),
            ("cat a.txt > b.txt 2>/dev/null", "cat"),
            ("if grep -q x f; then rm f; fi", "grep"),
            ("cat > notes.md <<'EOF'\ncurl is only text here\nrm -rf /\nEOF", "cat"),
            ("cat <<EOF | tee out.txt\nssh host\nEOF\ncargo test", "cat"),
            ("cargo \\\n  test", "cargo test"),
            ("./scripts/deploy.sh prod && git push", "shell"),
            ("cd /tmp", "shell"),
            ("", "shell"),
            ("echo 'unterminated", "shell"),
        ];
        for (line, label) in cases {
            assert_eq!(command_label(line), *label, "{line}");
            assert!(from_tables(label), "{label}");
        }
    }

    #[test]
    fn nothing_from_a_command_line_reaches_a_label() {
        // (line, label, what must not show)
        let cases: &[(&str, &str, &str)] = &[
            ("kubectl --token s3cr3tt0k3n get pods", "kubectl get", "s3cr3t"),
            ("kubectl --made-up-option s3cr3t get pods", "kubectl", "s3cr3t"),
            ("npm --otp 123456 publish", "npm publish", "123456"),
            ("git -C clientacme push", "git push", "clientacme"),
            ("git -c http.extraheader=s3cr3t fetch", "git fetch", "s3cr3t"),
            ("git s3cr3t-branch", "git", "s3cr3t"),
            ("gh SECRET123", "gh", "SECRET123"),
            ("SECRET123 --flag", "shell", "SECRET123"),
            ("/opt/acme-client/bin/deploy now", "shell", "acme"),
            ("ls -la # password is hunter2; hunter2", "ls", "hunter2"),
            ("cat notes.txt # token | s3cr3t", "cat", "s3cr3t"),
            ("# s3cr3t\nls", "ls", "s3cr3t"),
            ("TOKENS=(abc123 def456); ls", "ls", "abc123"),
            ("grep foo url?a=1&s3cr3t", "grep", "s3cr3t"),
            ("TOKEN=s3cr3t curl https://x", "curl", "s3cr3t"),
            ("curl -H 'Authorization: Bearer s3cr3t' https://x", "curl", "s3cr3t"),
            ("cat <<\"E OF\"\ns3cr3t\nE OF\nls", "cat", "s3cr3t"),
            ("cat <<$DELIM\ncurl s3cr3t\n$DELIM", "cat", "s3cr3t"),
            ("cat <<-'END-1'\n\ts3cr3t; curl x\n\tEND-1\n", "cat", "s3cr3t"),
            ("cat <<EOF\ns3cr3t; curl x\n", "cat", "s3cr3t"),
            ("sudo -u deploy curl https://x", "curl", "deploy"),
            ("timeout -s KILL 30 curl x", "curl", "KILL"),
            ("xargs -I {} curl {}", "curl", "{}"),
            ("bash -c 'curl https://x/s3cr3t'", "curl", "s3cr3t"),
            ("bash -lc \"cd s3cr3t && git -C s3cr3t push\"", "git push", "s3cr3t"),
            ("zsh -c 's3cr3t --run'", "shell", "s3cr3t"),
            ("bash s3cr3t.sh", "bash", "s3cr3t"),
            ("echo s3cr3t; s3cr3t", "shell", "s3cr3t"),
            ("s3\u{202e}cr3t\u{200b} x", "shell", "cr3t"),
        ];
        for (line, label, secret) in cases {
            let got = command_label(line);
            assert_eq!(got, *label, "{line}");
            assert!(from_tables(&got) && !got.contains(secret), "{line} → {got}");
        }
    }

    #[test]
    fn argv_is_not_joined_and_split_again() {
        let cases: &[(&[&str], &str)] = &[
            (&["git", "push"], "git push"),
            (&["git", "-C", "client acme", "push"], "git push"),
            (&["/bin/zsh", "-lc", "curl -s https://x | jq ."], "curl"),
            (&["bash", "-c", "ls"], "ls"),
            // One word with a separator in it is one argument, not two commands.
            (&["ls", "a; curl s3cr3t"], "ls"),
            (&["s3cr3t tool", "--x"], "shell"),
            (&["gh", "s3cr3t"], "gh"),
            (&[], "shell"),
        ];
        for (words, label) in cases {
            assert_eq!(argv_label(words), *label, "{words:?}");
        }
    }

    #[test]
    fn a_file_label_needs_a_paths_shape() {
        let cases = [
            ("/Users/me/app/src/main.rs", "src/main.rs"),
            ("src/main.rs", "src/main.rs"),
            ("./notes (old)/a+b, v2@home.txt", "notes (old)/a+b, v2@home.txt"),
            ("main.rs", "main.rs"),
            ("/main.rs", "main.rs"),
            // Free text, with and without a slash in it.
            ("please read the secret plan for acme", "file"),
            ("the key is hunter2: see notes/2026 & more", "file"),
            ("/tmp/API_KEY=sk-ant-abcdef.txt", "file"),
            ("/tmp/a=b/c.txt", "file"),
            ("https://user:hunter2@example.com/a/b.txt", "file"),
            ("C:\\Users\\me\\secret.txt", "file"),
            ("/a/b\u{202e}c/d.rs", "file"),
            ("/a/b/d\u{200b}e\n.rs", "file"),
            ("/a/é/b.rs", "file"),
            ("", "file"),
            ("/", "file"),
        ];
        for (path, label) in cases {
            assert_eq!(path_label(path), label, "{path}");
        }
        assert_eq!(path_label(&format!("/a/{}/b.rs", "x".repeat(41))), "file");
        assert_eq!(path_label(&format!("/a/{}/b.rs", "x".repeat(40))).len(), 45);
        assert_eq!(path_label(&"x".repeat(65)), "file");
    }

    #[test]
    fn a_host_is_shown_only_from_a_plain_address() {
        let hosts = [
            ("https://example.com:8080/a/b?q=secret#f", "example.com"),
            ("http://sub.example.co.uk", "sub.example.co.uk"),
            ("https://x.example\\evil/", "x.example"),
            // Userinfo, wherever it seems to end.
            ("https://me:pw@example.com:8080/a", "web"),
            ("https://ZQuser:pa/ss@ZQhost.example/x", "web"),
            ("https://ZQtok/en@github.com/x", "web"),
            ("https://ZQuser:p?ss@host", "web"),
            ("https://ZQuser#x@host", "web"),
            ("https://x.example\\@evil.example/", "web"),
            ("https://example.com/a?mail=me@corp.example", "web"),
            // Not an address.
            ("ZQbare-token-no-scheme", "web"),
            ("sk-ant-ZQ03-abcdef", "web"),
            ("mailto:ZQme@ZQcorp.example", "web"),
            ("example.com/path", "web"),
            ("://example.com", "web"),
            ("1http://example.com", "web"),
            ("https://localhost/", "web"),
            ("https://ZQtoken/", "web"),
            ("https://sk-ant.example/", "web"),
            ("https://example.com:80a/", "web"),
            ("http://[::1]:8080/a", "web"),
            ("https://exa mple.com/", "web"),
            ("https://ex\u{202e}ample.com/", "web"),
            ("https://éxample.com/", "web"),
            ("", "web"),
        ];
        for (url, host) in hosts {
            assert_eq!(host_label(url), host, "{url}");
        }
    }

    #[test]
    fn a_name_is_not_free_text_and_not_a_credential() {
        for good in ["code-review", "Explore", "ns:tool", "claude-opus-5-5", "gpt-5.4", &"x".repeat(40)] {
            assert_eq!(name(good, "skill"), good);
        }
        let bad = [
            "",
            "review my secret",
            "a/b",
            "é",
            "a\u{200b}b",
            &"x".repeat(41),
            "ghp_abcdefghijklmnop",
            "gho_abc",
            "github_pat_11ABC",
            "sk-ant-api03-abc",
            "xoxb-123-abc",
            "AKIAIOSFODNN7EXAMPLE",
            "eyJhbGciOiJIUzI1NiJ9",
            "me:hunter2hunter2",
            "a:b:12345678",
            // The price of the `:` rule: a plugin's skill is not named.
            "my-plugin:code-review",
        ];
        for bad in bad {
            assert_eq!(name(bad, "skill"), "skill", "{bad}");
        }
    }

    #[test]
    fn error_codes_and_tools_come_from_tables() {
        let known = &["rate_limit", "overloaded"];
        assert_eq!(code("rate_limit", known, "error"), "rate_limit");
        for bad in ["", "Rate limit", "rate_limit ", "zz_made_up_code", "key sk-1 is over its limit"] {
            assert_eq!(code(bad, known, "error"), "error", "{bad}");
        }
        let tools = [
            ("Bash", "Bash"),
            ("exec_command", "exec_command"),
            ("mcp__github__create_issue", "github/create_issue"),
            ("mcp__my server__x", "mcp"),
            ("mcp__ghp_abcdef__x", "mcp"),
            ("mcp____x", "mcp"),
            ("mcp__server", "mcp"),
            ("SomethingNew", "tool"),
            ("zz_made_up_tool", "tool"),
            ("A private tool name", "tool"),
            ("", "tool"),
        ];
        for (tool, label) in tools {
            assert_eq!(tool_label(tool), label, "{tool}");
        }
    }

    #[test]
    fn more_command_forms() {
        let cases: &[(&str, &str)] = &[
            ("curl --header=X-Key:s3cr3t -us3cr3t:pw https://x", "curl"),
            ("git --git-dir=s3cr3t -Cs3cr3t status", "git status"),
            ("echo $(s3cr3t --x) `s3cr3t2`", "shell"),
            ("diff <(curl -s https://x/s3cr3t) <(cat s3cr3t)", "diff"),
            ("env -S 'curl https://x/s3cr3t' s3cr3t", "curl"),
            ("env --split-string 's3cr3t run'", "shell"),
            ("find s3cr3t -name s3cr3t -exec grep s3cr3t {} \\;", "grep"),
            ("ssh s3cr3t@s3cr3t.example 'rm -rf s3cr3t; curl s3cr3t'", "ssh"),
            ("bash -c \"bash -c 'zsh -c \\\"curl s3cr3t\\\"'\"", "curl"),
            ("bash -c \"bash -c 'zsh -c \\\"sh -c s3cr3t\\\"'\"", "sh"),
        ];
        for (line, label) in cases {
            let got = command_label(line);
            assert_eq!(got, *label, "{line}");
            assert!(from_tables(&got) && !got.contains("s3cr3t"), "{line}");
        }
    }

    #[test]
    fn a_huge_hostile_line_is_classified_quickly() {
        let mb = 1 << 20;
        let lines = [
            "A=1 ".repeat(mb / 4),
            "sudo -u x ".repeat(mb / 10) + "curl x",
            "a;".repeat(mb / 2),
            "$(".repeat(mb / 2),
            "((((".repeat(mb / 4),
            "bash -c '".repeat(mb / 9),
            "\"`".repeat(mb / 2),
            "<<A\n".repeat(mb / 4),
            "#\n".repeat(mb / 2),
            "x".repeat(mb),
            "'".repeat(mb),
            "\\".repeat(mb),
        ];
        let started = std::time::Instant::now();
        for line in &lines {
            assert!(from_tables(&command_label(line)));
            let words: Vec<&str> = line.split(' ').collect();
            assert!(from_tables(&argv_label(&words)));
        }
        // Linear work on at most the first 256 KiB of each: well under a second here, even unoptimised.
        assert!(started.elapsed() < std::time::Duration::from_secs(20), "{:?}", started.elapsed());
    }
}
