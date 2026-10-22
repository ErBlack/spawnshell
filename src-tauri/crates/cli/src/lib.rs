use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use clap::builder::NonEmptyStringValueParser;
use clap::{Args, Parser, Subcommand};
use config::Key;
use protocol::{Command, Connection, Failure, Reply, Role, TabInfo};

pub mod install;

const TIMEOUT: Duration = Duration::from_secs(15);
const UPDATE_WAIT: Duration = Duration::from_secs(10);
const RETRY_DELAY: Duration = Duration::from_millis(200);

/// Control Spawnshell from the command line
#[derive(Parser)]
#[command(
    name = "spawnshell",
    bin_name = "spawnshell",
    after_help = "Tab commands act on the tab they run in. Outside Spawnshell, pass --tab ID."
)]
struct Cli {
    #[command(subcommand)]
    toplevel: Toplevel,
}

#[derive(Subcommand)]
enum Toplevel {
    #[command(flatten)]
    Daemon(Action),
    /// Link the spawnshell command into ~/.local/bin
    Install,
}

#[derive(Subcommand)]
enum Action {
    /// List tabs: id, pinned, running, active, process, directory, title, notified
    List,
    /// Open a new tab
    New {
        /// Pin the new tab
        #[arg(long)]
        pinned: bool,
    },
    /// Close a tab
    Close {
        /// Close a pinned tab too
        #[arg(long)]
        force: bool,
        #[command(flatten)]
        target: Target,
    },
    /// Switch to a tab
    Select(Target),
    /// Pin a tab
    Pin(Target),
    /// Unpin a tab
    Unpin(Target),
    /// Move a tab within its group (0 is first)
    Move {
        position: usize,
        #[command(flatten)]
        target: Target,
    },
    /// Restart a pinned tab
    Restart(Target),
    /// Set a tab title; an empty title makes the tab follow the running program again
    Rename {
        title: String,
        #[command(flatten)]
        target: Target,
    },
    /// Show or set the command a pinned tab runs when it comes back; an empty command removes it
    Restore {
        command: Option<String>,
        #[command(flatten)]
        target: Target,
    },
    /// Show a system notification from a tab; clicking it opens the tab
    Notify {
        #[arg(value_parser = NonEmptyStringValueParser::new())]
        body: String,
        /// Notification title; defaults to the tab title
        #[arg(long)]
        title: Option<String>,
        #[command(flatten)]
        target: Target,
    },
    /// Show the window
    Show,
    /// Hide the window
    Hide,
    /// Show or hide the window
    Toggle,
    /// Quit Spawnshell; pinned tabs keep running
    Quit,
    /// Quit Spawnshell and stop pinned tabs
    Stop,
    /// Show settings, show one setting, or change it
    Config {
        key: Option<Key>,
        value: Option<String>,
    },
}

#[derive(Args)]
struct Target {
    /// Tab id; defaults to the tab this runs in
    #[arg(
        long,
        env = "SPAWNSHELL_TAB",
        hide_env_values = true,
        value_parser = NonEmptyStringValueParser::new()
    )]
    tab: String,
}

struct Client {
    connection: Connection,
    socket: PathBuf,
    deadline: Instant,
}

fn unanswered(error: io::Error) -> anyhow::Error {
    let timed_out = matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    );
    let context = if timed_out {
        "Spawnshell did not answer"
    } else {
        "lost the connection to Spawnshell"
    };
    anyhow::Error::new(error).context(context)
}

impl Client {
    fn open() -> Result<Self> {
        Client::connect(protocol::socket()?, Instant::now() + UPDATE_WAIT)
    }

