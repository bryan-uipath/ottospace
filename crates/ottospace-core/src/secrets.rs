//! Credential-shaped text in what a terminal showed or ran, so a workbook keeps none of it.
//! Best effort: a table of shapes, not a proof. `workbook_scrollback_lines: 0` keeps no text.

/// What a secret's place shows instead.
pub const HIDDEN: &str = "[hidden]";

/// A key whose value is a secret, in `key=value`, `--key=value` or `--key value`, any case.
const KEYS: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    "pwd",
    "apikey",
    "api-key",
    "api_key",
    "auth",
    "bearer",
    "credential",
    "private-key",
];

/// Well-known token prefixes: GitHub, OpenAI and others, Slack, AWS access keys, JWTs.
const PREFIXES: &[&str] =
    &["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_", "sk-", "xoxa-", "xoxb-", "xoxp-", "AKIA", "eyJ"];

/// Clients that take a password glued to `-p`: `mysql -phunter2`.
const GLUED_P: &[&str] = &["mysql", "mysqldump", "mysqladmin", "mariadb"];

fn secret_key(key: &str) -> bool {
    let key = key.trim_start_matches('-').to_ascii_lowercase();
    !key.is_empty() && KEYS.iter().any(|secret| key.contains(secret))
}

/// A word that is a token by itself: a known prefix with something after it, or 32 or more
/// characters of a token's alphabet with upper case, lower case and digits (not a git hash).
fn token(word: &str) -> bool {
    let alphabet = |c: char| c.is_ascii_alphanumeric() || "+/=_-.".contains(c);
    let prefixed = PREFIXES.iter().any(|prefix| word.starts_with(prefix) && word.len() >= prefix.len() + 8);
    let mixed = word.len() >= 32
        && word.chars().all(alphabet)
        && word.chars().any(|c| c.is_ascii_uppercase())
        && word.chars().any(|c| c.is_ascii_lowercase())
        && word.chars().any(|c| c.is_ascii_digit());
    prefixed || mixed
}

/// One word with its secret part hidden. `before`: the words before it on the line.
fn hide_word(word: &str, before: &[&str]) -> String {
    let bare = word.trim_matches(|c| c == '\'' || c == '"');
    let previous = before.last().map(|word| word.trim_matches(|c| c == '\'' || c == '"')).unwrap_or_default();
    // `--password hunter2`, `Bearer abc`: the word after a secret flag.
    let flag = previous.starts_with('-') && !previous.contains('=') && secret_key(previous);
    if (flag || previous.eq_ignore_ascii_case("bearer")) && !bare.starts_with('-') && !bare.is_empty() && bare != HIDDEN
    {
        return HIDDEN.into();
    }
    // `https://user:pass@host`: the user and password.
    if let Some((scheme, rest)) = bare.split_once("://")
        && let Some((userinfo, host)) = rest.split_once('@')
        && userinfo.contains(':')
        && !userinfo.contains('/')
    {
        return format!("{scheme}://{HIDDEN}@{host}");
    }
    if let Some((key, value)) = bare.split_once('=')
        && secret_key(key)
        && !value.is_empty()
    {
        return format!("{key}={HIDDEN}");
    }
    let client = |word: &&str| GLUED_P.contains(&word.rsplit('/').next().unwrap_or(word));
    if bare.len() > 2 && bare.starts_with("-p") && !bare.starts_with("--") && before.iter().any(client) {
        return format!("-p{HIDDEN}");
    }
    if token(bare) {
        return HIDDEN.into();
    }
    word.to_string()
}

/// `line` with each credential-shaped part replaced by `[hidden]`; the rest stays. Hiding again
/// changes nothing. e.g. `curl -H 'Authorization: Bearer x'` → `curl -H 'Authorization: [hidden]`.
pub fn hide(line: &str) -> String {
    // A header: everything after it.
    let lower = line.to_ascii_lowercase();
    if let Some(at) = lower.find("authorization:") {
        let end = at + "authorization:".len();
        return format!("{} {HIDDEN}", &line[..end]);
    }
    let words: Vec<&str> = line.split(' ').collect();
    let hidden: Vec<String> = words.iter().enumerate().map(|(at, word)| hide_word(word, &words[..at])).collect();
    hidden.join(" ")
}

