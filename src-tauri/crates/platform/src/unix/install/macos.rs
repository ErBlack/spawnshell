use std::path::Path;

use anyhow::{Result, bail};

pub fn check_location(target: &Path) -> Result<()> {
    if target.to_string_lossy().contains("/AppTranslocation/") {
        bail!("macOS runs Spawnshell from a temporary copy; move it to /Applications first");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::install_in;

    #[test]
    fn a_translocated_app_is_not_linked() {
        let dir = std::env::temp_dir().join(format!(
            "spawnshell-install-translocated-{}",
            std::process::id()
        ));
        let translocated = Path::new(
            "/private/var/folders/x/T/AppTranslocation/1F2E/d/Spawnshell.app/Contents/MacOS/spawnshell",
        );

        assert!(install_in(&dir, translocated).is_err());
        assert!(!dir.exists());
    }
}
