//! Manual `overrides.json`: the cleaned query string the matcher searched,
//! mapped to the AniList id the user asserts. Loaded once at Daemon startup
//! (editing the file means restarting mpv), never created or rewritten here.

use std::collections::HashMap;
use std::path::Path;

use crate::scorer;

/// The user's asserted query → AniList id mapping.
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    ids: HashMap<String, i64>,
}

impl Overrides {
    /// No overrides: matching runs normally.
    pub fn none() -> Self {
        Self::default()
    }

    /// Reads the file once. A missing file is silent; an unreadable or
    /// malformed file yields no overrides plus one console line.
    pub fn load(path: &Path) -> (Self, Vec<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse_at(&text, &path.display().to_string()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => (Self::none(), Vec::new()),
            Err(err) => (
                Self::none(),
                vec![format!(
                    "surtr: cannot read overrides at {}: {err}",
                    path.display()
                )],
            ),
        }
    }

    /// Parses the file body. Invalid JSON or a non-object top level ignores
    /// the whole file; a malformed entry is skipped, the rest still apply.
    pub fn parse(text: &str) -> (Self, Vec<String>) {
        Self::parse_at(text, "overrides.json")
    }

    fn parse_at(text: &str, label: &str) -> (Self, Vec<String>) {
        let value: serde_json::Value = match serde_json::from_str(text) {
            Ok(value) => value,
            Err(err) => {
                return (
                    Self::none(),
                    vec![format!(
                        "surtr: ignoring overrides at {label}: invalid JSON: {err}"
                    )],
                )
            }
        };
        let Some(object) = value.as_object() else {
            return (
                Self::none(),
                vec![format!(
                    "surtr: ignoring overrides at {label}: top level must be an object"
                )],
            );
        };
        let mut ids = HashMap::new();
        let mut messages = Vec::new();
        for (key, value) in object {
            match value.as_i64().filter(|id| *id > 0) {
                Some(id) => {
                    ids.insert(scorer::normalize(key), id);
                }
                None => messages.push(format!(
                    "surtr: ignoring overrides entry {key:?}: value must be a positive integer AniList id"
                )),
            }
        }
        (Self { ids }, messages)
    }

    /// The id the user asserted for a cleaned query string, if any. Keys are
    /// case-folded and whitespace-collapsed, so the user need not reproduce
    /// the exact casing or spacing of the miss line.
    pub fn id(&self, query: &str) -> Option<i64> {
        self.ids.get(&scorer::normalize(query)).copied()
    }
}
