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
mod pty;
mod save;
mod server;
mod session;
mod settings;

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};

use self::flow::Wanted;
use self::hub::{Hub, Restored, Tab};
use self::launch::Launch;
use self::notifications::TabNotification;
use self::process::Ownership;
use self::pty::Size;
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

fn block_signals() -> libc::sigset_t {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&raw mut set);
        for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            libc::sigaddset(&raw mut set, signal);
        }
        libc::pthread_sigmask(libc::SIG_BLOCK, &raw const set, std::ptr::null_mut());
        set
    }
}

fn watch_signals(exit: mpsc::Sender<Exit>, set: libc::sigset_t) {
    thread::spawn(move || {
        let mut signal = 0;
        while unsafe { libc::sigwait(&raw const set, &raw mut signal) } != 0 {}
        let _ = exit.send(Exit::Signal);
    });
}

fn bind(socket: &Path) -> std::io::Result<UnixListener> {
    let _ = fs::remove_file(socket);
    let previous = unsafe { libc::umask(0o077) };
    let listener = UnixListener::bind(socket);
    unsafe {
        libc::umask(previous);
    }
    let listener = listener?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

fn lock(file: &File, path: &Path) -> Result<bool> {
    let deadline = Instant::now() + LOCK_WAIT;
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EWOULDBLOCK) {
            return Err(error).with_context(|| format!("can't lock {}", path.display()));
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn open_lock(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
}

fn take_over_previous_data(dir: &Path) -> Result<()> {
    let previous = paths::previous_data_dir()?;
    if dir.exists() || !previous.exists() {
        return Ok(());
    }
    let lock_path = previous.join(paths::LOCK_FILE);
    let lock_file = match open_lock(&lock_path) {
        Ok(file) => file,
        Err(_) if dir.exists() => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("can't open {}", lock_path.display()));
        }
    };
    ensure!(
        lock(&lock_file, &lock_path)?,
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
    let lock_file =
        open_lock(&lock_path).with_context(|| format!("can't open {}", lock_path.display()))?;
    if !lock(&lock_file, &lock_path)? {
        return Ok(None);
    }
    let socket = protocol::socket()?;
    let listener =
        bind(&socket).with_context(|| format!("can't listen on {}", socket.display()))?;

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
    let hub = Hub::new(listener, lock_file, restored, exit, notification, version);
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
    let signals = block_signals();
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

    watch_signals(exit, signals);
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