    fn connect(socket: PathBuf, deadline: Instant) -> Result<Self> {
        loop {
            let mut connection =
                Connection::connect(&socket, TIMEOUT).context("Spawnshell is not running")?;
            match connection.call(Command::Hello { role: Role::Cli }) {
                Ok(Ok(_)) => {
                    return Ok(Client {
                        connection,
                        socket,
                        deadline,
                    });
                }
                Ok(Err(Failure::Updating)) if Instant::now() < deadline => {}
                Ok(Err(failure)) => bail!(failure),
                Err(error)
                    if error.kind() == io::ErrorKind::UnexpectedEof
                        && Instant::now() < deadline => {}
                Err(error) => return Err(unanswered(error)),
            }
            thread::sleep(RETRY_DELAY);
        }
    }

    fn call(&mut self, command: &Command) -> Result<Reply> {
        loop {
            match self.connection.call(command.clone()).map_err(unanswered)? {
                Err(Failure::Updating) if Instant::now() < self.deadline => {
                    thread::sleep(RETRY_DELAY);
                    *self = Client::connect(self.socket.clone(), self.deadline)?;
                }
                reply => return reply.map_err(|failure| anyhow!(failure)),
            }
        }
    }

    fn tabs(&mut self) -> Result<Vec<TabInfo>> {
        match self.call(&Command::List)? {
            Reply::Tabs { tabs } => Ok(tabs),
            _ => bail!("unexpected reply from Spawnshell"),
        }
    }
}

fn line(tab: TabInfo) -> String {
    let pinned = if tab.pinned { "pinned" } else { "-" };
    let state = if tab.alive { "running" } else { "stopped" };
    let active = if tab.active { "active" } else { "-" };
    let process = tab.process.unwrap_or_default();
    let cwd = tab.cwd.unwrap_or_default();
    let title = tab.title.unwrap_or_default();
    let notified = if tab.notified { "notified" } else { "-" };
    format!(
        "{}\t{pinned}\t{state}\t{active}\t{process}\t{cwd}\t{title}\t{notified}",
        tab.id
    )
}

fn execute(cli: Cli, out: &mut impl Write) -> Result<()> {
    match cli.toplevel {
        Toplevel::Daemon(action) => send(action, out),
        Toplevel::Install => {
            let link = install::install()?;
            writeln!(out, "installed {}", link.display())?;
            if let Some(dir) = link.parent()
                && !install::in_path(&std::env::var_os("PATH").unwrap_or_default(), dir)
            {
                eprintln!(
                    "spawnshell: {} is not in PATH, add it to run spawnshell by name",
                    dir.display()
                );
            }
            Ok(())
        }
    }
}

fn send(action: Action, out: &mut impl Write) -> Result<()> {
    let mut client = Client::open()?;
    let command = match action {
        Action::List => {
            for tab in client.tabs()? {
                writeln!(out, "{}", line(tab))?;
            }
            return Ok(());
        }
        Action::New { pinned } => Command::New { pinned },
        Action::Close {
            force,
            target: Target { tab },
        } => {
            return match client.call(&Command::Close { tab, force }) {
                Err(error) if error.downcast_ref::<Failure>() == Some(&Failure::Pinned) => {
                    bail!("the tab is pinned, pass --force to close it")
                }
                result => result.map(|_| ()),
            };
        }
        Action::Select(Target { tab }) => Command::Select { tab },
        Action::Pin(Target { tab }) => Command::Place {
            tab,
            pinned: Some(true),
            position: None,
        },
        Action::Unpin(Target { tab }) => Command::Place {
            tab,
            pinned: Some(false),
            position: None,
        },
        Action::Move {
            position,
            target: Target { tab },
        } => Command::Place {
            tab,
            pinned: None,
            position: Some(position),
        },
        Action::Restart(Target { tab }) => Command::Restart { tab },
        Action::Rename {
            title,
            target: Target { tab },
        } => Command::Rename { tab, title },
        Action::Restore {
            command: Some(restore),
            target: Target { tab },
        } => Command::SetRestore { tab, restore },
        Action::Restore {
            command: None,
            target: Target { tab },
        } => {
            let Reply::Restore { restore } = client.call(&Command::Restore { tab })? else {
                bail!("unexpected reply from Spawnshell");
            };
            if let Some(restore) = restore {
                writeln!(out, "{restore}")?;
            }
            return Ok(());
        }
        Action::Notify {
            body,
            title,
            target: Target { tab },
        } => Command::Notify { tab, title, body },
        Action::Show => Command::Show,
        Action::Hide => Command::Hide,
        Action::Toggle => Command::Toggle,
        Action::Quit => Command::Quit,
        Action::Stop => Command::Stop,
        Action::Config {
            key: Some(key),
            value: Some(value),
        } => Command::SetConfig { key, value },
        Action::Config { key, .. } => {
            let Reply::Config { config } = client.call(&Command::Config)? else {
                bail!("unexpected reply from Spawnshell");
            };
            match key {
                Some(key) => writeln!(out, "{}", config.get(key))?,
                None => write!(out, "{}", config::render(&config)?)?,
            }
            return Ok(());
        }
    };
    if let Reply::Tab { tab } = client.call(&command)? {
        writeln!(out, "{tab}")?;
    }
    Ok(())
}

