pub mod capture;
mod commands;
mod detector;
pub mod error;
mod jobs;
mod logging;
mod metrics;
mod providers;
mod sidecar;
pub mod store;
mod tray;

use tauri::Manager;

/// Passed by the login item only, so a start on login stays in the tray (FR-8.5).
const MINIMIZED_ARG: &str = "--minimized";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        // Must be first: a second launch only shows the running window.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .arg(MINIMIZED_ARG)
                .build(),
        )
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            app.manage(logging::init(&data_dir.join("logs"))?);
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "app started");

            let keys = store::key::KeyringStore::new(app.config().identifier.clone());
            let db = store::Store::open(&data_dir.join("db").join("app.sqlite"), &keys)
                .inspect_err(|err| tracing::error!(%err, "could not open the store"))?;
            // Close recordings a crash cut off before anything else touches meetings (NFR-6).
            if let Err(err) = capture::recovery::recover(&db, &data_dir) {
                tracing::error!(%err, "could not recover interrupted recordings");
            }
            app.manage(db);

            tray::init(app.handle())?;
            if !std::env::args().any(|arg| arg == MINIMIZED_ARG) {
                tray::show_main_window(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing hides to the tray; Quit in the tray menu exits.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == tray::MAIN_WINDOW {
                    api.prevent_close();
                    if let Err(err) = window.hide() {
                        tracing::warn!(%err, "could not hide the main window");
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::settings::get_settings,
            commands::settings::set_settings,
        ])
        .build(tauri::generate_context!());

    match app {
        Ok(app) => app.run(|handle, event| {
            if let tauri::RunEvent::Exit = event {
                tracing::info!("app exiting");
                if let Some(guard) = handle.try_state::<logging::LogGuard>() {
                    guard.flush();
                }
            }
        }),
        Err(err) => {
            // Setup failed, possibly before logging was up, so stderr is the fallback.
            eprintln!("failed to start the app: {err}");
            std::process::exit(1);
        }
    }
}
