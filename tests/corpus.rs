//! Golden match corpus: the 48 scored rows of the research corpus, matched
//! offline against the recorded AniList search fixtures
//! (`fixtures/anilist-corpus.json`, regenerate with `fixtures/generate.mjs`)
//! and the recorded `SEQUEL` chains (`fixtures/anilist-relations.json`), plus
//! row 50 — row 20's file under a folder that spells the title as AniList does.
//!
//! Expectations follow the agreed scorer (margin ≥ 0.10), pinned by
//! `tests/scorer.rs`:
//! 46 rows accept their expected entry, rows 4/20 yield no Match. Row 28's
//! five tie seasons fall to the Root Season (the one tied Candidate with no
//! season ordinal). Rows
//! 41/44/45/46 are absolute-numbered: the Parsed Episode exceeds the Root
//! Season's total, the `SEQUEL` walk finds the containing season, and the
//! episode is remapped to the Season Episode (Oshi no Ko 2nd Season, Kusuriya
//! 3rd Season, Yasei 2nd Season, Tensei Kizoku 3rd Season). Row 43's folder
//! marker resolves its season directly, so it never walks.

use std::collections::HashMap;

use surtr::anilist::Candidate;
use surtr::daemon::{Lookup, Metadata, MetadataClient, Policy, Runner};
use surtr::matcher::{Matcher, MediaSource, SourceError};
use surtr::overrides::Overrides;
use surtr::scorer;

mod support;

use support::{Host, SharedClaim, PID, T0};

/// The Metadata of a Match: `None` when the lookup answered anything else.
/// The corpus's misses and rejects are all `Missed` answers.
fn matched(answer: Lookup) -> Option<Metadata> {
    match answer {
        Lookup::Matched(metadata) => Some(metadata),
        _ => None,
    }
}

/// Recorded search results keyed by normalized query.
static FIXTURES: &str = include_str!("fixtures/anilist-corpus.json");
/// Recorded `SEQUEL` chains keyed by AniList id.
static RELATIONS: &str = include_str!("fixtures/anilist-relations.json");

/// One corpus row: the played path and the id the pipeline must resolve —
/// `None` for a documented reject or miss.
struct Row {
    path: &'static str,
    expect: Option<i64>,
    /// The remapped `Episode x/y` segment, when the row is absolute-numbered.
    episode: Option<&'static str>,
}

const fn ok(path: &'static str, id: i64) -> Row {
    Row {
        path,
        expect: Some(id),
        episode: None,
    }
}

const fn remapped(path: &'static str, id: i64, episode: &'static str) -> Row {
    Row {
        path,
        expect: Some(id),
        episode: Some(episode),
    }
}

const fn none(path: &'static str) -> Row {
    Row {
        path,
        expect: None,
        episode: None,
    }
}

