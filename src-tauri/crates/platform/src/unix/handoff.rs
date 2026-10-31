use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::Command;

use serde::{Deserialize, Serialize};

use super::check;

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Handle(RawFd);

impl Handle {
    pub(crate) fn of(source: &impl AsRawFd) -> Self {
        Handle(source.as_raw_fd())
    }
}

pub(crate) unsafe fn adopt(handle: Handle) -> io::Result<OwnedFd> {
    set_cloexec(handle.0, true)?;
    Ok(unsafe { OwnedFd::from_raw_fd(handle.0) })
}

fn set_cloexec(fd: RawFd, enabled: bool) -> io::Result<()> {
    let flags = check(unsafe { libc::fcntl(fd, libc::F_GETFD) })?;
    let flags = if enabled {
        flags | libc::FD_CLOEXEC
    } else {
        flags & !libc::FD_CLOEXEC
    };
    check(unsafe { libc::fcntl(fd, libc::F_SETFD, flags) }).map(drop)
}

pub fn inherit(command: &mut Command, handles: Vec<Handle>) {
    unsafe {
        command.pre_exec(move || {
            for handle in &handles {
                set_cloexec(handle.0, false)?;
            }
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handle_is_stored_as_a_plain_number() {
        assert_eq!(serde_json::to_string(&Handle(7)).unwrap(), "7");
    }
}
