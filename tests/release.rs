//! Golden filename-parsing tests: all 49 rows of the corpus in
//! `.scratch/surtr/research/filename-corpus.md`, plus the Anime Root gate.
//!
//! The corpus' expected AniList entry is issue 03's target; here the contract
//! is the *query* the parser builds — the cleaned title, the season marker and
//! the Parsed Episode — and the files that must yield no Presence at all.

use surtr::release::{self, ParsedRelease, Rejected};

/// One corpus row: path, expected cleaned query, expected episode.
#[derive(Debug)]
struct Row {
    n: u8,
    path: &'static str,
    title: &'static str,
    episode: Option<u32>,
}

const ROWS: &[Row] = &[
    Row {
        n: 1,
        path: r"D:\!Anime\Clannad\[Trix] Clannad Movie (2007) [49889AC3].mkv",
        title: "clannad movie",
        episode: None,
    },
    Row {
        n: 2,
        path: r"D:\!Anime\[Anime Time] Gekijouban Sword Art Online Progressive - Kuraki Yuuyami no Scherzo (2022) [BD] [1080p][HEVC 10bit x265][AAC][Eng Sub].mkv",
        title: "gekijouban sword art online progressive kuraki yuuyami no scherzo",
        episode: None,
    },
    Row {
        n: 3,
        path: r"D:\!Anime\[Judas] Jujutsu Kaisen - Movie 01 - Jujutsu Kaisen 0 [BD 1080p][HEVC x265 10bit][Dual-Audio][Multi-Subs].mkv",
        title: "jujutsu kaisen jujutsu kaisen 0",
        episode: None,
    },
    Row {
        n: 4,
        path: r"D:\!Anime\[Trix] Demon Slayer Infinity Castle I [WEBRip 1080p AV1] (Dual Audio, Multi Subs).mkv",
        title: "demon slayer infinity castle i",
        episode: None,
    },
    Row {
        n: 5,
        path: r"D:\!Anime\Cosmic.Princess.Kaguya.2026.1080p.NF.WEB-DL.DUAL.DDP5.1.H.264-VARYG.mkv",
        title: "cosmic princess kaguya",
        episode: None,
    },
    Row {
        n: 6,
        path: r"D:\!Anime\[BDFansub] Kimi no Na wa [1080p][AAC][x265].mkv",
        title: "kimi no na wa",
        episode: None,
    },
    Row {
        n: 7,
        path: r"D:\!Anime\[ASW] Spy x Family - Code White [1080p HEVC][D4935429].mkv",
        title: "spy x family code white",
        episode: None,
    },
    Row {
        n: 8,
        path: r"D:\!Anime\[ASW] Ansatsu Kyoushitsu Movie - Minna no Jikan [1080p HEVC][DF86E022].mkv",
        title: "ansatsu kyoushitsu movie minna no jikan",
        episode: None,
    },
    Row {
        n: 9,
        path: r"D:\!Anime\[Judas] No Game No Life Zero.mkv",
        title: "no game no life zero",
        episode: None,
    },
    Row {
        n: 10,
        path: r"D:\!Anime\[Judas] Sword Art Online - Movie 01 (Ordinal Scale) [BD 1080p][HEVC x265 10bit][Dual-Audio][Eng-Subs].mkv",
        title: "sword art online ordinal scale",
        episode: None,
    },
    Row {
        n: 11,
        path: r"D:\!Anime\Black Clover\Black Clover S1\[Judas] Black Clover - 001.mkv",
        title: "black clover",
        episode: Some(1),
    },
    Row {
        n: 12,
        path: r"D:\!Anime\Black Clover S2\[Judas] Black Clover - S02E01.mkv",
        title: "black clover season 2",
        episode: Some(1),
    },
    Row {
        n: 13,
        path: r"D:\!Anime\Black Clover\[Judas] Black Clover - Sword of the Wizard King [1080p][HEVC x265 10bit][Dual-Audio][Multi-Subs].mkv",
        title: "black clover sword of the wizard king",
        episode: None,
    },
    Row {
        n: 14,
        path: r"D:\!Anime\Monster\[Anime Time] Monster - 01.mkv",
        title: "monster",
        episode: Some(1),
    },
    Row {
        n: 15,
        path: r"D:\!Anime\Violet Evergarden\[Sokudo] Violet Evergarden - 01 [1080p BD][AV1][dual audio].mkv",
        title: "violet evergarden",
        episode: Some(1),
    },
    Row {
        n: 16,
        path: r"D:\!Anime\Shunkashuutou Daikousha - Haru no Mai\[ASW] Shunkashuutou Daikousha - Haru no Mai - 01 [1080p HEVC][44DFED18].mkv",
        title: "shunkashuutou daikousha haru no mai",
        episode: Some(1),
    },
    Row {
        n: 17,
        path: r"D:\!Anime\Doumo, Suki na Hito ni Horegusuri wo Irai sareta Majo desu\[ASW] Doumo, Suki na Hito ni Horegusuri wo Irai sareta Majo desu. - 01 [1080p HEVC][BC2C1B27].mkv",
        title: "doumo suki na hito ni horegusuri wo irai sareta majo desu",
        episode: Some(1),
    },
    Row {
        n: 18,
        path: r"D:\!Anime\Eiyuu Ou, Bu o Kiwameru Tame Tenseisu - Soshite, Sekai Saikyou no Minarai Kishi\[Anime Time] Eiyuu Ou, Bu o Kiwameru Tame Tenseisu - Soshite, Sekai Saikyou no Minarai Kishi - 01.mkv",
        title: "eiyuu ou bu o kiwameru tame tenseisu soshite sekai saikyou no minarai kishi",
        episode: Some(1),
    },
    Row {
        n: 19,
        path: r"D:\!Anime\Shinja Zero no Megami-sama to Hajimeru Isekai Kouryaku\[ASW] Shinja Zero no Megami-sama to Hajimeru Isekai Kouryaku - 01 [1080p HEVC][D30965EE].mkv",
        title: "shinja zero no megami sama to hajimeru isekai kouryaku",
        episode: Some(1),
    },
    Row {
        n: 20,
        path: r"D:\!Anime\Tensei shita Daiseijo wa, Seijo de Aru Koto wo Hitakakusu\[ASW] Tensei shita Daiseijo wa, Seijo de Aru Koto wo Hitakakusu - 01 [1080p HEVC][5F734ACB].mkv",
        title: "tensei shita daiseijo wa seijo de aru koto wo hitakakusu",
        episode: Some(1),
    },
    Row {
        n: 21,
        path: r"D:\!Anime\Kashita Maryoku wa Revo Barai de Kyousei Choushuu\[ASW] Kashita Maryoku wa Revo Barai de Kyousei Choushuu - 01 [1080p HEVC][E6356CB2].mkv",
        title: "kashita maryoku wa revo barai de kyousei choushuu",
        episode: Some(1),
    },
    Row {
        n: 22,
        path: r"D:\!Anime\Seitokai ni mo Ana wa Aru!\Even.the.Student.Council.Has.Its.Holes.S01E01.REPACK.1080p.UNCENSORED.ADN.WEB-DL.JPN.AAC2.0.H.264.MSubs-ToonsHub.mkv",
        title: "even the student council has its holes",
        episode: Some(1),
    },
    Row {
        n: 23,
        path: r"D:\!Anime\Medalist\Medalist.S01E01.A.Genius.on.the.Ice.1080p.HULU.WEB-DL.JPN.AAC2.0.H.264.ESub-ToonsHub.mkv",
        title: "medalist",
        episode: Some(1),
    },
    Row {
        n: 24,
        path: r"D:\!Anime\Cyberpunk - Edgerunners 2\[Erai-raws] Cyberpunk - Edgerunners 2 - 01 [1080p NF WEB-DL HEVC EAC3][MultiSub][E755130E].mkv",
        title: "cyberpunk edgerunners 2",
        episode: Some(1),
    },
    Row {
        n: 25,
        path: r"D:\!Anime\To Be Hero X\[ASW] To Be Hero X - 01 [1080p HEVC][73E37FCE].mkv",
        title: "to be hero x",
        episode: Some(1),
    },
    Row {
        n: 26,
        path: r"D:\!Anime\Soul Eater\[Judas] Soul Eater - S01E01.mkv",
        title: "soul eater",
        episode: Some(1),
    },
    Row {
        n: 27,
        path: r"D:\!Anime\Darling in the FranXX\DARLING In The FRANXX - S01E01 - Alone And Lonesome.mkv",
        title: "darling in the franxx",
        episode: Some(1),
    },
    Row {
        n: 28,
        path: r"D:\!Anime\Honzuki no Gekokujou\Honzuki no Gekokujou S1\S01E01-A World Without Books.mkv",
        title: "honzuki no gekokujou",
        episode: Some(1),
    },
    Row {
        n: 29,
        path: r"D:\!Anime\Honzuki no Gekokujou\Honzuki no Gekokujou S2\S02E01-Apprentice Priestess.mkv",
        title: "honzuki no gekokujou season 2",
        episode: Some(1),
    },
    Row {
        n: 30,
        path: r"D:\!Anime\Honzuki no Gekokujou\Honzuki no Gekokujou S3\S03E01-The Beginning of Winter [CBD7C179].mkv",
        title: "honzuki no gekokujou season 3",
        episode: Some(1),
    },
    Row {
        n: 32,
        path: r"D:\!Anime\Lord of Mysteries\S01E01-The Fool [8A4E8B1F].mkv",
        title: "lord of mysteries",
        episode: Some(1),
    },
    Row {
        n: 33,
        path: r"D:\!Anime\Lord of Mysteries Specials\[DB]Guimi Zhi Zhu Specials_-_01_(10bit_1080p_x265).mkv",
        title: "guimi zhi zhu specials",
        episode: Some(1),
    },
    Row {
        n: 34,
        path: r"D:\!Anime\Horimiya - The Missing Pieces\S01E01-Class Trip [E702628A].mkv",
        title: "horimiya the missing pieces",
        episode: Some(1),
    },
    Row {
        n: 35,
        path: r"D:\!Anime\Vinland Saga\Vinland Saga Season 2\[DB]Vinland Saga Season 2_-_01_(Dual Audio_10bit_BD1080p_x265).mkv",
        title: "vinland saga season 2",
        episode: Some(1),
    },
    Row {
        n: 36,
        path: r"D:\!Anime\Vinland Saga\Vinland Saga\[DB]Vinland Saga_-_01_(Dual Audio_10bit_BD1080p_x265).mkv",
        title: "vinland saga",
        episode: Some(1),
    },
    Row {
        n: 37,
        path: r"D:\!Anime\Higurashi\Higurashi no Naku Koro ni\[Cleo]Higurashi_no_Naku_Koro_ni_-_01_(Dual Audio_10bit_BD720p_x265).mkv",
        title: "higurashi no naku koro ni",
        episode: Some(1),
    },
    Row {
        n: 38,
        path: r"D:\!Anime\Higurashi\Higurashi no Naku Koro ni Gou\[Nii-sama] Higurashi no Naku Koro ni Gou - 01 [1080p][BD][HEVC][10bit][FLAC][2E01273A].mkv",
        title: "higurashi no naku koro ni gou",
        episode: Some(1),
    },
    Row {
        n: 39,
        path: r"D:\!Anime\Grand Blue S2\[ASW] Grand Blue S2 - 01 [1080p HEVC][A9885360].mkv",
        title: "grand blue season 2",
        episode: Some(1),
    },
    Row {
        n: 40,
        path: r"D:\!Anime\Youjo Senki S2\[ASW] Youjo Senki S2 - 01 [1080p HEVC][9985BC61].mkv",
        title: "youjo senki season 2",
        episode: Some(1),
    },
    Row {
        n: 41,
        path: r"D:\!Anime\Oshi no Ko\[ASW] Oshi no Ko - 13 [1080p HEVC][4913BFEF].mkv",
        title: "oshi no ko",
        episode: Some(13),
    },
    Row {
        n: 42,
        path: r"D:\!Anime\Oshi no Ko\[ASW] Oshi no Ko S3 - 01 [1080p HEVC][23040F40].mkv",
        title: "oshi no ko season 3",
        episode: Some(1),
    },
    Row {
        n: 43,
        path: r"D:\!Anime\Kikansha no Mahou wa Tokubetsu Desu S2\[ASW] Kikansha no Mahou wa Tokubetsu desu - 13 [1080p HEVC][91DD2CA6].mkv",
        title: "kikansha no mahou wa tokubetsu desu season 2",
        episode: Some(13),
    },
    // Absolute episode 50 with no season marker and no `[12]`-style tag:
    // whether it is season 1 or a later season is season resolution (03).
    Row {
        n: 44,
        path: r"D:\!Anime\Kusuriya no Hitorigoto\[ASW] Kusuriya no Hitorigoto - 50 [1080p HEVC][D5A7FF8E].mkv",
        title: "kusuriya no hitorigoto",
        episode: Some(50),
    },
    Row {
        n: 45,
        path: r"D:\!Anime\Yasei no Last Boss ga Arawareta!\[ASW] Yasei no Last Boss ga Arawareta! - 13 [1080p HEVC][B99B8675].mkv",
        title: "yasei no last boss ga arawareta",
        episode: Some(13),
    },
    Row {
        n: 46,
        path: r"D:\!Anime\Tensei Kizoku, Kantei Skill de Nariagaru\[ASW] Tensei Kizoku, Kantei Skill de Nariagaru - 25 [1080p HEVC][12EE1A3D].mkv",
        title: "tensei kizoku kantei skill de nariagaru",
        episode: Some(25),
    },
    Row {
        n: 47,
        path: r"D:\!Anime\Chii Fuyo\[ASW] Chii Fuyo - 01 [1080p HEVC][F1EFA948].mkv",
        title: "chii fuyo",
        episode: Some(1),
    },
    Row {
        n: 48,
        path: r"D:\!Anime\Honzuki no Gekokujou\Honzuki no Gekokujou S4\[ASW] Honzuki no Gekokujou S4 - 01 [1080p HEVC][9FF15919].mkv",
        title: "honzuki no gekokujou season 4",
        episode: Some(1),
    },
    Row {
        n: 49,
        path: r"D:\!Anime\Seishun Buta Yarou wa Randoseru Girl no Yume wo Minai (2023)\Seishun.Buta.Yarou.wa.Randoseru.Girl.no.Yume.wo.Minai.1080p.BDRip.10.bits.DTS.x265-EMBER.mkv",
        title: "seishun buta yarou wa randoseru girl no yume wo minai",
        episode: None,
    },
];

