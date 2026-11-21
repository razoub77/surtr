//! On-disk cache: fresh hits skip the source, expired entries are discarded
//! (never a stale fallback), and the file is versioned with an atomic write.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;

use surtr::anilist::Candidate;
use surtr::cache::{Cache, Cached, TTL_MS, VERSION};
use surtr::clock::Clock;
use surtr::daemon::Level;
use surtr::matcher::{MediaSource, SourceError};

mod support;

use support::candidate;

const T0: i64 = 1_700_000_000_000;

fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "surtr-cache-{}-{}-{label}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[derive(Clone)]
struct FixedClock(Rc<Cell<i64>>);

impl FixedClock {
    fn at(millis: i64) -> Self {
        Self(Rc::new(Cell::new(millis)))
    }
    fn set(&self, millis: i64) {
        self.0.set(millis);
    }
}

impl Clock for FixedClock {
    fn now_millis(&self) -> i64 {
        self.0.get()
    }
}

#[derive(Default)]
struct Calls {
    searches: Cell<usize>,
    sequels: Cell<usize>,
    entries: Cell<usize>,
}

/// A source whose search answers are scripted, recording every call.
struct Source {
    calls: Rc<Calls>,
    searches: RefCell<VecDeque<Result<Vec<Candidate>, SourceError>>>,
    sequel_response: Result<Vec<Candidate>, SourceError>,
    entry_response: Result<Candidate, SourceError>,
}

impl Source {
    fn new(
        searches: impl IntoIterator<Item = Result<Vec<Candidate>, SourceError>>,
    ) -> (Self, Rc<Calls>) {
        let calls = Rc::new(Calls::default());
        (
            Self {
                calls: calls.clone(),
                searches: RefCell::new(searches.into_iter().collect()),
                sequel_response: Ok(Vec::new()),
                entry_response: Err(SourceError::Unavailable),
            },
            calls,
        )
    }
}

impl MediaSource for Source {
    fn search(&mut self, _query: &str) -> Result<Vec<Candidate>, SourceError> {
        self.calls.searches.set(self.calls.searches.get() + 1);
        self.searches
            .borrow_mut()
            .pop_front()
            .unwrap_or(Err(SourceError::Unavailable))
    }

    fn sequels(&mut self, _id: i64) -> Result<Vec<Candidate>, SourceError> {
        self.calls.sequels.set(self.calls.sequels.get() + 1);
        self.sequel_response.clone()
    }

    fn entry(&mut self, _id: i64) -> Result<Candidate, SourceError> {
        self.calls.entries.set(self.calls.entries.get() + 1);
        self.entry_response.clone()
    }
}

#[test]
fn a_fresh_hit_skips_the_source_after_the_first_search() {
    let path = temp_path("fresh");
    let (source, calls) = Source::new([Ok(vec![candidate(19, "MONSTER")])]);
    let mut cached = Cached::new(source, Cache::load(path), FixedClock::at(T0));

    assert_eq!(cached.search("monster"), Ok(vec![candidate(19, "MONSTER")]));
    assert_eq!(calls.searches.get(), 1);

    // Same query in different case is the same normalized cache key.
    assert_eq!(cached.search("Monster"), Ok(vec![candidate(19, "MONSTER")]));
    assert_eq!(calls.searches.get(), 1, "a fresh hit makes no source call");
}

#[test]
fn an_expired_entry_is_never_a_stale_fallback() {
    let path = temp_path("expired");
    let clock = FixedClock::at(T0);
    let (source, calls) = Source::new([
        Ok(vec![candidate(19, "MONSTER")]),
        // The refetch fails: no answer, so no presence — not the old value.
        Err(SourceError::Unavailable),
    ]);
    let mut cached = Cached::new(source, Cache::load(path), clock.clone());

    assert!(cached.search("monster").is_ok());
    clock.set(T0 + TTL_MS + 1);
    assert_eq!(cached.search("monster"), Err(SourceError::Unavailable));
    assert_eq!(calls.searches.get(), 2, "the expired entry was refetched");
}

#[test]
fn a_fresh_cache_file_survives_a_restart() {
    let path = temp_path("restart");
    {
        let (source, calls) = Source::new([Ok(vec![candidate(19, "MONSTER")])]);
        let mut cached = Cached::new(source, Cache::load(path.clone()), FixedClock::at(T0));
        assert!(cached.search("monster").is_ok());
        assert_eq!(calls.searches.get(), 1);
    }

    // A new Daemon process: a cache hit means the source is never asked.
    let (source, calls) = Source::new([]);
    let mut cached = Cached::new(source, Cache::load(path), FixedClock::at(T0 + 1_000));
    assert_eq!(cached.search("monster"), Ok(vec![candidate(19, "MONSTER")]));
    assert_eq!(calls.searches.get(), 0);
}

