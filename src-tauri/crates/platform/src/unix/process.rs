use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use super::check;

#[cfg_attr(target_os = "macos", path = "process/macos.rs")]
#[cfg_attr(not(target_os = "macos"), path = "process/other.rs")]
mod imp;

pub use imp::{cwd, name};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(transparent)]
pub struct Pid(pub(crate) i32);

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Build {
    pub device: u64,
    pub inode: u64,
}

#[must_use]
pub fn build_of(exe: &Path) -> Option<Build> {
    let metadata = std::fs::metadata(exe).ok()?;
    Some(Build {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

pub fn detach(command: &mut Command) {
    unsafe {
        command.pre_exec(|| check(libc::setsid()).map(drop));
    }
}

pub struct Termination(libc::sigset_t);

impl Termination {
    #[must_use]
    pub fn block() -> Self {
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&raw mut set);
            for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
                libc::sigaddset(&raw mut set, signal);
            }
            libc::pthread_sigmask(libc::SIG_BLOCK, &raw const set, std::ptr::null_mut());
            Termination(set)
        }
    }

    pub fn wait(&self) {
        let mut signal = 0;
        while unsafe { libc::sigwait(&raw const self.0, &raw mut signal) } != 0 {}
    }
}

pub fn hang_up(pid: Pid) {
    unsafe {
        libc::kill(-pid.0, libc::SIGHUP);
    }
}

pub fn kill(pid: Pid) {
    unsafe {
        libc::kill(-pid.0, libc::SIGKILL);
    }
}

#[must_use]
pub fn child_running(pid: Pid) -> bool {
    let mut status = 0;
    unsafe { libc::waitpid(pid.0, &raw mut status, libc::WNOHANG) == 0 }
}

#[must_use]
pub fn exists(pid: Pid) -> bool {
    unsafe { libc::kill(pid.0, 0) == 0 }
}

pub fn reap(pid: Pid) {
    let mut status = 0;
    unsafe {
        libc::waitpid(pid.0, &raw mut status, 0);
    }
}
