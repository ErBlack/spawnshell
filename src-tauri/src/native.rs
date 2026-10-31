#[cfg_attr(target_os = "macos", path = "native/macos.rs")]
#[cfg_attr(not(target_os = "macos"), path = "native/other.rs")]
mod imp;

pub use imp::{Window, hide_from_dock, make_panel, on_deactivation, on_screen_change, plugin};
