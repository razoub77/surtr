//! Matching tests: a played path in, one Lookup Answer out, over an injected
//! MediaSource of recorded Candidates. No network.

use std::collections::HashMap;

use surtr::anilist::{Candidate, Titles};
use surtr::daemon::{Lookup, Metadata, MetadataClient, Miss};
use surtr::matcher::{Matcher, MediaSource, SourceError, TitleLanguage};
use surtr::overrides::Overrides;
use surtr::scorer;

mod support;

use support::candidate;

/// Candidates keyed by normalized query; records every query it is asked.
/// `SEQUEL` chains and override entries are keyed by AniList id.
struct FixtureSource {
    fixtures: HashMap<String, Vec<Candidate>>,
    sequels: HashMap<i64, Vec<Candidate>>,
    entries: HashMap<i64, Candidate>,
    queries: Vec<String>,
    sequel_queries: Vec<i64>,
    entry_queries: Vec<i64>,
    /// When set, every relations request fails: a transport error.
    fail_sequels: bool,
}

impl FixtureSource {
    fn new(entries: &[(&str, Vec<Candidate>)]) -> Self {
        Self {
            fixtures: entries
                .iter()
                .map(|(query, candidates)| (scorer::normalize(query), candidates.clone()))
                .collect(),
            sequels: HashMap::new(),
            entries: HashMap::new(),
            queries: Vec::new(),
            sequel_queries: Vec::new(),
            entry_queries: Vec::new(),
            fail_sequels: false,
        }
    }

    fn with_sequels(mut self, chains: &[(i64, Vec<Candidate>)]) -> Self {
        self.sequels = chains.iter().cloned().collect();
        self
    }

    fn with_entries(mut self, entries: &[(i64, Candidate)]) -> Self {
        self.entries = entries.iter().cloned().collect();
        self
    }
}

impl MediaSource for FixtureSource {
    fn search(&mut self, query: &str) -> Result<Vec<Candidate>, SourceError> {
        self.queries.push(query.to_owned());
        Ok(self
            .fixtures
            .get(&scorer::normalize(query))
            .cloned()
            .unwrap_or_default())
    }

    fn sequels(&mut self, id: i64) -> Result<Vec<Candidate>, SourceError> {
        self.sequel_queries.push(id);
        if self.fail_sequels {
            return Err(SourceError::Unavailable);
        }
        Ok(self.sequels.get(&id).cloned().unwrap_or_default())
    }

    fn entry(&mut self, id: i64) -> Result<Candidate, SourceError> {
        self.entry_queries.push(id);
        self.entries
            .get(&id)
            .cloned()
            .ok_or(SourceError::Unavailable)
    }
}

/// The Metadata of a Match: `None` when the lookup answered anything else
/// (this source never throttles or fails, so a miss here is a `Missed`).
fn lookup(source: &mut FixtureSource, path: &str) -> Option<Metadata> {
    match Matcher::new(source).lookup(path) {
        Lookup::Matched(metadata) => Some(metadata),
        _ => None,
    }
}

/// Looks up with a startup-loaded title language preference.
fn lookup_with_lang(
    source: &mut FixtureSource,
    language: TitleLanguage,
    path: &str,
) -> Option<Metadata> {
    match Matcher::new(source).with_title_language(language).lookup(path) {
        Lookup::Matched(metadata) => Some(metadata),
        _ => None,
    }
}

/// Looks up with a startup-loaded overrides map.
fn lookup_with(
    source: &mut FixtureSource,
    overrides: Overrides,
    path: &str,
) -> Option<Metadata> {
    match Matcher::new(source).with_overrides(overrides).lookup(path) {
        Lookup::Matched(metadata) => Some(metadata),
        _ => None,
    }
}

/// A TV Candidate with an explicit episode total (`None` = unknown).
fn tv(id: i64, romaji: &str, episodes: Option<u32>) -> Candidate {
    Candidate {
        episodes,
        ..candidate(id, romaji)
    }
}

/// A Movie Candidate: traversed by an absolute walk, never counted.
fn movie(id: i64, romaji: &str) -> Candidate {
    Candidate {
        format: Some("MOVIE".into()),
        episodes: Some(1),
        ..candidate(id, romaji)
    }
}

