mod commands;
mod files;
mod flow;
mod handoff;
mod hub;
mod io;
mod launch;
mod modes;
mod notifications;
mod output;
mod paths;
mod process;
mod save;
mod server;
mod session;
mod settings;
mod shell;

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use platform::Size;
use platform::ipc::Lock;
use platform::process::Termination;

use self::flow::Wanted;
use self::hub::{Hub, Restored, Tab};
use self::launch::Launch;
use self::notifications::TabNotification;
use self::process::Ownership;
use self::save::{restore_history, set_aside};

const SAVE_INTERVAL: Duration = Duration::from_secs(15);
const STOP_GRACE: Duration = Duration::from_secs(2);
const LOCK_WAIT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
enum Exit {
    Idle,
    Stopped,
    Handoff,
    Signal,
}

fn report(error: &anyhow::Error) {
    eprintln!("spawnshell daemon: {error:#}");
}

fn watch_termination(exit: mpsc::Sender<Exit>, termination: Termination) {
    thread::spawn(move || {
        termination.wait();
        let _ = exit.send(Exit::Signal);
    });
}

fn acquire(lock: &Lock, path: &Path) -> Result<bool> {
    let deadline = Instant::now() + LOCK_WAIT;
    loop {
        let locked = lock
            .try_lock()
            .with_context(|| format!("can't lock {}", path.display()))?;
        if locked || Instant::now() >= deadline {
            return Ok(locked);
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn take_over_previous_data(dir: &Path) -> Result<()> {
    let previous = paths::previous_data_dir()?;
    if dir.exists() || !previous.exists() {
        return Ok(());
    }
    let lock_path = previous.join(paths::LOCK_FILE);
    let lock = match Lock::open(&lock_path) {
        Ok(lock) => lock,
        Err(_) if dir.exists() => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("can't open {}", lock_path.display()));
        }
    };
    ensure!(
        acquire(&lock, &lock_path)?,
        "the previous version, Summon, is still running: quit it with \"Quit and Stop Pinned Tabs\" and open Spawnshell again"
    );
    if dir.exists() {
        return Ok(());
    }
    fs::rename(&previous, dir)
        .with_context(|| format!("can't move {} to {}", previous.display(), dir.display()))
}

fn fresh(
    exit: mpsc::Sender<Exit>,
    notification: mpsc::Sender<TabNotification>,
    version: &'static str,
) -> Result<Option<Arc<Hub>>> {
    let dir = paths::data_dir()?;
    take_over_previous_data(&dir)?;
    fs::create_dir_all(&dir).with_context(|| format!("can't create {}", dir.display()))?;
    let lock_path = paths::lock_file()?;
    let lock =
        Lock::open(&lock_path).with_context(|| format!("can't open {}", lock_path.display()))?;
    if !acquire(&lock, &lock_path)? {
        return Ok(None);
    }
    let listener = server::listen()?;

    let session_file = paths::session_file()?;
    let history_dir = paths::history_dir()?;
    let saved = session::read(&session_file).unwrap_or_else(|error| {
        if let Err(error) = set_aside(error, &[&history_dir, &session_file]) {
            report(&error);
        }
        session::Session::default()
    });
    let size = saved.size.unwrap_or(Size::new(24, 80));
    let histories: Vec<_> = saved
        .tabs
        .iter()
        .map(|entry| restore_history(&entry.id))
        .collect();
    let restored = Restored {
        size,
        active: saved.selected,
    };
    let hub = Hub::new(listener, lock, restored, exit, notification, version);
    for (entry, history) in saved.tabs.into_iter().zip(histories) {
        let output = hub.new_output(&entry.id, size);
        let launch = Launch {
            cwd: entry.cwd.clone(),
            restore: entry.restore.clone(),
            history,
        };
        let spawned = hub.launch(&entry.id, &output, size, launch);
        let mut tab = Tab::new(entry, true, output);
        let mut state = hub.state.lock();
        let attached = spawned.and_then(|spawned| {
            tab.entry.cwd = Some(spawned.cwd);
            hub.attach_process(
                &mut tab,
                spawned.pid,
                spawned.master,
                Ownership::Child,
                Wanted::Run,
            )
        });
        if let Err(error) = attached {
            report(&error);
        }
        state.tabs.push(tab);
    }
    Ok(Some(hub))
}

pub fn run(adopting: bool, version: &'static str) -> ExitCode {
    protocol::build();
    let termination = Termination::block();
    let (exit, exits) = mpsc::channel();
    let (notification, notifications) = mpsc::channel();
    let started = if adopting {
        handoff::adopt(exit.clone(), notification, version).map(Some)
    } else {
        fresh(exit.clone(), notification, version)
    };
    let hub = match started {
        Ok(Some(hub)) => hub,
        Ok(None) => return ExitCode::SUCCESS,
        Err(error) => {
            report(&error);
            return ExitCode::FAILURE;
        }
    };
    if adopting {
        let mut stdout = std::io::stdout();
        let _ = writeln!(stdout, "ready");
        let _ = stdout.flush();
        hub.resume_all();
    }

    watch_termination(exit, termination);
    if let Err(error) = hub.watch_config() {
        report(&error);
    }
    let saver = Arc::clone(&hub);
    thread::spawn(move || {
        loop {
            thread::sleep(SAVE_INTERVAL);
            if let Err(error) = saver.save() {
                report(&error);
            }
        }
    });
    hub.watch_tabs();
    hub.watch_notifications(notifications);
    let acceptor = Arc::clone(&hub);
    thread::spawn(move || acceptor.accept());

    match exits.recv() {
        Ok(Exit::Idle | Exit::Signal) | Err(_) => {
            if let Err(error) = hub.save() {
                report(&error);
            }
        }
        Ok(Exit::Stopped) => hub.wait_for_clients(STOP_GRACE),
        Ok(Exit::Handoff) => {}
    }
    ExitCode::SUCCESS
}
