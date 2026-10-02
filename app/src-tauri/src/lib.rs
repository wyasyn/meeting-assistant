mod capture;
mod commands;
mod detector;
mod jobs;
mod metrics;
mod providers;
mod sidecar;
mod store;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Err(err) = tauri::Builder::default().run(tauri::generate_context!()) {
        // Logging is set up in roadmap task 0.4; until then stderr is all we have.
        eprintln!("failed to run the app: {err}");
        std::process::exit(1);
    }
}
