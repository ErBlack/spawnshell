use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

const NAME: &str = "spawnshell";

pub fn install() -> Result<PathBuf> {
    let exe = std::env::current_exe()
        .and_then(fs::canonicalize)
        .context("can't find the spawnshell binary")?;
    let home = dirs::home_dir().context("can't find the home directory")?;
    install_in(&home.join(".local").join("bin"), &exe)
}

fn install_in(dir: &Path, target: &Path) -> Result<PathBuf> {
    if target.to_string_lossy().contains("/AppTranslocation/") {
        bail!("macOS runs Spawnshell from a temporary copy; move it to /Applications first");
    }
    let link = dir.join(NAME);
    if fs::read_link(&link).is_ok_and(|current| current == target) {
        return Ok(link);
    }
    replace(dir, &link, target).with_context(|| format!("can't create {}", link.display()))?;
    Ok(link)
}

fn replace(dir: &Path, link: &Path, target: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    match fs::remove_file(link) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    symlink(target, link)
}

#[must_use]
pub fn in_path(path: &OsStr, dir: &Path) -> bool {
    std::env::split_paths(path).any(|entry| entry == dir)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    use super::*;

    #[test]
    fn the_link_is_created_kept_and_replaced() {
        let dir = std::env::temp_dir().join(format!("spawnshell-install-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let bin = dir.join("bin");
        let link = bin.join(NAME);
        let inode = || fs::symlink_metadata(&link).unwrap().ino();

        assert_eq!(install_in(&bin, Path::new("/a")).unwrap(), link);
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("/a"));
        let created = inode();
        assert_eq!(install_in(&bin, Path::new("/a")).unwrap(), link);
        assert_eq!(inode(), created, "an up-to-date link was recreated");
        assert_eq!(install_in(&bin, Path::new("/b")).unwrap(), link);
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("/b"));
        let _ = fs::remove_dir_all(&dir);
    }

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

    #[test]
    fn a_directory_is_found_in_path() {
        let dir = Path::new("/u/.local/bin");
        assert!(in_path(OsStr::new("/usr/bin:/u/.local/bin"), dir));
        assert!(in_path(OsStr::new("/u/.local/bin/:/bin"), dir));
        assert!(!in_path(OsStr::new("/usr/bin:/u/.local"), dir));
        assert!(!in_path(OsStr::new(""), dir));
    }
}
