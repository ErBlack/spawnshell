use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use config::Config;
use protocol::{Command, Connection, Reply, Role};

use super::REQUEST_TIMEOUT;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HELLO_ATTEMPTS: u32 = 10;
const REOPEN: &str = "Spawnshell was moved or updated, quit and reopen it";

fn log_dir() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join("Library/Logs").join(protocol::IDENTIFIER))
        .context("can't find the home folder")
}

fn log_file() -> Result<PathBuf> {
    log_dir().map(|dir| dir.join("daemon.log"))
}

fn spawn_daemon() -> Result<Child> {
    fs::create_dir_all(log_dir()?)?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file()?)?;
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command
        .arg("__daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command
        .spawn()
        .context("can't start the background process")
}

fn wait_for_socket(mut child: Option<&mut Child>) -> Result<Connection> {
    let socket = protocol::socket()?;
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        match Connection::connect(&socket, REQUEST_TIMEOUT) {
            Ok(connection) => return Ok(connection),
            Err(error) if Instant::now() >= deadline => {
                return Err(error).context("can't reach Spawnshell's background process");
            }
            Err(_) => {}
        }
        if let Some(child) = child.as_deref_mut()
            && let Ok(Some(status)) = child.try_wait()
            && !status.success()
        {
            let log = log_file()?;
            return Err(anyhow!(
                "the background process failed to start ({status}), see {}",
                log.display()
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn connect(spawn: bool) -> Result<Connection> {
    if let Ok(connection) = Connection::connect(&protocol::socket()?, REQUEST_TIMEOUT) {
        return Ok(connection);
    }
    if !spawn {
        return wait_for_socket(None);
    }
    let mut child = spawn_daemon()?;
    let connected = wait_for_socket(Some(&mut child));
    thread::spawn(move || {
        let _ = child.wait();
    });
    connected
}

fn disconnected(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset | io::ErrorKind::BrokenPipe
    )
}

fn failure(error: &io::Error) -> &'static str {
    match error.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {
            "the background process did not answer"
        }
        io::ErrorKind::InvalidData => "the background process speaks an incompatible protocol",
        _ if disconnected(error) => "the background process keeps disconnecting",
        _ => "can't talk to the background process",
    }
}

pub(super) struct Joined {
    pub(super) connection: Connection,
    pub(super) config: Config,
    pub(super) warning: Option<String>,
}

struct Greeting {
    connection: Connection,
    build: Option<protocol::Build>,
    warning: Option<String>,
    config: Option<Config>,
}

impl Greeting {
    fn into_joined(self, upgrade: Option<String>) -> Result<Joined> {
        let config = self
            .config
            .context("the background process sent no settings")?;
        let warning = match upgrade {
            Some(upgrade) => {
                log::warn!("{upgrade}");
                Some(match self.warning {
                    Some(warning) => format!("{warning}\n{upgrade}"),
                    None => upgrade,
                })
            }
            None => self.warning,
        };
        Ok(Joined {
            connection: self.connection,
            config,
            warning,
        })
    }
}

fn hello(spawn: bool) -> Result<Greeting> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let mut connection = connect(spawn)?;
        let command = Command::Hello { role: Role::Gui };
        match connection.call(command) {
            Ok(Ok(Reply::Hello {
                build,
                warning,
                config,
            })) => {
                return Ok(Greeting {
                    connection,
                    build,
                    warning,
                    config,
                });
            }
            Ok(Ok(_)) => bail!("unexpected reply from the background process"),
            Ok(Err(error)) => bail!(error),
            Err(error) if disconnected(&error) && attempt < HELLO_ATTEMPTS => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                let context = failure(&error);
                return Err(error).context(context);
            }
        }
    }
}

pub(super) fn join() -> Result<Joined> {
    let mine = protocol::build();
    let mut greeting = hello(true)?;
    if mine.is_none() || greeting.build == mine {
        return greeting.into_joined(None);
    }
    let exe = std::env::current_exe()?;
    let Some(target) = protocol::build_of(&exe) else {
        return greeting.into_joined(Some(REOPEN.to_string()));
    };
    if greeting.build != Some(target) {
        let Some(exe) = exe.to_str().map(str::to_string) else {
            return greeting.into_joined(Some(
                "couldn't update the background process: the app path is not valid UTF-8"
                    .to_string(),
            ));
        };
        log::info!("updating the background process");
        if let Ok(Err(error)) = greeting.connection.call(Command::Upgrade { exe }) {
            return greeting.into_joined(Some(format!(
                "couldn't update the background process: {error}"
            )));
        }
        greeting = hello(false)?;
    }
    let upgrade = if greeting.build != Some(target) {
        Some("the background process is still the previous version".to_string())
    } else if mine != Some(target) {
        Some(REOPEN.to_string())
    } else {
        None
    };
    greeting.into_joined(upgrade)
}
