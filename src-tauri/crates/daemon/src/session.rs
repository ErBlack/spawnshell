use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::files;
use crate::paths;
use crate::pty::Size;

#[derive(Clone, Serialize, Deserialize)]
pub struct TabEntry {
    #[serde(default = "new_id")]
    pub id: String,
    pub cwd: Option<PathBuf>,
    pub restore: Option<String>,
    pub title: Option<String>,
    #[serde(default)]
    pub notified: bool,
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Default, Serialize, Deserialize)]
pub struct Session {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<Size>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected: Option<String>,
    #[serde(default)]
    pub tabs: Vec<TabEntry>,
}

pub fn encode(session: &Session) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec_pretty(session)?)
}

fn history_file(dir: &Path, id: &str) -> Option<PathBuf> {
    let safe = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    safe.then(|| dir.join(id))
}

pub fn history_path(id: &str) -> Result<Option<PathBuf>> {
    Ok(history_file(&paths::history_dir()?, id))
}

pub fn write_history(id: &str, history: &[u8]) -> Result<()> {
    match history_path(id)? {
        Some(path) => files::write(&path, history),
        None => Ok(()),
    }
}

pub fn read_history(id: &str) -> Result<Option<Vec<u8>>> {
    let Some(path) = history_path(id)? else {
        return Ok(None);
    };
    match fs::read(&path) {
        Ok(history) => Ok(Some(history)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("can't read {}", path.display())),
    }
}

pub fn prune_histories(keep: &[TabEntry]) -> Result<()> {
    let dir = paths::history_dir()?;
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("can't list {}", dir.display()));
        }
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let set_aside = name.to_string_lossy().contains(BAD);
        if !set_aside && !keep.iter().any(|tab| name == tab.id.as_str()) {
            let _ = fs::remove_file(entry.path());
        }
    }
    Ok(())
}

const BAD: &str = ".bad";

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn free_suffix(paths: &[&Path]) -> String {
    let mut number = 0;
    loop {
        let suffix = match number {
            0 => BAD.to_string(),
            number => format!("{BAD}.{number}"),
        };
        if !paths.iter().any(|path| exists(&with_suffix(path, &suffix))) {
            return suffix;
        }
        number += 1;
    }
}

pub fn set_aside(paths: &[&Path]) -> Result<Vec<PathBuf>> {
    let suffix = free_suffix(paths);
    let mut moved = Vec::new();
    for path in paths.iter().filter(|path| exists(path)) {
        let target = with_suffix(path, &suffix);
        fs::rename(path, &target)
            .with_context(|| format!("can't move {} to {}", path.display(), target.display()))?;
        moved.push(target);
    }
    Ok(moved)
}

pub fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("can't read {}", path.display())),
    }
}

pub fn parse(path: &Path, bytes: Option<&[u8]>) -> Result<Session> {
    match bytes {
        Some(bytes) => serde_json::from_slice(bytes)
            .with_context(|| format!("{} is not valid", path.display())),
        None => Ok(Session::default()),
    }
}

pub fn read(path: &Path) -> Result<Session> {
    parse(path, read_bytes(path)?.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("spawnshell-{name}-{}", std::process::id()))
    }

    fn tab(id: &str, restore: Option<&str>) -> TabEntry {
        TabEntry {
            id: id.into(),
            cwd: Some(PathBuf::from("/tmp")),
            restore: restore.map(str::to_string),
            title: None,
            notified: false,
        }
    }

    #[test]
    fn round_trips_tabs_through_a_fresh_directory() {
        let dir = scratch("session");
        let path = dir.join("nested").join("session.json");
        let second = TabEntry {
            notified: true,
            ..tab("b", None)
        };
        let bytes = encode(&Session {
            size: Some(Size::new(50, 200)),
            selected: Some("b".into()),
            tabs: vec![tab("a", Some("claude --resume x")), second],
        })
        .unwrap();
        files::write(&path, &bytes).unwrap();
        let session = read(&path).unwrap();
        let _ = fs::remove_dir_all(&dir);

        assert_eq!(session.size, Some(Size::new(50, 200)));
        assert_eq!(session.selected.as_deref(), Some("b"));
        let loaded = session.tabs;
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "a");
        assert_eq!(loaded[0].restore.as_deref(), Some("claude --resume x"));
        assert!(!loaded[0].notified);
        assert!(loaded[1].restore.is_none());
        assert!(loaded[1].notified);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn missing_file_is_empty_but_corrupt_file_is_an_error() {
        let dir = scratch("corrupt");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.json");

        assert!(read(&path).unwrap().tabs.is_empty());
        fs::write(&path, b"{not json").unwrap();
        let result = read(&path);
        let _ = fs::remove_dir_all(&dir);

        assert!(result.is_err());
    }

    #[test]
    fn setting_aside_keeps_earlier_copies() {
        let dir = scratch("aside");
        let history = dir.join("history");
        let session = dir.join("session.json");
        fs::create_dir_all(&history).unwrap();
        fs::write(history.join("a"), "first history").unwrap();
        fs::write(&session, "first").unwrap();

        set_aside(&[&history, &session]).unwrap();
        fs::write(&session, "second").unwrap();
        let moved = set_aside(&[&history, &session]).unwrap();

        let first = fs::read_to_string(dir.join("session.json.bad")).unwrap();
        let second = fs::read_to_string(dir.join("session.json.bad.1")).unwrap();
        let kept = fs::read_to_string(dir.join("history.bad").join("a")).unwrap();
        let _ = fs::remove_dir_all(&dir);

        assert_eq!(first, "first");
        assert_eq!(second, "second");
        assert_eq!(kept, "first history");
        assert_eq!(moved, [dir.join("session.json.bad.1")]);
    }

    #[test]
    fn hand_written_entries_get_defaults() {
        let session: Session = serde_json::from_str(
            r#"{"tabs":[{"restore":"echo hi","screen":"left from an older version"}]}"#,
        )
        .unwrap();
        let entry = &session.tabs[0];
        assert!(!entry.id.is_empty());
        assert!(entry.cwd.is_none());
        assert_eq!(entry.restore.as_deref(), Some("echo hi"));
        assert!(!entry.notified);
    }
}
