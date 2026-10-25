use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use parking_lot::{Condvar, Mutex};
use protocol::{Event, Frame, FromDaemon, TabInfo};

use super::flow::Wanted;
use super::launch::Shell;
use super::notifications::TabNotification;
use super::output::{Output, Signal, Signals, Sink};
use super::process::Proc;
use super::settings::Settings;
use super::{Exit, report};
use crate::pty::{self, Size};
use crate::session::TabEntry;

const TAB_POLL_INTERVAL: Duration = Duration::from_secs(1);

pub(super) struct Tab {
    pub(super) entry: TabEntry,
    pub(super) pinned: bool,
    pub(super) output: Arc<Mutex<Output>>,
    pub(super) proc: Option<Proc>,
    pub(super) generation: u64,
}

impl Tab {
    pub(super) fn new(entry: TabEntry, pinned: bool, output: Arc<Mutex<Output>>) -> Self {
        Tab {
            entry,
            pinned,
            output,
            proc: None,
            generation: 1,
        }
    }

    pub(super) fn label(&self) -> String {
        if let Some(title) = &self.entry.title {
            return title.clone();
        }
        if let Some(title) = self.output.lock().title() {
            return title.to_string();
        }
        self.foreground()
            .and_then(pty::process_name)
            .unwrap_or_default()
    }

    pub(super) fn resize(&self, size: Size) {
        self.output.lock().resize(size);
        if let Some(proc) = &self.proc {
            let _ = pty::resize(proc.master.as_raw_fd(), size);
        }
    }

    fn foreground(&self) -> Option<i32> {
        let proc = self.proc.as_ref()?;
        pty::foreground(proc.master.as_raw_fd()).or(Some(proc.pid))
    }

    pub(super) fn refresh_cwd(&mut self) {
        if let Some(cwd) = self.foreground().and_then(pty::process_cwd) {
            self.entry.cwd = Some(cwd);
        }
    }
}

pub(super) struct Gui {
    pub(super) client: u64,
    pub(super) tx: mpsc::Sender<Frame>,
}

pub(super) struct State {
    pub(super) tabs: Vec<Tab>,
    pub(super) size: Size,
    pub(super) active: Option<String>,
}

impl State {
    pub(super) fn find(&self, id: &str) -> Result<usize> {
        self.tabs
            .iter()
            .position(|tab| tab.entry.id == id)
            .with_context(|| format!("no tab {id}"))
    }

    pub(super) fn insert(&mut self, tab: Tab, position: Option<usize>) {
        let pinned = self.tabs.partition_point(|tab| tab.pinned);
        let (start, length) = if tab.pinned {
            (0, pinned)
        } else {
            (pinned, self.tabs.len() - pinned)
        };
        let at = start + position.unwrap_or(length).min(length);
        self.tabs.insert(at, tab);
    }
}

pub(super) struct Restored {
    pub(super) size: Size,
    pub(super) active: Option<String>,
}

pub(super) struct Hub {
    pub(super) state: Mutex<State>,
    pub(super) gui: Arc<Mutex<Option<Gui>>>,
    pub(super) listener: UnixListener,
    pub(super) lock_file: File,
    pub(super) upgrading: AtomicBool,
    stopping: AtomicBool,
    pub(super) exit: mpsc::Sender<Exit>,
    pub(super) clients: AtomicU64,
    pub(super) connected: Mutex<usize>,
    pub(super) disconnected: Condvar,
    pub(super) settings: Mutex<Settings>,
    reported: Mutex<Vec<TabInfo>>,
    pub(super) saving: Mutex<()>,
    pub(super) home: Option<PathBuf>,
    pub(super) shell: Shell,
    notification: mpsc::Sender<TabNotification>,
    pub(super) window_open: AtomicBool,
    pub(super) notifications_failed: AtomicBool,
    pub(super) version: &'static str,
}

