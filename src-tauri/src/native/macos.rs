use std::ptr::NonNull;

use anyhow::Result;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, NSObjectProtocol};
use objc2::{MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::{
    NSApplication, NSApplicationDidChangeScreenParametersNotification,
    NSApplicationDidResignActiveNotification, NSEvent, NSModalPanelWindowLevel, NSScreen, NSView,
    NSWindow, NSWindowCollectionBehavior,
};
use objc2_foundation::{
    NSMouseInRect, NSNotification, NSNotificationCenter, NSNotificationName, NSPoint, NSRect,
    NSSize,
};
use tauri::AppHandle;
use tauri_nspanel::WebviewWindowExt;

mod panel {
    tauri_nspanel::tauri_panel! {
        panel!(SpawnshellPanel {
            config: {
                can_become_key_window: true,
                is_floating_panel: true
            }
        })
    }
}

pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_nspanel::init()
}

pub fn hide_from_dock(app: &mut tauri::App) {
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);
}

pub fn make_panel(window: &tauri::WebviewWindow) -> Result<()> {
    let panel = window.to_panel::<panel::SpawnshellPanel>()?;
    panel.set_level(NSModalPanelWindowLevel as i64);
    panel.set_collection_behavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    Ok(())
}

fn cursor_screen(mtm: MainThreadMarker) -> Option<NSRect> {
    let mouse = NSEvent::mouseLocation();
    NSScreen::screens(mtm)
        .iter()
        .map(|screen| screen.frame())
        .find(|frame| NSMouseInRect(mouse, *frame, false))
}

fn main_screen(mtm: MainThreadMarker) -> Option<NSRect> {
    NSScreen::mainScreen(mtm).map(|screen| screen.frame())
}

fn fit(window: &NSWindow, screen: NSRect, height_percent: u32) {
    let height = screen.size.height * f64::from(height_percent) / 100.0;
    let origin = NSPoint::new(
        screen.origin.x,
        screen.origin.y + screen.size.height - height,
    );
    let size = NSSize::new(screen.size.width, height);
    window.setFrame_display(NSRect::new(origin, size), true);
}

pub struct Window(Retained<NSWindow>);

impl Window {
    pub fn of(window: &tauri::WebviewWindow) -> Option<Self> {
        if MainThreadMarker::new().is_none() {
            log::error!("the window was accessed off the main thread");
            return None;
        }
        match window.ns_window() {
            Ok(pointer) => unsafe { Retained::retain(pointer.cast::<NSWindow>()) }.map(Window),
            Err(error) => {
                log::error!("can't reach the native window: {error}");
                None
            }
        }
    }

    pub fn place_at_cursor(&self, height_percent: u32) {
        let mtm = self.0.mtm();
        if let Some(screen) = cursor_screen(mtm).or_else(|| main_screen(mtm)) {
            fit(&self.0, screen, height_percent);
        }
    }

    pub fn place_on_own_screen(&self, height_percent: u32) {
        let mtm = self.0.mtm();
        let screen = self
            .0
            .screen()
            .map(|screen| screen.frame())
            .or_else(|| cursor_screen(mtm))
            .or_else(|| main_screen(mtm));
        if let Some(screen) = screen {
            fit(&self.0, screen, height_percent);
        }
    }

    pub fn show(&self) {
        if let Some(content) = self.0.contentView() {
            let target = AnyClass::get(c"WKWebView")
                .and_then(|class| webview_in(&content, class))
                .unwrap_or(content);
            self.0.makeFirstResponder(Some(&target));
        }
        NSApplication::sharedApplication(self.0.mtm()).activate();
        self.0.orderFrontRegardless();
        self.0.makeKeyWindow();
    }

    pub fn order_out(&self) {
        self.0.orderOut(None);
        NSApplication::sharedApplication(self.0.mtm()).hide(None);
    }
}

fn webview_in(view: &NSView, class: &AnyClass) -> Option<Retained<NSView>> {
    if view.isKindOfClass(class) {
        return Some(view.retain());
    }
    view.subviews()
        .iter()
        .find_map(|subview| webview_in(&subview, class))
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

pub fn on_screen_change(app: &AppHandle, react: fn(&AppHandle)) {
    observe(
        app,
        unsafe { NSApplicationDidChangeScreenParametersNotification },
        react,
    );
}

pub fn on_deactivation(app: &AppHandle, react: fn(&AppHandle)) {
    observe(
        app,
        unsafe { NSApplicationDidResignActiveNotification },
        react,
    );
}
