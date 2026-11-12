//! Which station a tool call belongs to, and its short label. Pure: names in, names out.

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
        | "local_shell_call" | "exec" | "write_stdin" => arg.map_or(Shell, classify_command),
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

/// The station of a shell command line, by the names of the commands in it, not by what they do.
/// Network if any command reaches the network; Files if every command only works on files; else Shell.
pub fn classify_command(line: &str) -> Station {
    overall(&parse(line))
}

/// A shell command line's label: the command's name, never its arguments. e.g. `cd x && git push -f` → `git push`.
pub fn command_label(line: &str) -> String {
    let commands = parse(line);
    let station = overall(&commands);
    let named = commands.iter().find(|c| c.0 == station).or(commands.first());
    named.map_or_else(|| "shell".to_string(), |c| clean(&c.1))
}

fn overall(commands: &[(Station, String)]) -> Station {
    if commands.iter().any(|c| c.0 == Network) {
        Network
    } else if !commands.is_empty() && commands.iter().all(|c| c.0 == Files) {
        Files
    } else {
        Shell
    }
}

/// A path's label: its last two parts, e.g. `/Users/me/app/src/main.rs` → `src/main.rs`.
pub fn path_label(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    clean(&parts[parts.len().saturating_sub(2)..].join("/"))
}

/// A web address's label: the host alone, e.g. `https://me:pw@example.com:8080/a?b` → `example.com`.
pub fn host_label(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("").split(':').next().unwrap_or("");
    if host.is_empty() { "web".to_string() } else { clean(host) }
}

/// A label fit to show: no control characters, at most 60 characters.
pub fn clean(label: &str) -> String {
    label.chars().filter(|c| !c.is_control()).take(60).collect()
}

/// Every command in a line with a station of its own, as (station, label). `cd`, `echo` and the like have none.
fn parse(line: &str) -> Vec<(Station, String)> {
    split(&without_heredocs(line)).iter().filter_map(|words| command(words)).collect()
}

/// Drops heredoc bodies, which are data, not commands. e.g. the lines between `<<'EOF'` and `EOF`.
fn without_heredocs(line: &str) -> String {
    let mut out = String::new();
    let mut until: Option<String> = None;
    for l in line.lines() {
        if let Some(end) = &until {
            if l.trim() == end {
                until = None;
            }
            continue;
        }
        if let Some((_, after)) = l.split_once("<<") {
            let word: String = after
                .trim_start_matches(['-', '~', ' '])
                .chars()
                .filter(|c| !matches!(c, '\'' | '"' | '\\'))
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            // `<<<` is a here-string, on one line.
            if !word.is_empty() && !after.starts_with('<') {
                until = Some(word);
            }
        }
        out.push_str(l);
        out.push('\n');
    }
    out
}

/// Splits a line into commands, each a list of words. Separators: `; | & ( )`, a new line, a backtick,
/// and `$(`, also inside double quotes. Quotes are honoured; redirections are dropped.
fn split(line: &str) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let (mut single, mut double) = (false, false);
    // For each open `$(`: whether it began inside double quotes.
    let mut subs: Vec<bool> = Vec::new();
    let mut chars = line.chars().peekable();
    let end_word = |word: &mut String, words: &mut Vec<String>| {
        if !word.is_empty() {
            words.push(std::mem::take(word));
        }
    };
    while let Some(c) = chars.next() {
        if single {
            if c == '\'' {
                single = false
            } else {
                word.push(c)
            }
            continue;
        }
        if c == '\\' {
            word.extend(chars.next());
            continue;
        }
        if c == '$' && chars.peek() == Some(&'(') {
            chars.next();
            subs.push(double);
            double = false;
            end_word(&mut word, &mut words);
            commands.push(std::mem::take(&mut words));
            continue;
        }
        if double {
            if c == '"' {
                double = false
            } else {
                word.push(c)
            }
            continue;
        }
        match c {
            '\'' => single = true,
            '"' => double = true,
            ' ' | '\t' => end_word(&mut word, &mut words),
            // `2>&1` and `&>` are redirections, not separators.
            '&' if word.ends_with('>') || chars.peek() == Some(&'>') => word.push(c),
            ')' if !subs.is_empty() => {
                end_word(&mut word, &mut words);
                commands.push(std::mem::take(&mut words));
                double = subs.pop().unwrap_or(false);
            }
            ';' | '|' | '&' | '(' | ')' | '\n' | '`' => {
                end_word(&mut word, &mut words);
                commands.push(std::mem::take(&mut words));
            }
            _ => word.push(c),
        }
    }
    end_word(&mut word, &mut words);
    commands.push(words);
    commands
}

