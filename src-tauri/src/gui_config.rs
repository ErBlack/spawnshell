#![expect(
    clippy::needless_pass_by_value,
    reason = "Tauri commands receive their arguments by value"
)]

use config::Config;
use parking_lot::Mutex;
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};

use crate::shortcut::Outcome;

#[derive(Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct Font {
    pub family: String,
    pub size: u32,
}

impl From<&Config> for Font {
    fn from(config: &Config) -> Self {
        Font {
            family: config.font_family.clone(),
            size: config.font_size,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Appearance {
    pub font: Font,
    pub tab_bar: config::TabBar,
}

impl From<&Config> for Appearance {
    fn from(config: &Config) -> Self {
        Appearance {
            font: Font::from(config),
            tab_bar: config.tab_bar,
        }
    }
}

#[derive(Default)]
struct Shared {
    config: Option<Config>,
    shortcut: Option<String>,
    watcher: Option<Channel<Appearance>>,
}

#[derive(Default)]
pub struct GuiConfig(Mutex<Shared>);

impl GuiConfig {
    pub fn height(&self) -> u32 {
        self.0
            .lock()
            .config
            .as_ref()
            .map_or(Config::default().height, |config| config.height)
    }
}

pub fn apply(app: &AppHandle, config: Config) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

    let gui_config = app.state::<GuiConfig>();
    let (previous, registered) = {
        let mut shared = gui_config.0.lock();
        let previous = shared.config.replace(config.clone());
        let appearance = Appearance::from(&config);
        if let Some(watcher) = &shared.watcher
            && previous.as_ref().map(Appearance::from) != Some(appearance.clone())
        {
            let _ = watcher.send(appearance);
        }
        (previous, shared.shortcut.clone())
    };
    if registered.as_deref() != Some(config.shortcut.as_str()) {
        let (active, failure) =
            match crate::shortcut::apply(app, &config.shortcut, registered.as_deref()) {
                Outcome::Registered => (Some(config.shortcut.clone()), None),
                Outcome::KeptPrevious(error) => (registered, Some((error, ""))),
                Outcome::NoneActive(error) => {
                    (None, Some((error, " No global shortcut is active.")))
                }
            };
        gui_config.0.lock().shortcut = active;
        let changed = previous
            .as_ref()
            .is_none_or(|previous| previous.shortcut != config.shortcut);
        if let Some((error, consequence)) = failure {
            log::warn!("can't register the shortcut {}: {error:#}", config.shortcut);
            if changed {
                app.dialog()
                    .message(format!(
                        "Couldn't register the global shortcut \"{}\": {error:#}.{consequence}",
                        config.shortcut
                    ))
                    .title("Shortcut unavailable")
                    .kind(MessageDialogKind::Warning)
                    .show(|_| {});
            }
        }
    }
    if previous.is_none_or(|previous| previous.height != config.height) {
        crate::window::position(app);
    }
}

#[tauri::command]
#[specta::specta]
pub fn watch_appearance(gui_config: tauri::State<'_, GuiConfig>, on_change: Channel<Appearance>) {
    let mut shared = gui_config.0.lock();
    if let Some(config) = &shared.config {
        let _ = on_change.send(Appearance::from(config));
    }
    shared.watcher = Some(on_change);
}