const ROWS: &[(u8, Row)] = &[
    (
        1,
        ok(
            r"D:\!Anime\Clannad\[Trix] Clannad Movie (2007) [49889AC3].mkv",
            1723,
        ),
    ),
    (
        2,
        ok(
            r"D:\!Anime\[Anime Time] Gekijouban Sword Art Online Progressive - Kuraki Yuuyami no Scherzo (2022) [BD] [1080p][HEVC 10bit x265][AAC][Eng Sub].mkv",
            140999,
        ),
    ),
    (
        3,
        ok(
            r"D:\!Anime\[Judas] Jujutsu Kaisen - Movie 01 - Jujutsu Kaisen 0 [BD 1080p][HEVC x265 10bit][Dual-Audio][Multi-Subs].mkv",
            131573,
        ),
    ),
    (
        4,
        none(
            r"D:\!Anime\[Trix] Demon Slayer Infinity Castle I [WEBRip 1080p AV1] (Dual Audio, Multi Subs).mkv",
        ),
    ),
    (
        5,
        ok(
            r"D:\!Anime\Cosmic.Princess.Kaguya.2026.1080p.NF.WEB-DL.DUAL.DDP5.1.H.264-VARYG.mkv",
            201903,
        ),
    ),
    (6, ok(r"D:\!Anime\[BDFansub] Kimi no Na wa [1080p][AAC][x265].mkv", 21519)),
    (
        7,
        ok(
            r"D:\!Anime\[ASW] Spy x Family - Code White [1080p HEVC][D4935429].mkv",
            158928,
        ),
    ),
    (
        8,
        ok(
            r"D:\!Anime\[ASW] Ansatsu Kyoushitsu Movie - Minna no Jikan [1080p HEVC][DF86E022].mkv",
            198373,
        ),
    ),
    (9, ok(r"D:\!Anime\[Judas] No Game No Life Zero.mkv", 21875)),
    (
        10,
        ok(
            r"D:\!Anime\[Judas] Sword Art Online - Movie 01 (Ordinal Scale) [BD 1080p][HEVC x265 10bit][Dual-Audio][Eng-Subs].mkv",
            21403,
        ),
    ),
    (
        11,
        ok(
            r"D:\!Anime\Black Clover\Black Clover S1\[Judas] Black Clover - 001.mkv",
            97940,
        ),
    ),
    (12, ok(r"D:\!Anime\Black Clover S2\[Judas] Black Clover - S02E01.mkv", 195604)),
    (
        13,
        ok(
            r"D:\!Anime\Black Clover\[Judas] Black Clover - Sword of the Wizard King [1080p][HEVC x265 10bit][Dual-Audio][Multi-Subs].mkv",
            131680,
        ),
    ),
    (14, ok(r"D:\!Anime\Monster\[Anime Time] Monster - 01.mkv", 19)),
    (
        15,
        ok(
            r"D:\!Anime\Violet Evergarden\[Sokudo] Violet Evergarden - 01 [1080p BD][AV1][dual audio].mkv",
            21827,
        ),
    ),
    (
        16,
        ok(
            r"D:\!Anime\Shunkashuutou Daikousha - Haru no Mai\[ASW] Shunkashuutou Daikousha - Haru no Mai - 01 [1080p HEVC][44DFED18].mkv",
            190143,
        ),
    ),
    (
        17,
        ok(
            r"D:\!Anime\Doumo, Suki na Hito ni Horegusuri wo Irai sareta Majo desu\[ASW] Doumo, Suki na Hito ni Horegusuri wo Irai sareta Majo desu. - 01 [1080p HEVC][BC2C1B27].mkv",
            207191,
        ),
    ),
    // The filename's `Bu o` and AniList's `Bu wo` are the same object
    // particle; folding them lands the entry exactly, and its TV format
    // bonus separates it from the `Mini` ONA that shares the title.
    (
        18,
        ok(
            r"D:\!Anime\Eiyuu Ou, Bu o Kiwameru Tame Tenseisu - Soshite, Sekai Saikyou no Minarai Kishi\[Anime Time] Eiyuu Ou, Bu o Kiwameru Tame Tenseisu - Soshite, Sekai Saikyou no Minarai Kishi - 01.mkv",
            142193,
        ),
    ),
    (
        19,
        ok(
            r"D:\!Anime\Shinja Zero no Megami-sama to Hajimeru Isekai Kouryaku\[ASW] Shinja Zero no Megami-sama to Hajimeru Isekai Kouryaku - 01 [1080p HEVC][D30965EE].mkv",
            205896,
        ),
    ),
    (
        20,
        none(
            r"D:\!Anime\Tensei shita Daiseijo wa, Seijo de Aru Koto wo Hitakakusu\[ASW] Tensei shita Daiseijo wa, Seijo de Aru Koto wo Hitakakusu - 01 [1080p HEVC][5F734ACB].mkv",
        ),
    ),
    (
        21,
        ok(
            r"D:\!Anime\Kashita Maryoku wa Revo Barai de Kyousei Choushuu\[ASW] Kashita Maryoku wa Revo Barai de Kyousei Choushuu - 01 [1080p HEVC][E6356CB2].mkv",
            202250,
        ),
    ),
    (
        22,
        ok(
            r"D:\!Anime\Seitokai ni mo Ana wa Aru!\Even.the.Student.Council.Has.Its.Holes.S01E01.REPACK.1080p.UNCENSORED.ADN.WEB-DL.JPN.AAC2.0.H.264.MSubs-ToonsHub.mkv",
            191656,
        ),
    ),
    (
        23,
        ok(
            r"D:\!Anime\Medalist\Medalist.S01E01.A.Genius.on.the.Ice.1080p.HULU.WEB-DL.JPN.AAC2.0.H.264.ESub-ToonsHub.mkv",
            165171,
        ),
    ),
    (
        24,
        ok(
            r"D:\!Anime\Cyberpunk - Edgerunners 2\[Erai-raws] Cyberpunk - Edgerunners 2 - 01 [1080p NF WEB-DL HEVC EAC3][MultiSub][E755130E].mkv",
            195539,
        ),
    ),
    (
        25,
        ok(
            r"D:\!Anime\To Be Hero X\[ASW] To Be Hero X - 01 [1080p HEVC][73E37FCE].mkv",
            156092,
        ),
    ),
    (26, ok(r"D:\!Anime\Soul Eater\[Judas] Soul Eater - S01E01.mkv", 3588)),
    (
        27,
        ok(
            r"D:\!Anime\Darling in the FranXX\DARLING In The FRANXX - S01E01 - Alone And Lonesome.mkv",
            99423,
        ),
    ),
    (
        28,
        ok(
            r"D:\!Anime\Honzuki no Gekokujou\Honzuki no Gekokujou S1\S01E01-A World Without Books.mkv",
            108268,
        ),
    ),
    (
        29,
        ok(
            r"D:\!Anime\Honzuki no Gekokujou\Honzuki no Gekokujou S2\S02E01-Apprentice Priestess.mkv",
            113693,
        ),
    ),
    (
        30,
        ok(
            r"D:\!Anime\Honzuki no Gekokujou\Honzuki no Gekokujou S3\S03E01-The Beginning of Winter [CBD7C179].mkv",
            121176,
        ),
    ),
    (32, ok(r"D:\!Anime\Lord of Mysteries\S01E01-The Fool [8A4E8B1F].mkv", 137667)),
    (
        33,
        ok(
            r"D:\!Anime\Lord of Mysteries Specials\[DB]Guimi Zhi Zhu Specials_-_01_(10bit_1080p_x265).mkv",
            199448,
        ),
    ),
    (
        34,
        ok(
            r"D:\!Anime\Horimiya - The Missing Pieces\S01E01-Class Trip [E702628A].mkv",
            163132,
        ),
    ),
    (
        35,
        ok(
            r"D:\!Anime\Vinland Saga\Vinland Saga Season 2\[DB]Vinland Saga Season 2_-_01_(Dual Audio_10bit_BD1080p_x265).mkv",
            136430,
        ),
    ),
    (
        36,
        ok(
            r"D:\!Anime\Vinland Saga\Vinland Saga\[DB]Vinland Saga_-_01_(Dual Audio_10bit_BD1080p_x265).mkv",
            101348,
        ),
    ),
    (
        37,
        ok(
            r"D:\!Anime\Higurashi\Higurashi no Naku Koro ni\[Cleo]Higurashi_no_Naku_Koro_ni_-_01_(Dual Audio_10bit_BD720p_x265).mkv",
            934,
        ),
    ),
    (
        38,
        ok(
            r"D:\!Anime\Higurashi\Higurashi no Naku Koro ni Gou\[Nii-sama] Higurashi no Naku Koro ni Gou - 01 [1080p][BD][HEVC][10bit][FLAC][2E01273A].mkv",
            114446,
        ),
    ),
    (
        39,
        ok(
            r"D:\!Anime\Grand Blue S2\[ASW] Grand Blue S2 - 01 [1080p HEVC][A9885360].mkv",
            182309,
        ),
    ),
    (
        40,
        ok(
            r"D:\!Anime\Youjo Senki S2\[ASW] Youjo Senki S2 - 01 [1080p HEVC][9985BC61].mkv",
            135865,
        ),
    ),
    (
        41,
        remapped(
            r"D:\!Anime\Oshi no Ko\[ASW] Oshi no Ko - 13 [1080p HEVC][4913BFEF].mkv",
            166531,
            "Episode 2/13",
        ),
    ),
    (
        42,
        ok(
            r"D:\!Anime\Oshi no Ko\[ASW] Oshi no Ko S3 - 01 [1080p HEVC][23040F40].mkv",
            182587,
        ),
    ),
    // The season marker lives in the parent folder, so the query carries
    // `Season 2` and lands the 2nd Season entry exactly: an explicit marker is
    // trusted, never remapped through the walk. Its total is unknown, so the
    // display keeps the parsed episode with a `?` denominator.
    (
        43,
        ok(
            r"D:\!Anime\Kikansha no Mahou wa Tokubetsu Desu S2\[ASW] Kikansha no Mahou wa Tokubetsu desu - 13 [1080p HEVC][91DD2CA6].mkv",
            172192,
        ),
    ),
    (
        44,
        remapped(
            r"D:\!Anime\Kusuriya no Hitorigoto\[ASW] Kusuriya no Hitorigoto - 50 [1080p HEVC][D5A7FF8E].mkv",
            195516,
            "Episode 2/12",
        ),
    ),
    (
        45,
        remapped(
            r"D:\!Anime\Yasei no Last Boss ga Arawareta!\[ASW] Yasei no Last Boss ga Arawareta! - 13 [1080p HEVC][B99B8675].mkv",
            204389,
            "Episode 1/?",
        ),
    ),
    (
        46,
        remapped(
            r"D:\!Anime\Tensei Kizoku, Kantei Skill de Nariagaru\[ASW] Tensei Kizoku, Kantei Skill de Nariagaru - 25 [1080p HEVC][12EE1A3D].mkv",
            185756,
            "Episode 1/?",
        ),
    ),
    (47, ok(r"D:\!Anime\Chii Fuyo\[ASW] Chii Fuyo - 01 [1080p HEVC][F1EFA948].mkv", 207329)),
    (
        48,
        ok(
            r"D:\!Anime\Honzuki no Gekokujou\Honzuki no Gekokujou S4\[ASW] Honzuki no Gekokujou S4 - 01 [1080p HEVC][9FF15919].mkv",
            171110,
        ),
    ),
    (
        49,
        ok(
            r"D:\!Anime\Seishun Buta Yarou wa Randoseru Girl no Yume wo Minai (2023)\Seishun.Buta.Yarou.wa.Randoseru.Girl.no.Yume.wo.Minai.1080p.BDRip.10.bits.DTS.x265-EMBER.mkv",
            161474,
        ),
    ),
    // Row 20's file under a folder named the way AniList spells the title.
    // The filename's glued `Daiseijo` returns zero Candidates, so the folder's
    // `Dai Seijo` is searched and matches #187402.
    (
        50,
        ok(
            r"D:\!Anime\Tensei shita Dai Seijo wa, Seijo de Aru Koto wo Hita Kakusu\[ASW] Tensei shita Daiseijo wa, Seijo de Aru Koto wo Hitakakusu - 01 [1080p HEVC][5F734ACB].mkv",
            187402,
        ),
    ),
];

