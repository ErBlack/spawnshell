use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use platform::Size;
use platform::handoff::{self, Handle};
use platform::ipc::{Listener, Lock};
use platform::process::{Pid, detach};
use platform::pty::Master;
use serde::{Deserialize, Serialize};

use super::Exit;
use super::flow::{Gate, Wanted};
use super::hub::{Hub, Restored, Tab};
use super::modes::ProgramTitle;
use super::notifications::TabNotification;
use super::process::Ownership;
use super::save::restore_history;
use crate::session::{self, TabEntry};

const PARK_TIMEOUT: Duration = Duration::from_secs(2);
const HANDOFF_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Serialize, Deserialize)]
struct Manifest {
    listener: Handle,
    lock: Handle,
    size: Size,
    active: Option<String>,
    tabs: Vec<ManifestTab>,
}

#[derive(Serialize, Deserialize)]
struct ManifestTab {
    #[serde(flatten)]
    entry: TabEntry,
    pinned: bool,
    screen: String,
    #[serde(default)]
    alternate: bool,
    #[serde(default)]
    modes: String,
    program_title: Option<ProgramTitle>,
    process: Option<(Pid, Handle)>,
}

impl Hub {
    pub(super) fn upgrade(&self, exe: &Path) -> Result<()> {
        let _saving = self.saving.lock();
        ensure!(
            !self.upgrading.swap(true, Ordering::SeqCst),
            "an update is already in progress"
        );
        let gates: Vec<Gate> = self
            .state
            .lock()
            .tabs
            .iter()
            .filter_map(|tab| tab.proc.as_ref().map(|proc| proc.gate.clone()))
            .collect();
        for gate in &gates {
            gate.hold(true);
        }
        let result = self.hand_off(exe, &gates);
        if result.is_err() {
            self.upgrading.store(false, Ordering::SeqCst);
            for gate in &gates {
                gate.hold(false);
            }
        }
        result
    }

    fn hand_off(&self, exe: &Path, gates: &[Gate]) -> Result<()> {
        let deadline = Instant::now() + PARK_TIMEOUT;
        ensure!(
            gates.iter().all(|gate| gate.wait_parked(deadline)),
            "terminal readers did not pause"
        );

        let (manifest, histories) = {
            let state = self.state.lock();
            let histories: Vec<(String, Vec<u8>)> = state
                .tabs
                .iter()
                .map(|tab| (tab.entry.id.clone(), tab.output.lock().history().to_vec()))
                .collect();
            let manifest = Manifest {
                listener: self.listener.handle(),
                lock: self.lock.handle(),
                size: state.size,
                active: state.active.clone(),
                tabs: state.tabs.iter().map(manifest_tab).collect(),
            };
            (manifest, histories)
        };
        for (id, history) in &histories {
            session::write_history(id, history)?;
        }
        spawn_successor(exe, &manifest)?;
        self.request_exit(Exit::Handoff);
        Ok(())
    }
}

fn manifest_tab(tab: &Tab) -> ManifestTab {
    let output = tab.output.lock();
    ManifestTab {
        entry: tab.entry.clone(),
        pinned: tab.pinned,
        screen: output.screen(),
        alternate: output.alternate(),
        modes: output.modes(),
        program_title: Some(output.program_title()),
        process: tab
            .proc
            .as_ref()
            .map(|proc| (proc.pid, proc.master.handle())),
    }
}

fn spawn_successor(exe: &Path, manifest: &Manifest) -> Result<()> {
    let json = serde_json::to_vec(manifest)?;
    let mut handles = vec![manifest.listener, manifest.lock];
    handles.extend(
        manifest
            .tabs
            .iter()
            .filter_map(|tab| tab.process.map(|(_, handle)| handle)),
    );
    let mut command = std::process::Command::new(exe);
    command
        .arg("__daemon")
        .arg("--adopt")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    detach(&mut command);
    handoff::inherit(&mut command, handles);
    let mut child = command
        .spawn()
        .with_context(|| format!("can't start {}", exe.display()))?;

    let ready = (|| {
        let mut stdin = child.stdin.take().context("no stdin")?;
        let stdout = child.stdout.take().context("no stdout")?;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut line = String::new();
            let answered = stdin.write_all(&json).and_then(|()| {
                drop(stdin);
                BufReader::new(stdout).read_line(&mut line)
            });
            let _ = tx.send(answered.map(|_| line));
        });
        match rx.recv_timeout(HANDOFF_TIMEOUT) {
            Ok(Ok(line)) if line.trim() == "ready" => Ok(()),
            Ok(Ok(_)) => bail!("the new background process failed to start"),
            Ok(Err(error)) => Err(error).context("can't talk to the new background process"),
            Err(_) => bail!("the new background process did not answer"),
        }
    })();
    if ready.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    ready
}

pub(super) fn adopt(
    exit: mpsc::Sender<Exit>,
    notification: mpsc::Sender<TabNotification>,
    version: &'static str,
) -> Result<Arc<Hub>> {
    let mut text = String::new();
    io::stdin().read_to_string(&mut text)?;
    let manifest: Manifest = serde_json::from_str(&text)?;
    let listener = unsafe { Listener::adopt(manifest.listener) }?;
    let lock = unsafe { Lock::adopt(manifest.lock) }?;
    let histories: Vec<_> = manifest
        .tabs
        .iter()
        .map(|adopted| restore_history(&adopted.entry.id))
        .collect();
    let restored = Restored {
        size: manifest.size,
        active: manifest.active,
    };
    let hub = Hub::new(listener, lock, restored, exit, notification, version);
    let mut state = hub.state.lock();
    for (adopted, history) in manifest.tabs.into_iter().zip(histories) {
        let output = hub.new_output(&adopted.entry.id, manifest.size);
        {
            let mut output = output.lock();
            if adopted.alternate {
                output.parser.process(b"\x1b[?1049h");
            }
            output.parser.process(adopted.screen.as_bytes());
            output.restore_modes(&adopted.modes);
            if let Some(title) = adopted.program_title {
                output.restore_program_title(title);
            }
            output.set_history(history.unwrap_or_else(|| adopted.screen.clone().into_bytes()));
        }
        let mut tab = Tab::new(adopted.entry, adopted.pinned, output);
        if let Some((pid, handle)) = adopted.process {
            let master = unsafe { Master::adopt(handle) }?;
            hub.attach_process(&mut tab, pid, master, Ownership::Adopted, Wanted::Pause)?;
        }
        state.tabs.push(tab);
    }
    drop(state);
    Ok(hub)
}

#[cfg(test)]
mod tests {
    use super::{ManifestTab, Pid};

    #[test]
    fn a_process_from_an_older_daemon_is_read_from_plain_numbers() {
        let tab: ManifestTab =
            serde_json::from_str(r#"{"id":"a","pinned":true,"screen":"$ ","process":[42,7]}"#)
                .unwrap();
        assert_eq!(
            tab.process.unwrap().0,
            serde_json::from_str::<Pid>("42").unwrap()
        );
    }

    #[test]
    fn a_tab_from_an_older_daemon_is_adopted() {
        let tab: ManifestTab = serde_json::from_str(
            r#"{"id":"a","pinned":true,"restore":"echo hi","cwd":"/tmp","screen":"$ ","process":null}"#,
        )
        .unwrap();
        assert_eq!(tab.entry.id, "a");
        assert_eq!(tab.entry.restore.as_deref(), Some("echo hi"));
        assert!(tab.entry.title.is_none());
        assert!(tab.pinned);
    }
}
