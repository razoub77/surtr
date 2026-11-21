//! Verbose Lookup Answer detail: the deep view behind `--msg-level=all=v`.
//! These lines are `Level::Verbose` Diagnostics drained from the Matcher (and
//! its MediaSource) by the Core; they never become OSD toasts.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use surtr::anilist::{Candidate, Titles};
use surtr::cache::{Cache, Cached};
use surtr::clock::Clock;
use surtr::daemon::{Level, Policy, Runner};
use surtr::matcher::{Matcher, MediaSource, SourceError};
use surtr::scorer;

mod support;

use support::{Host, SharedClaim, PID, T0};

/// A unique cache file per test; a fresh hit must mean this run's earlier
/// lookup, never a previous test's leftovers.
fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "surtr-verbose-{}-{}-{label}.json",
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
}

impl Clock for FixedClock {
    fn now_millis(&self) -> i64 {
        self.0.get()
    }
}

fn with_format(mut candidate: Candidate, format: &str) -> Candidate {
    candidate.format = Some(format.into());
    candidate
}

fn show(id: i64, title: &str, episodes: Option<u32>) -> Candidate {
    Candidate {
        id,
        format: Some("TV".into()),
        season_year: None,
        season: None,
        status: None,
        episodes,
        genres: Vec::new(),
        title: Some(Titles {
            romaji: Some(title.into()),
            english: None,
        }),
        synonyms: Vec::new(),
        cover_image: None,
    }
}

/// An in-memory MediaSource: searches and SEQUEL chains scripted by the test.
struct Source {
    searches: HashMap<String, Vec<Candidate>>,
    sequels: HashMap<i64, Vec<Candidate>>,
}

impl Source {
    fn new(searches: Vec<(&str, Vec<Candidate>)>, sequels: Vec<(i64, Vec<Candidate>)>) -> Self {
        Self {
            searches: searches
                .into_iter()
                .map(|(query, candidates)| (scorer::normalize(query), candidates))
                .collect(),
            sequels: sequels.into_iter().collect(),
        }
    }
}

impl MediaSource for Source {
    fn search(&mut self, query: &str) -> Result<Vec<Candidate>, SourceError> {
        Ok(self
            .searches
            .get(&scorer::normalize(query))
            .cloned()
            .unwrap_or_default())
    }

    fn sequels(&mut self, id: i64) -> Result<Vec<Candidate>, SourceError> {
        Ok(self.sequels.get(&id).cloned().unwrap_or_default())
    }

    fn entry(&mut self, _id: i64) -> Result<Candidate, SourceError> {
        Err(SourceError::Unavailable)
    }
}

fn harness<'m, S: MediaSource + 'm>(
    matcher: &'m mut Matcher<S>,
) -> (Host, Runner<'m, SharedClaim>) {
    let host = Host::new(true);
    let runner = Runner::new(matcher, host.claim(), PID, T0, Policy::default(), Vec::new());
    (host, runner)
}

/// The cache sits beneath the matcher: each request names itself as a disk hit
/// or a live fetch, and the match itself says nothing at the default level.
#[tokio::test]
async fn a_cache_hit_is_named_in_the_verbose_trace() {
    let source = Source::new(vec![("monster", vec![show(19, "Monster", Some(74))])], vec![]);
    let mut cached = Cached::new(source, Cache::load(temp_path("hit")), FixedClock::at(T0));
    let mut matcher = Matcher::new(&mut cached);
    let (mut host, mut runner) = harness(&mut matcher);

    host.load(&mut runner, r"D:\Anime\Monster\[ASW] Monster - 01.mkv")
        .await;
    host.load(&mut runner, r"D:\Anime\Monster\[ASW] Monster - 02.mkv")
        .await;

    assert_eq!(
        host.logs_at(Level::Verbose),
        vec![
            r#"cache: search "monster": live fetch"#.to_owned(),
            r#"top Candidates for "monster": [id=19 title="Monster" score=1.000]"#.to_owned(),
            r#"cache: search "monster": hit"#.to_owned(),
            r#"top Candidates for "monster": [id=19 title="Monster" score=1.000]"#.to_owned(),
        ]
    );
    assert!(host.osds().is_empty());
}

/// The deep view of one scored result set: the top three ids, titles and
/// scores, in score order, before the info-level decision line.
#[tokio::test]
async fn a_scored_result_set_lists_its_top_three_candidates_verbosely() {
    let mut source = Source::new(
        vec![(
            "show",
            vec![
                show(1, "Show", Some(12)),
                show(2, "Show Special", Some(12)),
                show(3, "Show Alpha", Some(12)),
                show(4, "Show Beta", Some(12)),
            ],
        )],
        vec![],
    );
    let mut matcher = Matcher::new(&mut source);
    let (mut host, mut runner) = harness(&mut matcher);

    host.load(&mut runner, r"D:\Anime\Show\[ASW] Show - 01.mkv")
        .await;

    assert_eq!(
        host.logs_at(Level::Verbose),
        vec![
            r#"top Candidates for "show": [id=1 title="Show" score=1.000, id=2 title="Show Special" score=0.870, id=3 title="Show Alpha" score=0.870]"#.to_owned(),
        ]
    );
    assert!(host.osds().is_empty());
}

