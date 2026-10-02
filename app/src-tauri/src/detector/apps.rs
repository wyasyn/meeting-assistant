//! Meeting apps the detector knows, matched by process or binary name (FR-1.1, FR-1.2).

use serde::{Deserialize, Serialize};

/// The app a signal came from. Serialised as the `meetings.source_app` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceApp {
    Zoom,
    Slack,
    Teams,
    Discord,
    /// Any web browser; the extension (Phase 3) will tell which site.
    Browser,
}

impl SourceApp {
    pub const ALL: [Self; 5] = [
        Self::Zoom,
        Self::Slack,
        Self::Teams,
        Self::Discord,
        Self::Browser,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zoom => "zoom",
            Self::Slack => "slack",
            Self::Teams => "teams",
            Self::Discord => "discord",
            Self::Browser => "browser",
        }
    }

    /// The `source_app` value back to the app; `None` for values detection never makes.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|app| app.as_str() == value)
    }

    /// Name shown in prompts and titles.
    pub fn label(self) -> &'static str {
        match self {
            Self::Zoom => "Zoom",
            Self::Slack => "Slack",
            Self::Teams => "Teams",
            Self::Discord => "Discord",
            Self::Browser => "Browser",
        }
    }

    /// Title of a meeting recorded from a detection, until the calendar names it (FR-1.8).
    pub fn meeting_title(self) -> String {
        format!("{} meeting", self.label())
    }
}

const NAMES: &[(&str, SourceApp)] = &[
    ("zoom", SourceApp::Zoom),
    ("zoom.real", SourceApp::Zoom),
    ("slack", SourceApp::Slack),
    ("teams-for-linux", SourceApp::Teams),
    ("teams", SourceApp::Teams),
    ("ms-teams", SourceApp::Teams),
    ("discord", SourceApp::Discord),
    ("discordcanary", SourceApp::Discord),
    ("discordptb", SourceApp::Discord),
    ("firefox", SourceApp::Browser),
    ("firefox-bin", SourceApp::Browser),
    ("firefox-esr", SourceApp::Browser),
    ("librewolf", SourceApp::Browser),
    ("zen", SourceApp::Browser),
    ("zen-bin", SourceApp::Browser),
    ("chrome", SourceApp::Browser),
    ("google-chrome", SourceApp::Browser),
    ("chromium", SourceApp::Browser),
    ("chromium-browser", SourceApp::Browser),
    // Linux cuts process names to 15 characters.
    ("chromium-browse", SourceApp::Browser),
    ("brave", SourceApp::Browser),
    ("brave-browser", SourceApp::Browser),
    ("msedge", SourceApp::Browser),
    ("microsoft-edge", SourceApp::Browser),
    ("opera", SourceApp::Browser),
    ("vivaldi", SourceApp::Browser),
    ("vivaldi-bin", SourceApp::Browser),
    ("epiphany", SourceApp::Browser),
];

/// Maps a process or binary name to a known app. Case does not matter and `.exe` is ignored.
pub fn from_process_name(name: &str) -> Option<SourceApp> {
    let name = name.trim().to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    NAMES
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, app)| *app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_1_1_matches_known_apps_by_name() {
        let cases = [
            ("zoom", Some(SourceApp::Zoom)),
            ("zoom.real", Some(SourceApp::Zoom)),
            ("Zoom.exe", Some(SourceApp::Zoom)),
            ("slack", Some(SourceApp::Slack)),
            ("Slack.exe", Some(SourceApp::Slack)),
            ("teams-for-linux", Some(SourceApp::Teams)),
            ("ms-teams.exe", Some(SourceApp::Teams)),
            ("Discord", Some(SourceApp::Discord)),
            ("DiscordCanary", Some(SourceApp::Discord)),
            ("firefox", Some(SourceApp::Browser)),
            ("chrome", Some(SourceApp::Browser)),
            ("chromium-browse", Some(SourceApp::Browser)),
            ("msedge.exe", Some(SourceApp::Browser)),
            ("vivaldi-bin", Some(SourceApp::Browser)),
            ("meeting-assistant", None),
            ("gsd-media-keys", None),
            ("zoomer", None),
            ("", None),
        ];
        for (name, expected) in cases {
            assert_eq!(from_process_name(name), expected, "{name}");
        }
    }

    #[test]
    fn serialises_as_the_source_app_value() {
        for app in SourceApp::ALL {
            let json = serde_json::to_value(app).unwrap();
            assert_eq!(json, serde_json::Value::String(app.as_str().into()));
            assert_eq!(serde_json::from_value::<SourceApp>(json).unwrap(), app);
            assert_eq!(SourceApp::parse(app.as_str()), Some(app));
        }
        assert_eq!(SourceApp::parse("other"), None);
        assert_eq!(SourceApp::Zoom.meeting_title(), "Zoom meeting");
    }
}
