#![expect(
    clippy::needless_pass_by_value,
    reason = "Tauri commands receive their arguments by value"
)]

use anyhow::anyhow;
use protocol::{Command, Frame, Reply, TabInfo};
use serde::{Serialize, Serializer};
use specta::Type;
use specta::datatype::DataType;
use tauri::State;
use tauri::ipc::Channel;

use super::{Daemon, Output, Route};

pub struct CommandError(anyhow::Error);

impl<E: Into<anyhow::Error>> From<E> for CommandError {
    fn from(error: E) -> Self {
        CommandError(error.into())
    }
}

impl Serialize for CommandError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{:#}", self.0))
    }
}

impl Type for CommandError {
    fn definition(types: &mut specta::Types) -> DataType {
        String::definition(types)
    }
}

pub type CommandResult<T> = Result<T, CommandError>;

const INPUT_CHUNK: usize = 1024 * 1024;

#[tauri::command]
#[specta::specta]
pub async fn tabs_list(daemon: State<'_, Daemon>) -> CommandResult<Vec<TabInfo>> {
    match daemon.request(Command::List).await? {
        Reply::Tabs { tabs } => Ok(tabs),
        _ => Err(anyhow!("unexpected reply from the background process").into()),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn tab_new(daemon: State<'_, Daemon>) -> CommandResult<()> {
    daemon.done(Command::New { pinned: false }).await
}

#[tauri::command]
#[specta::specta]
pub async fn tab_close(daemon: State<'_, Daemon>, tab: String) -> CommandResult<()> {
    daemon.done(Command::Close { tab }).await
}

#[tauri::command]
#[specta::specta]
pub async fn tab_activate(daemon: State<'_, Daemon>, tab: String) -> CommandResult<()> {
    daemon.done(Command::Activate { tab }).await
}

#[tauri::command]
#[specta::specta]
pub async fn tab_place(
    daemon: State<'_, Daemon>,
    tab: String,
    pinned: bool,
    position: Option<u32>,
) -> CommandResult<()> {
    let command = Command::Place {
        tab,
        pinned: Some(pinned),
        position: position.map(usize::try_from).transpose()?,
    };
    daemon.done(command).await
}

#[tauri::command]
#[specta::specta]
pub async fn tab_restart(daemon: State<'_, Daemon>, tab: String) -> CommandResult<()> {
    daemon.done(Command::Restart { tab }).await
}

#[tauri::command]
#[specta::specta]
pub async fn tab_rename(
    daemon: State<'_, Daemon>,
    tab: String,
    title: String,
) -> CommandResult<()> {
    daemon.done(Command::Rename { tab, title }).await
}

#[tauri::command]
#[specta::specta]
pub async fn tab_attach(
    daemon: State<'_, Daemon>,
    tab: String,
    on_output: Channel<Output>,
) -> CommandResult<()> {
    daemon.0.routes.lock().insert(
        tab.clone(),
        Route {
            channel: on_output,
            live: false,
        },
    );
    daemon.done(Command::Attach { tab }).await
}

#[tauri::command]
#[specta::specta]
pub fn tab_detach(daemon: State<'_, Daemon>, tab: String) {
    daemon.0.routes.lock().remove(&tab);
}

#[tauri::command]
#[specta::specta]
pub fn tab_write(daemon: State<'_, Daemon>, tab: String, data: Vec<u8>) {
    for chunk in data.chunks(INPUT_CHUNK) {
        let input = Frame::Input {
            tab: tab.clone(),
            data: chunk.to_vec(),
        };
        if daemon.send(input).is_err() {
            return;
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn terminal_resize(daemon: State<'_, Daemon>, rows: u16, cols: u16) {
    daemon.resize(rows, cols);
}

#[tauri::command]
#[specta::specta]
pub fn tab_pause(daemon: State<'_, Daemon>, tab: String) {
    daemon.post(Command::Pause { tab });
}

#[tauri::command]
#[specta::specta]
pub fn tab_resume(daemon: State<'_, Daemon>, tab: String) {
    daemon.post(Command::Resume { tab });
}

#[tauri::command]
#[specta::specta]
pub async fn session_save(daemon: State<'_, Daemon>) -> CommandResult<()> {
    daemon.done(Command::Save).await
}

#[tauri::command]
#[specta::specta]
pub fn history_lines() -> u32 {
    u32::try_from(protocol::HISTORY_LINES).unwrap_or(u32::MAX)
}

#[tauri::command]
#[specta::specta]
pub fn take_warning(daemon: State<'_, Daemon>) -> Option<String> {
    daemon.0.warning.lock().take()
}