/// A parsed release year names the Candidates it ruled out of the search.
#[tokio::test]
async fn a_parsed_release_year_names_the_candidates_it_excludes() {
    let old = Candidate {
        season_year: Some(2001),
        ..show(120, "Fruits Basket", Some(26))
    };
    let new = Candidate {
        season_year: Some(2019),
        ..show(105334, "Fruits Basket: 1st Season", Some(25))
    };
    let mut source = Source::new(vec![("fruits basket", vec![old, new])], vec![]);
    let mut matcher = Matcher::new(&mut source);
    let (mut host, mut runner) = harness(&mut matcher);

    host.load(
        &mut runner,
        r"D:\Anime\Fruits Basket (2019)\[ASW] Fruits Basket - 01.mkv",
    )
    .await;

    assert_eq!(
        host.logs_at(Level::Verbose),
        vec![
            r#"year 2019 excludes 1 Candidate: [id=120 title="Fruits Basket"]"#.to_owned(),
            r#"top Candidates for "fruits basket": [id=105334 title="Fruits Basket: 1st Season" score=0.900]"#.to_owned(),
        ]
    );
    assert!(host.osds().is_empty());
}

/// An explicit season marker earns a line saying whether the entry's titles
/// or synonyms carried that ordinal. The line is console-only.
#[tokio::test]
async fn season_corroboration_names_the_marker_check() {
    let mut source = Source::new(
        vec![(
            "show season 2",
            vec![show(2, "Show 2nd Season", Some(12))],
        )],
        vec![],
    );
    let mut matcher = Matcher::new(&mut source);
    let (mut host, mut runner) = harness(&mut matcher);

    host.load(&mut runner, r"D:\Anime\Show\Show S2 - 01.mkv")
        .await;

    assert_eq!(
        host.logs_at(Level::Verbose),
        vec![
            r#"top Candidates for "show season 2": [id=2 title="Show 2nd Season" score=1.000]"#
                .to_owned(),
            r#"season marker 2 in titles or synonyms of id=2: found"#.to_owned(),
        ]
    );
    assert!(host.osds().is_empty());
}

/// The Absolute Episode walk names its root, every counted season's running
/// remaining count, and the reason it stopped. All console-only.
#[tokio::test]
async fn an_absolute_episode_walk_traces_root_remaining_and_stop() {
    let mut source = Source::new(
        vec![("show", vec![show(1, "Show", Some(24))])],
        vec![
            (
                1,
                vec![with_format(show(4, "Show Movie", Some(1)), "MOVIE")],
            ),
            (4, vec![show(2, "Show 2nd Season", Some(24))]),
            (2, vec![show(3, "Show 3rd Season", Some(12))]),
        ],
    );
    let mut matcher = Matcher::new(&mut source);
    let (mut host, mut runner) = harness(&mut matcher);

    host.load(&mut runner, r"D:\Anime\Show\[ASW] Show - 50.mkv")
        .await;

    assert_eq!(
        host.logs_at(Level::Verbose),
        vec![
            r#"top Candidates for "show": [id=1 title="Show" score=1.000]"#.to_owned(),
            r#"absolute walk: root id=1 total=24 episode=50 remaining=26"#.to_owned(),
            r#"absolute walk: visited continuity id=4 format=MOVIE (budget=1)"#.to_owned(),
            r#"absolute walk: counted season id=2 total=24: remaining=2 (budget=2)"#
                .to_owned(),
            r#"absolute walk: stop: containment in id=3 at episode 2/12 (budget=3)"#
                .to_owned(),
        ]
    );
    assert_eq!(
        host.infos().last().map(String::as_str),
        Some(r#"Match id=3 title="Show 3rd Season" episode="Episode 2/12""#)
    );
    assert!(host.osds().is_empty());
}

/// The Root Season tie rung still picks its root, and the new trace only adds
/// the scored-result line: no extra report, no OSD.
#[tokio::test]
async fn a_tie_resolved_by_the_root_season_keeps_its_decision_trace() {
    let mut source = Source::new(
        vec![(
            "show",
            vec![show(1, "Show Alpha", Some(12)), show(2, "Show Beta", Some(12))],
        )],
        vec![(1, vec![show(2, "Show Beta", Some(12))])],
    );
    let mut matcher = Matcher::new(&mut source);
    let (mut host, mut runner) = harness(&mut matcher);

    host.load(&mut runner, r"D:\Anime\Show\[ASW] Show - 01.mkv")
        .await;

    assert_eq!(
        host.logs_at(Level::Verbose),
        vec![
            r#"top Candidates for "show": [id=1 title="Show Alpha" score=0.870, id=2 title="Show Beta" score=0.870]"#.to_owned(),
        ]
    );
    assert_eq!(
        host.infos().last().map(String::as_str),
        Some(r#"Match id=1 title="Show Alpha" episode="Episode 1/12""#)
    );
    assert!(host.osds().is_empty());
}

/// A matched entry whose known total is below the Parsed Episode contradicts
/// the filename; the contradiction is named in the console trace only.
#[tokio::test]
async fn an_episode_beyond_the_match_total_names_the_contradiction() {
    let mut source = Source::new(
        vec![(
            "show season 2",
            vec![show(2, "Show 2nd Season", Some(3))],
        )],
        vec![],
    );
    let mut matcher = Matcher::new(&mut source);
    let (mut host, mut runner) = harness(&mut matcher);

    host.load(&mut runner, r"D:\Anime\Show\Show S2 - 05.mkv")
        .await;

    assert_eq!(
        host.logs_at(Level::Verbose).last().map(String::as_str),
        Some("episode 5 exceeds Total Episodes 3 in id=2: season marker 2 contradicted")
    );
    assert!(host
        .osds()
        .iter()
        .all(|(text, _)| !text.contains("Total Episodes")));
}
