//! What runs in a terminal: the foreground process group of a PTY, and a process's name, parent
//! and children.

use std::ffi::{c_int, c_void};
use std::os::fd::{AsRawFd, BorrowedFd};

unsafe extern "C" {
    fn tcgetpgrp(fd: c_int) -> c_int;
    fn proc_name(pid: c_int, buffer: *mut c_void, size: u32) -> c_int;
    fn proc_pidinfo(pid: c_int, flavor: c_int, arg: u64, buffer: *mut c_void, size: c_int) -> c_int;
    fn proc_listchildpids(parent: c_int, buffer: *mut c_void, size: c_int) -> c_int;
    fn sysctl(
        name: *const c_int,
        len: u32,
        old: *mut c_void,
        oldlen: *mut usize,
        new: *const c_void,
        newlen: usize,
    ) -> c_int;
}

/// `CTL_KERN`, `KERN_PROCARGS2` (`<sys/sysctl.h>`): a process's argument count, path and arguments.
const KERN_PROCARGS: [c_int; 2] = [1, 49];

/// The most bytes of arguments read: the system's `ARG_MAX`.
const ARG_MAX: usize = 1024 * 1024;

/// `PROC_PIDT_SHORTBSDINFO` and its `struct proc_bsdshortinfo` (`<sys/proc_info.h>`).
const SHORT_INFO: c_int = 13;

#[repr(C)]
#[derive(Default)]
struct ShortInfo {
    pid: u32,
    parent: u32,
    group: u32,
    status: u32,
    name: [u8; 16],
    flags: u32,
    ids: [u32; 6],
    reserved: u32,
}

/// The foreground process group of the terminal behind `pty` (its master side). At a prompt it
/// is the shell's own pid. `None`: it could not be read.
pub fn foreground(pty: BorrowedFd) -> Option<i32> {
    let group = unsafe { tcgetpgrp(pty.as_raw_fd()) };
    (group > 0).then_some(group)
}

