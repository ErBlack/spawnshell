pub mod commands;
pub mod events;
mod launch;
pub mod menu;

use std::collections::HashMap;
use std::fmt;
use std::io::BufReader;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::{Result, anyhow};
use parking_lot::Mutex;
use protocol::{Command, Connection, Event, Failure, Frame, FromDaemon, Reply, Request, Seq};
use specta::Type;
use specta::datatype::DataType;
use tauri::AppHandle;
use tauri::ipc::{Channel, InvokeResponseBody, IpcResponse};
use tauri_specta::Event as _;
use tokio::sync::oneshot;

use self::commands::CommandResult;
use self::events::{Reconnected, TabSelected, TabTitleChanged, TabsChanged, Warning};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

pub struct Output(Vec<u8>);

impl Output {
    fn new(snapshot: bool, data: &[u8]) -> Self {
        let mut body = Vec::with_capacity(data.len() + 1);
        body.push(u8::from(snapshot));
        body.extend_from_slice(data);
        Output(body)
    }
}

impl IpcResponse for Output {
    fn body(self) -> tauri::Result<InvokeResponseBody> {
        Ok(InvokeResponseBody::Raw(self.0))
    }
}

impl Type for Output {
    fn definition(_: &mut specta::Types) -> DataType {
        specta_typescript::define("ArrayBuffer").into()
    }
}

struct Route {
    channel: Channel<Output>,
    live: bool,
}

type Answer = Result<Reply, Failure>;

struct Pending {
    seq: u64,
    answer: oneshot::Receiver<Answer>,
}

#[derive(Debug)]
struct Disconnected;

impl fmt::Display for Disconnected {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("lost the connection to the background process")
    }
}

impl std::error::Error for Disconnected {}

#[derive(Debug)]
enum Unanswered {
    Lost,
    TimedOut,
}

impl fmt::Display for Unanswered {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Unanswered::Lost => Disconnected.fmt(formatter),
            Unanswered::TimedOut => formatter.write_str("the background process did not answer"),
        }
    }
}

impl std::error::Error for Unanswered {}

#[derive(Default)]
struct Inner {
    writer: Mutex<Option<mpsc::Sender<Frame>>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Answer>>>,
    routes: Mutex<HashMap<String, Route>>,
    seq: AtomicU64,
    size: Mutex<Option<(u16, u16)>>,
    warning: Mutex<Option<String>>,
    listening: AtomicBool,
    stopping: AtomicBool,
    window_open: AtomicBool,
}

#[derive(Clone)]
pub struct Daemon(Arc<Inner>);

impl Daemon {
    pub fn connect(app: &AppHandle) -> Result<Daemon> {
        let daemon = Daemon(Arc::default());
        let (reader, warning) = daemon.join(app)?;
        *daemon.0.warning.lock() = warning;
        let worker = daemon.clone();
        let app = app.clone();
        thread::spawn(move || worker.read_loop(&app, reader));
        Ok(daemon)
    }

    fn join(&self, app: &AppHandle) -> Result<(BufReader<UnixStream>, Option<String>)> {
        let joined = launch::join()?;
        let reader = self.install(joined.connection)?;
        crate::gui_config::apply(app, joined.config);
        Ok((reader, joined.warning))
    }

    fn install(&self, connection: Connection) -> Result<BufReader<UnixStream>> {
        let (reader, mut stream) = connection.into_parts()?;
        let (tx, rx) = mpsc::channel::<Frame>();
        thread::spawn(move || {
            for frame in rx {
                if let Err(error) = protocol::write(&mut stream, &frame) {
                    log::warn!("can't write to the background process: {error}");
                    let _ = stream.shutdown(Shutdown::Both);
                    break;
                }
            }
        });
        *self.0.writer.lock() = Some(tx);
        Ok(reader)
    }

