use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::menu::MenuBuilder;
use tauri::{AppHandle, Manager};
use tauri_specta::Event as _;

use super::Daemon;
use super::commands::CommandResult;

const QUIT: &str = "app:quit";
const STOP: &str = "app:stop";

#[derive(Clone, Serialize, Deserialize, Type, tauri_specta::Event)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum TabMenuChosen {
    New,
    Pin { tab: String, pinned: bool },
    Rename { tab: String },
    Restart { tab: String },
    Close { tab: String },
}

impl TabMenuChosen {
    fn id(&self) -> String {
        serde_json::to_string(self).expect("a menu choice always serializes")
    }

    fn parse(id: &str) -> Option<Self> {
        serde_json::from_str(id).ok()
    }
}

pub fn handle(app: &AppHandle, id: &str) {
    if let Some(chosen) = TabMenuChosen::parse(id) {
        let _ = chosen.emit(app);
    } else if id == QUIT {
        app.exit(0);
    } else if id == STOP
        && let Some(daemon) = app.try_state::<Daemon>()
    {
        daemon.stop(app);
    }
}

#[tauri::command]
#[specta::specta]
#[expect(clippy::needless_pass_by_value, reason = "Tauri command")]
pub fn show_tab_menu(
    window: tauri::WebviewWindow,
    tab: Option<String>,
    pinned: bool,
) -> CommandResult<()> {
    let menu = if let Some(tab) = tab {
        let rename = TabMenuChosen::Rename { tab: tab.clone() };
        let label = if pinned { "Unpin Tab" } else { "Pin Tab" };
        let pin = TabMenuChosen::Pin {
            tab: tab.clone(),
            pinned: !pinned,
        };
        let mut menu = MenuBuilder::new(&window)
            .text(rename.id(), "Rename Tab…")
            .separator()
            .text(pin.id(), label);
        if pinned {
            let restart = TabMenuChosen::Restart { tab: tab.clone() };
            menu = menu.text(restart.id(), "Restart Tab");
        }
        menu.text(TabMenuChosen::Close { tab }.id(), "Close Tab")
    } else {
        MenuBuilder::new(&window)
            .text(TabMenuChosen::New.id(), "New Tab")
            .separator()
            .text(QUIT, "Quit Spawnshell")
            .text(STOP, "Quit and Stop Pinned Tabs")
    };
    window.popup_menu(&menu.build()?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{QUIT, TabMenuChosen};

    #[test]
    fn menu_ids_parse_back_to_the_choice() {
        let pin = TabMenuChosen::Pin {
            tab: "a".into(),
            pinned: true,
        };
        assert!(matches!(
            TabMenuChosen::parse(&pin.id()),
            Some(TabMenuChosen::Pin { tab, pinned: true }) if tab == "a"
        ));
        assert!(matches!(
            TabMenuChosen::parse(&TabMenuChosen::New.id()),
            Some(TabMenuChosen::New)
        ));
        assert!(TabMenuChosen::parse(QUIT).is_none());
    }
}
