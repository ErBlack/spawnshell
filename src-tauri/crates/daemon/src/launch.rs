use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use platform::Size;
use platform::process::Pid;
use platform::pty::{self, Master};
use protocol::Frame;

use super::flow::{Gate, Wanted};
use super::hub::{Hub, Tab};
use super::io::{start_io, start_writer};
use super::output::{Output, RESET_AFTER_HISTORY};
use super::process::{Ownership, Proc, shut_down};

#[derive(Default)]
pub(super) struct Launch {
    pub(super) cwd: Option<PathBuf>,
    pub(super) restore: Option<String>,
    pub(super) history: Option<Vec<u8>>,
}

pub(super) struct Spawned {
    pub(super) pid: Pid,
    pub(super) master: Master,
    pub(super) cwd: PathBuf,
}

impl Spawned {
    pub(super) fn discard(self) {
        shut_down(self.pid, None, Ownership::Child);
    }
}

impl Hub {
    pub(super) fn launch(
        &self,
        id: &str,
        output: &Mutex<Output>,
        size: Size,
        launch: Launch,
    ) -> Result<Spawned> {
        let (cwd, notice) = match launch.cwd {
            Some(dir) if dir.is_dir() => (Some(dir), None),
            Some(dir) => (None, Some(format!("{} no longer exists", dir.display()))),
            None => (None, None),
        };
        let restore = launch.restore.filter(|_| notice.is_none());

        let mut prelude = Vec::new();
        if let Some(history) = &launch.history {
            prelude.extend_from_slice(b"\x1bc");
            prelude.extend_from_slice(history);
            prelude.extend_from_slice(RESET_AFTER_HISTORY);
        }
        if let Some(notice) = &notice {
            prelude.extend_from_slice(
                format!("\x1b[33m[Spawnshell: {notice}, restore skipped]\x1b[0m\r\n").as_bytes(),
            );
        }
        if launch.history.is_some() {
            let mut output = output.lock();
            output.consume(&prelude);
            let data = output.snapshot();
            if let Some(gui) = self.gui.lock().as_ref() {
                let _ = gui.tx.send(Frame::Snapshot {
                    tab: id.to_string(),
                    data,
                });
            }
        } else if !prelude.is_empty() {
            output.lock().emit(&prelude);
        }

        let dir = cwd
            .or_else(|| self.home.clone())
            .unwrap_or_else(|| PathBuf::from("/"));
        let command = self.config().command;
        let args = self.shell.arguments(&command, restore.as_deref());
        let shell_env = self.shell.env();
        let mut env = vec![
            ("TERM", "xterm-256color"),
            ("COLORTERM", "truecolor"),
            ("SPAWNSHELL_TAB", id),
        ];
        env.extend(
            shell_env
                .iter()
                .map(|(name, value)| (*name, value.as_str())),
        );
        let spawned = pty::spawn(&pty::Spawn {
            program: &self.shell.path,
            args: &args,
            cwd: &dir,
            env: &env,
            size,
        });
        match spawned.context("Spawnshell couldn't start the shell") {
            Ok((pid, master)) => Ok(Spawned {
                pid,
                master,
                cwd: dir,
            }),
            Err(error) => {
                let mut output = output.lock();
                output.emit(format!("\x1b[31m{error:#}\x1b[0m\r\n").as_bytes());
                output.stop();
                Err(error)
            }
        }
    }

    pub(super) fn attach_process(
        self: &Arc<Self>,
        tab: &mut Tab,
        pid: Pid,
        master: Master,
        ownership: Ownership,
        initial: Wanted,
    ) -> Result<()> {
        let gate = Gate::new(initial);
        let hub = Arc::clone(self);
        let id = tab.entry.id.clone();
        let generation = tab.generation;
        let started = start_writer(&master).and_then(|input| {
            start_io(&master, Arc::clone(&tab.output), gate.clone(), move || {
                hub.exited(&id, generation);
            })?;
            Ok(input)
        });
        let input = match started {
            Ok(input) => input,
            Err(error) => {
                shut_down(pid, None, ownership);
                return Err(error.into());
            }
        };
        tab.proc = Some(Proc {
            pid,
            master,
            input,
            gate,
            ownership,
        });
        Ok(())
    }
}