    fn read_loop(&self, app: &AppHandle, mut reader: BufReader<UnixStream>) {
        loop {
            let error = loop {
                match protocol::read(&mut reader) {
                    Ok(frame) => self.dispatch(app, frame),
                    Err(error) => break error,
                }
            };
            self.disconnected();
            if self.0.stopping.load(Ordering::SeqCst) {
                app.exit(0);
                return;
            }
            log::warn!("lost the connection to the background process ({error}), reconnecting");
            reader = self.reconnect(app);
            log::info!("reconnected to the background process");
            let size = self.0.size.lock();
            if let Some((rows, cols)) = *size {
                self.post(Command::Resize { rows, cols });
            }
            drop(size);
            let open = self.0.window_open.load(Ordering::SeqCst);
            self.post(Command::SetWindowOpen { open });
            let _ = Reconnected.emit(app);
            crate::folders::open_waiting(app);
        }
    }

    fn reconnect(&self, app: &AppHandle) -> BufReader<UnixStream> {
        let mut delay = Duration::from_millis(100);
        let mut reported = false;
        loop {
            thread::sleep(delay);
            match self.join(app) {
                Ok((reader, warning)) => {
                    if let Some(warning) = warning {
                        let _ = Warning(warning).emit(app);
                    }
                    return reader;
                }
                Err(error) => {
                    log::warn!("can't reconnect: {error:#}");
                    if !reported {
                        reported = true;
                        let _ = Warning(format!(
                            "lost the connection to the background process and can't restore it: {error:#}"
                        ))
                        .emit(app);
                    }
                }
            }
            delay = (delay * 2).min(Duration::from_secs(5));
        }
    }

    fn disconnected(&self) {
        *self.0.writer.lock() = None;
        self.0.routes.lock().clear();
        self.0.pending.lock().clear();
    }

    fn dispatch(&self, app: &AppHandle, frame: Frame) {
        match frame {
            Frame::Output { tab, data } => {
                if let Some(route) = self.0.routes.lock().get(&tab)
                    && route.live
                {
                    let _ = route.channel.send(Output::new(false, &data));
                }
            }
            Frame::Snapshot { tab, data } => {
                if let Some(route) = self.0.routes.lock().get_mut(&tab) {
                    route.live = true;
                    let _ = route.channel.send(Output::new(true, &data));
                }
            }
            Frame::Message(bytes) => match protocol::parse::<FromDaemon>(&bytes) {
                Ok(FromDaemon::Response { seq, result }) => self.answered(seq, result),
                Ok(FromDaemon::Event { event }) => self.event(app, event),
                Err(error) => {
                    log::warn!("unreadable message from the background process: {error}");
                    if let Ok(Seq { seq }) = protocol::parse::<Seq>(&bytes) {
                        self.answered(
                            seq,
                            Err(Failure::Message(format!("unreadable reply: {error}"))),
                        );
                    }
                }
            },
            Frame::Input { .. } => {}
        }
    }

    fn answered(&self, seq: u64, answer: Answer) {
        if let Some(waiter) = self.0.pending.lock().remove(&seq) {
            let _ = waiter.send(answer);
        }
    }

    fn event(&self, app: &AppHandle, event: Event) {
        match event {
            Event::Tabs { tabs } => {
                let _ = TabsChanged(tabs).emit(app);
            }
            Event::Select { tab } => {
                let _ = TabSelected(tab).emit(app);
            }
            Event::Title { tab, title } => {
                let _ = TabTitleChanged { tab, title }.emit(app);
            }
            Event::Config { config } => crate::gui_config::apply(app, config),
            Event::Warning { message } => self.warn(app, message),
            Event::Show => crate::window::show(app),
            Event::Hide => crate::window::hide(app),
            Event::Toggle => crate::window::toggle(app),
            Event::Quit => app.exit(0),
            Event::Stop => {
                self.0.stopping.store(true, Ordering::SeqCst);
                app.exit(0);
            }
        }
    }

    fn send(&self, frame: Frame) -> Result<(), Disconnected> {
        self.0
            .writer
            .lock()
            .as_ref()
            .ok_or(Disconnected)?
            .send(frame)
            .map_err(|_| Disconnected)
    }

