use std::fs::File;
use std::io;
use std::path::Path;

#[cfg_attr(target_os = "macos", path = "fs/macos.rs")]
#[cfg_attr(not(target_os = "macos"), path = "fs/other.rs")]
mod imp;

pub use imp::sync;

pub fn sync_dir(dir: &Path) -> io::Result<()> {
    sync(&File::open(dir)?)
}
