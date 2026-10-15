use anyhow::Result;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, NSObjectProtocol};
use objc2::{MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::{
    NSEvent, NSScreen, NSStatusWindowLevel, NSView, NSWindow, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::{NSMouseInRect, NSPoint, NSRect, NSSize};
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

pub fn make_panel(window: &tauri::WebviewWindow) -> Result<()> {
    let panel = window.to_panel::<panel::SpawnshellPanel>()?;
    panel.set_style_mask(NSWindowStyleMask::NonactivatingPanel)?;
    panel.set_level(NSStatusWindowLevel as i64);
    panel.set_collection_behavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    Ok(())
}

pub fn ns_window(
    window: &tauri::WebviewWindow,
    _mtm: MainThreadMarker,
) -> Option<Retained<NSWindow>> {
    match window.ns_window() {
        Ok(pointer) => unsafe { Retained::retain(pointer.cast::<NSWindow>()) },
        Err(error) => {
            log::error!("can't reach the native window: {error}");
            None
        }
    }
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

pub fn place_at_cursor(window: &NSWindow, height_percent: u32) {
    let mtm = window.mtm();
    if let Some(screen) = cursor_screen(mtm).or_else(|| main_screen(mtm)) {
        fit(window, screen, height_percent);
    }
}

pub fn place_on_own_screen(window: &NSWindow, height_percent: u32) {
    let mtm = window.mtm();
    let screen = window
        .screen()
        .map(|screen| screen.frame())
        .or_else(|| cursor_screen(mtm))
        .or_else(|| main_screen(mtm));
    if let Some(screen) = screen {
        fit(window, screen, height_percent);
    }
}

pub fn show(window: &NSWindow) {
    if let Some(content) = window.contentView() {
        let target = AnyClass::get(c"WKWebView")
            .and_then(|class| webview_in(&content, class))
            .unwrap_or(content);
        window.makeFirstResponder(Some(&target));
    }
    window.orderFrontRegardless();
    window.makeKeyWindow();
}

fn webview_in(view: &NSView, class: &AnyClass) -> Option<Retained<NSView>> {
    if view.isKindOfClass(class) {
        return Some(view.retain());
    }
    view.subviews()
        .iter()
        .find_map(|subview| webview_in(&subview, class))
}

pub fn order_out(window: &NSWindow) {
    window.orderOut(None);
}
