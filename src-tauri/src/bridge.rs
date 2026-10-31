#![expect(
    clippy::needless_pass_by_value,
    reason = "Tauri commands receive their arguments by value"
)]

use client::{Daemon, TabInfo};
use serde::{Deserialize, Serialize, Serializer};
use specta::Type;
use specta::datatype::DataType;
use tauri::ipc::{Channel, InvokeResponseBody, IpcResponse};
use tauri::{AppHandle, State};
use tauri_specta::{Event as _, collect_commands, collect_events};

use crate::gui_config::{Appearance, GuiConfig};
use crate::{folders, gui_config, menu, window};

pub struct Output(Vec<u8>);

impl From<client::Output<'_>> for Output {
    fn from(output: client::Output) -> Self {
        let mut body = Vec::with_capacity(output.data.len() + 1);
        body.push(u8::from(output.snapshot));
        body.extend_from_slice(output.data);
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

#[tauri::command]
#[specta::specta]
pub async fn tabs_list(daemon: State<'_, Daemon>) -> CommandResult<Vec<TabInfo>> {
    Ok(daemon.list().await?)
}

#[tauri::command]
#[specta::specta]
pub async fn tab_new(daemon: State<'_, Daemon>) -> CommandResult<()> {
    Ok(daemon.new_tab().await?)
}

#[tauri::command]
#[specta::specta]
pub async fn tab_close(daemon: State<'_, Daemon>, tab: String) -> CommandResult<()> {
    Ok(daemon.close(tab).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn tab_activate(daemon: State<'_, Daemon>, tab: String) -> CommandResult<()> {
    Ok(daemon.activate(tab).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn tab_place(
    daemon: State<'_, Daemon>,
    tab: String,
    pinned: bool,
    position: Option<u32>,
) -> CommandResult<()> {
    let position = position.map(usize::try_from).transpose()?;
    Ok(daemon.place(tab, pinned, position).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn tab_restart(daemon: State<'_, Daemon>, tab: String) -> CommandResult<()> {
    Ok(daemon.restart(tab).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn tab_rename(
    daemon: State<'_, Daemon>,
    tab: String,
    title: String,
) -> CommandResult<()> {
    Ok(daemon.rename(tab, title).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn tab_attach(
    daemon: State<'_, Daemon>,
    tab: String,
    on_output: Channel<Output>,
) -> CommandResult<()> {
    let on_output = move |output: client::Output| {
        let _ = on_output.send(output.into());
    };
    Ok(daemon.attach(tab, on_output).await?)
}

#[tauri::command]
#[specta::specta]
pub fn tab_detach(daemon: State<'_, Daemon>, tab: String) {
    daemon.detach(&tab);
}

#[tauri::command]
#[specta::specta]
pub fn tab_write(daemon: State<'_, Daemon>, tab: String, data: Vec<u8>) {
    daemon.write(&tab, &data);
}

#[tauri::command]
#[specta::specta]
pub fn terminal_resize(daemon: State<'_, Daemon>, rows: u16, cols: u16) {
    daemon.resize(rows, cols);
}

#[tauri::command]
#[specta::specta]
pub fn tab_pause(daemon: State<'_, Daemon>, tab: String) {
    daemon.pause(tab);
}

#[tauri::command]
#[specta::specta]
pub fn tab_resume(daemon: State<'_, Daemon>, tab: String) {
    daemon.resume(tab);
}

#[tauri::command]
#[specta::specta]
pub async fn session_save(daemon: State<'_, Daemon>) -> CommandResult<()> {
    Ok(daemon.save().await?)
}

#[tauri::command]
#[specta::specta]
pub fn history_lines() -> u32 {
    u32::try_from(client::HISTORY_LINES).unwrap_or(u32::MAX)
}

#[tauri::command]
#[specta::specta]
pub fn take_warning(daemon: State<'_, Daemon>) -> Option<String> {
    daemon.take_warning()
}

#[tauri::command]
#[specta::specta]
pub fn show_tab_menu(
    window: tauri::WebviewWindow,
    tab: Option<String>,
    pinned: bool,
) -> CommandResult<()> {
    Ok(menu::show(&window, tab, pinned)?)
}

#[tauri::command]
#[specta::specta]
pub fn hide_window(app: AppHandle) {
    window::order_out(&app);
}

#[tauri::command]
#[specta::specta]
pub fn is_open(app: AppHandle) -> bool {
    window::opened(&app)
}

#[tauri::command]
#[specta::specta]
pub fn watch_appearance(gui_config: State<'_, GuiConfig>, on_change: Channel<Appearance>) {
    gui_config.watch(move |appearance| {
        let _ = on_change.send(appearance);
    });
}

#[tauri::command]
#[specta::specta]
pub fn report_fatal(app: AppHandle, message: String) {
    crate::fail_to_start(&app, message);
}

#[tauri::command]
#[specta::specta]
pub fn report_error(app: AppHandle, message: String) {
    crate::show_message(&app, message);
}

#[derive(Clone, Serialize, Type, tauri_specta::Event)]
pub struct TabsChanged(pub Vec<TabInfo>);

#[derive(Clone, Serialize, Type, tauri_specta::Event)]
pub struct TabSelected(pub String);

#[derive(Clone, Serialize, Type, tauri_specta::Event)]
pub struct TabTitleChanged {
    pub tab: String,
    pub title: Option<String>,
}

#[derive(Clone, Serialize, Type, tauri_specta::Event)]
pub struct Reconnected;

#[derive(Clone, Serialize, Type, tauri_specta::Event)]
pub struct Warning(pub String);

#[derive(Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum TabMenuChosen {
    New,
    Pin { tab: String, pinned: bool },
    Rename { tab: String },
    Restart { tab: String },
    Close { tab: String },
}

#[derive(Clone, Serialize, Type, tauri_specta::Event)]
pub struct WindowShown;

#[derive(Clone, Serialize, Type, tauri_specta::Event)]
pub struct WindowHidden;

pub fn tab_menu_chosen(app: &AppHandle, chosen: TabMenuChosen) {
    let _ = chosen.emit(app);
}

pub fn window_shown(app: &AppHandle) {
    let _ = WindowShown.emit(app);
}

pub fn window_hidden(app: &AppHandle) {
    let _ = WindowHidden.emit(app);
}

pub fn on_client_event(app: &AppHandle, event: client::Event) {
    match event {
        client::Event::Tabs(tabs) => {
            let _ = TabsChanged(tabs).emit(app);
        }
        client::Event::Selected(tab) => {
            let _ = TabSelected(tab).emit(app);
        }
        client::Event::Title { tab, title } => {
            let _ = TabTitleChanged { tab, title }.emit(app);
        }
        client::Event::Config(config) => gui_config::apply(app, &config),
        client::Event::Warning(message) => {
            let _ = Warning(message).emit(app);
        }
        client::Event::Show => window::show(app),
        client::Event::Hide => window::hide(app),
        client::Event::Toggle => window::toggle(app),
        client::Event::Reconnected => {
            let _ = Reconnected.emit(app);
            folders::open_waiting(app);
        }
        client::Event::TabNotOpened(cwd) => folders::retry(app, cwd),
        client::Event::Exit => app.exit(0),
    }
}

pub fn bindings() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::new()
        .error_handling(tauri_specta::ErrorHandlingMode::Throw)
        .commands(collect_commands![
            tabs_list,
            tab_new,
            tab_close,
            tab_activate,
            tab_place,
            tab_restart,
            tab_rename,
            tab_attach,
            tab_detach,
            tab_write,
            terminal_resize,
            tab_pause,
            tab_resume,
            session_save,
            take_warning,
            history_lines,
            show_tab_menu,
            hide_window,
            is_open,
            watch_appearance,
            report_fatal,
            report_error,
        ])
        .events(collect_events![
            TabsChanged,
            TabSelected,
            TabTitleChanged,
            Reconnected,
            Warning,
            TabMenuChosen,
            WindowShown,
            WindowHidden,
        ])
}

#[cfg(test)]
mod tests {
    #[test]
    fn typescript_bindings_are_up_to_date() {
        let path = std::path::Path::new("../src/bindings.ts");
        let before = std::fs::read_to_string(path).unwrap_or_default();
        super::bindings()
            .export(specta_typescript::Typescript::default(), path)
            .unwrap();
        let after = std::fs::read_to_string(path).unwrap();
        assert!(
            before == after,
            "src/bindings.ts was out of date and has been regenerated"
        );
    }
}
