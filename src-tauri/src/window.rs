#![expect(
    clippy::needless_pass_by_value,
    reason = "Tauri commands receive their arguments by value"
)]

use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSApplicationDidChangeScreenParametersNotification, NSApplicationDidResignActiveNotification,
    NSWindow,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSNotificationName};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_specta::Event as _;

use crate::client::Daemon;
use crate::gui_config::GuiConfig;
use crate::native;

const MAIN: &str = "main";

#[derive(Default)]
pub struct WindowState {
    open: AtomicBool,
}

#[derive(Clone, Serialize, specta::Type, tauri_specta::Event)]
pub struct WindowShown;

#[derive(Clone, Serialize, specta::Type, tauri_specta::Event)]
pub struct WindowHidden;

fn on_main_thread(app: &AppHandle, work: impl FnOnce(&AppHandle) + Send + 'static) {
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || work(&handle)) {
        log::error!("can't reach the main thread: {error}");
    }
}

fn main_window(app: &AppHandle) -> Option<Retained<NSWindow>> {
    let Some(mtm) = MainThreadMarker::new() else {
        log::error!("the window was accessed off the main thread");
        return None;
    };
    native::ns_window(&app.get_webview_window(MAIN)?, mtm)
}

fn height(app: &AppHandle) -> u32 {
    app.state::<GuiConfig>().height()
}

fn opened(app: &AppHandle) -> bool {
    app.state::<WindowState>().open.load(Ordering::Relaxed)
}

fn show_now(app: &AppHandle) {
    if opened(app) {
        return;
    }
    let Some(ns_window) = main_window(app) else {
        return;
    };
    app.state::<WindowState>()
        .open
        .store(true, Ordering::Relaxed);
    native::place_at_cursor(&ns_window, height(app));
    native::show(&ns_window);
    report_window_open(app, true);
    let _ = WindowShown.emit(app);
}

fn hide_now(app: &AppHandle) {
    if app
        .state::<WindowState>()
        .open
        .swap(false, Ordering::Relaxed)
    {
        report_window_open(app, false);
        let _ = WindowHidden.emit(app);
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
        if let Some(ns_window) = main_window(app) {
            native::place_on_own_screen(&ns_window, height(app));
        }
    });
}

pub fn setup_panel(app: &AppHandle) -> anyhow::Result<()> {
    let Some(window) = app.get_webview_window(MAIN) else {
        anyhow::bail!("main window is missing");
    };
    native::make_panel(&window)
}

fn observe(app: &AppHandle, name: &NSNotificationName, react: fn(&AppHandle)) {
    let handle = app.clone();
    let block = RcBlock::new(move |_: NonNull<NSNotification>| react(&handle));
    let _ = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(name),
            None,
            None,
            &block,
        )
    };
}

pub fn watch_screen_changes(app: &AppHandle) {
    observe(
        app,
        unsafe { NSApplicationDidChangeScreenParametersNotification },
        position,
    );
}

pub fn watch_deactivation(app: &AppHandle) {
    observe(
        app,
        unsafe { NSApplicationDidResignActiveNotification },
        hide,
    );
}

#[tauri::command]
#[specta::specta]
pub fn is_open(app: AppHandle) -> bool {
    opened(&app)
}

#[tauri::command]
#[specta::specta]
pub fn hide_window(app: AppHandle) {
    on_main_thread(&app, |app| {
        if !opened(app)
            && let Some(ns_window) = main_window(app)
        {
            native::order_out(&ns_window);
        }
    });
}
