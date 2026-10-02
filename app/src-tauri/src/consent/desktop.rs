//! Linux notifications with buttons, over `org.freedesktop.Notifications` (notify-rust).
//! GNOME, KDE and most other desktops support actions; where one does not, the buttons
//! are missing and the window prompt is still there.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use notify_rust::{Notification, NotificationHandle, NotificationResponse, Timeout};

use super::{Choice, Note, NoteId, Notifier, OnChoice};

type Shown = Arc<Mutex<HashMap<u32, Arc<NotificationHandle>>>>;

pub struct DesktopNotifier {
    app_name: String,
    /// Notifications still on screen, so `close` can reach them.
    shown: Shown,
}

impl DesktopNotifier {
    pub fn new(app_name: String) -> Self {
        Self {
            app_name,
            shown: Arc::default(),
        }
    }
}

fn lock(shown: &Shown) -> MutexGuard<'_, HashMap<u32, Arc<NotificationHandle>>> {
    shown.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Notifier for DesktopNotifier {
    fn show(&self, note: Note, on_choice: OnChoice) -> Option<NoteId> {
        let mut notification = Notification::new();
        notification
            .appname(&self.app_name)
            .summary(&note.summary)
            .body(&note.body)
            // Clicking the notification itself; most desktops show no button for it.
            .action(Choice::Open.key(), "Open");
        for (choice, label) in &note.buttons {
            notification.action(choice.key(), label);
        }
        if note.sticky {
            notification.timeout(Timeout::Never);
        }
        // Blocks for one D-Bus call; callers are never on the UI thread.
        let handle = match notification.show() {
            Ok(handle) => Arc::new(handle),
            Err(err) => {
                tracing::warn!(%err, "could not show a notification");
                return None;
            }
        };
        let id = handle.id();
        lock(&self.shown).insert(id, Arc::clone(&handle));

        let shown = Arc::clone(&self.shown);
        tauri::async_runtime::spawn(async move {
            let mut choice = Choice::Dismissed;
            handle
                .wait_for_action_async(|response| {
                    choice = match response {
                        NotificationResponse::Default => Choice::Open,
                        NotificationResponse::Action(key) => Choice::from_key(key),
                        _ => Choice::Dismissed,
                    };
                })
                .await;
            lock(&shown).remove(&id);
            // The answer may start a recording, which blocks on device setup.
            tauri::async_runtime::spawn_blocking(move || on_choice(choice));
        });
        Some(NoteId(id))
    }

    fn close(&self, id: NoteId) {
        let handle = lock(&self.shown).remove(&id.0);
        if let Some(handle) = handle {
            tauri::async_runtime::spawn(async move { handle.close_async().await });
        }
    }
}
