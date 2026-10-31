use std::fs::{self, File};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use platform::fs::{sync, sync_dir};

use crate::report;

fn resolve(path: &Path) -> io::Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(target) => Ok(target),
        Err(error) if error.kind() == ErrorKind::NotFound && !path.is_symlink() => {
            Ok(path.to_path_buf())
        }
        Err(error) => Err(error),
    }
}

fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let target = resolve(path)?;
    let dir = target.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    let staging = target.with_file_name(name);
    let mut file = File::create(&staging)?;
    let staged = (|| {
        if let Ok(metadata) = fs::metadata(&target) {
            file.set_permissions(metadata.permissions())?;
        }
        file.write_all(bytes)?;
        sync(&file)?;
        fs::rename(&staging, &target)
    })();
    if staged.is_err() {
        let _ = fs::remove_file(&staging);
    }
    staged?;
    if let Err(error) = sync_dir(dir) {
        report(&anyhow::Error::new(error).context(format!("can't sync {}", dir.display())));
    }
    Ok(())
}

pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    write_synced(path, bytes).with_context(|| format!("can't write {}", path.display()))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn writing_through_a_symlink_keeps_the_link() {
        let dir = std::env::temp_dir().join(format!("spawnshell-files-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let real = dir.join("real.toml");
        let link = dir.join("link.toml");
        fs::write(&real, "old").unwrap();
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(&real, &link).unwrap();

        write(&link, b"new").unwrap();
        let is_link = fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink();
        let content = fs::read_to_string(&real).unwrap();
        let _ = fs::remove_dir_all(&dir);

        assert!(is_link);
        assert_eq!(content, "new");
    }

    #[test]
    fn writing_keeps_permissions_and_dangling_links() {
        use std::os::unix::fs::PermissionsExt;

        let dir =
            std::env::temp_dir().join(format!("spawnshell-files-mode-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.toml");
        fs::write(&file, "old").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        let dangling = dir.join("dangling.toml");
        let _ = fs::remove_file(&dangling);
        std::os::unix::fs::symlink(dir.join("missing").join("target.toml"), &dangling).unwrap();

        write(&file, b"new").unwrap();
        let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        let refused = write(&dangling, b"new").is_err();
        let still_link = dangling.is_symlink();
        let _ = fs::remove_dir_all(&dir);

        assert_eq!(mode, 0o600);
        assert!(refused);
        assert!(still_link);
    }
}
