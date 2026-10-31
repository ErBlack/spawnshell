mod launch;

use std::collections::HashMap;
use std::fmt;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use config::Config;
use parking_lot::Mutex;
use platform::ipc::Stream;
use protocol::{Command, Connection, Failure, Frame, FromDaemon, Reply, Request, Seq};
pub use protocol::{HISTORY_LINES, TabInfo};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const INPUT_CHUNK: usize = 1024 * 1024;

static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .build()
        .expect("the client runtime always starts")
});

pub fn capture_build() {
    protocol::build();
}

pub enum Event {
    Tabs(Vec<TabInfo>),
    Selected(String),
    Title { tab: String, title: Option<String> },
    Config(Config),
    Warning(String),
    Show,
    Hide,
    Toggle,
    Reconnected,
    TabNotOpened(PathBuf),
    Exit,
}

pub struct Output<'a> {
    pub snapshot: bool,
    pub data: &'a [u8],
}

struct Route {
    on_output: Box<dyn Fn(Output) + Send>,
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

struct Inner {
    on_event: Box<dyn Fn(Event) + Send + Sync>,
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
    pub fn connect(on_event: impl Fn(Event) + Send + Sync + 'static) -> Result<Daemon> {
        let daemon = Daemon(Arc::new(Inner {
            on_event: Box::new(on_event),
            writer: Mutex::default(),
            pending: Mutex::default(),
            routes: Mutex::default(),
            seq: AtomicU64::default(),
            size: Mutex::default(),
            warning: Mutex::default(),
            listening: AtomicBool::default(),
            stopping: AtomicBool::default(),
            window_open: AtomicBool::default(),
        }));
        let (reader, warning) = daemon.join()?;
        *daemon.0.warning.lock() = warning;
        let worker = daemon.clone();
        thread::spawn(move || worker.read_loop(reader));
        Ok(daemon)
    }

    fn emit(&self, event: Event) {
        (self.0.on_event)(event);
    }

    fn join(&self) -> Result<(BufReader<Stream>, Option<String>)> {
        let joined = launch::join()?;
        let reader = self.install(joined.connection)?;
        self.emit(Event::Config(joined.config));
        Ok((reader, joined.warning))
    }

    fn install(&self, connection: Connection) -> Result<BufReader<Stream>> {
        let (reader, mut stream) = connection.into_parts()?;
        let (tx, rx) = mpsc::channel::<Frame>();
        thread::spawn(move || {
            for frame in rx {
                if let Err(error) = protocol::write(&mut stream, &frame) {
                    log::warn!("can't write to the background process: {error}");
                    let _ = stream.shutdown();
                    break;
                }
            }
        });
        *self.0.writer.lock() = Some(tx);
        Ok(reader)
    }

    fn read_loop(&self, mut reader: BufReader<Stream>) {
        loop {
            let error = loop {
                match protocol::read(&mut reader) {
                    Ok(frame) => self.dispatch(frame),
                    Err(error) => break error,
                }
            };
            self.disconnected();
            if self.0.stopping.load(Ordering::SeqCst) {
                self.emit(Event::Exit);
                return;
            }
            log::warn!("lost the connection to the background process ({error}), reconnecting");
            reader = self.reconnect();
            log::info!("reconnected to the background process");
            let size = self.0.size.lock();
            if let Some((rows, cols)) = *size {
                self.post(Command::Resize { rows, cols });
            }
            drop(size);
            let open = self.0.window_open.load(Ordering::SeqCst);
            self.post(Command::SetWindowOpen { open });
            self.emit(Event::Reconnected);
        }
    }

