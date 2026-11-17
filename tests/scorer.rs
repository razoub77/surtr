//! Candidate scoring and acceptance tests: recorded AniList shapes in,
//! accepted Matches (or None) out. The numbers mirror the measured config in
//! `.scratch/surtr/research/match-quality.md`.

use surtr::anilist::Candidate;
use surtr::scorer::{self, Context};

fn candidate(id: i64, romaji: &str) -> Candidate {
    Candidate {
        id,
        format: Some("TV".into()),
        season_year: None,
        season: None,
        status: None,
        episodes: Some(12),
        genres: Vec::new(),
        title: Some(surtr::anilist::Titles {
            romaji: Some(romaji.into()),
            english: None,
        }),
        synonyms: Vec::new(),
        cover_image: None,
    }
}

fn ctx() -> Context {
    Context {
        year: None,
        movie: false,
        episodic: true,
        season: None,
    }
}

#[test]
fn exact_normalized_title_match_is_accepted() {
    let candidates = [
        candidate(1, "Frieren: Beyond Journey's End"),
        candidate(2, "Sousou no Frieren"),
    ];
    let selected = scorer::select("sousou no frieren", &candidates, &ctx());
    assert_eq!(selected.map(|c| c.id), Some(2));
}

#[test]
fn token_containment_scores_085_and_accepts() {
    // Every query token appears in the candidate title: containment.
    let candidates = [candidate(7, "Vinland Saga Season 2")];
    let selected = scorer::select("vinland saga", &candidates, &ctx());
    assert_eq!(selected.map(|c| c.id), Some(7));
}

#[test]
fn low_fuzzy_similarity_is_rejected() {
    // Corpus row 47: "Chii Fuyo" against its real title is a 0.097 fuzzy.
    let candidates = [candidate(47, "Chii's Sweet Home")];
    assert_eq!(scorer::select("chii fuyo", &candidates, &ctx()), None);
}

#[test]
fn a_score_tie_never_resolves() {
    // Two Candidates whose titles both contain the query tokens tie at 0.85:
    // no arbitrary winner, no Match.
    let candidates = [
        candidate(1, "Honzuki no Gekokujou Fanbook"),
        candidate(2, "Honzuki no Gekokujou Anthology"),
    ];
    assert_eq!(scorer::select("honzuki no gekokujou", &candidates, &ctx()), None);
}

#[test]
fn a_sub_floor_tie_is_never_rescued_by_the_root_season() {
    // Both titles stay below ACCEPT; a synonym naming a later season must not
    // make the other the Match. The floor binds the tie rung too.
    let later = Candidate {
        synonyms: vec!["Chii's Sweet Home Season 2".into()],
        ..candidate(2, "Chii's Sweet Home")
    };
    let candidates = [candidate(1, "Chii's Sweet Home"), later];
    assert_eq!(scorer::select("chii fuyo", &candidates, &ctx()), None);
}

#[test]
fn a_tie_for_a_season_1_file_falls_to_the_root_season() {
    // Corpus row 28: the folder's `Honzuki no Gekokujou S1` queries the plain
    // franchise name, so four TV seasons contain it equally at 0.87; only the
    // first carries no season ordinal in any title.
    let later = |id: i64, romaji: &str, synonym: &str| Candidate {
        synonyms: vec![synonym.into()],
        ..candidate(id, romaji)
    };
    let candidates = [
        candidate(
            108268,
            "Honzuki no Gekokujou: Shisho ni Naru Tame ni wa Shudan wo Erandeiraremasen",
        ),
        later(
            113693,
            "Honzuki no Gekokujou: Shisho ni Naru Tame ni wa Shudan wo Erandeiraremasen 2nd Season",
            "Ascendance of a Bookworm Season 2",
        ),
        later(
            121176,
            "Honzuki no Gekokujou: Shisho ni Naru Tame ni wa Shudan wo Erandeiraremasen 3rd Season",
            "Ascendance of a Bookworm Season 3",
        ),
        later(
            171110,
            "Honzuki no Gekokujou: Ryoushu no Youjo",
            "Ascendance of a Bookworm Season 4",
        ),
    ];
    let selected = scorer::select("honzuki no gekokujou", &candidates, &ctx());
    assert_eq!(selected.map(|c| c.id), Some(108268));
}

