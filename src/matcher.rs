//! Matching: turns a parsed Release Filename into a Match (or None).
//!
//! Matching runs after Season Resolution, never before: an explicit season
//! marker `N > 1` is in the search query already (`release::ParsedRelease::
//! title`), the winning entry must corroborate that ordinal in its titles or
//! synonyms, and a marker contradicted by the entry's known episode total
//! yields no Presence. An unmarked file whose Parsed Episode exceeds the Root
//! Season's total is an Absolute Episode: the `SEQUEL` walk finds the unique
//! containing season and remaps the episode to its Season Episode.

use std::collections::HashSet;

use crate::anilist::Candidate;
use crate::clock::Millis;
use crate::daemon::{Metadata, MetadataClient, Miss};
use crate::overrides::Overrides;
use crate::release;
use crate::scorer;

/// Supplies Candidates for a search query and the `SEQUEL` edges of an
/// AniList id: live AniList in production, recorded fixtures in tests.
pub trait MediaSource {
    /// Candidates for a search query. `None` is no answer (transport or an
    /// unusable response), distinct from `Some(vec![])`, a real zero result.
    fn search(&mut self, query: &str) -> Option<Vec<Candidate>>;
    /// The `SEQUEL`-related entries of `id`; an empty list when the chain ends.
    fn sequels(&mut self, id: i64) -> Option<Vec<Candidate>>;
    /// One entry by id, for an `overrides.json` short-circuit.
    fn entry(&mut self, id: i64) -> Option<Candidate>;
    /// The `Retry-After` delay of the most recent request, when it was a 429.
    fn retry_after(&self) -> Option<Millis> {
        None
    }
    /// Forget any previous rate-limit signal before one file's lookup.
    fn clear_retry(&mut self) {}
}

impl<S: MediaSource + ?Sized> MediaSource for &mut S {
    fn search(&mut self, query: &str) -> Option<Vec<Candidate>> {
        (**self).search(query)
    }

    fn sequels(&mut self, id: i64) -> Option<Vec<Candidate>> {
        (**self).sequels(id)
    }

    fn entry(&mut self, id: i64) -> Option<Candidate> {
        (**self).entry(id)
    }

    fn retry_after(&self) -> Option<Millis> {
        (**self).retry_after()
    }

    fn clear_retry(&mut self) {
        (**self).clear_retry()
    }
}

/// No single season has a thousand episodes: a larger Parsed Episode against
/// an unknown total is unresolvable absolute numbering (`ONE PIECE - 1071`),
/// not a season-relative number.
const MAX_SEASON_EPISODES: u32 = 999;

/// Which AniList title language leads the card's main title: the entry's
/// other language stands in when the preferred one is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TitleLanguage {
    /// The official English title (`Frieren: Beyond Journey's End`).
    #[default]
    English,
    /// The romanized title (`Sousou no Frieren`).
    Romaji,
}

/// Implements [`MetadataClient`] over the Release Filename parser and the
/// Candidate scorer.
pub struct Matcher<S: MediaSource> {
    pub source: S,
    /// The user's `overrides.json` mapping, loaded once at startup.
    overrides: Overrides,
    /// Which title language leads the card's main title, loaded once at
    /// startup.
    title_language: TitleLanguage,
    /// Why the most recent lookup missed, for the Daemon's report (issue 07).
    miss: Option<Miss>,
}

impl<S: MediaSource> Matcher<S> {
    pub fn new(source: S) -> Self {
        Self {
            source,
            overrides: Overrides::none(),
            title_language: TitleLanguage::default(),
            miss: None,
        }
    }

    /// Adds the startup-loaded overrides; a matching key short-circuits
    /// search and fetches that AniList entry directly.
    pub fn with_overrides(mut self, overrides: Overrides) -> Self {
        self.overrides = overrides;
        self
    }

    /// Adds the startup-loaded title language preference.
    pub fn with_title_language(mut self, title_language: TitleLanguage) -> Self {
        self.title_language = title_language;
        self
    }

