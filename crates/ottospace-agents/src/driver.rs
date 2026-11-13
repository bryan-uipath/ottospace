//! Runs an agent CLI as a child process and speaks its protocol: one thread reads stdout into
//! events and replies, one writes stdin from a queue (so neither a send nor a reply waits on the
//! other), one keeps the end of stderr, one waits for the exit.

use std::io::{self, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use crate::lines::{Line, Lines, MAX_LINE};
use crate::{Event, Input, Protocol, Runner, Untrusted};

/// How much of stderr is kept for `Exited`.
const STDERR_TAIL: usize = 4096;

/// A lock that a panicked thread left behind is still the state: take it.
fn lock<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Lines for stdin, in order. `None` closes it.
type Writes = Sender<Option<Vec<String>>>;

/// Writes what is queued to the child's stdin until `None` or a write fails. A CLI that stops
/// reading stdin stalls only this thread: stdout is still read, a send still returns.
fn write(mut stdin: ChildStdin, queue: Receiver<Option<Vec<String>>>) {
    for lines in queue {
        let Some(lines) = lines else { return };
        for line in lines {
            let written = stdin.write_all(line.as_bytes()).and_then(|()| stdin.write_all(b"\n"));
            if written.and_then(|()| stdin.flush()).is_err() {
                return;
            }
        }
    }
}

/// Kills the CLI's process group, unless the CLI was reaped: its id may then name another group.
fn kill_group(group: i32, reaped: &Mutex<bool>) -> io::Result<()> {
    // Held while killing, so the CLI is not reaped (and its id freed) in between.
    let reaped = lock(reaped);
    // SAFETY: `killpg` takes plain integers and touches no memory of ours.
    if *reaped || unsafe { libc::killpg(group, libc::SIGKILL) } == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

/// One running agent CLI.
pub struct Driver {
    /// The CLI's process group: its own id. Killing it ends the tools and servers it started too.
    group: i32,
    /// True once the CLI is reaped: its id may then name another group, so nothing is killed.
    reaped: Arc<Mutex<bool>>,
    /// `None` once stopped.
    writes: Option<Writes>,
    protocol: Arc<Mutex<dyn Protocol>>,
    events: Receiver<Event>,
}

impl Driver {
    /// Starts `command` (its program, argv and directory already set) with piped stdio, in a
    /// process group of its own, and writes the protocol's opening lines.
    pub fn spawn(mut command: Command, protocol: impl Protocol) -> io::Result<Self> {
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
        let mut child = command.spawn()?;
        let group = i32::try_from(child.id()).map_err(|_| io::Error::other("pid out of range"))?;
        let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
        let (writes, queue) = mpsc::channel();
        if let Some(stdin) = child.stdin.take() {
            thread::spawn(move || write(stdin, queue));
        }
        let protocol: Arc<Mutex<dyn Protocol>> = Arc::new(Mutex::new(protocol));
        let reaped = Arc::new(Mutex::new(false));
        let (exit_tx, exit) = mpsc::channel();
        let waiting = reaped.clone();
        thread::spawn(move || {
            let _ = exit_tx.send(wait(child, group, &waiting));
        });
        let opening = lock(&protocol).opening();
        let (tx, events) = mpsc::channel();
        // Read after the exit, not joined: a process that kept stderr open must not hold `Exited` back.
        let tail = Arc::new(Mutex::new(Vec::new()));
        let keeping = tail.clone();
        let tailing = stderr.map(|stderr| thread::spawn(move || keep_tail(stderr, &keeping)));
        let _ = writes.send(Some(opening));
        let (reading, replies, killing) = (protocol.clone(), writes.clone(), reaped.clone());
        let kill = move || drop(kill_group(group, &killing));
        thread::spawn(move || read(stdout, &reading, &replies, &tx, (&tail, tailing), &exit, kill));
        Ok(Self { group, reaped, writes: Some(writes), protocol, events })
    }
}

/// Waits for the CLI to exit, kills what it left in its group, then reaps it and gives its code.
/// Until it is reaped its id cannot be reused, so the group named by it is still the CLI's.
fn wait(mut child: Child, group: i32, reaped: &Mutex<bool>) -> Option<i32> {
    let pid = libc::id_t::try_from(group).unwrap_or_default();
    loop {
        // SAFETY: `info` is a zeroed `siginfo_t` that `waitid` writes into; `WNOWAIT` leaves the child unreaped.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let waited = unsafe { libc::waitid(libc::P_PID, pid, &mut info, libc::WEXITED | libc::WNOWAIT) };
        if waited == 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            break;
        }
    }
    let mut reaped = lock(reaped);
    // SAFETY: plain integers. What the CLI left behind (a background command holding its stdout) ends here.
    unsafe { libc::killpg(group, libc::SIGKILL) };
    let code = child.wait().ok().and_then(|status| status.code());
    *reaped = true;
    code
}

/// Keeps the last `STDERR_TAIL` bytes the child wrote to stderr.
fn keep_tail(mut stderr: impl Read, tail: &Mutex<Vec<u8>>) {
    let mut buffer = [0; 4096];
    while let Some(n) = read_some(&mut stderr, &mut buffer) {
        let mut tail = lock(tail);
        tail.extend_from_slice(&buffer[..n]);
        let extra = tail.len().saturating_sub(STDERR_TAIL);
        tail.drain(..extra);
    }
}

/// The next bytes read, retrying when a signal cut a read short. `None` at the end or on an error.
fn read_some(from: &mut impl Read, buffer: &mut [u8]) -> Option<usize> {
    loop {
        match from.read(buffer) {
            Ok(0) => return None,
            Ok(n) => return Some(n),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        }
    }
}

/// Reads stdout to its end, then takes the exit code from `wait` and sends `Exited`.
fn read(
    stdout: Option<impl Read>,
    protocol: &Mutex<dyn Protocol>,
    writes: &Writes,
    tx: &Sender<Event>,
    (tail, tailing): (&Mutex<Vec<u8>>, Option<thread::JoinHandle<()>>),
    exit: &Receiver<Option<i32>>,
    kill: impl Fn(),
) {
    let mut lines = Lines::new(MAX_LINE);
    let mut each = |line: Line| {
        let unknown =
            |kind: &str| crate::Output { events: vec![Event::Unknown { kind: kind.into() }], replies: vec![] };
        let out = match line {
            Line::Text(text) => lock(protocol).read(&text),
            // A request too long to read cannot be answered, and the CLI would wait on it for
            // good: the session ends instead.
            Line::Oversized(head) if lock(protocol).awaits_answer(&head) => {
                kill();
                let error = Event::Error { kind: "oversized_request".into(), retrying: false };
                crate::Output { events: vec![error], replies: vec![] }
            }
            Line::Oversized(_) => unknown("oversized"),
            Line::NotUtf8 => unknown("not_utf8"),
        };
        // Replies first: the CLI may be waiting on one.
        if !out.replies.is_empty() {
            let _ = writes.send(Some(out.replies));
        }
        out.events.into_iter().for_each(|event| drop(tx.send(event)));
    };
    if let Some(mut stdout) = stdout {
        let mut buffer = vec![0; 64 << 10];
        while let Some(n) = read_some(&mut stdout, &mut buffer) {
            lines.push(&buffer[..n]).into_iter().for_each(&mut each);
        }
    }
    lines.finish().into_iter().for_each(&mut each);
    let _ = writes.send(None);
    let code = exit.recv().ok().flatten();
    // Stderr's last words, given half a second to arrive: a process left behind may hold it open.
    for _ in 0..50 {
        if tailing.as_ref().is_none_or(thread::JoinHandle::is_finished) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let stderr = Untrusted::new(&String::from_utf8_lossy(&lock(tail)), STDERR_TAIL);
    let _ = tx.send(Event::Exited { code, stderr });
}

impl Runner for Driver {
    fn send(&self, input: Input) -> io::Result<()> {
        let closed = || io::Error::new(io::ErrorKind::BrokenPipe, "stdin is closed");
        let writes = self.writes.as_ref().ok_or_else(closed)?;
        let lines = lock(&self.protocol)
            .write(input)
            .map_err(|refused| io::Error::new(io::ErrorKind::InvalidInput, refused.0))?;
        writes.send(Some(lines)).map_err(|_| closed())
    }

    fn events(&self) -> &Receiver<Event> {
        &self.events
    }

    /// Closes stdin once what was sent before is written.
    fn stop(&mut self) {
        if let Some(writes) = self.writes.take() {
            let _ = writes.send(None);
        }
    }

    /// Kills the CLI's whole process group: its shell commands and MCP servers with it.
    fn kill(&mut self) -> io::Result<()> {
        kill_group(self.group, &self.reaped)
    }
}

/// A driver dropped takes its process with it: no CLI is left running unseen.
impl Drop for Driver {
    fn drop(&mut self) {
        let _ = self.kill();
        self.stop();
    }
}
