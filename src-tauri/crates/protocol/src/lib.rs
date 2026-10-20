use std::fmt;
use std::io::{self, BufReader, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result};
use config::{Config, Key};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const IDENTIFIER: &str = "com.erblack.spawnshell-terminal";
pub const HISTORY_LINES: usize = 10_000;

pub fn data_dir() -> Result<PathBuf> {
    dirs::data_dir()
        .map(|dir| dir.join(IDENTIFIER))
        .context("can't find the Application Support folder")
}

pub fn socket() -> Result<PathBuf> {
    match std::env::var_os("SPAWNSHELL_SOCKET") {
        Some(path) => Ok(PathBuf::from(path)),
        None => data_dir().map(|dir| dir.join("daemon.sock")),
    }
}

const MESSAGE: u8 = 0;
const OUTPUT: u8 = 1;
const INPUT: u8 = 2;
const SNAPSHOT: u8 = 3;
const MAX_FRAME: usize = 64 * 1024 * 1024;

pub enum Frame {
    Message(Vec<u8>),
    Output { tab: String, data: Vec<u8> },
    Input { tab: String, data: Vec<u8> },
    Snapshot { tab: String, data: Vec<u8> },
}

fn rejected(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, reason)
}

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

pub fn write(writer: &mut impl Write, frame: &Frame) -> io::Result<()> {
    let (kind, tab, data) = match frame {
        Frame::Message(bytes) => (MESSAGE, None, bytes),
        Frame::Output { tab, data } => (OUTPUT, Some(tab), data),
        Frame::Input { tab, data } => (INPUT, Some(tab), data),
        Frame::Snapshot { tab, data } => (SNAPSHOT, Some(tab), data),
    };
    let tagged = tab.map_or(0, |tab| 2 + tab.len());
    let length = 1 + tagged + data.len();
    let header = u32::try_from(length)
        .ok()
        .filter(|_| length <= MAX_FRAME)
        .ok_or_else(|| rejected("frame too large"))?;
    let mut buffer = Vec::with_capacity(4 + length);
    buffer.extend_from_slice(&header.to_be_bytes());
    buffer.push(kind);
    if let Some(tab) = tab {
        let tab_length = u16::try_from(tab.len()).map_err(|_| rejected("tab id too long"))?;
        buffer.extend_from_slice(&tab_length.to_be_bytes());
        buffer.extend_from_slice(tab.as_bytes());
    }
    buffer.extend_from_slice(data);
    writer.write_all(&buffer)?;
    writer.flush()
}

fn read_bytes(reader: &mut impl Read, count: usize) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0u8; count];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

pub fn read(reader: &mut impl Read) -> io::Result<Frame> {
    let mut header = [0u8; 5];
    reader.read_exact(&mut header)?;
    let [a, b, c, d, kind] = header;
    let length = usize::try_from(u32::from_be_bytes([a, b, c, d]))
        .map_err(|_| invalid("bad frame length"))?;
    if length == 0 || length > MAX_FRAME {
        return Err(invalid("bad frame length"));
    }
    let rest = length - 1;
    if kind == MESSAGE {
        return read_bytes(reader, rest).map(Frame::Message);
    }
    let frame: fn(String, Vec<u8>) -> Frame = match kind {
        OUTPUT => |tab, data| Frame::Output { tab, data },
        INPUT => |tab, data| Frame::Input { tab, data },
        SNAPSHOT => |tab, data| Frame::Snapshot { tab, data },
        _ => return Err(invalid("unknown frame kind")),
    };
    let mut tab_length = [0u8; 2];
    let rest = rest
        .checked_sub(tab_length.len())
        .ok_or_else(|| invalid("malformed frame"))?;
    reader.read_exact(&mut tab_length)?;
    let tab_length = usize::from(u16::from_be_bytes(tab_length));
    let rest = rest
        .checked_sub(tab_length)
        .ok_or_else(|| invalid("malformed frame"))?;
    let tab = String::from_utf8(read_bytes(reader, tab_length)?)
        .map_err(|_| invalid("malformed frame"))?;
    let data = read_bytes(reader, rest)?;
    Ok(frame(tab, data))
}

pub fn message(value: &impl Serialize) -> Frame {
    Frame::Message(serde_json::to_vec(value).unwrap_or_default())
}

