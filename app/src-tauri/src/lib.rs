mod capture;
mod commands;
mod detector;
pub mod error;
mod jobs;
mod logging;
mod metrics;
mod providers;
mod sidecar;
mod store;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            app.manage(logging::init(&data_dir.join("logs"))?);
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "app started");

            let keys = store::key::KeyringStore::new(app.config().identifier.clone());
            let db = store::Store::open(&data_dir.join("db").join("app.sqlite"), &keys)
                .inspect_err(|err| tracing::error!(%err, "could not open the store"))?;
            app.manage(db);
            Ok(())
        })
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
