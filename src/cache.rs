//! Versioned on-disk cache of AniList answers, and the [`Cached`] wrapper that
//! puts it in front of a [`MediaSource`].
//!
//! One JSON file at `%LOCALAPPDATA%\surtr\cache.json` holds search results by
//! normalized query and entry/relation data by AniList id. Entries live for 30
//! days; an expired entry is refetched, never used as a stale fallback. Writes
//! go to a temp file and are renamed into place so a crash cannot leave a
//! half-written cache.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::anilist::Candidate;
use crate::clock::{Clock, Millis};
use crate::daemon::{Diagnostic, Level};
use crate::matcher::{MediaSource, SourceError};
use crate::scorer;

/// The cache document format. A different version starts from empty. Version
/// 2 added `status`, `season` and `genres` to every Candidate; version 1
/// entries would render an empty hover line, so they are discarded rather
/// than guessed at.
pub const VERSION: u32 = 2;

/// 30 days in milliseconds.
pub const TTL_MS: Millis = 30 * 24 * 60 * 60 * 1_000;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Document {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    searches: BTreeMap<String, Record<Vec<Candidate>>>,
    #[serde(default)]
    entries: BTreeMap<i64, Record<Candidate>>,
    #[serde(default)]
    sequels: BTreeMap<i64, Record<Vec<Candidate>>>,
}

impl Document {
    fn prune(&mut self, now: Millis) {
        self.searches.retain(|_, record| record.fresh(now));
        self.entries.retain(|_, record| record.fresh(now));
        self.sequels.retain(|_, record| record.fresh(now));
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Record<T> {
    /// When the answer was fetched, epoch ms.
    fetched_ms: Millis,
    value: T,
}

impl<T> Record<T> {
    fn fresh(&self, now: Millis) -> bool {
        // A clock that moved backwards is not trusted either.
        now.checked_sub(self.fetched_ms)
            .is_some_and(|age| age < TTL_MS)
    }
}

/// The one cache file's contents, plus where to persist them.
#[derive(Debug, Default)]
pub struct Cache {
    path: Option<PathBuf>,
    document: Document,
}

impl Cache {
    /// Loads the file. Missing, unreadable, corrupt or other-version contents
    /// start an empty cache rather than failing the Daemon.
    pub fn load(path: PathBuf) -> Self {
        let document = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Document>(&text).ok())
            .filter(|document| document.version == VERSION)
            .unwrap_or_default();
        Self {
            path: Some(path),
            document,
        }
    }

    /// No file: every read misses and every write is dropped.
    pub fn disabled() -> Self {
        Self::default()
    }

    pub fn search(&self, query: &str, now: Millis) -> Option<Vec<Candidate>> {
        lookup(&self.document.searches, &scorer::normalize(query), now)
    }

    pub fn store_search(&mut self, query: &str, value: Vec<Candidate>, now: Millis) {
        remember(
            &mut self.document.searches,
            scorer::normalize(query),
            value,
            now,
        );
        self.save(now);
    }

    pub fn entry(&self, id: i64, now: Millis) -> Option<Candidate> {
        lookup(&self.document.entries, &id, now)
    }

    pub fn store_entry(&mut self, id: i64, value: Candidate, now: Millis) {
        remember(&mut self.document.entries, id, value, now);
        self.save(now);
    }

    pub fn sequels(&self, id: i64, now: Millis) -> Option<Vec<Candidate>> {
        lookup(&self.document.sequels, &id, now)
    }

    pub fn store_sequels(&mut self, id: i64, value: Vec<Candidate>, now: Millis) {
        remember(&mut self.document.sequels, id, value, now);
        self.save(now);
    }

    /// Write temp + rename, creating the folder first. The temp name carries
    /// the process id so two mpv windows' Daemons never rename the same file.
    /// A cache failure is silent: it must never block playback or the Daemon.
    fn save(&mut self, now: Millis) {
        let Some(path) = &self.path else {
            return;
        };
        self.document.version = VERSION;
        self.document.prune(now);
        let Ok(bytes) = serde_json::to_vec(&self.document) else {
            return;
        };
        let Some(parent) = path.parent() else {
            return;
        };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        if std::fs::write(&temp, bytes).is_err() {
            return;
        }
        let _ = std::fs::rename(&temp, path);
    }
}

/// A fresh value from one cache map; an expired or absent record misses.
fn lookup<T: Clone, K: Ord>(map: &BTreeMap<K, Record<T>>, key: &K, now: Millis) -> Option<T> {
    map.get(key)
        .filter(|record| record.fresh(now))
        .map(|record| record.value.clone())
}

/// Stores one answer under its key.
fn remember<T, K: Ord>(map: &mut BTreeMap<K, Record<T>>, key: K, value: T, now: Millis) {
    map.insert(
        key,
        Record {
            fetched_ms: now,
            value,
        },
    );
}

/// A [`MediaSource`] that answers fresh cache hits itself and fills the cache
/// only with real answers: an `Err` (a failed request) is never persisted.
pub struct Cached<S, C> {
    inner: S,
    cache: Cache,
    clock: C,
    /// One Verbose Diagnostic per request: disk hit or live fetch. The Matcher
    /// drains this through the MediaSource seam and merges it into its trace.
    trace: Vec<Diagnostic>,
}

impl<S: MediaSource, C: Clock> Cached<S, C> {
    pub fn new(inner: S, cache: Cache, clock: C) -> Self {
        Self {
            inner,
            cache,
            clock,
            trace: Vec::new(),
        }
    }

    fn trace_verbose(&mut self, text: impl Into<String>) {
        self.trace.push(Diagnostic::new(Level::Verbose, text));
    }
}

impl<S: MediaSource, C: Clock> MediaSource for Cached<S, C> {
    fn search(&mut self, query: &str) -> Result<Vec<Candidate>, SourceError> {
        let now = self.clock.now_millis();
        if let Some(cached) = self.cache.search(query, now) {
            self.trace_verbose(format!("surtr: cache: search \"{query}\": hit"));
            return Ok(cached);
        }
        self.trace_verbose(format!("surtr: cache: search \"{query}\": live fetch"));
        let result = self.inner.search(query)?;
        self.cache.store_search(query, result.clone(), now);
        Ok(result)
    }

    fn sequels(&mut self, id: i64) -> Result<Vec<Candidate>, SourceError> {
        let now = self.clock.now_millis();
        if let Some(cached) = self.cache.sequels(id, now) {
            self.trace_verbose(format!("surtr: cache: relations id={id}: hit"));
            return Ok(cached);
        }
        self.trace_verbose(format!("surtr: cache: relations id={id}: live fetch"));
        let result = self.inner.sequels(id)?;
        self.cache.store_sequels(id, result.clone(), now);
        Ok(result)
    }

    fn entry(&mut self, id: i64) -> Result<Candidate, SourceError> {
        let now = self.clock.now_millis();
        if let Some(cached) = self.cache.entry(id, now) {
            self.trace_verbose(format!("surtr: cache: entry id={id}: hit"));
            return Ok(cached);
        }
        self.trace_verbose(format!("surtr: cache: entry id={id}: live fetch"));
        let result = self.inner.entry(id)?;
        self.cache.store_entry(id, result.clone(), now);
        Ok(result)
    }

    fn take_trace(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.trace)
    }
}
