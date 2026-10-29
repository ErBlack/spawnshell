use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};
use std::thread;

use anyhow::{Result, bail, ensure};
use protocol::{Command, Event, Failure, Frame, Reply, Role};

use super::flow::Wanted;
use super::hub::{Gui, Hub, Tab, event_frame};
use super::launch::{Launch, Spawned};
use super::process::{Ownership, Proc};
use super::{Exit, report};
use crate::pty::Size;
use crate::session::{self, TabEntry};

const UNPINNED_RESTORE: &str = "only pinned tabs have a restore command";

impl Hub {
    fn create(self: &Arc<Self>, pinned: bool, cwd: Option<PathBuf>) -> Result<String> {
        if let Some(cwd) = &cwd {
            ensure!(
                cwd.is_absolute(),
                "{} is not an absolute path",
                cwd.display()
            );
            ensure!(cwd.exists(), "{} does not exist", cwd.display());
            ensure!(cwd.is_dir(), "{} is not a directory", cwd.display());
        }
        let size = self.state.lock().size;
        let id = session::new_id();
        let output = self.new_output(&id, size);
        let launch = Launch {
            cwd,
            ..Launch::default()
        };
        let spawned = self.launch(&id, &output, size, launch)?;
        let mut state = self.state.lock();
        if self.upgrading.load(Ordering::SeqCst) {
            spawned.discard();
            bail!(Failure::Updating);
        }
        let Spawned { pid, master, cwd } = spawned;
        let entry = TabEntry {
            id: id.clone(),
            cwd: Some(cwd),
            restore: None,
            title: None,
            notified: false,
        };
        let mut tab = Tab::new(entry, pinned, output);
        self.attach_process(&mut tab, pid, master, Ownership::Child, Wanted::Run)?;
        if state.size != size {
            tab.resize(state.size);
        }
        state.insert(tab, None);
        Ok(id)
    }

    fn close(&self, id: &str, force: bool) -> Result<()> {
        let mut state = self.state.lock();
        let index = state.find(id)?;
        if state.tabs[index].pinned && !force {
            state.tabs[index].unload();
            drop(state);
            self.changed();
            return Ok(());
        }
        let mut tab = state.tabs.remove(index);
        let empty = state.tabs.is_empty();
        drop(state);
        tab.dismiss_notification();
        if let Some(proc) = tab.proc {
            proc.terminate();
        }
        self.changed();
        if empty {
            self.emptied();
        }
        Ok(())
    }

    fn activate(&self, id: &str) -> Result<()> {
        let mut state = self.state.lock();
        state.find(id)?;
        state.active = Some(id.to_string());
        drop(state);
        self.activated(id);
        Ok(())
    }

    fn place(&self, id: &str, pinned: Option<bool>, position: Option<usize>) -> Result<()> {
        let mut state = self.state.lock();
        let index = state.find(id)?;
        let mut tab = state.tabs.remove(index);
        tab.pinned = pinned.unwrap_or(tab.pinned);
        state.insert(tab, position);
        drop(state);
        self.changed();
        Ok(())
    }

    fn rename(&self, id: &str, title: &str) -> Result<()> {
        let title = title.trim();
        ensure!(
            !title.chars().any(char::is_control),
            "a tab title can't contain control characters"
        );
        let mut state = self.state.lock();
        let index = state.find(id)?;
        state.tabs[index].entry.title = (!title.is_empty()).then(|| title.to_string());
        drop(state);
        self.changed();
        Ok(())
    }

    fn restore(&self, id: &str) -> Result<Option<String>> {
        let state = self.state.lock();
        let tab = &state.tabs[state.find(id)?];
        ensure!(tab.pinned, UNPINNED_RESTORE);
        Ok(tab.entry.restore.clone())
    }

    fn set_restore(&self, id: &str, restore: &str) -> Result<()> {
        let restore = restore.trim();
        let mut state = self.state.lock();
        let index = state.find(id)?;
        let tab = &mut state.tabs[index];
        ensure!(tab.pinned, UNPINNED_RESTORE);
        tab.entry.restore = (!restore.is_empty()).then(|| restore.to_string());
        drop(state);
        self.changed();
        Ok(())
    }