/// The recorded fixture search: normalize the query and look it up. An
/// unknown query yields zero candidates — which is exactly how the corpus'
/// recorded zero-result queries behave, so only a fixture *gap* on a row that
/// should match would surface as a failure here. Sequel chains are keyed by
/// AniList id and empty when the walk ends.
struct FixtureSource {
    fixtures: HashMap<String, Vec<Candidate>>,
    sequels: HashMap<i64, Vec<Candidate>>,
    /// The one entry an Override can fetch by id; the corpus never needs one.
    entry: Option<Candidate>,
}

impl FixtureSource {
    fn recorded() -> Self {
        Self {
            fixtures: serde_json::from_str(FIXTURES).expect("valid fixture JSON"),
            sequels: serde_json::from_str(RELATIONS).expect("valid relations JSON"),
            entry: None,
        }
    }

    fn with_entry(mut self, entry: Candidate) -> Self {
        self.entry = Some(entry);
        self
    }
}

impl MediaSource for FixtureSource {
    fn search(&mut self, query: &str) -> Result<Vec<Candidate>, SourceError> {
        Ok(self
            .fixtures
            .get(&scorer::normalize(query))
            .cloned()
            .unwrap_or_default())
    }

    fn sequels(&mut self, id: i64) -> Result<Vec<Candidate>, SourceError> {
        Ok(self.sequels.get(&id).cloned().unwrap_or_default())
    }

