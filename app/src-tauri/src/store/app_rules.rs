//! `app_rules` table: what to do when a meeting app is detected (FR-1.7).
//! An app with no row is `ask`.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{now_ms, StoreError};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppRule {
    /// Show the consent prompt (FR-1.3).
    #[default]
    Ask,
    /// Start recording on detection; the rule is the user's action (rule 1).
    Always,
    /// Ignore detections from this app.
    Never,
}

impl AppRule {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Always => "always",
            Self::Never => "never",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "ask" => Some(Self::Ask),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }
}

pub struct AppRulesRepo<'a> {
    conn: &'a Connection,
}

impl<'a> AppRulesRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// The rule for `source_app`; `ask` when none is stored or the stored value is unknown.
    pub fn get(&self, source_app: &str) -> Result<AppRule, StoreError> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT rule FROM app_rules WHERE source_app = ?1",
                [source_app],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value
            .as_deref()
            .and_then(AppRule::parse)
            .unwrap_or_default())
    }

    pub fn set(&self, source_app: &str, rule: AppRule) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO app_rules (source_app, rule, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(source_app) DO UPDATE SET rule = excluded.rule, updated_at = excluded.updated_at",
            params![source_app, rule.as_str(), now_ms()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn fr_1_7_unset_app_is_ask_and_rules_overwrite() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn().unwrap();
        let repo = AppRulesRepo::new(&conn);

        assert_eq!(repo.get("zoom").unwrap(), AppRule::Ask);
        repo.set("zoom", AppRule::Never).unwrap();
        assert_eq!(repo.get("zoom").unwrap(), AppRule::Never);
        repo.set("zoom", AppRule::Always).unwrap();
        assert_eq!(repo.get("zoom").unwrap(), AppRule::Always);
        assert_eq!(repo.get("slack").unwrap(), AppRule::Ask);

        let rows: i64 = conn
            .query_row("SELECT count(*) FROM app_rules", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn unknown_stored_rule_reads_as_ask() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn().unwrap();
        conn.execute(
            "INSERT INTO app_rules (source_app, rule, updated_at) VALUES ('zoom', 'sometimes', 0)",
            [],
        )
        .unwrap();
        assert_eq!(AppRulesRepo::new(&conn).get("zoom").unwrap(), AppRule::Ask);
    }

    #[test]
    fn rules_use_lowercase_on_the_wire() {
        for rule in [AppRule::Ask, AppRule::Always, AppRule::Never] {
            let json = serde_json::to_value(rule).unwrap();
            assert_eq!(json, serde_json::Value::String(rule.as_str().into()));
            assert_eq!(serde_json::from_value::<AppRule>(json).unwrap(), rule);
        }
    }
}