    /// The search queries for one file, in order: the cleaned title first,
    /// then a Movie's tail (`Jujutsu Kaisen - Movie 01 - Jujutsu Kaisen 0`
    /// searches the full title, then `Jujutsu Kaisen 0`).
    fn plan(parsed: &release::ParsedRelease) -> Vec<String> {
        let mut queries = vec![parsed.title.clone()];
        if let Some(tail) = &parsed.tail {
            if scorer::normalize(tail) != scorer::normalize(&parsed.title) {
                queries.push(tail.clone());
            }
        }
        let mut seen = std::collections::HashSet::new();
        queries.retain(|query| seen.insert(scorer::normalize(query)));
        queries
    }

    fn lookup_release(&mut self, path: &str) -> Option<Metadata> {
        // A 429 from an earlier file must not schedule a retry for this one.
        self.source.clear_retry();
        self.miss = None;
        let parsed = match release::parse(path) {
            Ok(parsed) => parsed,
            Err(rejected) => {
                self.miss = Some(Miss::Fatal {
                    reason: rejection_reason(rejected),
                });
                return None;
            }
        };
        // Non-episodic OVAs/specials carry no episode number and are not
        // Movies: out of scope, no Presence.
        if !parsed.movie && parsed.episode.is_none() {
            self.miss = Some(Miss::Fatal {
                reason: "non-episodic file (OVA or special)".to_owned(),
            });
            return None;
        }
        let context = scorer::Context {
            year: parsed.year,
            movie: parsed.movie,
            episodic: parsed.episode.is_some(),
        };
        let mut miss_key = None;
        let mut answered = false;
        for query in Self::plan(&parsed) {
            miss_key = Some(query.clone());
            // An override is the user's assertion: skip search and fetch the
            // declared id. A dead id or a contradicted season is no Presence —
            // never a silent fallback to search.
            if let Some(id) = self.overrides.id(&query) {
                let Some(candidate) = self.source.entry(id) else {
                    self.miss = Some(Miss::Fatal {
                        reason: format!("the override for \"{query}\" names no AniList entry"),
                    });
                    return None;
                };
                return self.resolve_override(&candidate, &parsed);
            }
            let Some(candidates) = self.source.search(&query) else {
                continue;
            };
            answered = true;
            if candidates.is_empty() {
                continue;
            }
            if let Some(candidate) = scorer::select(&query, &candidates, &context) {
                return self.resolve(candidate, &parsed);
            }
        }
        // A query that got no answer at all is a transport failure, not a
        // miss; only a definitive zero or a lost score-off is repairable by
        // one `overrides.json` line naming the searched key (issue 07).
        if answered {
            self.miss = miss_key.map(|query| Miss::Repairable {
                key: scorer::normalize(&query),
            });
        }
        None
    }

    /// Turns the winning Candidate into display metadata: season corroboration
    /// and contradiction first, then Absolute Episode remapping for an
    /// unmarked file.
    fn resolve(
        &mut self,
        candidate: &Candidate,
        parsed: &release::ParsedRelease,
    ) -> Option<Metadata> {
        self.resolve_with(candidate, parsed, true)
    }

    /// The override path: the user asserted the id, so the title need not
    /// corroborate an explicit season; only an impossible episode does.
    fn resolve_override(
        &mut self,
        candidate: &Candidate,
        parsed: &release::ParsedRelease,
    ) -> Option<Metadata> {
        self.resolve_with(candidate, parsed, false)
    }

