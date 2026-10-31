use std::path::PathBuf;

use anyhow::Result;

pub fn log_dir(identifier: &str) -> Result<PathBuf> {
    super::data_dir(identifier).map(|dir| dir.join("logs"))
}
