pub mod fs;
pub mod handoff;
pub mod install;
pub mod ipc;
pub mod paths;
pub mod process;
pub mod pty;
pub mod shell;

use std::io;

fn check(result: libc::c_int) -> io::Result<libc::c_int> {
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}
