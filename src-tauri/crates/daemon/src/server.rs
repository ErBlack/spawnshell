use std::io::BufReader;
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use platform::ipc::{self, Listener, Stream};
use protocol::{Event, Failure, Frame, FromDaemon, Request, Seq};

use super::hub::Hub;
use super::report;

const RETRY_FIRST: Duration = Duration::from_millis(5);
const RETRY_LONGEST: Duration = Duration::from_secs(1);

#[derive(Default)]
struct Backoff {
    delay: Option<Duration>,
}

impl Backoff {
    fn failing(&self) -> bool {
        self.delay.is_some()
    }

    fn failed(&mut self) -> Duration {
        let delay = self
            .delay
            .map_or(RETRY_FIRST, |delay| (delay * 2).min(RETRY_LONGEST));
        self.delay = Some(delay);
        delay
    }

    fn succeeded(&mut self) {
        self.delay = None;
    }
}

fn failure(error: &anyhow::Error) -> Failure {
    match error.downcast_ref::<Failure>() {
        Some(failure) => failure.clone(),
        None => Failure::Message(format!("{error:#}")),
    }
}

pub(super) fn listen() -> Result<Listener> {
    let socket = protocol::socket()?;
    Listener::bind(&socket).with_context(|| format!("can't listen on {}", socket.display()))
}

pub(super) fn remove_socket() {
    if let Ok(socket) = protocol::socket() {
        ipc::remove_socket(&socket);
    }
}

pub(super) fn event_frame(event: Event) -> Frame {
    protocol::message(&FromDaemon::Event { event })
}

impl Hub {
    fn serve(self: Arc<Self>, stream: Stream) {
        let client = self.clients.fetch_add(1, Ordering::Relaxed);
        let Ok(mut writer) = stream.try_clone() else {
            return;
        };
        *self.connected.lock() += 1;
        let (tx, rx) = mpsc::channel::<Frame>();
        thread::spawn(move || {
            for frame in rx {
                if protocol::write(&mut writer, &frame).is_err() {
                    break;
                }
            }
        });

        let mut reader = BufReader::new(stream);
        while let Ok(frame) = protocol::read(&mut reader) {
            match frame {
                Frame::Message(bytes) => {
                    let (seq, result) = match protocol::parse::<Request>(&bytes) {
                        Ok(request) => (
                            request.seq,
                            self.handle(request.command, client, &tx)
                                .map_err(|error| failure(&error)),
                        ),
                        Err(error) => {
                            let Ok(Seq { seq }) = protocol::parse::<Seq>(&bytes) else {
                                break;
                            };
                            let refused = format!(
                                "unsupported request ({error}); the background process may be older, restart Spawnshell"
                            );
                            (seq, Err(Failure::Message(refused)))
                        }
                    };
                    let _ = tx.send(protocol::message(&FromDaemon::Response { seq, result }));
                }
                Frame::Input { tab, data } => self.input(&tab, data),
                Frame::Output { .. } | Frame::Snapshot { .. } => {}
            }
        }
        self.gui_disconnected(client);
        *self.connected.lock() -= 1;
        self.disconnected.notify_all();
    }

    pub(super) fn accept(self: &Arc<Self>) {
        let mut backoff = Backoff::default();
        for stream in self.listener.incoming() {
            let stream = match stream {
                Ok(stream) => stream,
                Err(error) => {
                    if !backoff.failing() {
                        report(&anyhow::Error::new(error).context("can't accept a connection"));
                    }
                    thread::sleep(backoff.failed());
                    continue;
                }
            };
            backoff.succeeded();
            if self.upgrading.load(Ordering::SeqCst) || !stream.same_user() {
                continue;
            }
            let hub = Arc::clone(self);
            thread::spawn(move || hub.serve(stream));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_slow_down_and_start_over_after_a_success() {
        let mut backoff = Backoff::default();
        let delays: Vec<Duration> = (0..10).map(|_| backoff.failed()).collect();

        assert_eq!(delays[0], RETRY_FIRST);
        assert!(
            delays
                .windows(2)
                .all(|pair| pair[0] < pair[1] || pair[1] == RETRY_LONGEST)
        );
        assert_eq!(delays.last(), Some(&RETRY_LONGEST));
        assert!(backoff.failing());

        backoff.succeeded();
        assert!(!backoff.failing());
        assert_eq!(backoff.failed(), RETRY_FIRST);
    }
}