#[test]
fn a_match_renders_the_available_title_and_episode_state() {
    let mut source = FixtureSource::new(&[("Monster", vec![candidate(19, "MONSTER")])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Monster\[Anime Time] Monster - 01.mkv");
    assert_eq!(
        metadata,
        Some(Metadata {
            id: 19,
            title: "MONSTER".into(),
            episode: Some("Episode 1/12".into()),
            // The only metadata the helper carries is the Format.
            hover: "TV".into(),
            cover: None,
        })
    );
    assert_eq!(source.queries, ["monster"]);
}

#[test]
fn the_main_title_prefers_english_and_falls_back_to_romaji() {
    let frieren = Candidate {
        title: Some(Titles {
            romaji: Some("Sousou no Frieren".into()),
            english: Some("Frieren: Beyond Journey's End".into()),
        }),
        ..candidate(154587, "Sousou no Frieren")
    };
    let mut source = FixtureSource::new(&[("sousou no frieren", vec![frieren])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Sousou no Frieren - 01.mkv");
    assert_eq!(metadata.unwrap().title, "Frieren: Beyond Journey's End");

    let mut source = FixtureSource::new(&[("monster", vec![candidate(19, "MONSTER")])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Monster - 01.mkv");
    // No English title on AniList: the romaji title stands in.
    assert_eq!(metadata.unwrap().title, "MONSTER");
}

#[test]
fn the_details_title_prefers_english_and_falls_back_to_romaji() {
    // The built-in default leads both slots with English; an entry without
    // one stands in with its romaji title.
    let frieren = Candidate {
        title: Some(Titles {
            romaji: Some("Sousou no Frieren".into()),
            english: Some("Frieren: Beyond Journey's End".into()),
        }),
        ..candidate(154587, "Sousou no Frieren")
    };
    let mut source = FixtureSource::new(&[("sousou no frieren", vec![frieren])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Sousou no Frieren - 01.mkv");
    assert_eq!(metadata.unwrap().title, "Frieren: Beyond Journey's End");

    let mut source = FixtureSource::new(&[("monster", vec![candidate(19, "MONSTER")])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Monster - 01.mkv");
    // No English title on AniList: the romaji title stands in.
    assert_eq!(metadata.unwrap().title, "MONSTER");
}

#[test]
fn a_romaji_preference_promotes_the_romaji_title() {
    let frieren = Candidate {
        title: Some(Titles {
            romaji: Some("Sousou no Frieren".into()),
            english: Some("Frieren: Beyond Journey's End".into()),
        }),
        ..candidate(154587, "Sousou no Frieren")
    };
    let mut source = FixtureSource::new(&[("sousou no frieren", vec![frieren])]);
    let metadata = lookup_with_lang(
        &mut source,
        TitleLanguage::Romaji,
        r"D:\!Anime\Sousou no Frieren - 01.mkv",
    )
    .expect("a match");
    assert_eq!(metadata.title, "Sousou no Frieren");
}

#[test]
fn an_unknown_total_renders_a_question_mark_denominator() {
    let short = Candidate {
        episodes: None,
        ..candidate(9, "Short OVA")
    };
    let mut source = FixtureSource::new(&[("Short OVA", vec![short])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Short OVA - 03.mkv");
    assert_eq!(metadata.unwrap().episode, Some("Episode 3/?".into()));
}

// ---------------------------------------------------------------------------
// State line: the Episode segment
// ---------------------------------------------------------------------------

#[test]
fn a_one_episode_entry_renders_the_bare_episode_1() {
    // However the file numbered it, a one-episode entry is `Episode 1`.
    let mut source = FixtureSource::new(&[("Short OVA", vec![tv(1, "Short OVA", Some(1))])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Short OVA - 01.mkv");
    assert_eq!(metadata.unwrap().episode, Some("Episode 1".into()));
}

#[test]
fn a_movie_with_a_known_count_of_one_renders_episode_1() {
    // AniList movies usually report `episodes: 1`: it still names the movie.
    let entry = Candidate {
        format: Some("MOVIE".into()),
        episodes: Some(1),
        ..candidate(1, "Show Movie")
    };
    let mut source = FixtureSource::new(&[("Show Movie", vec![entry])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Show Movie.mkv");
    assert_eq!(metadata.unwrap().episode, Some("Episode 1".into()));
}

#[test]
fn a_movie_with_an_unknown_count_renders_no_episode() {
    let entry = Candidate {
        format: Some("MOVIE".into()),
        episodes: None,
        ..candidate(1, "Show Movie")
    };
    let mut source = FixtureSource::new(&[("Show Movie", vec![entry])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Show Movie.mkv");
    assert_eq!(metadata.unwrap().episode, None);
}

#[test]
fn a_multi_part_movie_renders_its_episode() {
    // A two-part movie has a meaningful count: which part is playing matters.
    let entry = Candidate {
        format: Some("MOVIE".into()),
        episodes: Some(2),
        ..candidate(1, "Show Movie")
    };
    let mut source = FixtureSource::new(&[("Show Movie", vec![entry])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Show Movie - 02.mkv");
    assert_eq!(metadata.unwrap().episode, Some("Episode 2/2".into()));
}

#[test]
fn an_unnumbered_file_with_no_known_total_renders_no_episode() {
    // A Movie matched to an entry with an unknown count: nothing to publish.
    let entry = Candidate {
        format: None,
        episodes: None,
        ..candidate(1, "Show Movie")
    };
    let mut source = FixtureSource::new(&[("Show Movie", vec![entry])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Show Movie.mkv");
    assert_eq!(metadata.unwrap().episode, None);
}

#[test]
fn a_movie_covers_its_title_and_tail_queries() {
    // The full movie title (`jujutsu kaisen jujutsu kaisen 0`) finds nothing;
    // the tail after `- Movie 01 -` does.
    let mut source = FixtureSource::new(&[("Jujutsu Kaisen 0", vec![movie(131573, "Jujutsu Kaisen 0")])]);
    let metadata = lookup(
        &mut source,
        r"D:\!Anime\[Judas] Jujutsu Kaisen - Movie 01 - Jujutsu Kaisen 0 [BD 1080p].mkv",
    );
    assert_eq!(source.queries, ["jujutsu kaisen jujutsu kaisen 0", "jujutsu kaisen 0"]);
    assert_eq!(
        metadata,
        Some(Metadata {
            id: 131573,
            title: "Jujutsu Kaisen 0".into(),
            episode: Some("Episode 1".into()),
            hover: "Movie".into(),
            cover: None,
        })
    );
}

#[test]
fn a_movie_takes_its_first_accepted_query() {
    // `Black Clover - Sword of the Wizard King` matches on the full title;
    // the tail never runs.
    let mut source = FixtureSource::new(&[(
        "Black Clover Sword of the Wizard King",
        vec![Candidate {
            format: Some("MOVIE".into()),
            title: Some(Titles {
                romaji: Some("Black Clover: Mahou Tei no Ken".into()),
                english: Some("Black Clover: Sword of the Wizard King".into()),
            }),
            ..candidate(131680, "")
        }],
    )]);
    let metadata = lookup(
        &mut source,
        r"D:\!Anime\Black Clover\[Judas] Black Clover - Sword of the Wizard King [1080p].mkv",
    );
    assert_eq!(source.queries, ["black clover sword of the wizard king"]);
    // English leads `details` under the default preference.
    assert_eq!(
        metadata.unwrap().title,
        "Black Clover: Sword of the Wizard King"
    );
}

#[test]
fn a_romaji_preference_restores_the_romaji_details_title() {
    // `details_title_lang=romaji`: the romanized title leads again.
    let mut source = FixtureSource::new(&[(
        "Black Clover Sword of the Wizard King",
        vec![Candidate {
            format: Some("MOVIE".into()),
            title: Some(Titles {
                romaji: Some("Black Clover: Mahou Tei no Ken".into()),
                english: Some("Black Clover: Sword of the Wizard King".into()),
            }),
            ..candidate(131680, "")
        }],
    )]);
    let metadata = lookup_with_lang(
        &mut source,
        TitleLanguage::Romaji,
        r"D:\!Anime\Black Clover\[Judas] Black Clover - Sword of the Wizard King [1080p].mkv",
    )
    .expect("a match");
    assert_eq!(metadata.title, "Black Clover: Mahou Tei no Ken");
}

#[test]
fn rejected_releases_produce_no_lookup() {
    let mut source = FixtureSource::new(&[("Show", vec![candidate(1, "Show")])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Show\[X] Show - 01-12 [1080p].mkv");
    assert_eq!(metadata, None);
    assert!(source.queries.is_empty(), "no query for a rejected file");
}

#[test]
fn an_ova_without_an_episode_number_produces_no_match() {
    let mut source = FixtureSource::new(&[(
        "Honzuki no Gekokujou OVA",
        vec![candidate(121769, "Honzuki no Gekokujou: Shou no Nai Seikatsu")],
    )]);
    let metadata = lookup(
        &mut source,
        r"D:\!Anime\Honzuki no Gekokujou\Honzuki no Gekokujou OVA\S01OVA-Eustachius's Incognito Operation Downtown, Visiting Missus Corinna.mkv",
    );
    assert_eq!(metadata, None);
}

#[test]
fn an_english_only_entry_renders_the_english_title() {
    // No romaji title to stand in: the English one renders alone.
    let english = Candidate {
        title: Some(Titles {
            romaji: None,
            english: Some("Lord of Mysteries".into()),
        }),
        ..candidate(186715, "")
    };
    let mut source = FixtureSource::new(&[("Lord of Mysteries", vec![english])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Lord of Mysteries\S01E01-The Fool.mkv");
    assert_eq!(metadata.unwrap().title, "Lord of Mysteries");
}

// ---------------------------------------------------------------------------
// Hover line: Format • Status • Season • Genres
// ---------------------------------------------------------------------------

#[test]
fn the_hover_line_joins_every_segment_the_entry_carries() {
    // The example from https://anilist.co/anime/16498.
    let entry = Candidate {
        season_year: Some(2013),
        season: Some("SPRING".into()),
        status: Some("FINISHED".into()),
        genres: ["Action", "Drama", "Fantasy", "Mystery", "Horror"]
            .iter()
            .map(|genre| genre.to_string())
            .collect(),
        ..candidate(16498, "Shingeki no Kyojin")
    };
    let mut source = FixtureSource::new(&[("Shingeki no Kyojin", vec![entry])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Shingeki no Kyojin - 01.mkv");
    assert_eq!(
        metadata.unwrap().hover,
        "TV • Finished • Spring 2013 • Action, Drama, Fantasy, Mystery",
        "genres are capped at four"
    );
}

#[test]
fn each_hover_segment_is_omitted_when_the_entry_lacks_it() {
    let entry = Candidate {
        format: Some("MOVIE".into()),
        status: Some("NOT_YET_RELEASED".into()),
        genres: ["Drama"].iter().map(|genre| genre.to_string()).collect(),
        ..candidate(1, "Show Movie")
    };
    let mut source = FixtureSource::new(&[("Show Movie", vec![entry])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Show Movie.mkv");
    assert_eq!(metadata.unwrap().hover, "Movie • Drama");
}

#[test]
fn the_hover_season_falls_back_to_the_year_alone() {
    // Pre-2014 entries and most movies carry no airing season.
    let entry = Candidate {
        season_year: Some(2007),
        status: Some("RELEASING".into()),
        ..candidate(1, "Show")
    };
    let mut source = FixtureSource::new(&[("Show", vec![entry])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Show - 01.mkv");
    assert_eq!(metadata.unwrap().hover, "TV • Ongoing • 2007");
}

#[test]
fn an_unrecognized_status_or_season_renders_nothing() {
    let entry = Candidate {
        season: Some("FUTURE_SEASON".into()),
        status: Some("CANCELLED".into()),
        ..candidate(1, "Show")
    };
    let mut source = FixtureSource::new(&[("Show", vec![entry])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Show - 01.mkv");
    // Format alone, then: an unrecognized status or season is omitted, not
    // guessed at.
    assert_eq!(metadata.unwrap().hover, "TV");
}

#[test]
fn the_hover_title_stands_in_when_the_entry_carries_no_metadata() {
    // An entry with no Format, Status, Season or Genres: the title renders.
    let entry = Candidate {
        format: None,
        ..candidate(19, "MONSTER")
    };
    let mut source = FixtureSource::new(&[("Monster", vec![entry])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Monster\[Anime Time] Monster - 01.mkv");
    assert_eq!(metadata.unwrap().hover, "MONSTER");
}

#[test]
fn a_hiatus_entry_is_marked_hiatus() {
    let entry = Candidate {
        status: Some("HIATUS".into()),
        ..candidate(1, "Show")
    };
    let mut source = FixtureSource::new(&[("Show", vec![entry])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Show - 01.mkv");
    assert_eq!(metadata.unwrap().hover, "TV • Hiatus");
}

// ---------------------------------------------------------------------------
// Season markers
// ---------------------------------------------------------------------------

#[test]
fn an_explicit_season_marker_injects_the_season_and_requires_corroboration() {
    let mut source = FixtureSource::new(&[(
        "Youjo Senki Season 2",
        vec![candidate(135865, "Youjo Senki II")],
    )]);
    let metadata = lookup(
        &mut source,
        r"D:\!Anime\Youjo Senki S2\[ASW] Youjo Senki S2 - 01 [1080p].mkv",
    );
    assert_eq!(source.queries, ["youjo senki season 2"]);
    let metadata = metadata.expect("the roman numeral corroborates season 2");
    assert_eq!(metadata.id, 135865);
    assert_eq!(metadata.episode, Some("Episode 1/12".into()));
}

#[test]
fn an_explicit_season_marker_uncorroborated_by_the_winner_yields_no_presence() {
    let mut source = FixtureSource::new(&[(
        "Youjo Senki Season 2",
        vec![candidate(21613, "Youjo Senki")],
    )]);
    assert_eq!(
        lookup(
            &mut source,
            r"D:\!Anime\Youjo Senki S2\[ASW] Youjo Senki S2 - 01.mkv"
        ),
        None,
        "the winner carries no season-2 ordinal"
    );
}

#[test]
fn an_explicit_season_marker_contradicted_by_the_total_yields_no_presence() {
    let mut source = FixtureSource::new(&[(
        "Show Season 2",
        vec![tv(2, "Show 2nd Season", Some(12))],
    )]);
    assert_eq!(
        lookup(&mut source, r"D:\!Anime\Show S2\[X] Show S2 - 13.mkv"),
        None,
        "13 episodes in a marked 12-episode season is a contradiction"
    );
    assert!(
        source.sequel_queries.is_empty(),
        "a contradicted marker never walks forward"
    );
}

#[test]
fn the_filename_season_marker_wins_over_the_folder() {
    let mut source = FixtureSource::new(&[(
        "Show Season 2",
        vec![tv(2, "Show 2nd Season", Some(12))],
    )]);
    let metadata = lookup(&mut source, r"D:\!Anime\Show S3\[X] Show S2 - 01.mkv");
    assert_eq!(source.queries, ["show season 2"]);
    assert_eq!(metadata.unwrap().id, 2);
}

// ---------------------------------------------------------------------------
// Absolute episode remapping
// ---------------------------------------------------------------------------

#[test]
fn an_absolute_episode_walks_sequels_and_remaps_to_the_containing_season() {
    let mut source = FixtureSource::new(&[("Show", vec![tv(1, "Show", Some(24))])]).with_sequels(&[
        (1, vec![tv(2, "Show 2nd Season", Some(24))]),
        (2, vec![tv(3, "Show 3rd Season", Some(12))]),
    ]);
    let metadata = lookup(&mut source, r"D:\!Anime\Show\[X] Show - 50.mkv").expect("a match");
    assert_eq!(metadata.id, 3);
    assert_eq!(metadata.title, "Show 3rd Season");
    assert_eq!(metadata.episode, Some("Episode 2/12".into()));
    assert_eq!(source.sequel_queries, [1, 2]);
}

#[test]
fn a_movie_sequel_is_traversed_but_not_counted() {
    let mut source = FixtureSource::new(&[("Show", vec![tv(1, "Show", Some(12))])]).with_sequels(&[
        (1, vec![movie(2, "Show Movie")]),
        (2, vec![tv(3, "Show 2nd Season", Some(12))]),
    ]);
    let metadata = lookup(&mut source, r"D:\!Anime\Show\[X] Show - 13.mkv").expect("a match");
    assert_eq!(metadata.id, 3);
    assert_eq!(metadata.episode, Some("Episode 1/12".into()));
}

#[test]
fn an_unknown_target_total_at_the_end_of_the_chain_remaps_to_a_question_mark() {
    let mut source = FixtureSource::new(&[("Show", vec![tv(1, "Show", Some(12))])])
        .with_sequels(&[(1, vec![tv(2, "Show 2nd Season", None)])]);
    let metadata = lookup(&mut source, r"D:\!Anime\Show\[X] Show - 13.mkv").expect("a match");
    assert_eq!(metadata.id, 2);
    assert_eq!(metadata.episode, Some("Episode 1/?".into()));
}

/// The shape of a real library entry: `Kikansha no Mahou wa Tokubetsu desu`
/// season 1 has 12 episodes and the season-2 sequel (`172192`) reports
/// `episodes: null`. An unmarked `- 13` is absolute 13, so it lands on season 2
/// episode 1 and publishes `Episode 1/?` instead of silently producing nothing.
#[test]
fn an_unmarked_episode_past_a_known_first_season_lands_in_a_sequel_with_an_unknown_total() {
    let mut source = FixtureSource::new(&[(
        "Kikansha no Mahou wa Tokubetsu desu",
        vec![tv(163142, "Kikansha no Mahou wa Tokubetsu desu", Some(12))],
    )])
    .with_sequels(&[(
        163142,
        vec![tv(
            172192,
            "Kikansha no Mahou wa Tokubetsu desu 2nd Season",
            None,
        )],
    )]);
    let path = r"D:\!Anime\Kikansha no Mahou wa Tokubetsu Desu\[ASW] Kikansha no Mahou wa Tokubetsu desu - 13 [1080p HEVC][91DD2CA6].mkv";
    let metadata = lookup(&mut source, path).expect("a match, not a silent miss");
    assert_eq!(metadata.id, 172192);
    assert_eq!(metadata.episode, Some("Episode 1/?".into()));
}

#[test]
fn an_unknown_count_ahead_of_a_further_sequel_yields_no_presence() {
    let mut source = FixtureSource::new(&[("Show", vec![tv(1, "Show", Some(12))])]).with_sequels(&[
        (1, vec![tv(2, "Show 2nd Season", None)]),
        (2, vec![tv(3, "Show 3rd Season", Some(12))]),
    ]);
    assert_eq!(
        lookup(&mut source, r"D:\!Anime\Show\[X] Show - 13.mkv"),
        None,
        "a counted season after the unknown total makes the containment ambiguous"
    );
}

#[test]
fn a_branching_sequel_chain_yields_no_presence() {
    let mut source = FixtureSource::new(&[("Show", vec![tv(1, "Show", Some(12))])]).with_sequels(&[(
        1,
        vec![tv(2, "Show 2nd Season", Some(12)), tv(3, "Show Another Season", Some(12))],
    )]);
    assert_eq!(lookup(&mut source, r"D:\!Anime\Show\[X] Show - 13.mkv"), None);
}

#[test]
fn an_episode_overrunning_the_known_chain_yields_no_presence() {
    let mut source = FixtureSource::new(&[("Show", vec![tv(1, "Show", Some(12))])])
        .with_sequels(&[(1, vec![tv(2, "Show 2nd Season", Some(12))])]);
    assert_eq!(lookup(&mut source, r"D:\!Anime\Show\[X] Show - 30.mkv"), None);
}

#[test]
fn an_unknown_base_total_yields_no_presence_for_an_absolute_number() {
    let mut source = FixtureSource::new(&[("ONE PIECE", vec![tv(21, "ONE PIECE", None)])]);
    assert_eq!(
        lookup(&mut source, r"D:\!Anime\ONE PIECE\[X] ONE PIECE - 1071.mkv"),
        None,
        "there is no known range to test 1071 against"
    );
}

#[test]
fn an_unknown_target_total_followed_only_by_a_movie_still_resolves() {
    let mut source = FixtureSource::new(&[("Show", vec![tv(1, "Show", Some(12))])]).with_sequels(&[
        (1, vec![tv(2, "Show 2nd Season", None)]),
        (2, vec![movie(3, "Show Movie")]),
    ]);
    let metadata = lookup(&mut source, r"D:\!Anime\Show\[X] Show - 13.mkv").expect("a match");
    assert_eq!(metadata.id, 2);
    assert_eq!(metadata.episode, Some("Episode 1/?".into()));
}

#[test]
fn an_unknown_target_total_followed_by_another_season_yields_no_presence() {
    let mut source = FixtureSource::new(&[("Show", vec![tv(1, "Show", Some(12))])]).with_sequels(&[
        (1, vec![tv(2, "Show 2nd Season", None)]),
        (2, vec![movie(3, "Show Movie")]),
        (3, vec![tv(4, "Show 3rd Season", Some(12))]),
    ]);
    assert_eq!(
        lookup(&mut source, r"D:\!Anime\Show\[X] Show - 13.mkv"),
        None,
        "a counted season after the unknown total makes the containment ambiguous"
    );
}

#[test]
fn a_sequel_to_an_ignored_format_yields_no_presence() {
    let music = Candidate {
        format: Some("MUSIC".into()),
        ..candidate(2, "Show Music Video")
    };
    let mut source = FixtureSource::new(&[("Show", vec![tv(1, "Show", Some(12))])])
        .with_sequels(&[(1, vec![music])]);
    assert_eq!(lookup(&mut source, r"D:\!Anime\Show\[X] Show - 13.mkv"), None);
}

// ---------------------------------------------------------------------------
// Overrides
// ---------------------------------------------------------------------------

#[test]
fn an_override_hit_publishes_without_searching() {
    let mut source = FixtureSource::new(&[])
        .with_entries(&[(108268, candidate(108268, "Honzuki no Gekokujou"))]);
    let (overrides, messages) = Overrides::parse(r#"{"honzuki no gekokujou": 108268}"#);
    assert!(messages.is_empty(), "got: {messages:?}");

    let metadata = lookup_with(
        &mut source,
        overrides,
        r"D:\!Anime\Honzuki no Gekokujou\[ASW] Honzuki no Gekokujou - 01 [1080p HEVC].mkv",
    )
    .expect("the override resolves");

    assert_eq!(metadata.id, 108268);
    assert_eq!(metadata.episode, Some("Episode 1/12".into()));
    assert!(source.queries.is_empty(), "an override skips search");
    assert_eq!(source.entry_queries, [108268]);
}

#[test]
fn an_override_key_ignores_casing_and_extra_whitespace() {
    let mut source = FixtureSource::new(&[])
        .with_entries(&[(108268, candidate(108268, "Honzuki no Gekokujou"))]);
    let (overrides, _) = Overrides::parse(r#"{"  HONZUKI   NO GEKOKUJOU ": 108268}"#);

    let metadata = lookup_with(
        &mut source,
        overrides,
        r"D:\!Anime\Honzuki no Gekokujou\[ASW] Honzuki no Gekokujou - 01.mkv",
    );

    assert_eq!(metadata.map(|m| m.id), Some(108268));
}

#[test]
fn a_key_miss_falls_back_to_normal_matching() {
    let mut source = FixtureSource::new(&[("Show", vec![candidate(1, "Show")])]);
    let (overrides, _) = Overrides::parse(r#"{"something else": 5}"#);

    let metadata = lookup_with(&mut source, overrides, r"D:\!Anime\Show\[X] Show - 01.mkv");

    assert_eq!(metadata.map(|m| m.id), Some(1));
    assert_eq!(source.queries, ["show"]);
}

#[test]
fn an_override_dead_id_yields_no_presence_and_no_search_fallback() {
    // A search would match, but the user's assertion is final.
    let mut source = FixtureSource::new(&[("Honzuki no Gekokujou", vec![candidate(1, "Honzuki no Gekokujou")])]);
    let (overrides, _) = Overrides::parse(r#"{"honzuki no gekokujou": 999999}"#);

    let metadata = lookup_with(
        &mut source,
        overrides,
        r"D:\!Anime\Honzuki no Gekokujou\[ASW] Honzuki no Gekokujou - 01.mkv",
    );

    assert_eq!(metadata, None);
    assert_eq!(source.entry_queries, [999999]);
    assert!(source.queries.is_empty(), "never fall back to search");
}

#[test]
fn an_override_contradicting_the_explicit_season_yields_no_presence() {
    let mut source =
        FixtureSource::new(&[]).with_entries(&[(2, tv(2, "Show 2nd Season", Some(12)))]);
    let (overrides, _) = Overrides::parse(r#"{"show season 2": 2}"#);

    assert_eq!(
        lookup_with(
            &mut source,
            overrides,
            r"D:\!Anime\Show S2\[X] Show S2 - 13.mkv"
        ),
        None,
        "13 episodes in a marked 12-episode season is a contradiction"
    );
    assert!(source.queries.is_empty(), "never fall back to search");
}

#[test]
fn an_override_skips_season_corroboration() {
    // The user asserted the id, so an entry whose title omits the ordinal is
    // still trusted: only the episode count can contradict the marker.
    let mut source = FixtureSource::new(&[]).with_entries(&[(2, tv(2, "Show", Some(12)))]);
    let (overrides, _) = Overrides::parse(r#"{"show season 2": 2}"#);

    let metadata = lookup_with(
        &mut source,
        overrides,
        r"D:\!Anime\Show S2\[X] Show S2 - 05.mkv",
    )
    .expect("the override is trusted");

    assert_eq!(metadata.id, 2);
    assert_eq!(metadata.episode, Some("Episode 5/12".into()));
}

// ---------------------------------------------------------------------------
// The folder's spelling of the title: the last-resort query
// ---------------------------------------------------------------------------

const GLUED: &str = r"D:\!Anime\Tensei shita Dai Seijo wa, Seijo de Aru Koto wo Hita Kakusu\[ASW] Tensei shita Daiseijo wa, Seijo de Aru Koto wo Hitakakusu - 05 [1080p HEVC][5F734ACB].mkv";

#[test]
fn a_zero_result_title_searches_the_folders_spelling() {
    // AniList finds the romaji title only as the folder spells it, and reports
    // the ONA's total as unknown (recorded live, 2026-11-15).
    let mut source = FixtureSource::new(&[(
        "tensei shita dai seijo wa seijo de aru koto wo hita kakusu",
        vec![Candidate {
            format: Some("ONA".into()),
            episodes: None,
            ..candidate(
                187402,
                "Tensei shita Dai Seijo wa, Seijo de Aru Koto wo Hita Kakusu",
            )
        }],
    )]);
    let metadata = lookup(&mut source, GLUED).expect("the folder spelling matches");
    assert_eq!(metadata.id, 187402);
    assert_eq!(metadata.episode, Some("Episode 5/?".into()));
    assert_eq!(
        source.queries,
        [
            "tensei shita daiseijo wa seijo de aru koto wo hitakakusu",
            "tensei shita dai seijo wa seijo de aru koto wo hita kakusu"
        ],
        "the filename's title is asked for first, the folder only after a zero"
    );
}

#[test]
fn a_title_that_returned_candidates_never_falls_back_to_the_folder() {
    // A lost score-off is not a zero result: the folder spelling could
    // otherwise turn a documented miss into a lucky match.
    let mut source = FixtureSource::new(&[(
        "tensei shita daiseijo wa seijo de aru koto wo hitakakusu",
        vec![candidate(1, "Some Other Show")],
    )]);
    assert_eq!(
        miss_of(&mut source, GLUED),
        Some(Miss::Repairable {
            key: "tensei shita daiseijo wa seijo de aru koto wo hitakakusu".into()
        })
    );
    assert_eq!(
        source.queries,
        ["tensei shita daiseijo wa seijo de aru koto wo hitakakusu"],
        "no second query"
    );
}

#[test]
fn a_folder_without_a_new_spelling_costs_no_extra_query() {
    let mut source = FixtureSource::new(&[("monster", vec![candidate(19, "MONSTER")])]);
    lookup(&mut source, r"D:\!Anime\Monster\[Anime Time] Monster - 01.mkv");
    assert_eq!(source.queries, ["monster"]);
}

#[test]
fn a_dead_folder_query_still_reports_the_files_own_title() {
    // Both spellings return zero: the override key must be the title the
    // filename names, not the folder's last-resort guess.
    let mut source = FixtureSource::new(&[]);
    assert_eq!(
        miss_of(&mut source, GLUED),
        Some(Miss::Repairable {
            key: "tensei shita dai seijo wa seijo de aru koto wo hita kakusu".into()
        })
    );
}

// ---------------------------------------------------------------------------
// Miss reports: why a lookup found no Match (issue 07)
// ---------------------------------------------------------------------------

/// Looks a path up and returns why it missed, when the answer is a Miss.
fn miss_of(source: &mut FixtureSource, path: &str) -> Option<Miss> {
    match Matcher::new(source).lookup(path) {
        Lookup::Missed(miss) => Some(miss),
        _ => None,
    }
}

#[test]
fn a_no_match_miss_names_the_override_key() {
    let mut source = FixtureSource::new(&[]);
    assert_eq!(
        miss_of(
            &mut source,
            r"D:\!Anime\Honzuki no Gekokujou\[ASW] Honzuki no Gekokujou S01E01.mkv"
        ),
        Some(Miss::Repairable {
            key: "honzuki no gekokujou".into()
        }),
        "the exact key one overrides.json line could use"
    );
}

#[test]
fn an_uncorroborated_season_marker_miss_names_the_seasoned_key() {
    let mut source = FixtureSource::new(&[(
        "Youjo Senki Season 2",
        vec![candidate(21613, "Youjo Senki")],
    )]);
    assert_eq!(
        miss_of(
            &mut source,
            r"D:\!Anime\Youjo Senki S2\[ASW] Youjo Senki S2 - 01.mkv"
        ),
        Some(Miss::Repairable {
            key: "youjo senki season 2".into()
        })
    );
}

#[test]
fn an_ambiguous_absolute_branch_is_a_repairable_miss() {
    // No unique containing season: one override naming a sequel could still
    // produce a card (issue 07).
    let mut source = FixtureSource::new(&[("Show", vec![tv(1, "Show", Some(12))])]).with_sequels(&[(
        1,
        vec![tv(2, "Show 2nd Season", Some(12)), tv(3, "Show Another Season", Some(12))],
    )]);
    assert_eq!(
        miss_of(&mut source, r"D:\!Anime\Show\[X] Show - 13.mkv"),
        Some(Miss::Repairable {
            key: "show".into()
        })
    );
}

#[test]
fn an_absolute_episode_overrunning_the_chain_is_a_fatal_miss() {
    let mut source = FixtureSource::new(&[("Show", vec![tv(1, "Show", Some(12))])])
        .with_sequels(&[(1, vec![tv(2, "Show 2nd Season", Some(12))])]);
    assert_eq!(
        miss_of(&mut source, r"D:\!Anime\Show\[X] Show - 30.mkv"),
        Some(Miss::Fatal {
            reason: "absolute episode beyond the known sequel chain".into()
        }),
        "no season would contain it, so no override could fix it"
    );
}

#[test]
fn an_absolute_episode_with_an_unknown_total_is_a_fatal_miss() {
    let mut source = FixtureSource::new(&[("ONE PIECE", vec![tv(21, "ONE PIECE", None)])]);
    assert_eq!(
        miss_of(&mut source, r"D:\!Anime\ONE PIECE\[X] ONE PIECE - 1071.mkv"),
        Some(Miss::Fatal {
            reason: "absolute episode with no known Total Episodes".into()
        }),
        "no single override line can fix an unknown total"
    );
}

#[test]
fn a_batch_range_is_a_fatal_miss() {
    let mut source = FixtureSource::new(&[]);
    assert_eq!(
        miss_of(&mut source, r"D:\!Anime\Show\[X] Show - 01-12.mkv"),
        Some(Miss::Fatal {
            reason: "batch range or multi-episode file".into()
        })
    );
}

#[test]
fn an_ova_is_a_fatal_miss() {
    let mut source = FixtureSource::new(&[]);
    assert_eq!(
        miss_of(&mut source, r"D:\!Anime\Show\Show OVA\S01OVA-Extra.mkv"),
        Some(Miss::Fatal {
            reason: "non-episodic file (OVA or special)".into()
        })
    );
}

#[test]
fn a_match_clears_any_previous_miss() {
    let mut source = FixtureSource::new(&[("Monster", vec![candidate(19, "MONSTER")])]);
    let mut matcher = Matcher::new(&mut source);
    assert!(matches!(
        matcher.lookup(r"D:\!Anime\Unknown Show\[X] Unknown Show - 01.mkv"),
        Lookup::Missed(_)
    ));
    assert!(matches!(
        matcher.lookup(r"D:\!Anime\Monster\[X] Monster - 01.mkv"),
        Lookup::Matched(_)
    ));
}

/// A source whose every request fails: a transport error, not an answer.
struct Unreachable;

impl MediaSource for Unreachable {
    fn search(&mut self, _query: &str) -> Result<Vec<Candidate>, SourceError> {
        Err(SourceError::Unavailable)
    }
    fn sequels(&mut self, _id: i64) -> Result<Vec<Candidate>, SourceError> {
        Err(SourceError::Unavailable)
    }
    fn entry(&mut self, _id: i64) -> Result<Candidate, SourceError> {
        Err(SourceError::Unavailable)
    }
}

#[test]
fn a_failed_request_is_no_answer_and_reports_nothing() {
    let mut source = Unreachable;
    let mut matcher = Matcher::new(&mut source);
    assert_eq!(
        matcher.lookup(r"D:\!Anime\Show\[X] Show - 01.mkv"),
        Lookup::Unavailable,
        "a transport failure is not a definitive miss"
    );
}

#[test]
fn a_failed_relations_request_reports_nothing() {
    // The walk starts past the Root Season's total, then the relations
    // request fails: no Presence, but no miss report either.
    let mut source = FixtureSource::new(&[("Show", vec![tv(1, "Show", Some(12))])])
        .with_sequels(&[(1, vec![tv(2, "Show 2nd Season", Some(12))])]);
    source.fail_sequels = true;
    let mut matcher = Matcher::new(&mut source);
    assert_eq!(
        matcher.lookup(r"D:\!Anime\Show\[X] Show - 25.mkv"),
        Lookup::Unavailable
    );
}
