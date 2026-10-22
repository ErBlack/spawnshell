use protocol::TabInfo;
use serde::Serialize;
use specta::Type;

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