    /// `corroborate` is false for an override: the user asserted the id, so
    /// only an episode beyond the entry's known Total Episodes contradicts
    /// the season.
    fn resolve_with(
        &mut self,
        candidate: &Candidate,
        parsed: &release::ParsedRelease,
        corroborate: bool,
    ) -> Option<Metadata> {
        // The override key a user could add to repair this miss (issue 07).
        let key = scorer::normalize(&parsed.title);
        if let Some(season) = parsed.season {
            // `Season N` was in the query; the entry must carry the same
            // ordinal in its titles or synonyms. No corroboration, no
            // Presence — never fall back to the base entry.
            if corroborate && season > 1 && !corroborates(candidate, season) {
                self.miss = Some(Miss::Repairable { key });
                return None;
            }
            // An explicit marker is trusted, never walked forward from: an
            // episode beyond the entry's known total contradicts it.
            if let (Some(episode), Some(total)) = (parsed.episode, candidate.episodes) {
                if episode > total {
                    self.miss = Some(Miss::Repairable { key });
                    return None;
                }
            }
            return self.matched(candidate, parsed);
        }

        match (parsed.movie, parsed.episode, candidate.episodes) {
            // The Parsed Episode exceeds the Root Season: an Absolute Episode.
            // `remap` classifies its own failures.
            (false, Some(episode), Some(total)) if episode > total => {
                self.remap(candidate, episode, &key)
            }
            // No known range to test the number against (`ONE PIECE`).
            (false, Some(episode), None) if episode > MAX_SEASON_EPISODES => {
                self.miss = Some(Miss::Fatal {
                    reason: "absolute episode with no known Total Episodes".to_owned(),
                });
                None
            }
            _ => self.matched(candidate, parsed),
        }
    }

    /// The Match was won but could not render: a console-only failure, since
    /// no override line can give the entry a title.
    fn matched(
        &mut self,
        candidate: &Candidate,
        parsed: &release::ParsedRelease,
    ) -> Option<Metadata> {
        let metadata = metadata(candidate, parsed, self.title_language);
        if metadata.is_none() {
            self.miss = Some(Miss::Fatal {
                reason: "the matched entry carries no usable title".to_owned(),
            });
        }
        metadata
    }

    /// Walks `SEQUEL` relations from the Root Season along the unique chain,
    /// counting TV, ONA and TV_SHORT seasons and traversing movies, OVAs and
    /// specials without counting them, until a season contains the Absolute
    /// Episode. A season whose own total is unknown is the target once the
    /// chain ends (its episode renders without a denominator, the same as an
    /// unknown total on a direct match); only a second counted season *after*
    /// it blocks the walk, since the number could belong to either. Each way
    /// the walk can fail carries its own report (issue 07): an ambiguous
    /// branch is repairable — one override naming a sequel could still
    /// produce a card — while an overrun, a chain leaving the seasons, and a
    /// failed request are console-only or silent.
    fn remap(&mut self, root: &Candidate, episode: u32, key: &str) -> Option<Metadata> {
        let mut remaining = episode.checked_sub(root.episodes?)?;
        let mut current = root.id;
        // A counted season whose own total is unknown, and the episode number
        // relative to it: the target when the chain ends, or when only movies
        // and other continuity entries follow it.
        let mut unknown: Option<(Candidate, u32)> = None;
        let mut visited = HashSet::from([root.id]);
        loop {
            // A failed relations request is no answer, not the end of the
            // chain: no Presence and no report rather than guessing.
            let mut sequels = self.source.sequels(current)?;
            // Dedupe a repeated edge, then require exactly one: no sequel
            // means the number overruns the known chain, several mean an
            // ambiguous branch. Neither yields a Presence.
            sequels.sort_by_key(|sequel| sequel.id);
            sequels.dedup_by_key(|sequel| sequel.id);
            if sequels.is_empty() {
                if let Some((season, episode)) = unknown {
                    return self.matched_season(&season, episode);
                }
                self.miss = Some(Miss::Fatal {
                    reason: "absolute episode beyond the known sequel chain".to_owned(),
                });
                return None;
            }
            if sequels.len() != 1 {
                self.miss = Some(Miss::Repairable {
                    key: key.to_owned(),
                });
                return None;
            }
            let next = sequels.remove(0);
            if !visited.insert(next.id) {
                self.miss = Some(Miss::Fatal {
                    reason: "the sequel chain cycles".to_owned(),
                });
                return None;
            }
            if is_counted(next.format.as_deref()) {
                // A second counted season after an unknown total makes the
                // containment ambiguous: the number may belong to either.
                if unknown.is_some() {
                    self.miss = Some(Miss::Fatal {
                        reason: "absolute episode across a season with no known Total Episodes"
                            .to_owned(),
                    });
                    return None;
                }
                match next.episodes {
                    Some(total) if remaining <= total => {
                        return self.matched_season(&next, remaining);
                    }
                    Some(total) => remaining -= total,
                    None => unknown = Some((next.clone(), remaining)),
                }
            } else if !is_continuity(next.format.as_deref()) {
                // A SEQUEL to anything that is not a season (music, manga,
                // an unknown format) is not part of the tally.
                self.miss = Some(Miss::Fatal {
                    reason: "the sequel chain leaves the counted seasons".to_owned(),
                });
                return None;
            }
            current = next.id;
        }
    }

