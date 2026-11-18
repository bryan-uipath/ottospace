//! A signal handler may make only async-signal-safe calls: it writes a byte to a pipe (`ring`),
//! and a thread blocked on the pipe's other end acts. Nothing polls: the thread sleeps until then.

use std::sync::atomic::{AtomicI32, Ordering::SeqCst};

/// A pipe whose every byte calls `then` on a thread of its own. Returns the end to write. Both
/// ends close on exec: a terminal's shell inherits neither (made at start, before any terminal).
/// The write end never blocks: a full pipe has a byte to read already.
pub(crate) fn start(name: &str, then: impl Fn() + Send + 'static) -> Option<i32> {
    let mut fds = [0; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return None;
    }
    let [read, write] = fds;
    for fd in fds {
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    unsafe { libc::fcntl(write, libc::F_SETFL, libc::O_NONBLOCK) };
    let reader = move || {
        let mut byte = 0u8;
        loop {
            match unsafe { libc::read(read, (&raw mut byte).cast(), 1) } {
                1 => then(),
                -1 if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted => {}
                _ => return,
            }
        }
    };
    std::thread::Builder::new().name(name.into()).spawn(reader).ok()?;
    Some(write)
}

/// From a signal handler: a byte to the pipe `start` gave, if it is there. `errno` is kept for
/// the code the signal interrupted.
pub(crate) fn ring(fd: &AtomicI32) {
    let fd = fd.load(SeqCst);
    if fd >= 0 {
        let errno = unsafe { *libc::__error() };
        unsafe { libc::write(fd, [1u8].as_ptr().cast(), 1) };
        unsafe { *libc::__error() = errno };
    }
}
