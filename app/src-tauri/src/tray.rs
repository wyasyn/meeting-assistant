//! Tray icon and menu (FR-1.4, FR-1.5, FR-8.5). The app keeps running here when the window
//! is closed. While recording, the icon carries a red dot and the label reads "Recording"
//! (NFR-12: the recording state is always visible).

use tauri::image::Image;
use tauri::menu::{MenuBuilder, MenuItem, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Runtime};

use crate::recording::{Phase, RecordingService};

pub const MAIN_WINDOW: &str = "main";
const TRAY_ID: &str = "main";

const MENU_OPEN: &str = "open";
const MENU_RECORD: &str = "record";
const MENU_PAUSE: &str = "pause";
const MENU_QUIT: &str = "quit";

const RED: [u8; 4] = [220, 38, 38, 255];
const GREY: [u8; 4] = [140, 140, 140, 255];

/// Menu items whose text changes with the recording state.
struct TrayItems<R: Runtime> {
    record: MenuItem<R>,
    pause: MenuItem<R>,
}

pub fn init<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let open = MenuItemBuilder::with_id(MENU_OPEN, "Open").build(app)?;
    let record = MenuItemBuilder::with_id(MENU_RECORD, "Start recording").build(app)?;
    let pause = MenuItemBuilder::with_id(MENU_PAUSE, "Pause")
        .enabled(false)
        .build(app)?;
    let quit = MenuItemBuilder::with_id(MENU_QUIT, "Quit").build(app)?;
    let menu = MenuBuilder::new(app)
        .items(&[&open, &record, &pause])
        .separator()
        .item(&quit)
        .build()?;

    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_OPEN => show_main_window(app),
            MENU_RECORD => off_main_thread(app, toggle_recording),
            MENU_PAUSE => off_main_thread(app, toggle_pause),
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
    app.manage(TrayItems { record, pause });
    Ok(())
}

/// Device setup and file flushing block, so tray actions run on a worker thread.
fn off_main_thread<R: Runtime>(app: &AppHandle<R>, action: fn(&AppHandle<R>)) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || action(&app));
}

/// Start or stop from the tray. Errors are logged; the window shows them on its next state.
fn toggle_recording<R: Runtime>(app: &AppHandle<R>) {
    let Some(service) = app.try_state::<RecordingService>() else {
        return;
    };
    let result = match service.state().map(|s| s.state) {
        Ok(Phase::Recording | Phase::Paused) => service.stop().map(drop),
        Ok(_) => service.start(None, None).map(drop),
        Err(err) => Err(err),
    };
    if let Err(err) = result {
        tracing::warn!(%err, "tray recording action failed");
        show_main_window(app);
    }
}

fn toggle_pause<R: Runtime>(app: &AppHandle<R>) {
    let Some(service) = app.try_state::<RecordingService>() else {
        return;
    };
    let result = match service.state().map(|s| s.state) {
        Ok(Phase::Recording) => service.pause().map(drop),
        Ok(Phase::Paused) => service.resume().map(drop),
        Ok(_) => Ok(()),
        Err(err) => Err(err),
    };
    if let Err(err) = result {
        tracing::warn!(%err, "tray pause action failed");
        show_main_window(app);
    }
}

/// Updates icon, label, tooltip and menu to the recording state.
pub fn show_recording_state<R: Runtime>(app: &AppHandle<R>, phase: Phase) {
    let (label, dot, record_text, pause_text, pause_enabled) = match phase {
        Phase::Recording => (
            Some("Recording"),
            Some(RED),
            "Stop recording",
            "Pause",
            true,
        ),
        Phase::Paused => (Some("Paused"), Some(GREY), "Stop recording", "Resume", true),
        Phase::Idle | Phase::Stopped => (None, None, "Start recording", "Pause", false),
    };
    if let Some(items) = app.try_state::<TrayItems<R>>() {
        let updated = items
            .record
            .set_text(record_text)
            .and_then(|()| items.pause.set_text(pause_text))
            .and_then(|()| items.pause.set_enabled(pause_enabled));
        if let Err(err) = updated {
            tracing::warn!(%err, "could not update the tray menu");
        }
    }
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let icon = app.default_window_icon().map(|base| match dot {
        Some(color) => {
            let (w, h) = (base.width(), base.height());
            Image::new_owned(with_dot(base.rgba(), w, h, color), w, h)
        }
        None => base.clone().to_owned(),
    });
    let name = app.config().product_name.clone().unwrap_or_default();
    let tooltip = match label {
        Some(label) => format!("{name} ({label})"),
        None => name,
    };
    let updated = tray
        .set_icon(icon)
        .and_then(|()| tray.set_title(label))
        .and_then(|()| tray.set_tooltip(Some(tooltip)));
    if let Err(err) = updated {
        tracing::warn!(%err, "could not update the tray icon");
    }
}

/// Copies an RGBA icon and paints a filled dot in its lower-right corner.
pub fn with_dot(rgba: &[u8], width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    let size = width.min(height) as f32;
    let radius = size * 0.22;
    let (cx, cy) = (
        width as f32 - radius - size * 0.02,
        height as f32 - radius - size * 0.02,
    );
    for y in 0..height {
        for x in 0..width {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            if dx * dx + dy * dy <= radius * radius {
                let i = ((y * width + x) * 4) as usize;
                if let Some(px) = out.get_mut(i..i + 4) {
                    px.copy_from_slice(&color);
                }
            }
        }
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_1_5_dot_is_painted_in_the_corner_only() {
        let (w, h) = (32u32, 32u32);
        let base = vec![10u8; (w * h * 4) as usize];
        let out = with_dot(&base, w, h, RED);
        let px = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            [out[i], out[i + 1], out[i + 2], out[i + 3]]
        };
        assert_eq!(px(25, 25), RED);
        assert_eq!(px(2, 2), [10; 4]);
        assert_eq!(px(25, 2), [10; 4]);
        assert_eq!(out.len(), base.len());
    }
}