/// Programs that only read, search, move, delete or edit files.
const FILES: &[&str] = &[
    "cat", "head", "tail", "less", "more", "nl", "ls", "grep", "egrep", "fgrep", "rg", "ag", "find", "fd", "mv", "cp",
    "rm", "rmdir", "mkdir", "touch", "sed", "awk", "tee", "wc", "sort", "uniq", "cut", "tr", "tree", "stat", "file",
    "diff", "ln", "chmod", "chown", "du", "df", "basename", "dirname", "realpath", "readlink", "jq", "yq", "xxd", "od",
    "strings", "tar", "zip", "unzip", "gzip", "gunzip",
];

/// Programs that reach the network whatever their arguments.
const NETWORK: &[&str] = &[
    "curl", "wget", "gh", "ssh", "scp", "sftp", "rsync", "nc", "ncat", "telnet", "ftp", "ping", "dig", "nslookup",
    "host", "http", "https", "aws", "az", "gcloud",
];

/// Programs that reach the network with these subcommands, e.g. `git push`.
const NETWORK_SUB: &[(&str, &[&str])] = &[
    ("git", &["push", "pull", "fetch", "clone", "ls-remote"]),
    ("npm", &["install", "i", "add", "ci", "update", "upgrade", "publish"]),
    ("pnpm", &["install", "i", "add", "update", "upgrade", "publish", "dlx"]),
    ("yarn", &["install", "add", "upgrade", "publish", "dlx"]),
    ("bun", &["install", "i", "add", "update", "publish"]),
    ("pip", &["install", "download"]),
    ("pip3", &["install", "download"]),
    ("uv", &["add", "sync", "pip"]),
    ("cargo", &["install", "fetch", "publish", "update", "add", "search"]),
    ("brew", &["install", "update", "upgrade", "fetch", "tap"]),
    ("docker", &["pull", "push", "login"]),
    ("podman", &["pull", "push", "login"]),
    ("go", &["get", "install"]),
    ("gem", &["install"]),
];

/// More programs whose label keeps the subcommand, e.g. `make test`.
const WITH_SUB: &[&str] = &["kubectl", "make", "just", "npx"];

/// Words in front of a command that run it, e.g. `sudo`, `timeout 30`, `xargs`, `if`.
const WRAPPERS: &[&str] = &[
    "sudo", "env", "nohup", "time", "command", "exec", "nice", "xargs", "timeout", "if", "then", "else", "elif", "do",
    "while", "until", "!", "{",
];

/// Shell words that do nothing a station shows.
const NEUTRAL: &[&str] = &[
    "cd", "pushd", "popd", "export", "set", "unset", "local", "true", "false", ":", "echo", "printf", "fi", "done",
    "}", "for", "case", "esac", "wait", "sleep", "source", ".", "[", "[[", "test", "pwd", "exit", "return",
];

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

