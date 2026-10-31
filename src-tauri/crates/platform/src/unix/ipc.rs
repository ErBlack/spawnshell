use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::Duration;

use super::handoff::{Handle, adopt};

pub fn remove_socket(path: &Path) {
    let _ = fs::remove_file(path);
}

pub struct Listener(UnixListener);

impl Listener {
    pub fn bind(path: &Path) -> io::Result<Self> {
        remove_socket(path);
        let previous = unsafe { libc::umask(0o077) };
        let listener = UnixListener::bind(path);
        unsafe {
            libc::umask(previous);
        }
        let listener = listener?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        Ok(Listener(listener))
    }

    #[must_use]
    pub fn handle(&self) -> Handle {
        Handle::of(&self.0)
    }

    /// # Safety
    ///
    /// `handle` must come from the previous daemon and must be adopted only once.
    pub unsafe fn adopt(handle: Handle) -> io::Result<Self> {
        unsafe { adopt(handle) }.map(|fd| Listener(UnixListener::from(fd)))
    }

    pub fn incoming(&self) -> impl Iterator<Item = io::Result<Stream>> {
        self.0.incoming().map(|stream| stream.map(Stream))
    }
}

pub struct Stream(UnixStream);

impl Stream {
    pub fn connect(path: &Path) -> io::Result<Self> {
        UnixStream::connect(path).map(Stream)
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        self.0.try_clone().map(Stream)
    }

    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.0.set_read_timeout(timeout)
    }

    pub fn shutdown(&self) -> io::Result<()> {
        self.0.shutdown(Shutdown::Both)
    }

    #[cfg(not(target_os = "linux"))]
    #[must_use]
    pub fn same_user(&self) -> bool {
        let mut uid: libc::uid_t = 0;
        let mut gid: libc::gid_t = 0;
        unsafe {
            libc::getpeereid(self.0.as_raw_fd(), &raw mut uid, &raw mut gid) == 0
                && uid == libc::geteuid()
        }
    }

    #[cfg(target_os = "linux")]
    #[must_use]
    pub fn same_user(&self) -> bool {
        let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
        let Ok(mut length) = libc::socklen_t::try_from(std::mem::size_of::<libc::ucred>()) else {
            return false;
        };
        unsafe {
            libc::getsockopt(
                self.0.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&raw mut credentials).cast(),
                &raw mut length,
            ) == 0
                && credentials.uid == libc::geteuid()
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

pub struct Lock(File);

impl Lock {
    pub fn open(path: &Path) -> io::Result<Self> {
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .map(Lock)
    }

    #[must_use]
    pub fn handle(&self) -> Handle {
        Handle::of(&self.0)
    }

    /// # Safety
    ///
    /// `handle` must come from the previous daemon and must be adopted only once.
    pub unsafe fn adopt(handle: Handle) -> io::Result<Self> {
        unsafe { adopt(handle) }.map(|fd| Lock(File::from(fd)))
    }

    pub fn try_lock(&self) -> io::Result<bool> {
        match self.0.try_lock() {
            Ok(()) => Ok(true),
            Err(TryLockError::WouldBlock) => Ok(false),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }
}