#[test]
fn a_tie_does_not_fall_to_the_root_season_for_a_later_season() {
    // An explicit `Season 2` file may tie too, but the Root Season default is
    // a season-1 answer only: a later season's tie stays unresolved.
    let candidates = [
        candidate(1, "Some Show Alpha"),
        candidate(2, "Some Show Part II"),
    ];
    let context = Context {
        season: Some(2),
        ..ctx()
    };
    assert_eq!(scorer::select("some show", &candidates, &context), None);
}

#[test]
fn a_small_margin_is_rejected() {
    // 1.0 vs 0.926 is inside the 0.10 margin.
    let top = candidate(142193, "Eiyuu Ou, Bu o Kiwameru Tame Tenseisu: Soshite, Sekai Saikyou no Minarai Kishi");
    let runner = candidate(999, "Eiyuu Wo, Bu o Kiwameru Tame Tenseisu: Soshite, Sekai Saikyou no Minarai Kishi");
    assert_eq!(scorer::select("eiyuu ou bu o kiwameru tame tenseisu soshite sekai saikyou no minarai kishi", &[top, runner], &ctx()), None);
}

#[test]
fn the_two_spellings_of_the_object_particle_score_equal() {
    // Corpus row 18: the filename's `Bu o Kiwameru` is AniList's `Bu wo
    // Kiwameru`; folding them makes the primary title land exactly.
    let real = candidate(142193, "Eiyuu-ou, Bu wo Kiwameru Tame Tenseisu: Soshite, Sekai Saikyou no Minarai Kishi");
    let candidates = [real];
    let selected = scorer::select("eiyuu ou bu o kiwameru tame tenseisu soshite sekai saikyou no minarai kishi", &candidates, &ctx());
    assert_eq!(selected.map(|c| c.id), Some(142193));
}

#[test]
fn the_release_year_is_a_small_bonus() {
    // Containment 0.85 alone fails against a rival exact match, but the
    // query's release year lifts its containment score.
    let expected = Candidate {
        season_year: Some(2007),
        ..candidate(1723, "Clannad: The Motion Picture")
    };
    let rival = Candidate {
        season_year: Some(2005),
        ..candidate(2167, "Clannad Movie: Another Title")
    };
    let context = Context {
        year: Some(2007),
        movie: false,
        episodic: false,
        season: None,
    };
    // Containment + year = 0.88 vs rival containment 0.85: margin 0.03 is
    // still inside 0.10, so this stays a reject — the bonus is small.
    assert_eq!(scorer::select("clannad movie the motion picture", &[expected, rival], &context), None);
}

#[test]
fn a_penalized_containment_hit_is_not_rescued_by_a_synonym() {
    // The format penalty is applied after synonyms are scored, so a synonym
    // cannot smuggle a format-conflicting entry past the floor: the primary
    // title only contains the query (0.85), and the movie-vs-TV conflict
    // (−0.05) drops it to 0.80 whether or not the synonym is scored.
    let context = Context {
        year: None,
        movie: true,
        episodic: false,
        season: None,
    };
    let penalized = Candidate {
        format: Some("TV".into()),
        synonyms: vec!["Ordinal Scale Zero".into()],
        ..candidate(1, "Sword Art Online Ordinal Scale Recap")
    };
    let candidates = [penalized];
    // Primary containment (0.85) − movie conflict (0.05) = 0.80: no match.
    assert_eq!(
        scorer::select("sword art online ordinal scale", &candidates, &context),
        None
    );
}