/// One command's station and label, from its words. e.g. `["sudo", "git", "push", "-f"]` → (Network, "git push").
fn command(words: &[String]) -> Option<(Station, String)> {
    let mut rest: Vec<&str> = words.iter().map(String::as_str).filter(|w| !is_redirection(w)).collect();
    // Skip `A=b` and wrappers, with the wrapper's own options and `timeout`'s duration.
    loop {
        let first = *rest.first()?;
        if is_assignment(first) {
            rest.remove(0);
        } else if WRAPPERS.contains(&first) {
            rest.remove(0);
            while rest.first().is_some_and(|w| w.starts_with('-') || w.starts_with(|c: char| c.is_ascii_digit())) {
                rest.remove(0);
            }
        } else {
            break;
        }
    }
    // A program run by its path goes by its name, e.g. `/usr/bin/curl` and `CURL.EXE` are `curl`.
    let program = rest[0].rsplit('/').next().unwrap_or("").to_ascii_lowercase();
    let program = program.strip_suffix(".exe").unwrap_or(&program).to_string();
    if NEUTRAL.contains(&program.as_str()) || program.is_empty() {
        return None;
    }
    // `find … -exec rm {} +` is read as the command it runs.
    if program == "find"
        && let Some(i) = rest.iter().position(|w| matches!(*w, "-exec" | "-execdir" | "-ok"))
    {
        let inner: Vec<String> = rest[i + 1..].iter().map(|w| w.to_string()).collect();
        return command(&inner).or(Some((Files, program)));
    }
    let sub = rest[1..].iter().find(|w| !w.starts_with('-')).copied().unwrap_or("");
    let is_name =
        !sub.is_empty() && sub.len() <= 24 && sub.chars().all(|c| c.is_ascii_alphanumeric() || "-_:".contains(c));
    let network_sub = NETWORK_SUB.iter().find(|(p, _)| *p == program);
    let station = if NETWORK.contains(&program.as_str())
        || network_sub.is_some_and(|(_, subs)| subs.contains(&sub))
        || (program == "npx" && rest.iter().any(|w| matches!(*w, "-y" | "--yes")))
    {
        Network
    } else if FILES.contains(&program.as_str()) {
        Files
    } else {
        Shell
    };
    let keep_sub = is_name && (network_sub.is_some() || WITH_SUB.contains(&program.as_str()) || program == "gh");
    Some((station, if keep_sub { format!("{program} {sub}") } else { program }))
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
            ("ls && cargo build", Shell, "cargo build"),
            ("python -c \"import urllib\"", Shell, "python"),
            ("make test", Shell, "make test"),
            ("curl -s https://example.com/a?token=secret", Network, "curl"),
            ("gh pr create --draft --title 'secret title'", Network, "gh pr"),
            ("git push -f origin main", Network, "git push"),
            ("git status", Shell, "git status"),
            ("git commit -m 'curl is mentioned; rm too'", Shell, "git commit"),
            ("npm install left-pad", Network, "npm install"),
            ("npm test", Shell, "npm test"),
            ("cargo install ripgrep", Network, "cargo install"),
            ("docker pull alpine", Network, "docker pull"),
            ("npx -y cowsay hi", Network, "npx cowsay"),
            ("npx tsc", Shell, "npx tsc"),
            ("sudo curl example.com", Network, "curl"),
            ("FOO=1 BAR=2 env -i timeout 30 ssh host uptime", Network, "ssh"),
            ("ls | xargs rm", Files, "ls"),
            ("ls | xargs -n1 curl", Network, "curl"),
            ("nohup /usr/local/bin/wget http://example.com &", Network, "wget"),
            ("CURL.EXE http://example.com", Network, "curl"),
            ("echo \"$(curl -s example.com)\"", Network, "curl"),
            ("x=`git fetch`", Network, "git fetch"),
            ("cargo build 2>&1 | tail -5", Shell, "cargo build"),
            ("cat a.txt > b.txt 2>/dev/null", Files, "cat"),
            ("if grep -q x f; then rm f; fi", Files, "grep"),
            ("cat > notes.md <<'EOF'\ncurl is only text here\nrm -rf /\nEOF", Files, "cat"),
            ("cat <<EOF | tee out.txt\nssh host\nEOF\ncargo test", Shell, "cargo test"),
            ("cd /tmp", Shell, "shell"),
            ("", Shell, "shell"),
            ("echo 'unterminated", Shell, "shell"),
        ];
        for (line, station, label) in cases {
            assert_eq!(classify_command(line), *station, "{line}");
            assert_eq!(command_label(line), *label, "{line}");
        }
    }

    #[test]
    fn labels_are_names_only() {
        assert_eq!(path_label("/Users/me/app/src/main.rs"), "src/main.rs");
        assert_eq!(path_label("main.rs"), "main.rs");
        assert_eq!(path_label(""), "");
        assert_eq!(host_label("https://me:pw@example.com:8080/a/b?q=secret#f"), "example.com");
        assert_eq!(host_label("example.com/path"), "example.com");
        assert_eq!(host_label(""), "web");
        assert_eq!(clean("a\u{1b}[31mb\nc"), "a[31mbc");
        assert_eq!(clean(&"x".repeat(200)).len(), 60);
    }
}