fn broken_pipe(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<io::Error>()
        .is_some_and(|error| error.kind() == io::ErrorKind::BrokenPipe)
}

#[must_use]
pub fn run() -> ExitCode {
    match execute(Cli::parse(), &mut io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if broken_pipe(&error) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("spawnshell: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::BufReader;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread;
    use std::time::{Duration, Instant};

    use clap::CommandFactory;
    use protocol::{Command, Failure, Frame, FromDaemon, Reply, Request};

    use super::{Cli, Client, UPDATE_WAIT};

    #[test]
    fn arguments_are_consistent() {
        Cli::command().debug_assert();
    }

    fn serve(stream: UnixStream, commands: &AtomicUsize, refusals: usize, done: &Sender<usize>) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;
        while let Ok(Frame::Message(bytes)) = protocol::read(&mut reader) {
            let request: Request = protocol::parse(&bytes).unwrap();
            let result = if matches!(request.command, Command::Hello { .. }) {
                Ok(Reply::Done)
            } else {
                let seen = commands.fetch_add(1, Ordering::SeqCst) + 1;
                if seen <= refusals {
                    Err(Failure::Updating)
                } else {
                    let _ = done.send(seen);
                    Ok(Reply::Tab { tab: "tab".into() })
                }
            };
            let response = FromDaemon::Response {
                seq: request.seq,
                result,
            };
            protocol::write(&mut writer, &protocol::message(&response)).unwrap();
        }
    }

    fn updating_daemon(name: &str, refusals: usize) -> (PathBuf, Receiver<usize>) {
        let dir =
            std::env::temp_dir().join(format!("spawnshell-cli-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("daemon.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let (done, answered) = mpsc::channel();
        thread::spawn(move || {
            let commands = Arc::new(AtomicUsize::new(0));
            for stream in listener.incoming() {
                let stream = stream.unwrap();
                let commands = Arc::clone(&commands);
                let done = done.clone();
                thread::spawn(move || serve(stream, &commands, refusals, &done));
            }
        });
        (socket, answered)
    }

    #[test]
    fn a_command_refused_while_updating_is_retried() {
        let (socket, answered) = updating_daemon("retry", 2);
        let mut client = Client::connect(socket.clone(), Instant::now() + UPDATE_WAIT).unwrap();

        let reply = client.call(&Command::New { pinned: false }).unwrap();
        let commands = answered.recv_timeout(UPDATE_WAIT).unwrap();
        let _ = fs::remove_dir_all(socket.parent().unwrap());

        assert!(matches!(reply, Reply::Tab { tab } if tab == "tab"));
        assert_eq!(commands, 3);
    }

    #[test]
    fn retrying_stops_at_the_deadline() {
        let (socket, _answered) = updating_daemon("deadline", usize::MAX);
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut client = Client::connect(socket.clone(), deadline).unwrap();

        let error = client.call(&Command::New { pinned: false }).unwrap_err();
        let _ = fs::remove_dir_all(socket.parent().unwrap());

        assert_eq!(error.to_string(), Failure::Updating.to_string());
    }
}
