mod client;
mod folders;
mod gui_config;
mod native;
mod shortcut;
mod window;

use std::process::ExitCode;

use tauri::{AppHandle, Manager, RunEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_global_shortcut::ShortcutState;
use tauri_specta::{collect_commands, collect_events};

fn fail_to_start(app: &AppHandle, message: String) {
    let handle = app.clone();
    app.dialog()
        .message(message)
        .title("Spawnshell failed to start")
        .show(move |_| handle.exit(1));
}

#[tauri::command]
#[specta::specta]
#[expect(clippy::needless_pass_by_value, reason = "Tauri command")]
fn report_fatal(app: AppHandle, message: String) {
    fail_to_start(&app, message);
}

#[tauri::command]
#[specta::specta]
#[expect(clippy::needless_pass_by_value, reason = "Tauri command")]
fn report_error(app: AppHandle, message: String) {
    app.dialog()
        .message(message)
        .title("Spawnshell")
        .show(|_| {});
}

fn bindings() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::new()
        .error_handling(tauri_specta::ErrorHandlingMode::Throw)
        .commands(collect_commands![
            client::commands::tabs_list,
            client::commands::tab_new,
            client::commands::tab_close,
            client::commands::tab_activate,
            client::commands::tab_place,
            client::commands::tab_restart,
            client::commands::tab_rename,
            client::commands::tab_attach,
            client::commands::tab_detach,
            client::commands::tab_write,
            client::commands::terminal_resize,
            client::commands::tab_pause,
            client::commands::tab_resume,
            client::commands::session_save,
            client::commands::take_warning,
            client::commands::history_lines,
            client::menu::show_tab_menu,
            window::hide_window,
            window::is_open,
            gui_config::watch_appearance,
            report_fatal,
            report_error,
        ])
        .events(collect_events![
            client::events::TabsChanged,
            client::events::TabSelected,
            client::events::TabTitleChanged,
            client::events::Reconnected,
            client::events::Warning,
            client::menu::TabMenuChosen,
            window::WindowShown,
            window::WindowHidden,
        ])
}

fn main() -> ExitCode {
    protocol::build();
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("__daemon") => daemon::run(
            args.next().as_deref() == Some("--adopt"),
            env!("CARGO_PKG_VERSION"),
        ),
        Some(_) => cli::run(env!("CARGO_PKG_VERSION"), env!("CARGO_PKG_DESCRIPTION")),
        None => match app() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("spawnshell: {error:#}");
                ExitCode::FAILURE
            }
        },
    }
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
    notifier::listen();
    let bindings = bindings();
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
        .plugin(tauri_nspanel::init())
        .manage(window::WindowState::default())
        .manage(gui_config::GuiConfig::default())
        .manage(folders::Waiting::default())
        .invoke_handler(bindings.invoke_handler())
        .setup(move |app| {
            bindings.mount_events(app);
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
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

            match client::Daemon::connect(app.handle()) {
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
        .on_menu_event(|app, event| client::menu::handle(app, event.id().0.as_str()))
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

#[cfg(test)]
mod tests {
    #[test]
    fn typescript_bindings_are_up_to_date() {
        let path = std::path::Path::new("../src/bindings.ts");
        let before = std::fs::read_to_string(path).unwrap_or_default();
        super::bindings()
            .export(specta_typescript::Typescript::default(), path)
            .unwrap();
        let after = std::fs::read_to_string(path).unwrap();
        assert!(
            before == after,
            "src/bindings.ts was out of date and has been regenerated"
        );
    }
}
