#![expect(
    clippy::needless_pass_by_value,
    reason = "Tauri commands receive their arguments by value"
)]

use std::sync::atomic::{AtomicBool, Ordering};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::NSWindow;
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_specta::Event as _;
use tokio::sync::oneshot;

use crate::gui_config::GuiConfig;
use crate::native;

const MAIN: &str = "main";

#[derive(Default)]
pub struct WindowState {
    open: AtomicBool,
    confirming: AtomicBool,
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

fn confirming(app: &AppHandle) -> bool {
    app.state::<WindowState>()
        .confirming
        .load(Ordering::Relaxed)
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
    let _ = WindowShown.emit(app);
}

fn hide_now(app: &AppHandle) {
    if app
        .state::<WindowState>()
        .open
        .swap(false, Ordering::Relaxed)
    {
        let _ = WindowHidden.emit(app);
    }
}

pub fn show(app: &AppHandle) {
    on_main_thread(app, show_now);
}

pub fn hide(app: &AppHandle) {
    on_main_thread(app, hide_now);
}

pub fn blurred(app: &AppHandle) {
    on_main_thread(app, |app| {
        if !confirming(app) {
            hide_now(app);
        }
    });
}

pub async fn confirm(app: &AppHandle, title: &str, message: &str, action: &str) -> bool {
    let state = app.state::<WindowState>();
    if state.confirming.swap(true, Ordering::Relaxed) {
        return false;
    }
    let (answer, answered) = oneshot::channel();
    app.dialog()
        .message(message)
        .title(title)
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            action.to_string(),
            "Cancel".to_string(),
        ))
        .show(move |confirmed| {
            let _ = answer.send(confirmed);
        });
    let confirmed = answered.await.unwrap_or(false);
    state.confirming.store(false, Ordering::Relaxed);
    on_main_thread(app, |app| {
        if opened(app)
            && let Some(ns_window) = main_window(app)
        {
            native::show(&ns_window);
        }
    });
    confirmed
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

pub fn watch_screen_changes(app: &AppHandle) {
    use block2::RcBlock;
    use objc2_app_kit::NSApplicationDidChangeScreenParametersNotification;
    use objc2_foundation::{NSNotification, NSNotificationCenter};

    let handle = app.clone();
    let block = RcBlock::new(move |_: core::ptr::NonNull<NSNotification>| {
        position(&handle);
    });
    let _ = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(NSApplicationDidChangeScreenParametersNotification),
            None,
            None,
            &block,
        )
    };
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