/// The name of process `pid`, e.g. "cargo". `None`: it is gone.
pub fn name(pid: i32) -> Option<String> {
    let mut buffer = [0u8; 64];
    let length = unsafe { proc_name(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    let long = usize::try_from(length).ok().and_then(|length| buffer.get(..length));
    // `proc_name` gives nothing for another user's process, e.g. `login`, which runs as root.
    // The short record does: the first 15 bytes of the name.
    let short = short_info(pid).map(|info| info.name);
    let short = short.iter().map(|name| name.split(|byte| *byte == 0).next().unwrap_or_default());
    let name = long.into_iter().chain(short).find(|name| !name.is_empty())?;
    Some(String::from_utf8_lossy(name).into_owned())
}

fn short_info(pid: i32) -> Option<ShortInfo> {
    let mut info = ShortInfo::default();
    let size = size_of::<ShortInfo>() as c_int;
    let read = unsafe { proc_pidinfo(pid, SHORT_INFO, 0, (&raw mut info).cast(), size) };
    (read == size).then_some(info)
}

/// `PROC_PIDVNODEPATHINFO` and the size of its `struct proc_vnodepathinfo`: two `vnode_info_path`,
/// each a 152-byte `vnode_info` and a 1024-byte path. The first is the working directory.
const VNODE_PATHS: c_int = 9;
const VNODE_PATHS_SIZE: usize = 2 * (152 + 1024);

/// The working directory of process `pid`, resolved (`/private/tmp`, not `/tmp`). `None`: it is
/// gone, or another user's.
pub fn cwd(pid: i32) -> Option<String> {
    let mut info = [0u8; VNODE_PATHS_SIZE];
    let size = VNODE_PATHS_SIZE as c_int;
    let read = unsafe { proc_pidinfo(pid, VNODE_PATHS, 0, info.as_mut_ptr().cast(), size) };
    let path = info.get(152..152 + 1024).filter(|_| read == size)?;
    let path = path.split(|byte| *byte == 0).next().filter(|path| !path.is_empty())?;
    String::from_utf8(path.to_vec()).ok()
}

/// The parent of process `pid`. `None`: it is gone.
pub fn parent(pid: i32) -> Option<i32> {
    short_info(pid).map(|info| info.parent as i32)
}

/// The arguments process `pid` was started with, e.g. `["sleep", "300"]`. `None`: it is gone,
/// another user's, or they do not read (`parse_args`). As the process holds them now: one that
/// set its title (Node) has rewritten them.
pub fn args(pid: i32) -> Option<Vec<String>> {
    let name = [KERN_PROCARGS[0], KERN_PROCARGS[1], pid];
    let mut size = ARG_MAX;
    let mut buffer = vec![0u8; size];
    let read = unsafe { sysctl(name.as_ptr(), 3, buffer.as_mut_ptr().cast(), &raw mut size, std::ptr::null(), 0) };
    parse_args(buffer.get(..size).filter(|_| read == 0 && pid > 0)?)
}

/// The most arguments believed: more is a buffer read wrong.
const MAX_ARGS: usize = 4096;

/// The arguments in a `KERN_PROCARGS2` buffer: an `int` count, the executable's path, NULs to
/// the next multiple of 8 bytes from the path's start, then exactly that many NUL-ended
/// arguments (the environment follows; it is never read). `None` for anything off the layout:
/// a short buffer, a count out of bounds, padding that is not NULs, an argument with no NUL,
/// one that is not UTF-8, or an empty first one (as with `exec -a ""`).
pub fn parse_args(buffer: &[u8]) -> Option<Vec<String>> {
    let count = usize::try_from(i32::from_ne_bytes(buffer.get(..4)?.try_into().ok()?)).ok()?;
    if !(1..=MAX_ARGS).contains(&count) {
        return None;
    }
    let area = buffer.get(4..)?;
    let path = area.iter().position(|byte| *byte == 0)?;
    let start = (path + 1).div_ceil(8) * 8;
    if !area.get(path..start)?.iter().all(|byte| *byte == 0) {
        return None;
    }
    let mut rest = area.get(start..)?;
    let mut args = Vec::with_capacity(count);
    for _ in 0..count {
        let end = rest.iter().position(|byte| *byte == 0)?;
        args.push(String::from_utf8(rest[..end].to_vec()).ok()?);
        rest = &rest[end + 1..];
    }
    (!args[0].is_empty()).then_some(args)
}

/// The children of process `pid`: a shell's jobs. Empty: none, or they could not be read.
pub fn children(pid: i32) -> Vec<i32> {
    let mut pids = [0 as c_int; 256];
    let count = unsafe { proc_listchildpids(pid, pids.as_mut_ptr().cast(), size_of_val(&pids) as c_int) };
    let count = usize::try_from(count).unwrap_or(0).min(pids.len());
    pids[..count].iter().copied().filter(|pid| *pid > 0).collect()
}

#[cfg(test)]
mod tests {
    use std::os::fd::AsFd;

    use super::*;

    #[test]
    fn a_process_has_a_name_and_a_file_is_no_terminal() {
        let own = name(std::process::id() as i32).expect("own name");
        assert!(own.starts_with("ottospace_mac"), "{own}");
        assert_eq!(name(i32::MAX), None);
        let here = std::env::current_dir().and_then(|dir| dir.canonicalize()).expect("a directory");
        assert_eq!(cwd(std::process::id() as i32).as_deref(), here.to_str());
        assert_eq!((cwd(i32::MAX), cwd(1)), (None, None), "gone, and root's");
        let file = std::fs::File::open("/dev/null").expect("open");
        assert_eq!(foreground(file.as_fd()), None);
    }

    /// A `KERN_PROCARGS2` buffer: `count`, the path and its padding, then `rest` as it lies.
    fn procargs(count: i32, path: &str, rest: &[u8]) -> Vec<u8> {
        let mut buffer = count.to_ne_bytes().to_vec();
        buffer.extend(path.as_bytes());
        buffer.extend(vec![0; (path.len() + 1).div_ceil(8) * 8 - path.len()]);
        buffer.extend(rest);
        buffer
    }

    #[test]
    fn arguments_are_read_by_the_layout_and_never_past_them() {
        let args = |list: &[&str]| Some(list.iter().map(|arg| arg.to_string()).collect::<Vec<_>>());
        // As the system writes it, the environment after the arguments.
        assert_eq!(parse_args(&procargs(2, "/bin/sleep", b"sleep\0300\0HOME=/x\0")), args(&["sleep", "300"]));
        // An empty argument is one: the environment stays unread.
        assert_eq!(parse_args(&procargs(3, "/bin/echo", b"echo\0\0b\0TOKEN=s\0")), args(&["echo", "", "b"]));
        // An empty first one: the padding and it cannot be told apart. Nothing.
        assert_eq!(parse_args(&procargs(2, "/bin/sleep", b"\0300\0TOKEN=s\0")), None);
        // A count larger than what is there, out of bounds, or a buffer cut short: nothing.
        assert_eq!(parse_args(&procargs(5, "/bin/sleep", b"sleep\0300\0")), None);
        assert_eq!(parse_args(&procargs(0, "/bin/sleep", b"sleep\0")), None);
        assert_eq!(parse_args(&procargs(-1, "/bin/sleep", b"sleep\0")), None);
        assert_eq!(parse_args(&procargs(2, "/bin/sleep", b"sleep\03")), None);
        assert_eq!(parse_args(&[2, 0, 0]), None);
        assert_eq!(parse_args(&procargs(1, "/bin/x", b"\xff\0")), None);
        // Padding that is not NULs is not this layout.
        let mut odd = procargs(1, "/bin/sleep", b"sleep\0");
        odd[4 + 12] = b'x';
        assert_eq!(parse_args(&odd), None);
    }

    #[test]
    fn a_child_is_listed_with_its_parent_until_it_is_gone() {
        let own = std::process::id() as i32;
        let mut child = std::process::Command::new("/bin/sleep").arg("30").spawn().expect("spawn");
        let pid = child.id() as i32;
        assert_eq!(parent(pid), Some(own));
        assert!(children(own).contains(&pid), "{:?}", children(own));
        assert_eq!(name(pid).as_deref(), Some("sleep"));
        assert_eq!(args(pid), Some(vec!["/bin/sleep".to_string(), "30".to_string()]));
        // An empty first argument (`exec -a ""`): nothing, not the environment.
        let mut empty = std::process::Command::new("/bin/sh").args(["-c", "exec -a '' /bin/sleep 30"]).spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert_eq!(args(empty.id() as i32), None);
        empty.kill().expect("kill");
        empty.wait().expect("wait");
        child.kill().expect("kill");
        child.wait().expect("wait");
        assert!(!children(own).contains(&pid));
        assert_eq!((parent(i32::MAX), children(i32::MAX), args(i32::MAX)), (None, vec![], None));
        // Root's: `proc_name` alone reads nothing of it, as of `login` in a terminal.
        assert_eq!(name(1).as_deref(), Some("launchd"));
    }
}