#[test]
fn synonyms_are_scored_when_no_primary_title_is_an_exact_match() {
    // Corpus row 47: "Chii Fuyo" is nowhere near the real title, but the
    // recorded synonym `Chii-Fuyo` is exact.
    let candidate = Candidate {
        synonyms: vec!["Chii-Fuyo".into()],
        ..candidate(207329, "Chii's Sweet Home: New Home")
    };
    let candidates = [candidate];
    assert_eq!(scorer::select("chii fuyo", &candidates, &ctx()).map(|c| c.id), Some(207329));
}

#[test]
fn synonyms_are_ignored_when_a_primary_title_is_an_exact_match() {
    // Corpus row 6: the Suntory special's synonyms contain "Kimi no Na wa" too.
    // Scoring synonyms unconditionally would tie it with the film at 1.0 and
    // yield no Match; the primary exact match must win outright.
    let film = candidate(21519, "Kimi no Na wa.");
    let noise = Candidate {
        format: Some("SPECIAL".into()),
        synonyms: vec!["Kimi no Na wa".into()],
        ..candidate(97962, "Suntory Minami Alps no Tennen Mizu")
    };
    let candidates = [film, noise];
    let selected = scorer::select("kimi no na wa", &candidates, &ctx());
    assert_eq!(selected.map(|c| c.id), Some(21519));
}

#[test]
fn a_synonym_that_shortens_a_long_primary_title_wins() {
    // AniList #180894: the romaji title runs on past the release's short name,
    // so the primary title only *contains* the query (0.85) — while its sequel
    // #219243, a TV entry, collects the +0.02 format bonus and lands at the
    // same 0.85: a 0.02 margin, well inside the 0.10 gate, means no Match. The
    // entry's own `Romelia Senki` synonym is the exact spelling the query asks
    // for, so it must be allowed to score even though a primary containment
    // hit exists — only an *exact* primary title may close the gate.
    let romelia = |id: i64, format: &str, romaji: &str, synonyms: &[&str]| Candidate {
        format: Some(format.into()),
        synonyms: synonyms.iter().map(|s| (*s).to_owned()).collect(),
        ..candidate(id, romaji)
    };
    let candidates = [
        romelia(
            180894,
            "ONA",
            "Romelia Senki: Maou wo Taoshita nochi mo Jinrui Yabasou dakara Guntai Soshiki Shita",
            &["A History of the Romelia", "Romelia Senki"],
        ),
        romelia(
            219243,
            "TV",
            "Romelia Senki: Maou wo Taoshita nochi mo Jinrui Yabasou dakara Guntai Soshiki Shita Part II",
            &["Romelia Senki Part II"],
        ),
    ];
    let selected = scorer::select("romelia senki", &candidates, &ctx());
    assert_eq!(selected.map(|c| c.id), Some(180894));
}

#[test]
fn format_conflict_penalizes_and_drops_below_the_floor() {
    // A movie file against a TV candidate: containment 0.85 − 0.05 = 0.80,
    // below the floor.
    let context = Context {
        year: None,
        movie: true,
        episodic: false,
        season: None,
    };
    let candidates = [candidate(1, "Sword Art Online Ordinal Scale Recap")];
    assert_eq!(scorer::select("sword art online ordinal scale", &candidates, &context), None);
}

#[test]
fn the_movie_bonus_breaks_a_knife_edge_margin() {
    // Corpus row 10: a MOVIE candidate at containment 0.85 + 0.05 clears a
    // SPECIAL candidate at containment 0.85 − 0.05 by exactly 0.100.
    let context = Context {
        year: None,
        movie: true,
        episodic: false,
        season: None,
    };
    let movie = Candidate {
        format: Some("MOVIE".into()),
        ..candidate(21403, "Sword Art Online: Ordinal Scale")
    };
    let special = Candidate {
        format: Some("SPECIAL".into()),
        ..candidate(999, "Sword Art Online: Ordinal Scale - Sword Art Offline")
    };
    let candidates = [movie, special];
    let selected = scorer::select("sword art online ordinal scale", &candidates, &context);
    assert_eq!(selected.map(|c| c.id), Some(21403));
}
