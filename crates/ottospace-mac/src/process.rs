//! What runs in a terminal: the foreground process group of a PTY, and a process's name, parent
//! and children.

use std::ffi::{c_int, c_void};
use std::os::fd::{AsRawFd, BorrowedFd};

unsafe extern "C" {
    fn tcgetpgrp(fd: c_int) -> c_int;
    fn proc_name(pid: c_int, buffer: *mut c_void, size: u32) -> c_int;
    fn proc_pidinfo(pid: c_int, flavor: c_int, arg: u64, buffer: *mut c_void, size: c_int) -> c_int;
    fn proc_listchildpids(parent: c_int, buffer: *mut c_void, size: c_int) -> c_int;
}

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

    #[test]
    fn a_child_is_listed_with_its_parent_until_it_is_gone() {
        let own = std::process::id() as i32;
        let mut child = std::process::Command::new("/bin/sleep").arg("30").spawn().expect("spawn");
        let pid = child.id() as i32;
        assert_eq!(parent(pid), Some(own));
        assert!(children(own).contains(&pid), "{:?}", children(own));
        assert_eq!(name(pid).as_deref(), Some("sleep"));
        child.kill().expect("kill");
        child.wait().expect("wait");
        assert!(!children(own).contains(&pid));
        assert_eq!((parent(i32::MAX), children(i32::MAX)), (None, vec![]));
        // Root's: `proc_name` alone reads nothing of it, as of `login` in a terminal.
        assert_eq!(name(1).as_deref(), Some("launchd"));
    }
}