    fn submit(&self, command: Command) -> Result<Pending, Disconnected> {
        let seq = self.0.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, answer) = oneshot::channel();
        self.0.pending.lock().insert(seq, tx);
        if let Err(error) = self.send(protocol::message(&Request { seq, command })) {
            self.0.pending.lock().remove(&seq);
            return Err(error);
        }
        Ok(Pending { seq, answer })
    }

    async fn answer(&self, pending: Pending) -> Result<Answer, Unanswered> {
        match tokio::time::timeout(REQUEST_TIMEOUT, pending.answer).await {
            Ok(Ok(answer)) => Ok(answer),
            Ok(Err(_)) => Err(Unanswered::Lost),
            Err(_) => {
                self.0.pending.lock().remove(&pending.seq);
                Err(Unanswered::TimedOut)
            }
        }
    }

    async fn request(&self, command: Command) -> Result<Reply> {
        let pending = self.submit(command)?;
        self.answer(pending).await?.map_err(|error| anyhow!(error))
    }

    async fn done(&self, command: Command) -> CommandResult<()> {
        self.request(command).await?;
        Ok(())
    }

    fn post(&self, command: Command) {
        let Ok(pending) = self.submit(command) else {
            return;
        };
        let daemon = self.clone();
        tauri::async_runtime::spawn(async move {
            match daemon.answer(pending).await {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => log::warn!("the background process refused: {error}"),
                Err(error) => log::warn!("{error}"),
            }
        });
    }

    fn resize(&self, rows: u16, cols: u16) {
        let mut size = self.0.size.lock();
        *size = Some((rows, cols));
        self.post(Command::Resize { rows, cols });
    }

    pub fn set_window_open(&self, open: bool) {
        self.0.window_open.store(open, Ordering::SeqCst);
        self.post(Command::SetWindowOpen { open });
    }

    pub fn select(&self, tab: String) {
        self.post(Command::Select { tab });
    }

    pub fn new_tab_in(&self, app: &AppHandle, cwd: PathBuf) -> Result<(), PathBuf> {
        let command = Command::New {
            pinned: false,
            cwd: Some(cwd.clone()),
        };
        let Ok(pending) = self.submit(command) else {
            return Err(cwd);
        };
        let daemon = self.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let error = match daemon.answer(pending).await {
                Ok(Ok(_)) => return,
                Ok(Err(failure)) => failure.to_string(),
                Err(Unanswered::Lost) => return crate::folders::retry(&app, cwd),
                Err(error) => error.to_string(),
            };
            daemon.warn(&app, format!("couldn't open a tab: {error}"));
        });
        Ok(())
    }

    fn warn(&self, app: &AppHandle, message: String) {
        log::warn!("{message}");
        let mut waiting = self.0.warning.lock();
        if self.0.listening.load(Ordering::SeqCst) {
            drop(waiting);
            let _ = Warning(message).emit(app);
        } else if let Some(earlier) = waiting.as_mut() {
            earlier.push('\n');
            earlier.push_str(&message);
        } else {
            *waiting = Some(message);
        }
    }

    pub fn take_warning(&self) -> Option<String> {
        let mut waiting = self.0.warning.lock();
        self.0.listening.store(true, Ordering::SeqCst);
        waiting.take()
    }

    pub fn stop(&self, app: &AppHandle) {
        self.0.stopping.store(true, Ordering::SeqCst);
        let daemon = self.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let refused = match daemon.submit(Command::Stop) {
                Err(error) => Some(error.to_string()),
                Ok(pending) => match daemon.answer(pending).await {
                    Ok(Ok(_)) | Err(Unanswered::Lost) => None,
                    Ok(Err(failure)) => Some(failure.to_string()),
                    Err(error @ Unanswered::TimedOut) => Some(error.to_string()),
                },
            };
            if let Some(error) = refused {
                daemon.0.stopping.store(false, Ordering::SeqCst);
                log::warn!("couldn't stop the background process: {error}");
                let _ = Warning(format!("couldn't stop pinned tabs: {error}")).emit(&app);
            }
        });
    }
}
