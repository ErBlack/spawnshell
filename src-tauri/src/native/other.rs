use anyhow::Result;
use tauri::AppHandle;

pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("native").build()
}

pub fn hide_from_dock(_app: &mut tauri::App) {}

pub fn make_panel(_window: &tauri::WebviewWindow) -> Result<()> {
    Ok(())
}

pub struct Window(tauri::WebviewWindow);

impl Window {
    pub fn of(window: &tauri::WebviewWindow) -> Option<Self> {
        Some(Window(window.clone()))
    }

    pub fn place_at_cursor(&self, _height_percent: u32) {}

    pub fn place_on_own_screen(&self, _height_percent: u32) {}

    pub fn show(&self) {
        if let Err(error) = self.0.show().and_then(|()| self.0.set_focus()) {
            log::error!("can't show the window: {error}");
        }
    }

    pub fn order_out(&self) {
        if let Err(error) = self.0.hide() {
            log::error!("can't hide the window: {error}");
        }
    }
}

pub fn on_screen_change(_app: &AppHandle, _react: fn(&AppHandle)) {}

pub fn on_deactivation(_app: &AppHandle, _react: fn(&AppHandle)) {}
