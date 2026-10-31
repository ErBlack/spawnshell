use std::path::PathBuf;

use anyhow::{Context, Result};

pub fn log_dir(identifier: &str) -> Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join("Library/Logs").join(identifier))
        .context("can't find the home folder")
}