    fn entry(&mut self, id: i64) -> Result<Candidate, SourceError> {
        match &self.entry {
            Some(entry) if entry.id == id => Ok(entry.clone()),
            _ => Err(SourceError::Unavailable),
        }
    }
}

#[test]
fn golden_corpus_matches_the_recorded_entries() {
    let mut failures = Vec::new();
    for (n, row) in ROWS {
        let mut source = FixtureSource::recorded();
        let mut matcher = Matcher::new(&mut source);
        let metadata = matched(matcher.lookup(row.path));
        let selected = metadata.as_ref().map(|m| m.id);
        if selected != row.expect {
            failures.push(format!("row {n}: expected {:?}, got {selected:?}", row.expect));
            continue;
        }
        if let Some(episode) = row.episode {
            let actual = metadata.and_then(|m| m.episode);
            if actual.as_deref() != Some(episode) {
                failures.push(format!("row {n}: expected episode {episode:?}, got {actual:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} rows failed:\n{}",
        failures.len(),
        ROWS.len(),
        failures.join("\n")
    );
}

/// The headline absolute-remap case: absolute 50 crosses two 24-episode
/// seasons and lands on Season 3 episode 2 of 12.
#[test]
fn kusuriya_absolute_50_remaps_to_season_3_episode_2_of_12() {
    let mut source = FixtureSource::recorded();
    let mut matcher = Matcher::new(&mut source);
    let metadata = matched(matcher.lookup(
        r"D:\!Anime\Kusuriya no Hitorigoto\[ASW] Kusuriya no Hitorigoto - 50 [1080p HEVC][D5A7FF8E].mkv",
    ))
    .expect("the walk resolves the containing season");
    assert_eq!(metadata.id, 195516);
    // English leads `details` under the default title preference.
    assert_eq!(metadata.title, "The Apothecary Diaries Season 3");
    assert_eq!(metadata.episode, Some("Episode 2/12".into()));
    // The hover line comes from the entry's own metadata, fetched through
    // the SEQUEL walk: the season node, not the root, carries it.
    assert_eq!(
        metadata.hover,
        "TV • Ongoing • Fall 2026 • Drama, Mystery"
    );
}

#[test]
fn row_31_ova_is_out_of_scope_and_never_queried() {
    let mut source = FixtureSource::recorded();
    let mut matcher = Matcher::new(&mut source);
    let metadata = matched(matcher.lookup(
        r"D:\!Anime\Honzuki no Gekokujou\Honzuki no Gekokujou OVA\S01OVA-Eustachius's Incognito Operation Downtown, Visiting Missus Corinna.mkv",
    ));
    assert_eq!(metadata, None);
}

/// One Daemon wired to a real Matcher: the host every trace test drives, with
/// the Owner claim, clock and policy held constant.
fn trace_harness<'m, 's>(
    matcher: &'m mut Matcher<&'s mut FixtureSource>,
) -> (Host, Runner<'m, SharedClaim>) {
    let host = Host::new(true);
    let runner = Runner::new(
        matcher,
        host.claim(),
        PID,
        T0,
        Policy::default(),
        Vec::new(),
    );
    (host, runner)
}

/// The full block of info Diagnostics for a recorded fixture: the loaded path,
/// Release Filename parsed, each search and its Candidate count, the winning
/// Candidate's scores, and the Match. The trace is drained by the Daemon Core
/// through `MetadataClient`, so the harness here is the same one a user sees.
#[tokio::test]
async fn the_lookup_trace_pins_the_full_info_block_for_a_recorded_fixture() {
    let mut source = FixtureSource::recorded();
    let mut matcher = Matcher::new(&mut source);
    let (mut host, mut runner) = trace_harness(&mut matcher);

    let file = r"D:\!Anime\Monster\[Anime Time] Monster - 01.mkv";
    host.load(&mut runner, file).await;

    assert_eq!(
        host.infos(),
        vec![
            format!("playing: {file}"),
            r#"parsed Release Filename: title="monster", season=none, episode=1, movie=false, year=none"#.to_owned(),
            r#"search "monster" -> 10 Candidates"#.to_owned(),
            // "Monster Eater" (TV_SHORT): contained 0.85 + short-TV bonus 0.02.
            r#"chose Candidate id=19 title="Monster" score=1.000 runner-up=0.870"#.to_owned(),
            r#"Match id=19 title="Monster" episode="Episode 1/74""#.to_owned(),
        ]
    );
}

/// A search that returns zero Candidates is followed by the folder's spelling;
/// both queries and their counts appear, in the order they were tried.
#[tokio::test]
async fn a_zero_candidate_query_logs_the_fallback_query_it_tried_next() {
    let mut source = FixtureSource::recorded();
    let mut matcher = Matcher::new(&mut source);
    let (mut host, mut runner) = trace_harness(&mut matcher);

    let file = r"D:\!Anime\Tensei shita Dai Seijo wa, Seijo de Aru Koto wo Hita Kakusu\[ASW] Tensei shita Daiseijo wa, Seijo de Aru Koto wo Hitakakusu - 01 [1080p HEVC][5F734ACB].mkv";
    host.load(&mut runner, file).await;

    assert_eq!(
        host.infos(),
        vec![
            format!("playing: {file}"),
            r#"parsed Release Filename: title="tensei shita daiseijo wa seijo de aru koto wo hitakakusu", season=none, episode=1, movie=false, year=none"#.to_owned(),
            r#"search "tensei shita daiseijo wa seijo de aru koto wo hitakakusu" -> 0 Candidates"#.to_owned(),
            r#"search "tensei shita dai seijo wa seijo de aru koto wo hita kakusu" -> 1 Candidate"#.to_owned(),
            r#"chose Candidate id=187402 title="A Tale of the Secret Saint" score=1.000 runner-up=0.000"#.to_owned(),
            "Match id=187402 title=\"A Tale of the Secret Saint\" episode=\"Episode 1/?\"".to_owned(),
        ]
    );
}

/// An Override short-circuits search: its key and fetched id take the place
/// of a search Diagnostic, and the Match still closes the block.
#[tokio::test]
async fn an_override_logs_its_key_and_id_instead_of_a_search_diagnostic() {
    let (overrides, messages) = Overrides::parse(r#"{"monster": 19}"#);
    assert!(messages.is_empty());
    let source = FixtureSource::recorded();
    let entry = source.fixtures[&scorer::normalize("monster")][0].clone();
    let mut source = source.with_entry(entry);
    let mut matcher = Matcher::new(&mut source).with_overrides(overrides);
    let (mut host, mut runner) = trace_harness(&mut matcher);

    let file = r"D:\!Anime\Monster\[Anime Time] Monster - 01.mkv";
    host.load(&mut runner, file).await;

    assert_eq!(
        host.infos(),
        vec![
            format!("playing: {file}"),
            r#"parsed Release Filename: title="monster", season=none, episode=1, movie=false, year=none"#.to_owned(),
            r#"override "monster" -> id 19"#.to_owned(),
            r#"Match id=19 title="Monster" episode="Episode 1/74""#.to_owned(),
        ]
    );
}

/// A dead Override id was never fetched, so no fetch Diagnostic is written.
#[tokio::test]
async fn a_dead_override_id_logs_no_fetch_diagnostic() {
    let (overrides, messages) = Overrides::parse(r#"{"monster": 19}"#);
    assert!(messages.is_empty());
    let mut source = FixtureSource::recorded();
    let mut matcher = Matcher::new(&mut source).with_overrides(overrides);
    let (mut host, mut runner) = trace_harness(&mut matcher);

    let file = r"D:\!Anime\Monster\[Anime Time] Monster - 01.mkv";
    host.load(&mut runner, file).await;

    assert_eq!(
        host.infos(),
        vec![
            format!("playing: {file}"),
            r#"parsed Release Filename: title="monster", season=none, episode=1, movie=false, year=none"#.to_owned(),
        ]
    );
    assert_eq!(
        host.logs(),
        vec![format!(
            "no Anime match (the override for \"monster\" names no AniList entry): {file}"
        )]
    );
}

/// The Miss report stays exactly where it was; the trace before it is what
/// explains the two searches and their Candidate counts.
#[tokio::test]
async fn a_miss_keeps_its_error_and_osd_after_the_trace() {
    let mut source = FixtureSource::recorded();
    let mut matcher = Matcher::new(&mut source);
    let (mut host, mut runner) = trace_harness(&mut matcher);

    let file = r"D:\!Anime\[Trix] Demon Slayer Infinity Castle I [WEBRip-1080p][HEVC x265 10bit][Multi-Subs] (Dual Audio).mkv";
    host.load(&mut runner, file).await;

    assert_eq!(
        host.infos(),
        vec![
            format!("playing: {file}"),
            r#"parsed Release Filename: title="demon slayer infinity castle i", season=none, episode=none, movie=true, year=none"#.to_owned(),
            r#"search "demon slayer infinity castle i" -> 0 Candidates"#.to_owned(),
            r#"search "anime" -> 10 Candidates"#.to_owned(),
        ]
    );
    assert_eq!(
        host.logs(),
        vec![r#"no Anime match for "anime""#.to_owned()]
    );
    assert_eq!(
        host.osds(),
        vec![(r#"Surtr: no Anime match for "anime""#.to_owned(), 5)]
    );
}
