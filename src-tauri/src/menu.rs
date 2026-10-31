use client::Daemon;
use tauri::menu::MenuBuilder;
use tauri::{AppHandle, Manager};

use crate::bridge::{self, TabMenuChosen};

const INSTALL: &str = "app:install";
const QUIT: &str = "app:quit";
const STOP: &str = "app:stop";

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
        bridge::tab_menu_chosen(app, chosen);
    } else if id == INSTALL {
        install(app);
    } else if id == QUIT {
        app.exit(0);
    } else if id == STOP
        && let Some(daemon) = app.try_state::<Daemon>()
    {
        daemon.stop();
    }
}

fn install(app: &AppHandle) {
    let message = match platform::install::install() {
        Ok(link) => format!("The spawnshell command is installed as {}.", link.display()),
        Err(error) => format!("Couldn't install the spawnshell command: {error:#}"),
    };
    crate::show_message(app, message);
}

pub fn show(window: &tauri::WebviewWindow, tab: Option<String>, pinned: bool) -> tauri::Result<()> {
    let menu = if let Some(tab) = tab {
        let rename = TabMenuChosen::Rename { tab: tab.clone() };
        let label = if pinned { "Unpin Tab" } else { "Pin Tab" };
        let pin = TabMenuChosen::Pin {
            tab: tab.clone(),
            pinned: !pinned,
        };
        let mut menu = MenuBuilder::new(window)
            .text(rename.id(), "Rename Tab…")
            .separator()
            .text(pin.id(), label);
        if pinned {
            let restart = TabMenuChosen::Restart { tab: tab.clone() };
            menu = menu.text(restart.id(), "Restart Tab");
        }
        menu.text(TabMenuChosen::Close { tab }.id(), "Close Tab")
    } else {
        MenuBuilder::new(window)
            .text(TabMenuChosen::New.id(), "New Tab")
            .separator()
            .text(INSTALL, "Install Command Line Tool…")
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
