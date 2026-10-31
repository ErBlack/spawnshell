use std::fs::File;
use std::io::{self, ErrorKind};
use std::os::fd::AsRawFd;

use super::super::check;

fn retrying(call: impl Fn() -> io::Result<()>) -> io::Result<()> {
    loop {
        match call() {
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            result => return result,
        }
    }
}

fn sync_with(
    full: impl Fn() -> io::Result<()>,
    plain: impl Fn() -> io::Result<()>,
) -> io::Result<()> {
    retrying(full).or_else(|_| retrying(plain))
}

pub fn sync(file: &File) -> io::Result<()> {
    let fd = file.as_raw_fd();
    sync_with(
        || check(unsafe { libc::fcntl(fd, libc::F_FULLFSYNC) }).map(drop),
        || check(unsafe { libc::fsync(fd) }).map(drop),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syncing_falls_back_when_a_full_sync_is_unsupported() {
        use std::cell::Cell;

        let plain = Cell::new(0);
        let fell_back = sync_with(
            || Err(io::Error::from(ErrorKind::Unsupported)),
            || {
                plain.set(plain.get() + 1);
                Ok(())
            },
        );
        assert!(fell_back.is_ok());
        assert_eq!(plain.get(), 1);

        let interrupted = Cell::new(true);
        let full = sync_with(
            || {
                if interrupted.replace(false) {
                    Err(io::Error::from(ErrorKind::Interrupted))
                } else {
                    Ok(())
                }
            },
            || {
                plain.set(plain.get() + 1);
                Ok(())
            },
        );
        assert!(full.is_ok());
        assert_eq!(plain.get(), 1);
    }
}
