//! Candidate scoring: trust-but-verify selection over AniList search results.
//!
//! Scoring mirrors the measured configuration in
//! `.scratch/surtr/research/match-quality.md`: an exact normalized title match
//! scores 1.0, token containment 0.85, fuzzy similarity below that, plus small
//! format and release-year bonuses. A Match is accepted only when the best
//! score clears 0.85 and beats the runner-up by at least 0.10 — ties are never
//! broken arbitrarily.
//!
//! The thresholds and bonus sizes are coupled: 1.0 − 0.85 = 0.15 is what lets
//! an exact match clear the margin over a containment match. Change them
//! together, and re-measure against the corpus.

use crate::anilist::Candidate;

/// Best accepted score floor.
pub const ACCEPT: f64 = 0.85;
/// Required lead over the runner-up.
pub const MARGIN: f64 = 0.10;
/// Exact/containment/bonus arithmetic lives near 0.85/1.0; allow for float
/// dust so a knife-edge 0.100 margin still passes.
const EPS: f64 = 1e-9;

const CONTAINMENT: f64 = 0.85;
const YEAR_BONUS: f64 = 0.03;
const MOVIE_BONUS: f64 = 0.05;
const MOVIE_CONFLICT: f64 = -0.05;
const EPISODIC_TV_BONUS: f64 = 0.02;
const EPISODIC_NON_TV_PENALTY: f64 = -0.05;

/// Query words that carry no title meaning for containment.
const STOP_WORDS: &[&str] = &["the", "a", "an"];

/// Two romanizations of the object particle `wo` compared as one: AniList
/// writes `Bu wo Kiwameru` where a fansub filename writes `Bu o Kiwameru`.
const PARTICLES: &[(&str, &str)] = &[("wo", "o")];

/// Ordinal and roman season words that normalize away when comparing titles
/// (`Youjo Senki Season 2` and `Youjo Senki II` compare equal). Only the
/// numerals the corpus measured map: a lone `i`/`v` stays a title letter
/// (`Infinity Castle I`, `To Be Hero X`).
const ROMAN_NUMERALS: &[(&str, &str)] = &[
    ("ii", "2"),
    ("iii", "3"),
    ("iv", "4"),
    ("vi", "6"),
    ("vii", "7"),
    ("viii", "8"),
    ("ix", "9"),
    ("x", "10"),
];

/// What the played file contributes to scoring: release year and format
/// expectations from the parsed Release Filename.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Context {
    pub year: Option<u32>,
    pub movie: bool,
    pub episodic: bool,
}

/// Normalizes text for comparison: lowercase, accents folded, punctuation
/// collapsed to single spaces.
pub fn normalize(text: &str) -> String {
    crate::release::fold_words(text)
}

/// Normalized tokens with season ordinals folded in (`2nd`, `season`, `II`)
/// and the two spellings of the object particle folded together.
fn tokens(text: &str) -> Vec<String> {
    normalize(text)
        .split(' ')
        .filter(|token| !token.is_empty())
        .map(|token| {
            let token = PARTICLES
                .iter()
                .find(|(variant, _)| *variant == token)
                .map_or(token, |(_, folded)| *folded);
            let digits = token.len()
                - token
                    .trim_end_matches(['s', 't', 'n', 'd', 'r', 'h'])
                    .len();
            match digits {
                // `2nd` → `2`; the suffixes all end in two letters.
                2.. if token[..token.len() - 2]
                    .bytes()
                    .all(|b| b.is_ascii_digit())
                    && token.len() > 2 =>
                {
                    token[..token.len() - 2].to_owned()
                }
                _ => ROMAN_NUMERALS
                    .iter()
                    .find(|(roman, _)| *roman == token)
                    .map(|(_, number)| (*number).to_owned())
                    .unwrap_or_else(|| token.to_owned()),
            }
        })
        .filter(|token| token != "season" && token != "cour")
        .collect()
}

/// Tokens minus stop words: the containment vocabulary.
fn significant(text: &str) -> Vec<String> {
    tokens(text)
        .into_iter()
        .filter(|token| !STOP_WORDS.contains(&token.as_str()))
        .collect()
}

/// Levenshtein edit distance.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut current = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let substitute = previous[j] + usize::from(ca != cb);
            current.push((previous[j + 1] + 1).min((current[j] + 1).min(substitute)));
        }
        previous = current;
    }
    previous[b.len()]
}

