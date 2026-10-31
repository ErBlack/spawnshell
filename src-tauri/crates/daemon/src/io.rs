use std::io::{self, Read, Write};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use platform::pty::{Master, Reader};

use super::flow::{Gate, Wanted};
use super::output::Output;
use super::report;

const FLUSH_INTERVAL: Duration = Duration::from_millis(8);
const MAX_BATCH: usize = 64 * 1024;
const IDLE_WAIT: Duration = Duration::from_millis(100);

struct Batch {
    output: Arc<Mutex<Output>>,
    bytes: Vec<u8>,
    deadline: Option<Instant>,
}

impl Batch {
    fn add(&mut self, bytes: &[u8]) {
        if self.bytes.is_empty() {
            self.deadline = Some(Instant::now() + FLUSH_INTERVAL);
        }
        self.bytes.extend_from_slice(bytes);
        if self.bytes.len() >= MAX_BATCH {
            self.flush();
        }
    }

    fn flush(&mut self) {
        self.deadline = None;
        if !self.bytes.is_empty() {
            self.output.lock().emit(&std::mem::take(&mut self.bytes));
        }
    }

    fn flush_if_due(&mut self) {
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.flush();
        }
    }

    fn wait_timeout(&self) -> Duration {
        self.deadline.map_or(IDLE_WAIT, |deadline| {
            deadline.saturating_duration_since(Instant::now())
        })
    }
}

fn read_until_done(reader: &mut Reader, gate: &Gate, batch: &mut Batch) {
    let mut buffer = [0u8; 8192];
    loop {
        match gate.current() {
            Wanted::Stop => return,
            Wanted::Pause => {
                batch.flush();
                if !gate.wait_until_resumed() {
                    return;
                }
            }
            Wanted::Run => {}
        }
        let ready = match reader.wait_readable(batch.wait_timeout()) {
            Ok(ready) => ready,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                report(&anyhow::Error::new(error).context("can't wait for terminal output"));
                return;
            }
        };
        if ready {
            match reader.read(&mut buffer) {
                Ok(0) => return,
                Ok(count) => batch.add(&buffer[..count]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    report(&anyhow::Error::new(error).context("can't read terminal output"));
                    return;
                }
            }
        }
        batch.flush_if_due();
    }
}

pub(super) fn start_io(
    master: &Master,
    output: Arc<Mutex<Output>>,
    gate: Gate,
    on_eof: impl FnOnce() + Send + 'static,
) -> io::Result<()> {
    let mut reader = master.reader()?;
    thread::spawn(move || {
        let mut batch = Batch {
            output,
            bytes: Vec::new(),
            deadline: None,
        };
        read_until_done(&mut reader, &gate, &mut batch);
        batch.flush();
        gate.reader_finished();
        on_eof();
    });
    Ok(())
}

pub(super) fn start_writer(master: &Master) -> io::Result<mpsc::Sender<Vec<u8>>> {
    let mut writer = master.writer()?;
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        for chunk in rx {
            if writer.write_all(&chunk).is_err() {
                break;
            }
        }
    });
    Ok(tx)
}

#[cfg(all(test, unix))]
mod tests {
    use std::path::Path;
    use std::time::Instant;

    use platform::Size;
    use platform::process::{self, Pid};
    use platform::pty;

    use super::super::flow::Wanted;
    use super::*;

    struct Run {
        collected: Arc<Mutex<Vec<u8>>>,
        output: Arc<Mutex<Output>>,
        done: mpsc::Receiver<()>,
        gate: Gate,
        pid: Pid,
        input: mpsc::Sender<Vec<u8>>,
    }

