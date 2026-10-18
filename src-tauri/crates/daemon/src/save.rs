use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::Result;
use parking_lot::Mutex;

use super::hub::Hub;
use super::output::Output;
use super::report;
use crate::files;
use crate::paths;
use crate::pty::Size;
use crate::session::{self, TabEntry};

pub(super) fn set_aside(error: anyhow::Error, paths: &[&Path]) -> Result<()> {
    match session::set_aside(paths) {
        Ok(moved) => {
            let moved: Vec<String> = moved
                .iter()
                .map(|path| path.display().to_string())
                .collect();
            report(&error.context(format!("moved to {}", moved.join(", "))));
            Ok(())
        }
        Err(failed) => Err(failed.context(format!("{error:#}"))),
    }
}

pub(super) fn restore_history(id: &str) -> Option<Vec<u8>> {
    session::read_history(id).unwrap_or_else(|error| {
        let outcome = session::history_path(id).and_then(|path| match path {
            Some(path) => set_aside(error, &[&path]),
            None => Err(error),
        });
        if let Err(error) = outcome {
            report(&error);
        }
        None
    })
}

fn write_histories(outputs: &[(String, Arc<Mutex<Output>>)], rewrite: bool) -> Result<()> {
    let mut taken = Vec::new();
    for (id, output) in outputs {
        let mut locked = output.lock();
        if rewrite {
            locked.mark_dirty();
        }
        if let Some(history) = locked.take_dirty_history() {
            taken.push((id, history, output));
        }
    }
    let written = taken
        .iter()
        .try_for_each(|(id, history, _)| session::write_history(id, history));
    if written.is_err() {
        for (_, _, output) in &taken {
            output.lock().mark_dirty();
        }
    }
    written
}

impl Hub {
    pub(super) fn save(&self) -> Result<()> {
        self.persist(true)
    }

    pub(super) fn save_tabs(&self) -> Result<()> {
        self.persist(false)
    }

    fn persist(&self, histories: bool) -> Result<()> {
        let _guard = self.saving.lock();
        if self.upgrading.load(Ordering::SeqCst) {
            return Ok(());
        }
        let path = paths::session_file()?;
        let on_disk = session::read_bytes(&path)
            .and_then(|raw| session::parse(&path, raw.as_deref()).map(|_| raw));
        let (raw, rewrite_histories) = match on_disk {
            Ok(raw) => (raw, false),
            Err(error) => {
                let history = paths::history_dir()?;
                set_aside(error, &[&history, &path])?;
                (None, true)
            }
        };
        let mut outputs = Vec::new();
        let (size, selected, tabs): (Size, Option<String>, Vec<TabEntry>) = {
            let mut state = self.state.lock();
            let size = state.size;
            let selected = state.active.clone().filter(|id| {
                state
                    .tabs
                    .iter()
                    .any(|tab| tab.pinned && &tab.entry.id == id)
            });
            let tabs = state
                .tabs
                .iter_mut()
                .filter(|tab| tab.pinned)
                .map(|tab| {
                    tab.refresh_cwd();
                    outputs.push((tab.entry.id.clone(), Arc::clone(&tab.output)));
                    tab.entry.clone()
                })
                .collect();
            (size, selected, tabs)
        };
        if histories || rewrite_histories {
            write_histories(&outputs, rewrite_histories)?;
        }
        let session = session::Session {
            size: Some(size),
            selected,
            tabs,
        };
        let bytes = session::encode(&session)?;
        if raw.as_deref() == Some(bytes.as_slice()) {
            return Ok(());
        }
        files::write(&path, &bytes)?;
        session::prune_histories(&session.tabs)
    }
}
