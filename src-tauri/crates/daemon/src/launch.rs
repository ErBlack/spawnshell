use std::fs;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use protocol::Frame;

use super::flow::{Gate, Wanted};
use super::hub::{Hub, Tab};
use super::io::{start_io, start_writer};
use super::output::{Output, RESET_AFTER_HISTORY};
use super::process::{Ownership, Proc, hang_up};
use super::report;
use crate::paths;
use crate::pty::{self, Size};

#[derive(Default)]
pub(super) struct Launch {
    pub(super) cwd: Option<PathBuf>,
    pub(super) restore: Option<String>,
    pub(super) history: Option<Vec<u8>>,
}

pub(super) struct Spawned {
    pub(super) pid: i32,
    pub(super) master: OwnedFd,
    pub(super) cwd: PathBuf,
}

impl Spawned {
    pub(super) fn discard(self) {
        hang_up(self.pid, None, Ownership::Child);
    }
}

pub(super) struct Shell {
    path: String,
    integration: Option<PathBuf>,
}

impl Shell {
    pub(super) fn detect() -> Self {
        let path = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        let integration = install_integration(&path).unwrap_or_else(|error| {
            report(&error.context("shell integration is off"));
            None
        });
        Shell { path, integration }
    }

    fn arguments(&self, command: &str, restore: Option<&str>) -> Vec<String> {
        let mut args = vec!["-l".to_string()];
        let command = (!command.trim().is_empty()).then(|| command.to_string());
        match (restore, command) {
            (Some(restore), command) => {
                let next = command.unwrap_or_else(|| match &self.integration {
                    Some(dir) => format!(
                        "exec env ZDOTDIR={} {} -l",
                        quote(&dir.to_string_lossy()),
                        quote(&self.path)
                    ),
                    None => format!("exec {} -l", quote(&self.path)),
                });
                let start = if self.integration.is_some() {
                    "builtin print -n '\\e]133;C\\a'\n"
                } else {
                    ""
                };
                args.extend([
                    "-i".into(),
                    "-c".into(),
                    format!("{start}{restore}\n{next}"),
                ]);
            }
            (None, Some(command)) => args.extend(["-c".into(), command]),
            (None, None) => {}
        }
        args
    }
}

fn install_integration(shell: &str) -> Result<Option<PathBuf>> {
    if Path::new(shell)
        .file_name()
        .is_none_or(|name| name != "zsh")
    {
        return Ok(None);
    }
    let dir = paths::data_dir()?.join("shell-integration").join("zsh");
    fs::create_dir_all(&dir).with_context(|| format!("can't create {}", dir.display()))?;
    let file = dir.join(".zshenv");
    fs::write(&file, include_str!("../zsh/.zshenv"))
        .with_context(|| format!("can't write {}", file.display()))?;
    Ok(Some(dir))
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
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
        let integration = self
            .shell
            .integration
            .as_ref()
            .map(|dir| dir.to_string_lossy().into_owned());
        let original = std::env::var("ZDOTDIR").ok();
        let mut env = vec![
            ("TERM", "xterm-256color"),
            ("COLORTERM", "truecolor"),
            ("SPAWNSHELL_TAB", id),
        ];
        if let Some(integration) = &integration {
            env.push(("ZDOTDIR", integration));
            if let Some(original) = &original {
                env.push(("SPAWNSHELL_ZSH_ZDOTDIR", original));
            }
        }
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
                output
                    .lock()
                    .emit(format!("\x1b[31m{error:#}\x1b[0m\r\n").as_bytes());
                Err(error)
            }
        }
    }

    pub(super) fn attach_process(
        self: &Arc<Self>,
        tab: &mut Tab,
        pid: i32,
        master: OwnedFd,
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
                hang_up(pid, None, ownership);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_runs_before_the_next_command() {
        let mut shell = Shell {
            path: "/bin/zsh".into(),
            integration: None,
        };
        assert_eq!(shell.arguments("", None), ["-l"]);
        assert_eq!(
            shell.arguments("", Some("claude --resume 'x'")),
            ["-l", "-i", "-c", "claude --resume 'x'\nexec '/bin/zsh' -l"]
        );

        shell.integration = Some(PathBuf::from("/s"));
        assert_eq!(
            shell.arguments("", Some("claude")),
            [
                "-l",
                "-i",
                "-c",
                "builtin print -n '\\e]133;C\\a'\nclaude\nexec env ZDOTDIR='/s' '/bin/zsh' -l"
            ]
        );

        shell.integration = None;
        assert_eq!(shell.arguments("htop", None), ["-l", "-c", "htop"]);
        assert_eq!(
            shell.arguments("htop", Some("echo hi")),
            ["-l", "-i", "-c", "echo hi\nhtop"]
        );
    }
}
