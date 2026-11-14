//! User and writable state location: `%LOCALAPPDATA%\surtr\`.

use std::path::PathBuf;

/// The one folder holding `cache.json`, `overrides.json` and `owner.json`.
pub fn app_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("surtr"))
}

pub fn cache_file() -> Option<PathBuf> {
    app_dir().map(|dir| dir.join("cache.json"))
}

pub fn overrides_file() -> Option<PathBuf> {
    app_dir().map(|dir| dir.join("overrides.json"))
}

pub fn owner_file() -> Option<PathBuf> {
    app_dir().map(|dir| dir.join("owner.json"))
}
