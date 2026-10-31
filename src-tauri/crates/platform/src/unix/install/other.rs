use std::path::Path;

use anyhow::Result;

#[expect(
    clippy::unnecessary_wraps,
    reason = "the same signature as on macOS, where the check can fail"
)]
pub fn check_location(_target: &Path) -> Result<()> {
    Ok(())
}
