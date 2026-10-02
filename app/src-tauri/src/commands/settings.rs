//! `get_settings` / `set_settings` (FR-8.3, FR-8.5).
//! Start on login is read from the OS login item itself, so it can never drift from a stored copy.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
use tauri_plugin_autostart::ManagerExt;

use crate::error::AppError;

/// Mirrors `Settings` in `app/src/lib/ipc.ts`. FR-8.3 fields join later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub start_on_login: bool,
}

/// The OS "start on login" entry. Faked in tests.
pub trait LoginItem {
    fn is_enabled(&self) -> Result<bool, String>;
    fn set_enabled(&self, enabled: bool) -> Result<(), String>;
}

struct AutostartLoginItem<'a, R: Runtime>(&'a AppHandle<R>);

impl<R: Runtime> LoginItem for AutostartLoginItem<'_, R> {
    fn is_enabled(&self) -> Result<bool, String> {
        self.0.autolaunch().is_enabled().map_err(|e| e.to_string())
    }

    fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        let manager = self.0.autolaunch();
        if enabled {
            manager.enable()
        } else {
            manager.disable()
        }
        .map_err(|e| e.to_string())
    }
}

fn read(login: &dyn LoginItem) -> Result<Settings, AppError> {
    Ok(Settings {
        start_on_login: login.is_enabled().map_err(AppError::internal)?,
    })
}

fn apply(login: &dyn LoginItem, settings: &Settings) -> Result<Settings, AppError> {
    if login.is_enabled().map_err(AppError::internal)? != settings.start_on_login {
        login
            .set_enabled(settings.start_on_login)
            .map_err(AppError::internal)?;
        tracing::info!(enabled = settings.start_on_login, "start on login changed");
    }
    read(login)
}

#[tauri::command]
pub fn get_settings<R: Runtime>(app: AppHandle<R>) -> Result<Settings, AppError> {
    read(&AutostartLoginItem(&app))
}

#[tauri::command]
pub fn set_settings<R: Runtime>(
    app: AppHandle<R>,
    settings: Settings,
) -> Result<Settings, AppError> {
    apply(&AutostartLoginItem(&app), &settings)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[derive(Default)]
    struct FakeLoginItem {
        enabled: Cell<bool>,
        broken: bool,
    }

    impl LoginItem for FakeLoginItem {
        fn is_enabled(&self) -> Result<bool, String> {
            Ok(self.enabled.get())
        }
        fn set_enabled(&self, enabled: bool) -> Result<(), String> {
            if self.broken {
                return Err("permission denied writing autostart entry".into());
            }
            self.enabled.set(enabled);
            Ok(())
        }
    }

    #[test]
    fn fr_8_5_turns_start_on_login_on_and_off() {
        let login = FakeLoginItem::default();
        let on = Settings {
            start_on_login: true,
        };
        assert_eq!(apply(&login, &on).unwrap(), on);
        assert!(login.enabled.get());

        let off = Settings {
            start_on_login: false,
        };
        assert_eq!(apply(&login, &off).unwrap(), off);
        assert_eq!(read(&login).unwrap(), off);
    }

    #[test]
    fn login_item_failure_is_an_internal_error() {
        let login = FakeLoginItem {
            broken: true,
            ..FakeLoginItem::default()
        };
        let err = apply(
            &login,
            &Settings {
                start_on_login: true,
            },
        )
        .unwrap_err();
        assert_eq!(err.code(), "internal");
    }

    #[test]
    fn settings_use_camel_case_on_the_wire() {
        let json = serde_json::to_value(Settings {
            start_on_login: true,
        })
        .unwrap();
        assert_eq!(json, serde_json::json!({ "startOnLogin": true }));
    }
}