/// Terminal lines as a workbook keeps them: a PEM block (`-----BEGIN` to `-----END`) left out
/// whole, every other line with its secrets hidden.
pub fn hide_lines(lines: Vec<String>) -> Vec<String> {
    // The end of a block whose start is not among them (the lines are the last N): all before it go too.
    let orphan = lines.iter().position(|line| line.contains("-----END "));
    let started = |end: usize| lines[..end].iter().any(|line| line.contains("-----BEGIN "));
    let skip = orphan.filter(|end| !started(*end)).map_or(0, |end| end + 1);
    let mut kept = Vec::with_capacity(lines.len());
    let mut in_block = false;
    for line in lines.into_iter().skip(skip) {
        if line.contains("-----BEGIN ") {
            in_block = true;
        }
        if !in_block {
            kept.push(hide(&line));
        }
        if line.contains("-----END ") {
            in_block = false;
        }
    }
    kept
}

/// True if a command holds anything credential-shaped: it is then not kept at all, since a
/// command with a part hidden would not run as it did.
pub fn holds_secret(command: &str) -> bool {
    hide(command) != command || command.contains("-----BEGIN ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_shape_is_hidden_and_the_rest_of_the_line_stays() {
        let cases = [
            ("git clone https://me:hunter2@github.com/x.git", "git clone https://[hidden]@github.com/x.git"),
            ("export GITHUB_TOKEN=abc123", "export GITHUB_TOKEN=[hidden]"),
            ("deploy --api-key=k1 --region eu", "deploy --api-key=[hidden] --region eu"),
            ("login --password hunter2 --user me", "login --password [hidden] --user me"),
            ("mysql -u root -phunter2 db", "mysql -u root -p[hidden] db"),
            ("curl -H 'Authorization: Bearer abc' x", "curl -H 'Authorization: [hidden]"),
            ("echo Bearer abc", "echo Bearer [hidden]"),
            ("token is ghp_0123456789abcdefghij", "token is [hidden]"),
            ("key AKIAIOSFODNN7EXAMPLE", "key [hidden]"),
            ("xoxb-1234-5678-abcdef ok", "[hidden] ok"),
            ("jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig", "jwt [hidden]"),
            ("raw Zx9Qm2Lp8Rt4Vw6Yb1Nc3Kd5Hf7Jg0Ts", "raw [hidden]"),
        ];
        for (line, shown) in cases {
            assert_eq!(hide(line), shown, "{line}");
            assert_eq!(hide(shown), shown, "hiding again changes nothing: {shown}");
        }
        // Not secrets: a git hash, a port, plain words, a path.
        for plain in ["commit 9f3a61c2d4e5b7089f3a61c2d4e5b7089f3a61c2", "psql -p 5432", "npm run dev", "cd ~/code/app"]
        {
            assert_eq!(hide(plain), plain);
        }
    }

    #[test]
    fn a_pem_block_is_left_out_whole_and_a_command_with_any_secret_is_not_kept() {
        let lines = ["before", "-----BEGIN PRIVATE KEY-----", "MIIEv", "-----END PRIVATE KEY-----", "after"];
        assert_eq!(hide_lines(lines.map(String::from).to_vec()), ["before", "after"]);
        // Its start fell outside the lines kept: everything up to its end goes.
        assert_eq!(hide_lines(lines[2..].iter().map(|l| l.to_string()).collect()), ["after"]);
        assert!(holds_secret("psql postgres://u:p@db/x") && holds_secret("mysql -psecret"));
        assert!(!holds_secret("npm run dev") && !holds_secret("sleep 300"));
    }

    /// Random lines with shapes mixed in: what is kept holds none of them, and hiding again changes nothing.
    #[test]
    fn hidden_text_holds_no_shape_and_hiding_again_changes_nothing() {
        let mut rng = crate::fuzz::Rng::new(11);
        let shapes = [
            "token=",
            "--password",
            "Bearer",
            "https://a:b@",
            "ghp_0123456789ab",
            "-p",
            "mysql",
            "'",
            "Authorization:",
        ];
        for _ in 0..5000 {
            let words: Vec<String> = (0..rng.below(6))
                .map(|_| {
                    if rng.coin() {
                        rng.pick(&shapes).to_string() + &crate::fuzz::hostile(&mut rng)
                    } else {
                        crate::fuzz::hostile(&mut rng)
                    }
                })
                .collect();
            let line = words.join(" ");
            let once = hide(&line);
            assert_eq!(hide(&once), once, "{line:?}");
            assert!(!holds_secret(&once) || once.contains("-----BEGIN "), "{line:?}");
        }
    }
}