#[test]
fn golden_corpus_builds_the_expected_query() {
    let mut failures = Vec::new();
    for row in ROWS {
        match release::parse(row.path) {
            Ok(ParsedRelease {
                title,
                episode,
                movie,
                ..
            }) => {
                if title != row.title || episode != row.episode {
                    failures.push(format!(
                        "row {}: expected {row:?} got title {title:?} episode {episode:?}",
                        row.n
                    ));
                }
                if movie != row.episode.is_none() {
                    failures.push(format!("row {}: movie flag disagrees with episode", row.n));
                }
            }
            Err(err) => failures.push(format!("row {}: rejected {err:?}", row.n)),
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_corpus_titled_file_parses_but_the_ova() {
    // Row 31 writes `S01OVA` where an episode number goes: no numeric episode,
    // and the map says no Presence.
    let ova = r"D:\!Anime\Honzuki no Gekokujou\Honzuki no Gekokujou OVA\S01OVA-Eustachius's Incognito Operation Downtown, Visiting Missus Corinna.mkv";
    let parsed = release::parse(ova).unwrap();
    assert_eq!(parsed.title, "honzuki no gekokujou ova");
    assert_eq!(parsed.episode, None);
    assert!(!parsed.movie, "an OVA is not a Movie");
}

#[test]
fn corpus_has_48_parseable_rows() {
    assert_eq!(ROWS.len(), 48);
}

// ---------------------------------------------------------------------------
// Anime Root gate
// ---------------------------------------------------------------------------

fn roots(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn root_membership_is_per_component_and_case_insensitive() {
    let gate = roots(&[r"D:\Anime"]);
    assert!(release::within_roots(r"D:\Anime\Frieren\ep01.mkv", &gate));
    assert!(release::within_roots(r"d:\anime\Frieren\ep01.mkv", &gate));
    assert!(release::within_roots(r"D:\ANIME\a\b\c\d\ep01.mkv", &gate));
    assert!(!release::within_roots(r"D:\Anime2\ep01.mkv", &gate));
    assert!(!release::within_roots(r"D:\Anime ep01.mkv", &gate));
    assert!(
        !release::within_roots(r"D:\Anime", &gate),
        "the root itself is not a file"
    );
}

#[test]
fn multiple_roots_and_trailing_separators() {
    let gate = roots(&[r"D:\Anime\", r"E:\Shows"]);
    assert!(release::within_roots(r"E:\Shows\Frieren - 01.mkv", &gate));
    assert!(release::within_roots(r"D:\Anime\Frieren - 01.mkv", &gate));
    assert!(!release::within_roots(r"F:\Anime\Frieren - 01.mkv", &gate));
}

#[test]
fn outside_roots_urls_and_streams_are_gated_out() {
    let gate = roots(&[r"D:\!Anime"]);
    for path in [
        "https://example.com/stream.m3u8",
        r"http://tracker.example/a.torrent",
        r"\\nas\anime\Frieren - 01.mkv",
        "file:///D:/Anime/Frieren - 01.mkv",
        "",
    ] {
        assert!(
            !release::within_roots(path, &gate),
            "{path} should not be inside the root"
        );
        assert!(
            matches!(
                release::parse(path),
                Err(Rejected::NotLocal | Rejected::NoFilename)
            ),
            "{path} should not parse"
        );
    }
    assert!(!release::within_roots(r"D:\!Anime\Frieren - 01.mkv", &[]));
}

#[test]
fn a_file_outside_the_roots_never_reaches_the_parser() {
    // The gate runs first: a parsable file elsewhere is simply not eligible.
    let gate = roots(&[r"D:\!Anime"]);
    let file = r"D:\HomeVideos\birthday.mp4";
    assert!(!release::within_roots(file, &gate));
    assert!(
        release::parse(file).is_ok(),
        "parsing alone is not the gate"
    );
}

// ---------------------------------------------------------------------------
// Parser edges
// ---------------------------------------------------------------------------

#[test]
fn a_glued_release_spelling_keeps_the_folders_spelling_as_a_fallback() {
    // AniList search finds `Dai Seijo`, never the release's glued `Daiseijo`,
    // so the folder's spelling is the parser's second query.
    let parsed = release::parse(
        r"D:\!Anime\Tensei shita Dai Seijo wa, Seijo de Aru Koto wo Hita Kakusu\[ASW] Tensei shita Daiseijo wa, Seijo de Aru Koto wo Hitakakusu - 05 [1080p HEVC][5F734ACB].mkv",
    )
    .unwrap();
    assert_eq!(
        parsed.title,
        "tensei shita daiseijo wa seijo de aru koto wo hitakakusu"
    );
    assert_eq!(
        parsed.folder.as_deref(),
        Some("tensei shita dai seijo wa seijo de aru koto wo hita kakusu")
    );
}

#[test]
fn a_folder_repeating_the_files_title_is_no_fallback() {
    // The corpus' ordinary shape: the folder and the filename name the same
    // show, so there is nothing new to search.
    let parsed = release::parse(r"D:\!Anime\Monster\[Anime Time] Monster - 01.mkv").unwrap();
    assert_eq!(parsed.title, "monster");
    assert_eq!(parsed.folder, None);
}

#[test]
fn a_title_less_basename_has_no_separate_folder_fallback() {
    // The folder already *is* the title here; a second query would repeat it.
    let parsed = release::parse(r"D:\!Anime\Show\Show S2\S02E05-Episode Title.mkv").unwrap();
    assert_eq!(parsed.title, "show season 2");
    assert_eq!(parsed.folder, None);
}

#[test]
fn parent_folder_fallback_supplies_title_and_season() {
    let parsed = release::parse(r"D:\!Anime\Show\Show S2\S02E05-Episode Title.mkv").unwrap();
    assert_eq!(parsed.title, "show season 2");
    assert_eq!(parsed.season, Some(2));
    assert_eq!(parsed.episode, Some(5));
    assert!(!parsed.movie);

    // The season comes from the parent when the filename carries none.
    let parsed =
        release::parse(r"D:\!Anime\Vinland Saga\Vinland Saga Season 2\[DB] - 01.mkv").unwrap();
    assert_eq!(parsed.title, "vinland saga season 2");
    assert_eq!(parsed.episode, Some(1));
}

#[test]
fn seasons_normalize_across_ordinals_and_romans() {
    for (path, title, season) in [
        (
            r"D:\A\Show Season 2\[X] Show Season 2 - 01.mkv",
            "show season 2",
            2,
        ),
        (
            r"D:\A\Show\[X] Show 2nd Season - 01.mkv",
            "show season 2",
            2,
        ),
        (r"D:\A\Show\[X] Show II - 01.mkv", "show season 2", 2),
        (r"D:\A\Show\[X] Show S2 - 01.mkv", "show season 2", 2),
        (r"D:\A\Show\[X] Show Season 1 - 01.mkv", "show", 1),
        (
            r"D:\A\Show\[X] Show Second Season - 01.mkv",
            "show season 2",
            2,
        ),
    ] {
        let parsed = release::parse(path).unwrap_or_else(|e| panic!("{path}: {e:?}"));
        assert_eq!(parsed.title, title, "{path}");
        assert_eq!(parsed.season, Some(season), "{path}");
    }
}

#[test]
fn year_never_reaches_the_query() {
    for path in [
        r"D:\A\Clannad\[Trix] Clannad Movie (2007) [49889AC3].mkv",
        r"D:\A\Cosmic.Princess.Kaguya.2026.1080p.NF.WEB-DL.DUAL.DDP5.1.H.264-VARYG.mkv",
        r"D:\A\Movie (2023)\Some Movie 2023.mkv",
    ] {
        let parsed = release::parse(path).unwrap_or_else(|e| panic!("{path}: {e:?}"));
        assert!(
            !parsed.title.contains(char::is_numeric) || !parsed.title.contains("20"),
            "{path} → {:?}",
            parsed.title
        );
    }
}

#[test]
fn the_release_year_is_read_off_the_basename_with_word_boundaries() {
    let parsed = release::parse(r"D:\A\Clannad\[Trix] Clannad Movie (2007) [49889AC3].mkv").unwrap();
    assert_eq!(parsed.year, Some(2007));

    // A resolution claim is not a year: no word boundary after `1920`.
    let parsed = release::parse(r"D:\A\Show\[X] Show - 01 [1920x1080].mkv").unwrap();
    assert_eq!(parsed.year, None);
}

#[test]
fn a_versioned_episode_is_the_same_episode_re_released() {
    // `01v2` is fansub convention for a corrected re-release of episode 01,
    // not a multi-episode file.
    let parsed = release::parse(r"D:\A\Show\[X] Show - 01v2 [1080p].mkv").unwrap();
    assert_eq!(parsed.episode, Some(1));
    assert_eq!(parsed.title, "show");

    let parsed = release::parse(
        r"D:\!Anime\Kusuriya no Hitorigoto\[ASW] Kusuriya no Hitorigoto - 49v2 [1080p HEVC][B44A3854].mkv",
    )
    .unwrap();
    assert_eq!(parsed.title, "kusuriya no hitorigoto");
    assert_eq!(parsed.episode, Some(49));
}

#[test]
fn out_of_scope_releases_are_rejected() {
    for (path, what) in [
        (r"D:\A\Show\[X] Show - 01-12 [1080p].mkv", "range"),
        (r"D:\A\Show\[X] Show - 01 & 02 [1080p].mkv", "range"),
        (r"D:\A\Show\[X] Show [1080p.mkv", "bracket"),
        (r"D:\A\[X][1080p].mkv", "no title"),
        (r"D:\A\Show\[X] Show - [OAD].mkv", "no episode"),
    ] {
        assert!(
            release::parse(path).is_err(),
            "{what}: {path} should be rejected"
        );
    }
}

#[test]
fn title_tokens_keep_words_that_look_like_metadata_inside_the_name() {
    // `No Game No Life Zero` and `Monster` must not lose words; `11eyes` and
    // `Sword`/`Art` survive because only whole tokens are stripped.
    for (path, title) in [
        (
            r"D:\A\[Judas] No Game No Life Zero.mkv",
            "no game no life zero",
        ),
        (r"D:\A\Monster\[X] Monster - 01.mkv", "monster"),
        (r"D:\A\[X] Sword Art Online - 01.mkv", "sword art online"),
    ] {
        assert_eq!(release::parse(path).unwrap().title, title);
    }
}

#[test]
fn the_filename_season_marker_wins_over_the_folder_marker() {
    let parsed = release::parse(r"D:\A\Show S3\[X] Show S2 - 01.mkv").unwrap();
    assert_eq!(parsed.season, Some(2));
    assert_eq!(parsed.title, "show season 2");
}

#[test]
fn a_four_digit_episode_number_parses_but_a_year_does_not() {
    let parsed = release::parse(r"D:\A\ONE PIECE\[X] ONE PIECE - 1071.mkv").unwrap();
    assert_eq!(parsed.episode, Some(1071));
    assert!(!parsed.movie);

    let parsed = release::parse(r"D:\A\Show\[X] Show - 2026.mkv").unwrap();
    assert_eq!(parsed.episode, None);
}

#[test]
fn title_season_reads_the_same_marker_grammar() {
    for (title, season) in [
        ("Youjo Senki II", Some(2)),
        ("[Oshi no Ko] 3rd Season", Some(3)),
        ("VINLAND SAGA SEASON 2", Some(2)),
        ("Ascendance of a Bookworm Season 4", Some(4)),
        ("Kusuriya no Hitorigoto", None),
        ("Cyberpunk: Edgerunners 2", None),
        ("To Be Hero X", None),
    ] {
        assert_eq!(release::title_season(title), season, "{title}");
    }
}
