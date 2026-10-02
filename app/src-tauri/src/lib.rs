pub mod capture;
mod commands;
mod consent;
mod detector;
pub mod error;
mod jobs;
mod logging;
mod metrics;
mod providers;
mod recording;
mod sidecar;
pub mod store;
mod tray;

use std::sync::Arc;

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
            let db = Arc::new(db);
            app.manage(Arc::clone(&db));
            // API keys live in the keychain next to the database key (FR-8.1).
            let providers = providers::ProviderService::new(
                Arc::new(providers::keys::KeyringApiKeys::new(
                    app.config().identifier.clone(),
                )),
                providers::ProviderService::default_factory(),
            );
            app.manage(providers.clone());
            // Post-call processing (NFR-7, ADR-021).
            let transcribe = |track| {
                Box::new(jobs::steps::TranscribeStep {
                    store: Arc::clone(&db),
                    providers: providers.clone(),
                    data_dir: data_dir.clone(),
                    track,
                })
            };
            let pipeline = jobs::Pipeline::default()
                .with(jobs::Step::TranscribeMic, transcribe(capture::Track::Mic))
                .with(
                    jobs::Step::TranscribeSystem,
                    transcribe(capture::Track::Sys),
                )
                .with(
                    jobs::Step::Merge,
                    Box::new(jobs::steps::MergeStep {
                        store: Arc::clone(&db),
                    }),
                );
            app.manage(jobs::JobService::start(
                Arc::clone(&db),
                pipeline,
                Arc::new(commands::jobs::TauriJobEvents(app.handle().clone())),
            ));
            let recording = recording::RecordingService::new(
                capture::default_backend(),
                Arc::clone(&db),
                data_dir.clone(),
                Arc::new(commands::recording::TauriEvents(app.handle().clone())),
            );
            app.manage(recording.clone());
            // Detections reach the user only through consent (FR-1.3, FR-1.7).
            let consent = consent::ConsentService::new(
                db,
                Arc::new(recording),
                consent::default_notifier(
                    app.config()
                        .product_name
                        .clone()
                        .unwrap_or_else(|| app.config().identifier.clone()),
                ),
                Arc::new(commands::consent::TauriConsentEvents(app.handle().clone())),
            );
            app.manage(consent.clone());

            let handle = app.handle().clone();
            app.manage(detector::DetectorService::start(
                detector::default_processes(),
                detector::default_streams(),
                Arc::new(consent),
                // No meeting:detected while a recording runs or is paused; its end instead.
                Box::new(move || {
                    handle
                        .try_state::<recording::RecordingService>()
                        .and_then(|service| service.active())
                }),
            ));

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
            commands::recording::start_recording,
            commands::recording::pause_recording,
            commands::recording::resume_recording,
            commands::recording::stop_recording,
            commands::recording::get_recording_state,
            commands::recording::list_audio_devices,
            commands::consent::list_app_rules,
            commands::consent::set_app_rule,
            commands::providers::list_providers,
            commands::providers::set_api_key,
            commands::providers::clear_api_key,
            commands::providers::test_provider,
        ])
        .build(tauri::generate_context!());

    match app {
        Ok(app) => app.run(|handle, event| {
            if let tauri::RunEvent::Exit = event {
                tracing::info!("app exiting");
                if let Some(detector) = handle.try_state::<detector::DetectorService>() {
                    detector.shutdown();
                }
                if let Some(jobs) = handle.try_state::<jobs::JobService>() {
                    jobs.shutdown();
                }
                // Save a running recording before the process ends (rule 3).
                if let Some(service) = handle.try_state::<recording::RecordingService>() {
                    service.shutdown();
                }
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
