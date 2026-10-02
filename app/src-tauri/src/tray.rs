//! Tray icon and menu (FR-1.4, FR-8.5). The app keeps running here when the window is closed.

use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Runtime};

pub const MAIN_WINDOW: &str = "main";

const MENU_OPEN: &str = "open";
const MENU_START_RECORDING: &str = "start_recording";
const MENU_QUIT: &str = "quit";

pub fn init<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let open = MenuItemBuilder::with_id(MENU_OPEN, "Open").build(app)?;
    // Enabled by roadmap 1.3, together with the red "Recording" tray state.
    let start = MenuItemBuilder::with_id(MENU_START_RECORDING, "Start recording")
        .enabled(false)
        .build(app)?;
    let quit = MenuItemBuilder::with_id(MENU_QUIT, "Quit").build(app)?;
    let menu = MenuBuilder::new(app)
        .items(&[&open, &start])
        .separator()
        .item(&quit)
        .build()?;

    let mut tray = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_OPEN => show_main_window(app),
            MENU_QUIT => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    if let Some(name) = &app.config().product_name {
        tray = tray.tooltip(name);
    }
    tray.build(app)?;
    Ok(())
}

/// Shows, restores and focuses the main window.
pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        tracing::warn!("main window not found");
        return;
    };
    let result = window
        .show()
        .and_then(|()| window.unminimize())
        .and_then(|()| window.set_focus());
    if let Err(err) = result {
        tracing::warn!(%err, "could not show the main window");
    }
}
