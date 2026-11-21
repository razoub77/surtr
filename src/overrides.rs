//! Manual overrides: the cleaned query string the matcher searched, mapped to
//! the AniList id the user asserts. Loaded once at Daemon startup (editing the
//! file means restarting mpv). The one exception to "never created here" is the
//! migration of `%LOCALAPPDATA%\surtr\overrides.json` into the configured file:
//! it runs once, keeps the old data when validation or writing fails, and never
//! overwrites a target whose contents differ without the target winning.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use serde_json::{Map, Value};

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
                    "cannot read overrides at {}: {err}",
                    path.display()
                )],
            ),
        }
    }

    /// Loads the configured file, first moving or merging the legacy
    /// `%LOCALAPPDATA%\surtr\overrides.json` into it. The target wins every
    /// conflict; the legacy file is deleted only after a successful write.
    /// Every failure keeps the legacy data and is reported in `messages`.
    pub fn load_with_migration(target: Option<&Path>, legacy: Option<&Path>) -> (Self, Vec<String>) {
        let mut messages = Vec::new();
        // No target means the launcher rejected `overrides_path`: Overrides
        // are disabled for this run, even if the old appdata file exists.
        let Some(target) = target else {
            return (Self::none(), messages);
        };
        // A custom overrides_path can point at the legacy file itself. There
        // is nothing to move, and a merge would delete the only copy.
        if legacy.is_some_and(|legacy| same_path(target, legacy)) {
            return Self::load(target);
        }

        let target_text = match read_optional(target) {
            Ok(text) => text,
            Err(err) => {
                messages.push(format!(
                    "cannot read overrides at {}: {err}",
                    target.display()
                ));
                return (Self::none(), messages);
            }
        };
        let legacy_text = match legacy {
            Some(path) => match read_optional(path) {
                Ok(text) => text,
                Err(err) => {
                    messages.push(format!(
                        "cannot read overrides at {}: {err}",
                        path.display()
                    ));
                    None
                }
            },
            None => None,
        };

        match (target_text, legacy_text) {
            (None, None) => (Self::none(), messages),
            (Some(text), None) => {
                let overrides = parsed(&mut messages, &text, &target.display().to_string());
                (overrides, messages)
            }
            (None, Some(text)) => move_legacy(&text, target, legacy.unwrap(), messages),
            (Some(target_text), Some(legacy_text)) => merge_legacy(
                &target_text,
                &legacy_text,
                target,
                legacy.unwrap(),
                messages,
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
                        "ignoring overrides at {label}: invalid JSON: {err}"
                    )],
                )
            }
        };
        let Some(object) = value.as_object() else {
            return (
                Self::none(),
                vec![format!(
                    "ignoring overrides at {label}: top level must be an object"
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
                    "ignoring overrides entry {key:?}: value must be a positive integer AniList id"
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

/// Windows path equality for the one case that matters here: the user pointed
/// `overrides_path` at the legacy file with different casing or separators.
fn same_path(left: &Path, right: &Path) -> bool {
    let key = |path: &Path| path.to_string_lossy().replace('/', "\\").to_lowercase();
    key(left) == key(right)
}

/// Reads a file, distinguishing "absent" from "unreadable".
fn read_optional(path: &Path) -> io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Parses a file that must be wholly valid before the legacy file is deleted:
/// JSON object, positive integer ids, no skipped entries. The existing loader
/// stays tolerant; migration is the one place where "partly valid" is not safe.
/// One reason is enough: the console gets one line per failed migration.
fn valid_object(text: &str, label: &str) -> Result<Map<String, Value>, String> {
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(err) => return Err(format!("overrides at {label}: invalid JSON: {err}")),
    };
    let Some(object) = value.as_object() else {
        return Err(format!(
            "overrides at {label}: top level must be an object"
        ));
    };
    let invalid = object
        .values()
        .filter(|value| value.as_i64().filter(|id| *id > 0).is_none())
        .count();
    if invalid == 0 {
        Ok(object.clone())
    } else {
        Err(format!(
            "overrides at {label}: {invalid} value(s) must be positive integer AniList ids"
        ))
    }
}

/// Loads a file through the tolerant parser and appends its console lines.
fn parsed(messages: &mut Vec<String>, text: &str, label: &str) -> Overrides {
    let (overrides, mut parsed) = Overrides::parse_at(text, label);
    messages.append(&mut parsed);
    overrides
}

/// Copies a validated legacy file to the target, then removes the source.
/// A failed write leaves the source in charge for this run.
fn move_legacy(
    text: &str,
    target: &Path,
    legacy: &Path,
    mut messages: Vec<String>,
) -> (Overrides, Vec<String>) {
    if let Err(reason) = valid_object(text, &legacy.display().to_string()) {
        messages.push(format!("{reason}; legacy file left unchanged"));
        return (Overrides::none(), messages);
    }

    match write_atomic(target, text) {
        Ok(()) => {
            let removal = match remove_legacy(legacy) {
                Ok(()) => String::new(),
                Err(err) => format!("; could not remove legacy file: {err}"),
            };
            messages.push(format!(
                "moved overrides from {} to {}{removal}",
                legacy.display(),
                target.display()
            ));
            let overrides = parsed(&mut messages, text, &target.display().to_string());
            (overrides, messages)
        }
        Err(err) => {
            messages.push(format!(
                "cannot move overrides from {} to {}: {err}; reading the legacy file",
                legacy.display(),
                target.display()
            ));
            let overrides = parsed(&mut messages, text, &legacy.display().to_string());
            (overrides, messages)
        }
    }
}

/// Merges a validated legacy file into a validated target. Normalized keys are
/// compared; the target wins a conflicting id, and only missing keys are added.
fn merge_legacy(
    target_text: &str,
    legacy_text: &str,
    target: &Path,
    legacy: &Path,
    mut messages: Vec<String>,
) -> (Overrides, Vec<String>) {
    let target_label = target.display().to_string();
    let legacy_label = legacy.display().to_string();
    let legacy_result = valid_object(legacy_text, &legacy_label);
    let target_result = valid_object(target_text, &target_label);

    let (legacy_object, target_object) = match (legacy_result, target_result) {
        (Ok(legacy), Ok(target)) => (legacy, target),
        (Err(reason), Ok(_)) => {
            messages.push(format!("{reason}; files left unchanged"));
            let overrides = parsed(&mut messages, target_text, &target_label);
            return (overrides, messages);
        }
        (Ok(_), Err(reason)) => {
            messages.push(format!("{reason}; files left unchanged"));
            return (Overrides::none(), messages);
        }
        (Err(legacy_reason), Err(target_reason)) => {
            messages.push(format!(
                "cannot merge overrides: {target_reason}; {legacy_reason}; files left unchanged"
            ));
            return (Overrides::none(), messages);
        }
    };

    let (merged, mut conflicts) =
        merge_objects(&target_object, &legacy_object, &target_label, &legacy_label);
    messages.append(&mut conflicts);
    let text = serde_json::to_string_pretty(&Value::Object(merged)).unwrap_or_default() + "\n";
    if let Err(err) = write_atomic(target, &text) {
        messages.push(format!(
            "cannot write merged overrides to {}: {err}; leaving the legacy file",
            target.display()
        ));
        let overrides = parsed(&mut messages, target_text, &target_label);
        return (overrides, messages);
    }
    if let Err(err) = remove_legacy(legacy) {
        messages.push(format!(
            "merged overrides but could not remove legacy file {}: {err}",
            legacy.display()
        ));
    }
    let overrides = parsed(&mut messages, &text, &target_label);
    (overrides, messages)
}

/// Adds every legacy key the target does not already have. Normalized-key
/// duplicates within the legacy file resolve last-wins, matching the loader.
fn merge_objects(
    target: &Map<String, Value>,
    legacy: &Map<String, Value>,
    target_label: &str,
    legacy_label: &str,
) -> (Map<String, Value>, Vec<String>) {
    let mut legacy_by_key: HashMap<String, (String, Value)> = HashMap::new();
    for (key, value) in legacy {
        legacy_by_key.insert(scorer::normalize(key), (key.clone(), value.clone()));
    }

    let mut merged = target.clone();
    let mut keys: HashMap<String, String> = target
        .keys()
        .map(|key| (scorer::normalize(key), key.clone()))
        .collect();
    let mut messages = Vec::new();
    for (normalized, (key, value)) in legacy_by_key {
        if let Some(existing) = keys.get(&normalized) {
            let current = &merged[existing];
            if current != &value {
                messages.push(format!(
                    "overrides key {key:?} kept id {current} from {target_label}; {legacy_label} had {value}"
                ));
            }
        } else {
            merged.insert(key.clone(), value);
            keys.insert(normalized, key);
        }
    }
    (merged, messages)
}

fn remove_legacy(legacy: &Path) -> io::Result<()> {
    match std::fs::remove_file(legacy) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Writes a sibling temp file, then renames it into place. Parent folders are
/// never created: a custom path's typo must surface instead of scattering data.
fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent folder"))?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let temp = parent.join(format!(".{}.{}.tmp", name.to_string_lossy(), std::process::id()));
    let result = std::fs::write(&temp, text).and_then(|()| std::fs::rename(&temp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
