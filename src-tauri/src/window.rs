use std::sync::atomic::{AtomicBool, Ordering};

use client::Daemon;
use tauri::{AppHandle, Manager};

use crate::bridge;
use crate::gui_config::GuiConfig;
use crate::native;

const MAIN: &str = "main";

#[derive(Default)]
pub struct WindowState {
    open: AtomicBool,
}

fn on_main_thread(app: &AppHandle, work: impl FnOnce(&AppHandle) + Send + 'static) {
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || work(&handle)) {
        log::error!("can't reach the main thread: {error}");
    }
}

fn main_window(app: &AppHandle) -> Option<native::Window> {
    native::Window::of(&app.get_webview_window(MAIN)?)
}

fn height(app: &AppHandle) -> u32 {
    app.state::<GuiConfig>().height()
}

pub fn opened(app: &AppHandle) -> bool {
    app.state::<WindowState>().open.load(Ordering::Relaxed)
}

fn show_now(app: &AppHandle) {
    if opened(app) {
        return;
    }
    let Some(window) = main_window(app) else {
        return;
    };
    app.state::<WindowState>()
        .open
        .store(true, Ordering::Relaxed);
    window.place_at_cursor(height(app));
    window.show();
    report_window_open(app, true);
    bridge::window_shown(app);
}

fn hide_now(app: &AppHandle) {
    if app
        .state::<WindowState>()
        .open
        .swap(false, Ordering::Relaxed)
    {
        report_window_open(app, false);
        bridge::window_hidden(app);
    }
}

fn report_window_open(app: &AppHandle, open: bool) {
    if let Some(daemon) = app.try_state::<Daemon>() {
        daemon.set_window_open(open);
    }
}

pub fn show(app: &AppHandle) {
    on_main_thread(app, show_now);
}

pub fn hide(app: &AppHandle) {
    on_main_thread(app, hide_now);
}

pub fn toggle(app: &AppHandle) {
    on_main_thread(app, |app| {
        if opened(app) {
            hide_now(app);
        } else {
            show_now(app);
        }
    });
}

pub fn position(app: &AppHandle) {
    on_main_thread(app, |app| {
        if let Some(window) = main_window(app) {
            window.place_on_own_screen(height(app));
        }
    });
}

pub fn setup_panel(app: &AppHandle) -> anyhow::Result<()> {
    let Some(window) = app.get_webview_window(MAIN) else {
        anyhow::bail!("main window is missing");
    };
    native::make_panel(&window)
}

pub fn watch_screen_changes(app: &AppHandle) {
    native::on_screen_change(app, position);
}

pub fn watch_deactivation(app: &AppHandle) {
    native::on_deactivation(app, hide);
}

pub fn order_out(app: &AppHandle) {
    on_main_thread(app, |app| {
        if !opened(app)
            && let Some(window) = main_window(app)
        {
            window.order_out();
        }
    });
}