    /// The season an Absolute Episode landed in: a Match, or a console-only
    /// failure when it cannot render.
    fn matched_season(&mut self, season: &Candidate, episode: u32) -> Option<Metadata> {
        let metadata = season_metadata(season, episode, self.title_language);
        if metadata.is_none() {
            self.miss = Some(Miss::Fatal {
                reason: "the matched entry carries no usable title".to_owned(),
            });
        }
        metadata
    }
}

impl<S: MediaSource> MetadataClient for Matcher<S> {
    fn lookup(&mut self, path: &str) -> Option<Metadata> {
        self.lookup_release(path)
    }

    fn retry_after(&self) -> Option<Millis> {
        self.source.retry_after()
    }

    fn miss(&self) -> Option<Miss> {
        self.miss.clone()
    }
}

/// The console reason for a parser rejection: no single `overrides.json`
/// line can fix a batch, an OVA, or a path with no title (issue 07).
fn rejection_reason(rejected: release::Rejected) -> String {
    match rejected {
        release::Rejected::Unsupported(reason) => reason.to_owned(),
        release::Rejected::NotLocal => "not a local path".to_owned(),
        release::Rejected::NoFilename => "no filename".to_owned(),
        release::Rejected::NoTitle => "no title in the path".to_owned(),
        release::Rejected::UnresolvedEpisode => {
            "absolute episode with no resolvable season".to_owned()
        }
    }
}

/// A season whose episodes count towards the absolute tally.
fn is_counted(format: Option<&str>) -> bool {
    matches!(format, Some("TV" | "ONA" | "TV_SHORT"))
}

/// A node traversed for continuity but never counted towards the tally.
fn is_continuity(format: Option<&str>) -> bool {
    matches!(format, Some("MOVIE" | "OVA" | "SPECIAL"))
}

/// Whether the entry carries `season` as an ordinal in a primary title or a
/// synonym (`Honzuki S4` → `4th Season`; `Youjo Senki II` → the roman).
fn corroborates(candidate: &Candidate, season: u32) -> bool {
    let mut titles: Vec<&str> = candidate
        .title
        .iter()
        .flat_map(|title| [title.romaji.as_deref(), title.english.as_deref()])
        .flatten()
        .filter(|title| !title.is_empty())
        .collect();
    titles.extend(
        candidate
            .synonyms
            .iter()
            .map(String::as_str)
            .filter(|synonym| !synonym.is_empty()),
    );
    titles
        .iter()
        .any(|title| release::title_season(title) == Some(season))
}

/// The resolved display metadata for a Match: the configured title language
/// (the other as fallback), the Episode segment of the state line and the
/// hover line.
fn metadata(
    candidate: &Candidate,
    parsed: &release::ParsedRelease,
    title_language: TitleLanguage,
) -> Option<Metadata> {
    let title = candidate_title(candidate, title_language)?;
    Some(Metadata {
        id: candidate.id,
        title: title.clone(),
        episode: episode_segment(parsed.episode, candidate.episodes),
        hover: hover_line(candidate, &title),
        cover: candidate.cover_url().map(str::to_owned),
    })
}

/// The display metadata for the season an Absolute Episode lands in: its own
/// title, cover and total, with the remapped Season Episode.
fn season_metadata(
    season: &Candidate,
    episode: u32,
    title_language: TitleLanguage,
) -> Option<Metadata> {
    let title = candidate_title(season, title_language)?;
    Some(Metadata {
        id: season.id,
        title: title.clone(),
        episode: episode_segment(Some(episode), season.episodes),
        hover: hover_line(season, &title),
        cover: season.cover_url().map(str::to_owned),
    })
}

