//! User and writable state location: `%LOCALAPPDATA%\surtr\`.

use std::path::PathBuf;

/// The one folder holding `cache.json`, `owner.json` and the legacy
/// `overrides.json` that migration reads.
pub fn app_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("surtr"))
}

pub fn cache_file() -> Option<PathBuf> {
    app_dir().map(|dir| dir.join("cache.json"))
}

/// The pre-migration override file. Overrides now live beside `surtr.conf`,
/// but this one is moved (or merged) into the configured path once at startup.
pub fn legacy_overrides_file() -> Option<PathBuf> {
    app_dir().map(|dir| dir.join("overrides.json"))
}

pub fn owner_file() -> Option<PathBuf> {
    app_dir().map(|dir| dir.join("owner.json"))
}