pub fn parse<T: DeserializeOwned>(bytes: &[u8]) -> io::Result<T> {
    serde_json::from_slice(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Build {
    pub device: u64,
    pub inode: u64,
}

#[must_use]
pub fn build_of(exe: &Path) -> Option<Build> {
    let metadata = std::fs::metadata(exe).ok()?;
    Some(Build {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

pub fn build() -> Option<Build> {
    static BUILD: OnceLock<Option<Build>> = OnceLock::new();
    *BUILD.get_or_init(|| build_of(&std::env::current_exe().ok()?))
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Gui,
    Cli,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    Hello {
        role: Role,
    },
    List,
    New {
        pinned: bool,
    },
    Close {
        tab: String,
        #[serde(default)]
        force: bool,
    },
    Select {
        tab: String,
    },
    Activate {
        tab: String,
    },
    Place {
        tab: String,
        pinned: Option<bool>,
        position: Option<usize>,
    },
    Restart {
        tab: String,
    },
    Rename {
        tab: String,
        title: String,
    },
    Restore {
        tab: String,
    },
    SetRestore {
        tab: String,
        restore: String,
    },
    Resize {
        rows: u16,
        cols: u16,
    },
    Attach {
        tab: String,
    },
    Pause {
        tab: String,
    },
    Resume {
        tab: String,
    },
    Save,
    Show,
    Hide,
    Toggle,
    Quit,
    Stop,
    Upgrade {
        exe: String,
    },
    Config,
    SetConfig {
        key: Key,
        value: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, specta::Type)]
pub struct TabInfo {
    pub id: String,
    pub pinned: bool,
    pub alive: bool,
    pub active: bool,
    pub title: Option<String>,
    pub process: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum Reply {
    Done,
    Hello {
        build: Option<Build>,
        warning: Option<String>,
        config: Option<Config>,
    },
    Tabs {
        tabs: Vec<TabInfo>,
    },
    Tab {
        tab: String,
    },
    Restore {
        restore: Option<String>,
    },
    Config {
        config: Config,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Tabs { tabs: Vec<TabInfo> },
    Select { tab: String },
    Config { config: Config },
    Warning { message: String },
    Show,
    Hide,
    Toggle,
    Quit,
    Stop,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Request {
    pub seq: u64,
    pub command: Command,
}

#[derive(Deserialize)]
pub struct Seq {
    pub seq: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    Updating,
    Pinned,
    #[serde(untagged)]
    Message(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Failure::Updating => {
                formatter.write_str("Spawnshell is updating, try again in a moment")
            }
            Failure::Pinned => formatter.write_str("the tab is pinned"),
            Failure::Message(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for Failure {}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FromDaemon {
    Response {
        seq: u64,
        result: Result<Reply, Failure>,
    },
    Event {
        event: Event,
    },
}

pub struct Connection {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    seq: u64,
}

impl Connection {
    pub fn connect(socket: &Path, timeout: Duration) -> io::Result<Self> {
        let writer = UnixStream::connect(socket)?;
        writer.set_read_timeout(Some(timeout))?;
        Ok(Connection {
            reader: BufReader::new(writer.try_clone()?),
            writer,
            seq: 0,
        })
    }

    pub fn call(&mut self, command: Command) -> io::Result<Result<Reply, Failure>> {
        self.seq += 1;
        let request = Request {
            seq: self.seq,
            command,
        };
        write(&mut self.writer, &message(&request))?;
        loop {
            let Frame::Message(bytes) = read(&mut self.reader)? else {
                continue;
            };
            match parse::<FromDaemon>(&bytes) {
                Ok(FromDaemon::Response { seq, result }) if seq == self.seq => return Ok(result),
                Err(error) if parse::<Seq>(&bytes).is_ok_and(|reply| reply.seq == self.seq) => {
                    return Err(error);
                }
                _ => {}
            }
        }
    }

    pub fn into_parts(self) -> io::Result<(BufReader<UnixStream>, UnixStream)> {
        self.reader.get_ref().set_read_timeout(None)?;
        Ok((self.reader, self.writer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let frames = [
            message(&Request {
                seq: 7,
                command: Command::Place {
                    tab: "a".into(),
                    pinned: Some(true),
                    position: Some(2),
                },
            }),
            Frame::Output {
                tab: "tab-1".into(),
                data: b"hello\x1b[0m".to_vec(),
            },
            Frame::Input {
                tab: String::new(),
                data: Vec::new(),
            },
            Frame::Snapshot {
                tab: "x".into(),
                data: vec![0, 255],
            },
        ];
        let mut buffer = Vec::new();
        for frame in &frames {
            write(&mut buffer, frame).unwrap();
        }

        let mut cursor = io::Cursor::new(buffer);
        let Frame::Message(bytes) = read(&mut cursor).unwrap() else {
            panic!("expected a message");
        };
        let request: Request = parse(&bytes).unwrap();
        assert_eq!(request.seq, 7);
        assert!(matches!(
            request.command,
            Command::Place { ref tab, pinned: Some(true), position: Some(2) } if tab == "a"
        ));
        assert!(
            matches!(read(&mut cursor).unwrap(), Frame::Output { tab, data } if tab == "tab-1" && data == b"hello\x1b[0m")
        );
        assert!(
            matches!(read(&mut cursor).unwrap(), Frame::Input { tab, data } if tab.is_empty() && data.is_empty())
        );
        assert!(
            matches!(read(&mut cursor).unwrap(), Frame::Snapshot { tab, data } if tab == "x" && data == [0, 255])
        );
        assert!(read(&mut cursor).is_err());
    }

    #[test]
    fn rejects_oversized_frames() {
        let mut header = u32::MAX.to_be_bytes().to_vec();
        header.push(MESSAGE);
        let error = read(&mut io::Cursor::new(header)).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        let huge = Frame::Message(vec![0; MAX_FRAME]);
        let error = write(&mut Vec::new(), &huge).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn rejects_a_tab_longer_than_its_frame() {
        let mut bytes = 4u32.to_be_bytes().to_vec();
        bytes.extend_from_slice(&[OUTPUT, 0, 9, b'x']);
        let error = read(&mut io::Cursor::new(bytes)).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn failures_keep_their_kind_and_read_plain_messages() {
        let older: Result<Reply, Failure> =
            serde_json::from_str(r#"{"Err":"no such tab"}"#).unwrap();
        assert_eq!(older.err(), Some(Failure::Message("no such tab".into())));

        let updating = serde_json::to_string(&Err::<Reply, _>(Failure::Updating)).unwrap();
        let updating: Result<Reply, Failure> = serde_json::from_str(&updating).unwrap();
        assert_eq!(updating.err(), Some(Failure::Updating));

        let pinned = serde_json::to_string(&Err::<Reply, _>(Failure::Pinned)).unwrap();
        let pinned: Result<Reply, Failure> = serde_json::from_str(&pinned).unwrap();
        assert_eq!(pinned.err(), Some(Failure::Pinned));
    }
}
