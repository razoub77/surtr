//! Surtr daemon library: the Daemon Core State Machine and its IPC adapters.

pub mod anilist;
pub mod cache;
pub mod clock;
pub mod daemon;
pub mod discord;
pub mod host;
pub mod matcher;
pub mod mpv;
pub mod overrides;
pub mod owner;
pub mod paths;
pub mod release;
pub mod scorer;

/// The Daemon's command line, as the Lua launcher builds it. Lives here so the
/// configuration parsing — notably `anime_roots` — is covered by tests.
pub mod cli {
    /// The Anime Roots from a semicolon-separated list (mpv's `script-opts`
    /// value arrives verbatim): whitespace around an entry is trimmed and
    /// empty segments are ignored, so `"D:\Anime; E:\Anime2 ;"` is two roots.
    /// An empty list means "no filter" to the core.
    pub fn anime_roots(value: Option<&str>) -> Vec<String> {
        value
            .unwrap_or_default()
            .split(';')
            .map(str::trim)
            .filter(|root| !root.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// The Discord application id from `script-opts/surtr.conf` or
    /// `--client-id` (both arrive verbatim): whitespace trimmed, and `None`
    /// when absent — the publisher then falls back to `SURTR_CLIENT_ID`.
    pub fn client_id(value: Option<&str>) -> Option<String> {
        value
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
    }

    /// The title language preference from `script-opts/surtr.conf` or
    /// `--details-title-lang` (both arrive verbatim): matched
    /// case-insensitively, and anything unrecognized — including a blank conf
    /// line or a missing flag — keeps the built-in default, English. A title
    /// preference is too minor to refuse startup over.
    pub fn title_language(value: Option<&str>) -> crate::matcher::TitleLanguage {
        match value.map(str::trim).map(str::to_ascii_lowercase) {
            Some(lowercased) if lowercased == "romaji" => crate::matcher::TitleLanguage::Romaji,
            _ => crate::matcher::TitleLanguage::English,
        }
    }

    /// The pause timeout from `script-opts/surtr.conf` or `--pause-timeout`,
    /// in milliseconds. Whole seconds are expected; blank or missing means 0
    /// (no timeout). Negative, unparsable and out-of-range values are errors
    /// so the caller can report them and keep running with the timeout off.
    pub fn pause_timeout_ms(value: Option<&str>) -> Result<crate::clock::Millis, ()> {
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            None => Ok(0),
            Some(raw) => raw
                .parse::<u64>()
                .ok()
                .and_then(|seconds| seconds.checked_mul(1000))
                .and_then(|millis| i64::try_from(millis).ok())
                .ok_or(()),
        }
    }
}
