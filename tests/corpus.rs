//! Golden match corpus: the 48 scored rows of the research corpus, matched
//! offline against the recorded AniList search fixtures
//! (`fixtures/anilist-corpus.json`, regenerate with `fixtures/generate.mjs`)
//! and the recorded `SEQUEL` chains (`fixtures/anilist-relations.json`).
//!
//! Expectations come from the measured pipeline in
//! `.scratch/surtr/research/match-quality.md` (agreed scorer, margin ≥ 0.10):
//! 44 rows accept their expected entry, rows 4/18/20/28 yield no Match. Rows
//! 41/44/45/46 are absolute-numbered: the Parsed Episode exceeds the Root
//! Season's total, the `SEQUEL` walk finds the containing season, and the
//! episode is remapped to the Season Episode (Oshi no Ko 2nd Season, Kusuriya
//! 3rd Season, Yasei 2nd Season, Tensei Kizoku 3rd Season). Row 43's folder
//! marker resolves its season directly, so it never walks.

use std::collections::HashMap;

use surtr::anilist::Candidate;
use surtr::daemon::MetadataClient;
use surtr::matcher::{Matcher, MediaSource};
use surtr::scorer;

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
    (
        18,
        none(
            r"D:\!Anime\Eiyuu Ou, Bu o Kiwameru Tame Tenseisu - Soshite, Sekai Saikyou no Minarai Kishi\[Anime Time] Eiyuu Ou, Bu o Kiwameru Tame Tenseisu - Soshite, Sekai Saikyou no Minarai Kishi - 01.mkv",
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
        none(
            r"D:\!Anime\Honzuki no Gekokujou\Honzuki no Gekokujou S1\S01E01-A World Without Books.mkv",
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
];

/// The recorded fixture search: normalize the query and look it up. An
/// unknown query yields zero candidates — which is exactly how the corpus'
/// recorded zero-result queries behave, so only a fixture *gap* on a row that
/// should match would surface as a failure here. Sequel chains are keyed by
/// AniList id and empty when the walk ends.
struct FixtureSource {
    fixtures: HashMap<String, Vec<Candidate>>,
    sequels: HashMap<i64, Vec<Candidate>>,
}

impl FixtureSource {
    fn recorded() -> Self {
        Self {
            fixtures: serde_json::from_str(FIXTURES).expect("valid fixture JSON"),
            sequels: serde_json::from_str(RELATIONS).expect("valid relations JSON"),
        }
    }
}

impl MediaSource for FixtureSource {
    fn search(&mut self, query: &str) -> Option<Vec<Candidate>> {
        Some(
            self.fixtures
                .get(&scorer::normalize(query))
                .cloned()
                .unwrap_or_default(),
        )
    }

    fn sequels(&mut self, id: i64) -> Option<Vec<Candidate>> {
        Some(self.sequels.get(&id).cloned().unwrap_or_default())
    }

    fn entry(&mut self, _id: i64) -> Option<Candidate> {
        None
    }
}

#[test]
fn golden_corpus_matches_the_recorded_entries() {
    let mut failures = Vec::new();
    for (n, row) in ROWS {
        let mut source = FixtureSource::recorded();
        let mut matcher = Matcher::new(&mut source);
        let metadata = matcher.lookup(row.path);
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

/// The ticket's headline case: absolute 50 crosses two 24-episode seasons and
/// lands on Season 3 episode 2 of 12.
#[test]
fn kusuriya_absolute_50_remaps_to_season_3_episode_2_of_12() {
    let mut source = FixtureSource::recorded();
    let mut matcher = Matcher::new(&mut source);
    let metadata = matcher
        .lookup(
            r"D:\!Anime\Kusuriya no Hitorigoto\[ASW] Kusuriya no Hitorigoto - 50 [1080p HEVC][D5A7FF8E].mkv",
        )
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
    let metadata = matcher.lookup(
        r"D:\!Anime\Honzuki no Gekokujou\Honzuki no Gekokujou OVA\S01OVA-Eustachius's Incognito Operation Downtown, Visiting Missus Corinna.mkv",
    );
    assert_eq!(metadata, None);
}
