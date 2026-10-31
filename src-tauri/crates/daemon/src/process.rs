use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use platform::process::{Pid, child_running, exists, hang_up, kill, reap};
use platform::pty::Master;

use super::flow::{Gate, Wanted};

const TERMINATE_GRACE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Ownership {
    Child,
    Adopted,
}

pub(super) struct Proc {
    pub(super) pid: Pid,
    pub(super) master: Master,
    pub(super) input: mpsc::Sender<Vec<u8>>,
    pub(super) gate: Gate,
    pub(super) ownership: Ownership,
}

fn alive(pid: Pid, ownership: Ownership) -> bool {
    match ownership {
        Ownership::Child => child_running(pid),
        Ownership::Adopted => exists(pid),
    }
}

pub(super) fn shut_down(
    pid: Pid,
    foreground: Option<Pid>,
    ownership: Ownership,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if let Some(foreground) = foreground.filter(|&foreground| foreground != pid) {
            hang_up(foreground);
        }
        hang_up(pid);
        let deadline = Instant::now() + TERMINATE_GRACE;
        while Instant::now() < deadline {
            if !alive(pid, ownership) {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        kill(pid);
        if ownership == Ownership::Child {
            reap(pid);
        }
    })
}

impl Proc {
    pub(super) fn terminate(self) -> thread::JoinHandle<()> {
        self.gate.want(Wanted::Stop);
        let foreground = self.master.foreground();
        shut_down(self.pid, foreground, self.ownership)
    }

    pub(super) fn finished(self) {
        self.gate.want(Wanted::Stop);
        if self.ownership == Ownership::Child {
            thread::spawn(move || reap(self.pid));
        }
    }
}