#[test]
fn the_cache_file_is_versioned_and_written_atomically() {
    let path = temp_path("atomic");
    let (source, _) = Source::new([Ok(vec![candidate(19, "MONSTER")])]);
    let mut cached = Cached::new(source, Cache::load(path.clone()), FixedClock::at(T0));
    let _ = cached.search("monster");

    let text = std::fs::read_to_string(&path).expect("cache written");
    let document: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    assert_eq!(document["version"], VERSION);
    assert!(document["searches"]["monster"]["value"].is_array());

    let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    assert!(!temp.exists(), "the temp file is renamed, not left behind");
}

#[test]
fn a_cache_from_another_version_is_ignored() {
    let path = temp_path("version");
    std::fs::write(&path, r#"{"version":99,"searches":{}}"#).unwrap();
    let (source, calls) = Source::new([Ok(vec![candidate(19, "MONSTER")])]);
    let mut cached = Cached::new(source, Cache::load(path), FixedClock::at(T0));

    assert!(cached.search("monster").is_ok());
    assert_eq!(calls.searches.get(), 1, "an old version starts empty");
}

#[test]
fn a_corrupt_cache_file_starts_empty() {
    let path = temp_path("corrupt");
    std::fs::write(&path, "not json").unwrap();
    let (source, calls) = Source::new([Ok(vec![candidate(19, "MONSTER")])]);
    let mut cached = Cached::new(source, Cache::load(path), FixedClock::at(T0));

    assert!(cached.search("monster").is_ok());
    assert_eq!(calls.searches.get(), 1);
}

#[test]
fn relations_and_entries_are_cached_by_id() {
    let path = temp_path("ids");
    let (mut source, calls) = Source::new([]);
    source.sequel_response = Ok(vec![candidate(2, "Show 2nd Season")]);
    source.entry_response = Ok(candidate(108268, "Honzuki no Gekokujou"));
    let mut cached = Cached::new(source, Cache::load(path), FixedClock::at(T0));

    assert!(cached.sequels(1).is_ok());
    assert!(cached.sequels(1).is_ok());
    assert_eq!(calls.sequels.get(), 1, "relations are cached by id");

    assert_eq!(cached.entry(108268).unwrap().id, 108268);
    assert_eq!(cached.entry(108268).unwrap().id, 108268);
    assert_eq!(calls.entries.get(), 1, "entries are cached by id");
    // Each request names itself as a hit or a live fetch, at Verbose level so
    // mpv hides it without --msg-level=all=v.
    let trace: Vec<(Level, String)> = cached
        .take_trace()
        .into_iter()
        .map(|log| (log.level, log.text))
        .collect();
    assert_eq!(
        trace,
        vec![
            (
                Level::Verbose,
                "cache: relations id=1: live fetch".to_owned()
            ),
            (
                Level::Verbose,
                "cache: relations id=1: hit".to_owned()
            ),
            (
                Level::Verbose,
                "cache: entry id=108268: live fetch".to_owned()
            ),
            (
                Level::Verbose,
                "cache: entry id=108268: hit".to_owned()
            ),
        ]
    );
}

#[test]
fn id_keyed_entries_survive_a_restart() {
    let path = temp_path("ids-restart");
    {
        let (mut source, _) = Source::new([]);
        source.sequel_response = Ok(vec![candidate(2, "Show 2nd Season")]);
        source.entry_response = Ok(candidate(108268, "Honzuki no Gekokujou"));
        let mut cached = Cached::new(source, Cache::load(path.clone()), FixedClock::at(T0));
        assert!(cached.sequels(1).is_ok());
        assert!(cached.entry(108268).is_ok());
    }

    let (mut source, calls) = Source::new([]);
    source.entry_response = Err(SourceError::Unavailable);
    source.sequel_response = Err(SourceError::Unavailable);
    let mut cached = Cached::new(source, Cache::load(path), FixedClock::at(T0 + 1_000));
    assert_eq!(cached.sequels(1).unwrap()[0].id, 2);
    assert_eq!(cached.entry(108268).unwrap().id, 108268);
    assert_eq!(calls.sequels.get(), 0, "relations came from disk");
    assert_eq!(calls.entries.get(), 0, "entries came from disk");
}
