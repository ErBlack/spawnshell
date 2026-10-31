use std::path::PathBuf;

use super::Pid;

#[must_use]
pub fn name(_pid: Pid) -> Option<String> {
    None
}

#[must_use]
pub fn cwd(_pid: Pid) -> Option<PathBuf> {
    None
}