    fn restart(self: &Arc<Self>, id: &str) -> Result<()> {
        let size = self.state.lock().size;
        let output = self.new_output(id, size);
        let (previous, cwd, restore, generation) = {
            let mut state = self.state.lock();
            ensure!(!self.upgrading.load(Ordering::SeqCst), Failure::Updating);
            let index = state.find(id)?;
            let tab = &mut state.tabs[index];
            ensure!(tab.pinned, "only pinned tabs can be restarted");
            tab.output.lock().mute();
            tab.unload();
            let previous = std::mem::replace(&mut tab.output, Arc::clone(&output));
            (
                previous,
                tab.entry.cwd.clone(),
                tab.entry.restore.clone(),
                tab.generation,
            )
        };
        self.changed();

        let history = previous.lock().history().to_vec();
        let launch = Launch {
            cwd,
            restore,
            history: Some(history),
        };
        let spawned = self.launch(id, &output, size, launch)?;
        let mut state = self.state.lock();
        if self.upgrading.load(Ordering::SeqCst) {
            spawned.discard();
            bail!(Failure::Updating);
        }
        let current = state.size;
        let Some(tab) = state
            .tabs
            .iter_mut()
            .find(|tab| tab.entry.id == id && tab.generation == generation)
        else {
            spawned.discard();
            bail!("tab {id} changed while restarting");
        };
        let Spawned { pid, master, cwd } = spawned;
        tab.entry.cwd = Some(cwd);
        self.attach_process(tab, pid, master, Ownership::Child, Wanted::Run)?;
        if current != size {
            tab.resize(current);
        }
        drop(state);
        self.changed();
        Ok(())
    }

    fn resize(&self, size: Size) {
        let mut state = self.state.lock();
        state.size = size;
        for tab in &state.tabs {
            tab.resize(size);
        }
    }

    fn attach(&self, id: &str, tx: &mpsc::Sender<Frame>) -> Result<()> {
        let output = {
            let state = self.state.lock();
            let tab = &state.tabs[state.find(id)?];
            if let Some(proc) = &tab.proc {
                proc.gate.want(Wanted::Run);
            }
            Arc::clone(&tab.output)
        };
        let output = output.lock();
        let _ = tx.send(Frame::Snapshot {
            tab: id.to_string(),
            data: output.snapshot(),
        });
        let _ = tx.send(event_frame(Event::Title {
            tab: id.to_string(),
            title: output.title().map(str::to_string),
        }));
        Ok(())
    }

    fn set_paused(&self, id: &str, paused: bool) -> Result<()> {
        let state = self.state.lock();
        let index = state.find(id)?;
        if let Some(proc) = &state.tabs[index].proc {
            proc.gate.want(Wanted::paused(paused));
        }
        Ok(())
    }

    pub(super) fn input(&self, id: &str, data: Vec<u8>) {
        let state = self.state.lock();
        if let Some(proc) = state
            .tabs
            .iter()
            .find(|tab| tab.entry.id == id)
            .and_then(|tab| tab.proc.as_ref())
        {
            let _ = proc.input.send(data);
        }
    }

    pub(super) fn gui_disconnected(&self, client: u64) {
        {
            let mut slot = self.gui.lock();
            if slot.as_ref().map(|gui| gui.client) != Some(client) {
                return;
            }
            *slot = None;
        }
        if self.upgrading.load(Ordering::SeqCst) {
            return;
        }
        self.set_window_open(false);
        let closed: Vec<Tab> = self
            .state
            .lock()
            .tabs
            .extract_if(.., |tab| !tab.pinned)
            .collect();
        for mut tab in closed {
            tab.dismiss_notification();
            if let Some(proc) = tab.proc {
                proc.terminate();
            }
        }
        self.resume_all();
        if let Err(error) = self.save() {
            report(&error);
        }
        self.exit_if_idle();
    }