    impl Run {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.collected.lock()).into_owned()
        }

        fn wait(&self) -> String {
            let finished = self.done.recv_timeout(Duration::from_secs(15)).is_ok();
            assert!(finished, "aggregator thread did not finish");
            process::reap(self.pid);
            self.text()
        }
    }

    fn run(script: &str) -> Run {
        let args = vec!["-c".to_string(), script.to_string()];
        let (pid, master) = pty::spawn(&pty::Spawn {
            program: "/bin/sh",
            args: &args,
            cwd: Path::new("/"),
            env: &[],
            size: Size::new(24, 80),
        })
        .unwrap();
        let collected = Arc::new(Mutex::new(Vec::new()));
        let for_sink = Arc::clone(&collected);
        let output = Arc::new(Mutex::new(Output::new(
            Size::new(24, 80),
            Box::new(move |batch| for_sink.lock().extend_from_slice(batch)),
            Box::new(|_| {}),
        )));
        let input = start_writer(&master).unwrap();
        let (done_tx, done) = mpsc::channel();
        let gate = Gate::default();
        start_io(&master, Arc::clone(&output), gate.clone(), move || {
            let _ = done_tx.send(());
        })
        .unwrap();

        Run {
            collected,
            output,
            done,
            gate,
            pid,
            input,
        }
    }

    #[test]
    fn flushes_last_output_before_reporting_eof() {
        let text = run("printf 'LAST_LINE'").wait();
        assert!(text.contains("LAST_LINE"), "missing tail output: {text:?}");
    }

    #[test]
    fn preserves_order_across_batches() {
        const COUNT: u32 = 20_000;
        let text = run(&format!("seq 1 {COUNT}")).wait();

        let numbers: Vec<u32> = text
            .split_whitespace()
            .filter_map(|word| word.parse().ok())
            .collect();
        let expected: Vec<u32> = (1..=COUNT).collect();
        assert_eq!(numbers, expected, "output reordered, dropped or duplicated");
    }

    #[test]
    fn parser_tracks_the_screen() {
        let session = run("printf 'ON SCREEN'");
        session.wait();
        assert!(
            session
                .output
                .lock()
                .parser
                .screen()
                .contents()
                .contains("ON SCREEN")
        );
    }

    #[test]
    fn input_reaches_the_program() {
        let session = run("read line; echo GOT:$line");
        session.input.send(b"hello\n".to_vec()).unwrap();
        assert!(session.wait().contains("GOT:hello"));
    }

    #[test]
    fn pausing_parks_the_reader_and_resuming_restarts_delivery() {
        let session = run("while :; do echo tick; sleep 0.01; done");

        thread::sleep(Duration::from_millis(300));
        session.gate.want(Wanted::Pause);
        assert!(
            session
                .gate
                .wait_parked(Instant::now() + Duration::from_secs(2)),
            "reader did not park"
        );
        thread::sleep(Duration::from_millis(50));

        let paused_at = session.text().len();
        assert!(paused_at > 0, "nothing arrived before pausing");
        thread::sleep(Duration::from_millis(300));
        assert_eq!(
            session.text().len(),
            paused_at,
            "output kept arriving while paused"
        );

        session.gate.want(Wanted::Run);
        thread::sleep(Duration::from_millis(300));
        assert!(
            session.text().len() > paused_at,
            "output did not resume after unpausing"
        );

        process::kill(session.pid);
        session.wait();
    }

    #[test]
    fn shutting_down_wakes_a_paused_reader() {
        let session = run("while :; do echo tick; sleep 0.01; done");

        thread::sleep(Duration::from_millis(200));
        session.gate.want(Wanted::Pause);
        thread::sleep(Duration::from_millis(100));
        session.gate.want(Wanted::Stop);

        assert!(
            session.done.recv_timeout(Duration::from_secs(5)).is_ok(),
            "paused reader never woke up"
        );
        let pid = session.pid;
        drop(session);
        process::kill(pid);
        process::reap(pid);
    }

    #[test]
    fn steady_trickle_of_output_is_not_held_back() {
        let session = run("while :; do printf x; sleep 0.003; done");

        thread::sleep(Duration::from_millis(500));
        let early = session.text();
        assert!(early.contains('x'), "a steady trickle was withheld");

        process::kill(session.pid);
        session.wait();
    }

    #[test]
    fn flushes_short_output_while_the_shell_is_still_running() {
        let session = run("printf 'hi'; sleep 5");

        thread::sleep(Duration::from_millis(500));
        let early = session.text();
        assert!(early.contains("hi"), "short output was withheld: {early:?}");
        assert!(
            session.done.try_recv().is_err(),
            "shell exited before the check, test proves nothing"
        );
        process::kill(session.pid);
        session.wait();
    }
}
