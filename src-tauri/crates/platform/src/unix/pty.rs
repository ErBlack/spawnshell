use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use super::check;
use super::handoff::{Handle, adopt};
use super::process::Pid;
use crate::Size;

fn winsize(size: Size) -> libc::winsize {
    libc::winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

pub struct Master(OwnedFd);

impl Master {
    #[must_use]
    pub fn handle(&self) -> Handle {
        Handle::of(&self.0)
    }

    /// # Safety
    ///
    /// `handle` must come from the previous daemon and must be adopted only once.
    pub unsafe fn adopt(handle: Handle) -> io::Result<Self> {
        unsafe { adopt(handle) }.map(Master)
    }

    pub fn reader(&self) -> io::Result<Reader> {
        Ok(Reader(File::from(self.0.try_clone()?)))
    }

    pub fn writer(&self) -> io::Result<Writer> {
        Ok(Writer(File::from(self.0.try_clone()?)))
    }

    pub fn resize(&self, size: Size) -> io::Result<()> {
        let winsize = winsize(size);
        check(unsafe { libc::ioctl(self.0.as_raw_fd(), libc::TIOCSWINSZ, &winsize) }).map(drop)
    }

    #[must_use]
    pub fn foreground(&self) -> Option<Pid> {
        let group = unsafe { libc::tcgetpgrp(self.0.as_raw_fd()) };
        (group > 0).then_some(Pid(group))
    }
}

pub struct Reader(File);

impl Reader {
    pub fn wait_readable(&self, timeout: Duration) -> io::Result<bool> {
        let mut poll = libc::pollfd {
            fd: self.0.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let millis = timeout.as_micros().div_ceil(1000);
        let millis = libc::c_int::try_from(millis).unwrap_or(libc::c_int::MAX);
        check(unsafe { libc::poll(&raw mut poll, 1, millis) }).map(|ready| ready > 0)
    }
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.0.read(buf) {
            Err(error) if error.raw_os_error() == Some(libc::EIO) => Ok(0),
            result => result,
        }
    }
}

pub struct Writer(File);

impl Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

unsafe extern "C" {
    fn ptsname_r(fd: libc::c_int, buffer: *mut libc::c_char, length: libc::size_t) -> libc::c_int;
}

fn open(size: Size) -> io::Result<(Master, OwnedFd)> {
    let flags = libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC;
    let master = check(unsafe { libc::posix_openpt(flags) })?;
    let master = unsafe { OwnedFd::from_raw_fd(master) };
    check(unsafe { libc::grantpt(master.as_raw_fd()) })?;
    check(unsafe { libc::unlockpt(master.as_raw_fd()) })?;
    let mut name: [libc::c_char; 128] = [0; 128];
    check(unsafe { ptsname_r(master.as_raw_fd(), name.as_mut_ptr(), name.len()) })?;
    let slave = check(unsafe { libc::open(name.as_ptr(), flags) })?;
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };
    let master = Master(master);
    master.resize(size)?;
    Ok((master, slave))
}

pub struct Spawn<'a> {
    pub program: &'a str,
    pub args: &'a [String],
    pub cwd: &'a Path,
    pub env: &'a [(&'a str, &'a str)],
    pub size: Size,
}

pub fn spawn(spec: &Spawn) -> io::Result<(Pid, Master)> {
    let (master, slave) = open(spec.size)?;
    let mut command = Command::new(spec.program);
    command
        .args(spec.args)
        .current_dir(spec.cwd)
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave))
        .envs(spec.env.iter().copied());
    unsafe {
        command.pre_exec(|| {
            let mut signals: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&raw mut signals);
            let unblocked =
                libc::pthread_sigmask(libc::SIG_SETMASK, &raw const signals, std::ptr::null_mut());
            if unblocked != 0 {
                return Err(io::Error::from_raw_os_error(unblocked));
            }
            check(libc::setsid())?;
            #[allow(
                clippy::useless_conversion,
                reason = "the type of TIOCSCTTY differs between platforms"
            )]
            let set_controlling_terminal = libc::TIOCSCTTY.into();
            check(libc::ioctl(0, set_controlling_terminal, 0))?;
            Ok(())
        });
    }
    let child = command.spawn()?;
    Ok((Pid(child.id().cast_signed()), master))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_pty_closes_on_exec_and_has_its_size() {
        let (master, slave) = open(Size::new(30, 100)).unwrap();
        for fd in [master.0.as_raw_fd(), slave.as_raw_fd()] {
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
            assert_ne!(flags & libc::FD_CLOEXEC, 0);
        }
        let mut winsize = winsize(Size::new(1, 2));
        unsafe { libc::ioctl(slave.as_raw_fd(), libc::TIOCGWINSZ, &raw mut winsize) };
        assert_eq!((winsize.ws_row, winsize.ws_col), (30, 100));
    }
}
