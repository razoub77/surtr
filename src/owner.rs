//! Discord ownership claim file: `%LOCALAPPDATA%\surtr\owner.json`.
//!
//! The file holds `{"pid": <n>}`: the Daemon whose mpv window was last focused.
//! Writes are last-write-wins and atomic (temp + rename), so a half-written
//! claim never reads as somebody else's. Nothing deletes it on exit; a missing
//! or unreadable file simply means "no claim".

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::daemon::OwnerClaim;

#[derive(Debug, Serialize, Deserialize)]
struct Claim {
    pid: u32,
}

/// The claim file on disk. No path (no `%LOCALAPPDATA%`) means claims are
/// written nowhere and read as absent, which leaves every Daemon a non-Owner.
pub struct OwnerFile {
    path: Option<PathBuf>,
}

impl OwnerFile {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }
}

impl OwnerClaim for OwnerFile {
    fn claim(&mut self, pid: u32) {
        let Some(path) = &self.path else {
            return;
        };
        let Some(parent) = path.parent() else {
            return;
        };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let Ok(bytes) = serde_json::to_vec(&Claim { pid }) else {
            return;
        };
        let temp = path.with_extension(format!("json.{pid}.tmp"));
        if std::fs::write(&temp, bytes).is_err() {
            return;
        }
        let _ = std::fs::rename(&temp, path);
    }

    fn owner(&self) -> Option<u32> {
        let text = std::fs::read_to_string(self.path.as_ref()?).ok()?;
        serde_json::from_str::<Claim>(&text)
            .ok()
            .map(|claim| claim.pid)
    }
}