/// `1 − levenshtein / max(len)`, the fuzzy similarity.
fn similarity(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let distance = levenshtein(a, b) as f64;
    1.0 - distance / a.len().max(b.len()) as f64
}

/// A Candidate's comparable titles, optionally extended with synonyms.
fn titles(candidate: &Candidate, with_synonyms: bool) -> Vec<String> {
    let mut out = match candidate.title.as_ref() {
        Some(title) => [title.romaji.as_ref(), title.english.as_ref()]
            .into_iter()
            .flatten()
            .filter(|t| !t.is_empty())
            .cloned()
            .collect::<Vec<_>>(),
        None => Vec::new(),
    };
    if with_synonyms {
        out.extend(candidate.synonyms.iter().filter(|s| !s.is_empty()).cloned());
    }
    out
}

/// Scores one Candidate against the query over a set of titles: exact 1.0,
/// containment 0.85, fuzzy similarity below.
fn raw_score(query_norm: &str, query_sig: &[String], titles: &[String]) -> f64 {
    let mut best = 0.0_f64;
    for title in titles {
        let title_norm = tokens(title).join(" ");
        let score = if title_norm == query_norm {
            1.0
        } else if !query_sig.is_empty() {
            let title_sig = significant(title);
            let contained = query_sig
                .iter()
                .all(|token| title_sig.iter().any(|t| t == token));
            if contained {
                CONTAINMENT
            } else {
                similarity(query_norm, &title_norm)
            }
        } else {
            similarity(query_norm, &title_norm)
        };
        best = best.max(score);
    }
    best
}

/// The format and year bonuses on top of the raw title score.
fn bonus(candidate: &Candidate, context: &Context) -> f64 {
    let mut bonus = 0.0;
    if context.year.is_some() && context.year == candidate.season_year {
        bonus += YEAR_BONUS;
    }
    let format = candidate.format.as_deref();
    if context.movie {
        bonus += if format == Some("MOVIE") {
            MOVIE_BONUS
        } else {
            MOVIE_CONFLICT
        };
    } else if context.episodic {
        bonus += match format {
            Some("TV") | Some("TV_SHORT") => EPISODIC_TV_BONUS,
            Some("OVA") | Some("SPECIAL") | Some("MUSIC") => EPISODIC_NON_TV_PENALTY,
            _ => 0.0,
        };
    }
    bonus
}

/// A Candidate's final score for this query.
fn score(
    query_norm: &str,
    query_sig: &[String],
    candidate: &Candidate,
    with_synonyms: bool,
    context: &Context,
) -> f64 {
    let raw = raw_score(query_norm, query_sig, &titles(candidate, with_synonyms));
    (raw + bonus(candidate, context)).clamp(0.0, 1.0)
}

/// Selects the Match from a search result set, or None.
pub fn select<'a>(
    query: &str,
    candidates: &'a [Candidate],
    context: &Context,
) -> Option<&'a Candidate> {
    let query_norm = tokens(query).join(" ");
    let query_sig = significant(query);

    // Synonyms are a fallback, never a source: score them only when no
    // Candidate's *raw* primary-title score clears the floor — bonuses do not
    // count, or a penalized candidate would unlock synonym noise. This is the
    // measured gate from the research harness (`scoreSet`).
    let raw_scores = |with_synonyms: bool| {
        candidates
            .iter()
            .map(|candidate| {
                raw_score(&query_norm, &query_sig, &titles(candidate, with_synonyms))
            })
            .collect::<Vec<_>>()
    };
    let use_synonyms = raw_scores(false).iter().all(|raw| *raw < ACCEPT - EPS);

    let scores = candidates
        .iter()
        .map(|candidate| score(&query_norm, &query_sig, candidate, use_synonyms, context))
        .collect::<Vec<_>>();

    let best = scores.iter().cloned().fold(0.0_f64, f64::max);
    // The runner-up is the second-highest score value; with a single
    // Candidate it is zero, so the margin is the score itself.
    let runner_up = if scores.len() <= 1 {
        0.0
    } else {
        let mut sorted = scores.clone();
        sorted.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        sorted[1]
    };

    (best >= ACCEPT - EPS && best - runner_up >= MARGIN - EPS).then(|| {
        let index = scores
            .iter()
            .position(|score| (*score - best).abs() < EPS)
            .unwrap_or(0);
        &candidates[index]
    })
}