    fn reconnect(&self) -> BufReader<Stream> {
        let mut delay = Duration::from_millis(100);
        let mut reported = false;
        loop {
            thread::sleep(delay);
            match self.join() {
                Ok((reader, warning)) => {
                    if let Some(warning) = warning {
                        self.emit(Event::Warning(warning));
                    }
                    return reader;
                }
                Err(error) => {
                    log::warn!("can't reconnect: {error:#}");
                    if !reported {
                        reported = true;
                        self.emit(Event::Warning(format!(
                            "lost the connection to the background process and can't restore it: {error:#}"
                        )));
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

    fn dispatch(&self, frame: Frame) {
        match frame {
            Frame::Output { tab, data } => {
                if let Some(route) = self.0.routes.lock().get(&tab)
                    && route.live
                {
                    (route.on_output)(Output {
                        snapshot: false,
                        data: &data,
                    });
                }
            }
            Frame::Snapshot { tab, data } => {
                if let Some(route) = self.0.routes.lock().get_mut(&tab) {
                    route.live = true;
                    (route.on_output)(Output {
                        snapshot: true,
                        data: &data,
                    });
                }
            }
            Frame::Message(bytes) => match protocol::parse::<FromDaemon>(&bytes) {
                Ok(FromDaemon::Response { seq, result }) => self.answered(seq, result),
                Ok(FromDaemon::Event { event }) => self.event(event),
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

    fn event(&self, event: protocol::Event) {
        match event {
            protocol::Event::Tabs { tabs } => self.emit(Event::Tabs(tabs)),
            protocol::Event::Select { tab } => self.emit(Event::Selected(tab)),
            protocol::Event::Title { tab, title } => self.emit(Event::Title { tab, title }),
            protocol::Event::Config { config } => self.emit(Event::Config(config)),
            protocol::Event::Warning { message } => self.warn(message),
            protocol::Event::Show => self.emit(Event::Show),
            protocol::Event::Hide => self.emit(Event::Hide),
            protocol::Event::Toggle => self.emit(Event::Toggle),
            protocol::Event::Quit => self.emit(Event::Exit),
            protocol::Event::Stop => {
                self.0.stopping.store(true, Ordering::SeqCst);
                self.emit(Event::Exit);
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
        let daemon = self.clone();
        RUNTIME
            .spawn(async move { daemon.answer(pending).await })
            .await??
            .map_err(|error| anyhow!(error))
    }

    async fn done(&self, command: Command) -> Result<()> {
        self.request(command).await?;
        Ok(())
    }

    fn post(&self, command: Command) {
        let Ok(pending) = self.submit(command) else {
            return;
        };
        let daemon = self.clone();
        RUNTIME.spawn(async move {
            match daemon.answer(pending).await {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => log::warn!("the background process refused: {error}"),
                Err(error) => log::warn!("{error}"),
            }
        });
    }

    pub async fn list(&self) -> Result<Vec<TabInfo>> {
        match self.request(Command::List).await? {
            Reply::Tabs { tabs } => Ok(tabs),
            _ => bail!("unexpected reply from the background process"),
        }
    }

    pub async fn new_tab(&self) -> Result<()> {
        self.done(Command::New {
            pinned: false,
            cwd: None,
        })
        .await
    }

    pub async fn close(&self, tab: String) -> Result<()> {
        self.done(Command::Close { tab, force: false }).await
    }

    pub async fn activate(&self, tab: String) -> Result<()> {
        self.done(Command::Activate { tab }).await
    }

    pub async fn place(&self, tab: String, pinned: bool, position: Option<usize>) -> Result<()> {
        self.done(Command::Place {
            tab,
            pinned: Some(pinned),
            position,
        })
        .await
    }

    pub async fn restart(&self, tab: String) -> Result<()> {
        self.done(Command::Restart { tab }).await
    }

    pub async fn rename(&self, tab: String, title: String) -> Result<()> {
        self.done(Command::Rename { tab, title }).await
    }

    pub async fn attach(
        &self,
        tab: String,
        on_output: impl Fn(Output) + Send + 'static,
    ) -> Result<()> {
        self.0.routes.lock().insert(
            tab.clone(),
            Route {
                on_output: Box::new(on_output),
                live: false,
            },
        );
        self.done(Command::Attach { tab }).await
    }

    pub fn detach(&self, tab: &str) {
        self.0.routes.lock().remove(tab);
    }

    pub fn write(&self, tab: &str, data: &[u8]) {
        for chunk in data.chunks(INPUT_CHUNK) {
            let input = Frame::Input {
                tab: tab.to_string(),
                data: chunk.to_vec(),
            };
            if self.send(input).is_err() {
                return;
            }
        }
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        let mut size = self.0.size.lock();
        *size = Some((rows, cols));
        self.post(Command::Resize { rows, cols });
    }

    pub fn pause(&self, tab: String) {
        self.post(Command::Pause { tab });
    }

    pub fn resume(&self, tab: String) {
        self.post(Command::Resume { tab });
    }

    pub async fn save(&self) -> Result<()> {
        self.done(Command::Save).await
    }

    pub fn set_window_open(&self, open: bool) {
        self.0.window_open.store(open, Ordering::SeqCst);
        self.post(Command::SetWindowOpen { open });
    }

    pub fn select(&self, tab: String) {
        self.post(Command::Select { tab });
    }

    pub fn new_tab_in(&self, cwd: PathBuf) -> Result<(), PathBuf> {
        let command = Command::New {
            pinned: false,
            cwd: Some(cwd.clone()),
        };
        let Ok(pending) = self.submit(command) else {
            return Err(cwd);
        };
        let daemon = self.clone();
        RUNTIME.spawn(async move {
            let error = match daemon.answer(pending).await {
                Ok(Ok(_)) => return,
                Ok(Err(failure)) => failure.to_string(),
                Err(Unanswered::Lost) => return daemon.emit(Event::TabNotOpened(cwd)),
                Err(error) => error.to_string(),
            };
            daemon.warn(format!("couldn't open a tab: {error}"));
        });
        Ok(())
    }

    fn warn(&self, message: String) {
        log::warn!("{message}");
        let mut waiting = self.0.warning.lock();
        if self.0.listening.load(Ordering::SeqCst) {
            drop(waiting);
            self.emit(Event::Warning(message));
        } else if let Some(earlier) = waiting.as_mut() {
            earlier.push('\n');
            earlier.push_str(&message);
        } else {
            *waiting = Some(message);
        }
    }

    #[must_use]
    pub fn take_warning(&self) -> Option<String> {
        let mut waiting = self.0.warning.lock();
        self.0.listening.store(true, Ordering::SeqCst);
        waiting.take()
    }

    pub fn stop(&self) {
        self.0.stopping.store(true, Ordering::SeqCst);
        let daemon = self.clone();
        RUNTIME.spawn(async move {
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
                daemon.emit(Event::Warning(format!(
                    "couldn't stop pinned tabs: {error}"
                )));
            }
        });
    }
}