fn display_path(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

fn sink(gui: &Arc<Mutex<Option<Gui>>>, tab: &str) -> Sink {
    let gui = Arc::clone(gui);
    let tab = tab.to_string();
    Box::new(move |data| {
        if let Some(gui) = gui.lock().as_ref() {
            let _ = gui.tx.send(Frame::Output {
                tab: tab.clone(),
                data: data.to_vec(),
            });
        }
    })
}

pub(super) fn event_frame(event: Event) -> Frame {
    protocol::message(&FromDaemon::Event { event })
}

fn send_event(gui: &Mutex<Option<Gui>>, event: Event) -> bool {
    match gui.lock().as_ref() {
        Some(gui) => gui.tx.send(event_frame(event)).is_ok(),
        None => false,
    }
}

fn signals(
    gui: &Arc<Mutex<Option<Gui>>>,
    sender: &mpsc::Sender<TabNotification>,
    tab: &str,
) -> Signals {
    let gui = Arc::clone(gui);
    let sender = sender.clone();
    let tab = tab.to_string();
    Box::new(move |signal| {
        let tab = tab.clone();
        match signal {
            Signal::Title(title) => {
                send_event(&gui, Event::Title { tab, title });
            }
            Signal::Notification(notification) => {
                let _ = sender.send((tab, notification));
            }
        }
    })
}

impl Hub {
    pub(super) fn new(
        listener: UnixListener,
        lock_file: File,
        restored: Restored,
        exit: mpsc::Sender<Exit>,
        notification: mpsc::Sender<TabNotification>,
        version: &'static str,
    ) -> Arc<Self> {
        let shell = Shell::detect();
        Arc::new(Hub {
            state: Mutex::new(State {
                tabs: Vec::new(),
                size: restored.size,
                active: restored.active,
            }),
            gui: Arc::new(Mutex::new(None)),
            listener,
            lock_file,
            upgrading: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            exit,
            clients: AtomicU64::new(1),
            connected: Mutex::new(0),
            disconnected: Condvar::new(),
            settings: Mutex::new(Settings::load()),
            reported: Mutex::new(Vec::new()),
            saving: Mutex::new(()),
            home: dirs::home_dir(),
            shell,
            notification,
            window_open: AtomicBool::new(false),
            notifications_failed: AtomicBool::new(false),
            version,
        })
    }

    pub(super) fn request_exit(&self, exit: Exit) {
        let _ = self.exit.send(exit);
    }

    pub(super) fn wait_for_clients(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut connected = self.connected.lock();
        while *connected > 0 {
            if self
                .disconnected
                .wait_until(&mut connected, deadline)
                .timed_out()
            {
                return;
            }
        }
    }

    pub(super) fn new_output(&self, id: &str, size: Size) -> Arc<Mutex<Output>> {
        let sink = sink(&self.gui, id);
        let signals = signals(&self.gui, &self.notification, id);
        Arc::new(Mutex::new(Output::new(size, sink, signals)))
    }

    pub(super) fn notify(&self, event: Event) -> bool {
        send_event(&self.gui, event)
    }

    pub(super) fn require_gui(&self) -> Result<()> {
        ensure!(self.gui.lock().is_some(), "Spawnshell is not running");
        Ok(())
    }

    fn info(&self, tab: &mut Tab, active: bool) -> TabInfo {
        let pid = tab.foreground();
        tab.refresh_cwd();
        TabInfo {
            id: tab.entry.id.clone(),
            pinned: tab.pinned,
            alive: tab.proc.is_some(),
            active,
            title: tab.entry.title.clone(),
            process: pid.and_then(pty::process_name),
            cwd: tab
                .entry
                .cwd
                .as_deref()
                .map(|cwd| display_path(cwd, self.home.as_deref())),
            notified: tab.entry.notified,
        }
    }

    pub(super) fn infos(&self) -> Vec<TabInfo> {
        let mut state = self.state.lock();
        let active = state.active.clone();
        state
            .tabs
            .iter_mut()
            .map(|tab| {
                let is_active = active.as_ref() == Some(&tab.entry.id);
                self.info(tab, is_active)
            })
            .collect()
    }

    fn publish_tabs(&self, only_if_changed: bool) {
        let mut reported = self.reported.lock();
        let tabs = self.infos();
        if only_if_changed && *reported == tabs {
            return;
        }
        reported.clone_from(&tabs);
        self.notify(Event::Tabs { tabs });
    }

    pub(super) fn watch_tabs(self: &Arc<Self>) {
        let hub = Arc::clone(self);
        thread::spawn(move || {
            loop {
                thread::sleep(TAB_POLL_INTERVAL);
                if hub.gui.lock().is_some() {
                    hub.publish_tabs(true);
                }
            }
        });
    }

    pub(super) fn changed(&self) {
        self.publish_tabs(false);
        if let Err(error) = self.save_tabs() {
            report(&error);
        }
    }

    pub(super) fn begin_stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
    }

    pub(super) fn exit_if_idle(&self) {
        if !self.stopping.load(Ordering::SeqCst)
            && self.gui.lock().is_none()
            && self.state.lock().tabs.is_empty()
        {
            self.request_exit(Exit::Idle);
        }
    }

    pub(super) fn emptied(&self) {
        if !self.notify(Event::Hide) {
            self.exit_if_idle();
        }
    }

    pub(super) fn exited(&self, id: &str, generation: u64) {
        let mut state = self.state.lock();
        let Some(index) = state
            .tabs
            .iter()
            .position(|tab| tab.entry.id == id && tab.generation == generation)
        else {
            return;
        };
        if let Some(proc) = state.tabs[index].proc.take() {
            proc.finished();
        }
        let removed = (!state.tabs[index].pinned).then(|| state.tabs.remove(index));
        let empty = state.tabs.is_empty();
        drop(state);
        if let Some(mut tab) = removed {
            tab.dismiss_notification();
        }
        self.changed();
        if empty {
            self.emptied();
        }
    }

    pub(super) fn resume_all(&self) {
        for tab in &self.state.lock().tabs {
            if let Some(proc) = &tab.proc {
                proc.gate.want(Wanted::Run);
            }
        }
    }
}
