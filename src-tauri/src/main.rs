mod bridge;
mod folders;
mod gui_config;
mod menu;
mod native;
mod shortcut;
mod window;

use std::process::ExitCode;

use tauri::{AppHandle, Manager, RunEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_global_shortcut::ShortcutState;

fn show_message(app: &AppHandle, message: String) {
    app.dialog()
        .message(message)
        .title("Spawnshell")
        .show(|_| {});
}

fn fail_to_start(app: &AppHandle, message: String) {
    let handle = app.clone();
    app.dialog()
        .message(message)
        .title("Spawnshell failed to start")
        .show(move |_| handle.exit(1));
}

fn main() -> ExitCode {
    entry::run(
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_DESCRIPTION"),
        || match app() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("spawnshell: {error:#}");
                ExitCode::FAILURE
            }
        },
    )
}

fn logger() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

    tauri_plugin_log::Builder::new()
        .targets([
            Target::new(TargetKind::Stdout),
            Target::new(TargetKind::LogDir {
                file_name: Some("app".into()),
            }),
        ])
        .level(log::LevelFilter::Info)
        .max_file_size(1_000_000)
        .rotation_strategy(RotationStrategy::KeepSome(3))
        .timezone_strategy(TimezoneStrategy::UseLocal)
        .build()
}

fn setup_notifications(app: &AppHandle) {
    notifier::request_permission(|granted, error| {
        if !granted {
            let reason = error.map(|error| format!(": {error}")).unwrap_or_default();
            log::warn!("notifications are not allowed{reason}");
        }
    });
    let app = app.clone();
    notifier::on_click(move |tab| {
        window::show(&app);
        app.state::<client::Daemon>().select(tab);
    });
}

fn app() -> tauri::Result<()> {
    client::capture_build();
    notifier::listen();
    let bindings = bridge::bindings();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::show(app);
        }))
        .plugin(logger())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        window::toggle(app);
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(native::plugin())
        .manage(window::WindowState::default())
        .manage(gui_config::GuiConfig::default())
        .manage(folders::Waiting::default())
        .invoke_handler(bindings.invoke_handler())
        .setup(move |app| {
            bindings.mount_events(app);
            native::hide_from_dock(app);
            if let Err(error) = window::setup_panel(app.handle()) {
                log::warn!("can't turn the window into a panel: {error:#}");
                app.dialog()
                    .message(format!(
                        "Couldn't turn the window into a panel — it will behave like a normal window.\n\n{error:#}"
                    ))
                    .title("Panel unavailable")
                    .kind(MessageDialogKind::Warning)
                    .show(|_| {});
            }
            window::watch_screen_changes(app.handle());
            window::watch_deactivation(app.handle());

            let handle = app.handle().clone();
            match client::Daemon::connect(move |event| bridge::on_client_event(&handle, event)) {
                Ok(daemon) => {
                    app.manage(daemon);
                    setup_notifications(app.handle());
                    folders::open_waiting(app.handle());
                }
                Err(error) => {
                    log::error!("can't connect to the background process: {error:#}");
                    fail_to_start(app.handle(), format!("{error:#}"));
                }
            }
            Ok(())
        })
        .on_menu_event(|app, event| menu::handle(app, event.id().0.as_str()))
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                window::hide(window.app_handle());
            }
        })
        .build(tauri::generate_context!())?
        .run(|app, event| {
            if let RunEvent::Opened { urls } = event {
                folders::receive(app, &urls);
            }
        });
    Ok(())
}
