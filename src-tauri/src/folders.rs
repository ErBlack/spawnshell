use std::path::PathBuf;

use client::Daemon;
use parking_lot::Mutex;
use tauri::{AppHandle, Manager, Url};

use crate::window;

#[derive(Default)]
pub struct Waiting(Mutex<Vec<PathBuf>>);

pub fn receive(app: &AppHandle, urls: &[Url]) {
    let folders = urls.iter().filter_map(|url| url.to_file_path().ok());
    app.state::<Waiting>().0.lock().extend(folders);
    open_waiting(app);
}

pub fn retry(app: &AppHandle, cwd: PathBuf) {
    app.state::<Waiting>().0.lock().insert(0, cwd);
    open_waiting(app);
}

pub fn open_waiting(app: &AppHandle) {
    let Some(daemon) = app.try_state::<Daemon>() else {
        return;
    };
    let waiting = app.state::<Waiting>();
    let mut folders = std::mem::take(&mut *waiting.0.lock()).into_iter();
    let mut opened = false;
    while let Some(cwd) = folders.next() {
        if let Err(cwd) = daemon.new_tab_in(cwd) {
            waiting
                .0
                .lock()
                .splice(0..0, std::iter::once(cwd).chain(folders));
            break;
        }
        opened = true;
    }
    if opened {
        window::show(app);
    }
}
