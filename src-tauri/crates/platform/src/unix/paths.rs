use std::path::PathBuf;

use anyhow::{Context, Result};

#[cfg_attr(target_os = "macos", path = "paths/macos.rs")]
#[cfg_attr(not(target_os = "macos"), path = "paths/other.rs")]
mod imp;

pub use imp::log_dir;

pub fn data_dir(identifier: &str) -> Result<PathBuf> {
    dirs::data_dir()
        .map(|dir| dir.join(identifier))
        .context("can't find the application data folder")
}

pub fn socket(identifier: &str) -> Result<PathBuf> {
    data_dir(identifier).map(|dir| dir.join("daemon.sock"))
}