/// The Episode segment of the state line, `Episode {x}/{y}`.
///
/// An entry with exactly one episode renders the bare `Episode 1`, however
/// the file numbered it. A total that cannot be trusted as a denominator —
/// unknown, or smaller than the current episode — renders `?` instead.
/// `None` when the file carries no episode number and the entry is not a
/// one-episode entry: an unnumbered movie or OVA.
fn episode_segment(episode: Option<u32>, total: Option<u32>) -> Option<String> {
    match (episode, total) {
        (_, Some(1)) => Some("Episode 1".to_owned()),
        (Some(episode), Some(total)) if total >= episode => {
            Some(format!("Episode {episode}/{total}"))
        }
        (Some(episode), _) => Some(format!("Episode {episode}/?")),
        (None, _) => None,
    }
}

/// The cover-art hover text: `{Format} • {Status} • {Season} • {Genres}`,
/// each segment omitted when the entry does not carry it, the title standing
/// in when none of the four do. Genres are capped at four so a long list
/// cannot push the line past Discord's field limit.
fn hover_line(candidate: &Candidate, fallback: &str) -> String {
    let mut segments: Vec<String> = Vec::new();
    if let Some(label) = format_label(candidate.format.as_deref()) {
        segments.push(label.to_owned());
    }
    if let Some(status) = status_label(candidate.status.as_deref()) {
        segments.push(status.to_owned());
    }
    if let Some(season) = season_label(candidate.season.as_deref(), candidate.season_year) {
        segments.push(season);
    }
    if !candidate.genres.is_empty() {
        segments.push(candidate.genres.iter().take(4).cloned().collect::<Vec<_>>().join(", "));
    }
    if segments.is_empty() {
        fallback.to_owned()
    } else {
        segments.join(" • ")
    }
}

/// The Status's display label: only the states an in-progress watch can
/// carry. An unreleased, cancelled or unrecognized entry shows nothing.
fn status_label(status: Option<&str>) -> Option<&'static str> {
    match status {
        Some("FINISHED") => Some("Finished"),
        Some("RELEASING") => Some("Ongoing"),
        Some("HIATUS") => Some("Hiatus"),
        _ => None,
    }
}

/// The airing season's display label: `Spring 2013`. An entry with a season
/// but no year shows nothing (the season alone is unwearable); an entry with
/// a year but no season — most movies, OVAs, pre-2014 entries — shows the
/// year alone.
fn season_label(season: Option<&str>, year: Option<u32>) -> Option<String> {
    match (season, year) {
        (Some(season), Some(year)) => {
            let name = match season {
                "WINTER" => "Winter",
                "SPRING" => "Spring",
                "SUMMER" => "Summer",
                "FALL" => "Fall",
                _ => return None,
            };
            Some(format!("{name} {year}"))
        }
        (None, Some(year)) => Some(year.to_string()),
        _ => None,
    }
}

/// The Format's display label; `None` for a missing or unrecognized value,
/// which omits the leading hover segment.
fn format_label(format: Option<&str>) -> Option<&'static str> {
    match format {
        Some("TV") => Some("TV"),
        Some("TV_SHORT") => Some("TV Short"),
        Some("MOVIE") => Some("Movie"),
        Some("OVA") => Some("OVA"),
        Some("ONA") => Some("ONA"),
        Some("SPECIAL") => Some("Special"),
        Some("MUSIC") => Some("Music"),
        _ => None,
    }
}

/// The title for the card's main line: the preferred language first, the
/// entry's other language as fallback.
fn candidate_title(candidate: &Candidate, main: TitleLanguage) -> Option<String> {
    let title = candidate.title.as_ref()?;
    let (main, fallback) = match main {
        TitleLanguage::English => (&title.english, &title.romaji),
        TitleLanguage::Romaji => (&title.romaji, &title.english),
    };
    main.clone()
        .or_else(|| fallback.clone())
        .filter(|t| !t.is_empty())
}
