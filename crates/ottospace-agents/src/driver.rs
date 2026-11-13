//! Runs an agent CLI as a child process and speaks its protocol: one thread reads stdout into
//! events and writes the protocol's replies, one keeps the end of stderr. Sends go from the caller.

use std::io::{self, Read, Write};
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

type Stdin = Arc<Mutex<Option<ChildStdin>>>;

/// Writes lines to the child. Closed stdin: an error.
fn write_lines(stdin: &Stdin, lines: &[String]) -> io::Result<()> {
    let mut stdin = lock(stdin);
    let pipe = stdin.as_mut().ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "stdin is closed"))?;
    for line in lines {
        pipe.write_all(line.as_bytes())?;
        pipe.write_all(b"\n")?;
    }
    pipe.flush()
}

/// One running agent CLI.
pub struct Driver {
    child: Arc<Mutex<Child>>,
    stdin: Stdin,
    protocol: Arc<Mutex<dyn Protocol>>,
    events: Receiver<Event>,
}

impl Driver {
    /// Starts `command` (its program, argv and directory already set) with piped stdio and
    /// writes the protocol's opening lines.
    pub fn spawn(mut command: Command, protocol: impl Protocol) -> io::Result<Self> {
        let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
        let stdin: Stdin = Arc::new(Mutex::new(child.stdin.take()));
        let protocol: Arc<Mutex<dyn Protocol>> = Arc::new(Mutex::new(protocol));
        let child = Arc::new(Mutex::new(child));
        let opening = lock(&protocol).opening();
        let (tx, events) = mpsc::channel();
        let tail = thread::spawn(move || stderr.map(keep_tail).unwrap_or_default());
        let (reading, replies, waiting) = (protocol.clone(), stdin.clone(), child.clone());
        thread::spawn(move || read(stdout, &reading, &replies, &tx, tail, &waiting));
        // A CLI that exits at once gives an error here; its `Exited` still comes.
        let _ = write_lines(&stdin, &opening);
        Ok(Self { child, stdin, protocol, events })
    }
}

/// The last `STDERR_TAIL` bytes the child wrote to stderr.
fn keep_tail(mut stderr: impl Read) -> Vec<u8> {
    let (mut tail, mut buffer) = (Vec::new(), [0; 4096]);
    while let Some(n) = read_some(&mut stderr, &mut buffer) {
        tail.extend_from_slice(&buffer[..n]);
        let extra = tail.len().saturating_sub(STDERR_TAIL);
        tail.drain(..extra);
    }
    tail
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

/// Reads stdout to its end, then waits for the child and sends `Exited`.
fn read(
    stdout: Option<impl Read>,
    protocol: &Mutex<dyn Protocol>,
    stdin: &Stdin,
    tx: &Sender<Event>,
    tail: thread::JoinHandle<Vec<u8>>,
    child: &Mutex<Child>,
) {
    let mut lines = Lines::new(MAX_LINE);
    let mut each = |line: Line| {
        let out = match line {
            Line::Text(text) => lock(protocol).read(&text),
            Line::Oversized => {
                crate::Output { events: vec![Event::Unknown { kind: "oversized".into() }], replies: vec![] }
            }
            Line::NotUtf8 => {
                crate::Output { events: vec![Event::Unknown { kind: "not_utf8".into() }], replies: vec![] }
            }
        };
        // Replies first: the CLI may be waiting on one.
        let _ = write_lines(stdin, &out.replies);
        out.events.into_iter().for_each(|event| drop(tx.send(event)));
    };
    if let Some(mut stdout) = stdout {
        let mut buffer = vec![0; 64 << 10];
        while let Some(n) = read_some(&mut stdout, &mut buffer) {
            lines.push(&buffer[..n]).into_iter().for_each(&mut each);
        }
    }
    lines.finish().into_iter().for_each(&mut each);
    lock(stdin).take();
    let stderr = tail.join().unwrap_or_default();
    // Stdout closed; the process may take a moment to exit. Polled so `kill` can take the lock.
    let code = loop {
        match lock(child).try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(_) => break None,
        }
    };
    let stderr = Untrusted::new(&String::from_utf8_lossy(&stderr), STDERR_TAIL);
    let _ = tx.send(Event::Exited { code, stderr });
}

impl Runner for Driver {
    fn send(&self, input: Input) -> io::Result<()> {
        let lines = lock(&self.protocol)
            .write(input)
            .map_err(|refused| io::Error::new(io::ErrorKind::InvalidInput, refused.0))?;
        write_lines(&self.stdin, &lines)
    }

    fn events(&self) -> &Receiver<Event> {
        &self.events
    }

    fn stop(&mut self) {
        lock(&self.stdin).take();
    }

    fn kill(&mut self) -> io::Result<()> {
        lock(&self.child).kill()
    }
}

/// A driver dropped takes its process with it: no CLI is left running unseen.
impl Drop for Driver {
    fn drop(&mut self) {
        self.stop();
        let _ = self.kill();
    }
}