    fn stop(&self) {
        self.begin_stop();
        if let Err(error) = self.save() {
            report(&error);
        }
        let procs: Vec<Proc> = self
            .state
            .lock()
            .tabs
            .iter_mut()
            .filter_map(|tab| tab.proc.take())
            .collect();
        let waits: Vec<_> = procs.into_iter().map(Proc::terminate).collect();
        self.notify(Event::Stop);
        if let Ok(socket) = protocol::socket() {
            let _ = fs::remove_file(socket);
        }
        let exit = self.exit.clone();
        thread::spawn(move || {
            for wait in waits {
                let _ = wait.join();
            }
            let _ = exit.send(Exit::Stopped);
        });
    }

    fn forward(&self, event: Event) -> Result<Reply> {
        self.require_gui()?;
        self.notify(event);
        Ok(Reply::Done)
    }

    pub(super) fn handle(
        self: &Arc<Self>,
        command: Command,
        client: u64,
        tx: &mpsc::Sender<Frame>,
    ) -> Result<Reply> {
        if self.upgrading.load(Ordering::SeqCst)
            && !matches!(
                command,
                Command::List | Command::Restore { .. } | Command::Config
            )
        {
            bail!(Failure::Updating);
        }
        match command {
            Command::Hello { role } => {
                if role == Role::Gui {
                    *self.gui.lock() = Some(Gui {
                        client,
                        tx: tx.clone(),
                    });
                    self.resume_all();
                }
                let warning = self.settings.lock().problem.clone();
                Ok(Reply::Hello {
                    build: protocol::build(),
                    warning,
                    config: (role == Role::Gui).then(|| self.config()),
                    version: Some(self.version.to_string()),
                    pid: Some(std::process::id()),
                })
            }
            Command::List => Ok(Reply::Tabs { tabs: self.infos() }),
            Command::New { pinned, cwd } => {
                self.require_gui()?;
                let tab = self.create(pinned, cwd)?;
                self.activate(&tab)?;
                self.changed();
                self.notify(Event::Select { tab: tab.clone() });
                Ok(Reply::Tab { tab })
            }
            Command::Close { tab, force } => self.close(&tab, force).map(|()| Reply::Done),
            Command::Select { tab } => {
                self.require_gui()?;
                self.activate(&tab)?;
                self.notify(Event::Select { tab });
                Ok(Reply::Done)
            }
            Command::Activate { tab } => self.activate(&tab).map(|()| Reply::Done),
            Command::Place {
                tab,
                pinned,
                position,
            } => self.place(&tab, pinned, position).map(|()| Reply::Done),
            Command::Restart { tab } => self.restart(&tab).map(|()| Reply::Done),
            Command::Rename { tab, title } => self.rename(&tab, &title).map(|()| Reply::Done),
            Command::Restore { tab } => {
                self.restore(&tab).map(|restore| Reply::Restore { restore })
            }
            Command::SetRestore { tab, restore } => {
                self.set_restore(&tab, &restore).map(|()| Reply::Done)
            }
            Command::Resize { rows, cols } => {
                self.resize(Size::new(rows, cols));
                Ok(Reply::Done)
            }
            Command::Attach { tab } => self.attach(&tab, tx).map(|()| Reply::Done),
            Command::Pause { tab } => self.set_paused(&tab, true).map(|()| Reply::Done),
            Command::Resume { tab } => self.set_paused(&tab, false).map(|()| Reply::Done),
            Command::Save => self.save().map(|()| Reply::Done),
            Command::Show => self.forward(Event::Show),
            Command::Hide => self.forward(Event::Hide),
            Command::Toggle => self.forward(Event::Toggle),
            Command::Quit => self.forward(Event::Quit),
            Command::Stop => {
                self.stop();
                Ok(Reply::Done)
            }
            Command::Upgrade { exe } => self.upgrade(Path::new(&exe)).map(|()| Reply::Done),
            Command::Config => Ok(Reply::Config {
                config: self.config(),
            }),
            Command::SetConfig { key, value } => self.set_config(key, &value).map(|()| Reply::Done),
            Command::Notify { tab, title, body } => self
                .request_notification(&tab, title.as_deref(), &body)
                .map(|()| Reply::Done),
            Command::SetWindowOpen { open } => {
                self.set_window_open(open);
                Ok(Reply::Done)
            }
        }
    }
}
