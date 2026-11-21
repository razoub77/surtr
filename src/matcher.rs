//! Matching: turns a parsed Release Filename into a Lookup Answer — a Match,
//! or why not (a Miss, a rate-limit delay, or no answer).
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
use crate::daemon::{Diagnostic, Level, Lookup, Metadata, MetadataClient, Miss};
use crate::overrides::Overrides;
use crate::release;
use crate::scorer;

/// Why a [`MediaSource`] has no answer for one request: AniList throttled
/// it, or the request failed outright. Neither is a Miss — the file's
/// lookup is not answered yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceError {
    /// The 429's `Retry-After` delay, in milliseconds.
    RateLimited(Millis),
    /// A transport failure or an unusable response.
    Unavailable,
}

/// Supplies Candidates for a search query and the `SEQUEL` edges of an
/// AniList id: live AniList in production, recorded fixtures in tests.
pub trait MediaSource {
    /// Candidates for a search query. `Err` is no answer — a throttle or a
    /// failed request — distinct from `Ok(vec![])`, a real zero result.
    fn search(&mut self, query: &str) -> Result<Vec<Candidate>, SourceError>;
    /// The `SEQUEL`-related entries of `id`; an empty list when the chain ends.
    fn sequels(&mut self, id: i64) -> Result<Vec<Candidate>, SourceError>;
    /// One entry by id, for an `overrides.json` short-circuit. A dead id is
    /// no answer here either: it arrives as [`SourceError::Unavailable`],
    /// and the matcher reports it as a named-entry miss.
    fn entry(&mut self, id: i64) -> Result<Candidate, SourceError>;
    /// Drains this source's own console Diagnostics, in request order. A
    /// source with nothing to report — including test fakes — keeps the
    /// default; the cache layer uses this to name hits against live fetches.
    fn take_trace(&mut self) -> Vec<Diagnostic> {
        Vec::new()
    }
}

impl<S: MediaSource + ?Sized> MediaSource for &mut S {
    fn search(&mut self, query: &str) -> Result<Vec<Candidate>, SourceError> {
        (**self).search(query)
    }

    fn sequels(&mut self, id: i64) -> Result<Vec<Candidate>, SourceError> {
        (**self).sequels(id)
    }

    fn entry(&mut self, id: i64) -> Result<Candidate, SourceError> {
        (**self).entry(id)
    }
    fn take_trace(&mut self) -> Vec<Diagnostic> {
        (**self).take_trace()
    }
}

impl From<SourceError> for Lookup {
    fn from(error: SourceError) -> Self {
        match error {
            SourceError::RateLimited(delay) => Lookup::RateLimited(delay),
            SourceError::Unavailable => Lookup::Unavailable,
        }
    }
}

/// No single season has a thousand episodes: a larger Parsed Episode against
/// an unknown total is unresolvable absolute numbering (`ONE PIECE - 1071`),
/// not a season-relative number.
const MAX_SEASON_EPISODES: u32 = 999;

/// Relations requests one Root Season proof may spend. A wider or longer
/// franchise keeps the tie unresolved rather than drain the rate limit.
/// Relations requests one SEQUEL walk may spend — a tie's root proof or an
/// Absolute Episode's season hunt. A longer franchise stays unresolved rather
/// than drain AniList's rate limit.
const MAX_RELATIONS_REQUESTS: usize = 16;

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
    /// This Lookup's console trace, drained by the Core through the
    /// `MetadataClient` seam once the answer is known.
    trace: Vec<Diagnostic>,
    /// The strongest non-definitive failure so far and a label naming what
    /// met it, so the warning can name a query the Core never sees.
    failure: Option<(SourceError, String)>,
}

impl<S: MediaSource> Matcher<S> {
    pub fn new(source: S) -> Self {
        Self {
            source,
            overrides: Overrides::none(),
            title_language: TitleLanguage::default(),
            trace: Vec::new(),
            failure: None,
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

    /// Drains the source's own Diagnostics into this lookup's trace, in
    /// request order, so a cache line lands beside the decision it explains.
    fn merge_source_trace(&mut self) {
        let mut trace = self.source.take_trace();
        self.trace.append(&mut trace);
    }

    /// Merges the source's trace and keeps one failure label for the warning,
    /// then passes the answer on unchanged.
    fn track<T>(
        &mut self,
        answer: Result<T, SourceError>,
        label: impl FnOnce() -> String,
    ) -> Result<T, SourceError> {
        self.merge_source_trace();
        if let Err(error) = &answer {
            self.note_failure(*error, label());
            return Err(*error);
        }
        answer
    }

    fn search(&mut self, query: &str) -> Result<Vec<Candidate>, SourceError> {
        let answer = self.source.search(query);
        self.track(answer, || format!("query \"{query}\""))
    }

    fn sequels(&mut self, id: i64) -> Result<Vec<Candidate>, SourceError> {
        let answer = self.source.sequels(id);
        self.track(answer, || format!("SEQUEL relations for AniList id {id}"))
    }

    fn entry(&mut self, id: i64) -> Result<Candidate, SourceError> {
        let answer = self.source.entry(id);
        self.track(answer, || format!("override AniList id {id}"))
    }

    /// Keeps one failure: a 429 outranks a transport failure, and the latest
    /// 429 supplies the delay the Core will honour. A run of transport
    /// failures keeps the first query, mirroring `stronger`'s fold.
    fn note_failure(&mut self, error: SourceError, label: String) {
        let replace = match self.failure {
            None => true,
            Some((SourceError::Unavailable, _)) => matches!(error, SourceError::RateLimited(_)),
            Some((SourceError::RateLimited(_), _)) => matches!(error, SourceError::RateLimited(_)),
        };
        if replace {
            self.failure = Some((error, label));
        }
    }

    /// The one warning for a non-definitive answer. A failed request names
    /// its query; a 429 names the delay in seconds, matching Retry-After.
    fn failure_warning(error: SourceError, label: &str) -> Diagnostic {
        match error {
            SourceError::Unavailable => {
                Diagnostic::warn(format!("surtr: AniList request failed for {label}"))
            }
            SourceError::RateLimited(delay) => Diagnostic::warn(format!(
                "surtr: AniList throttled {label}; retrying in {}s",
                delay as f64 / 1_000.0
            )),
        }
    }

    fn trace(&mut self, text: impl Into<String>) {
        self.trace.push(Diagnostic::info(text));
    }

    fn trace_verbose(&mut self, text: impl Into<String>) {
        self.trace.push(Diagnostic::new(Level::Verbose, text));
    }

    /// One step of an Absolute Episode walk through SEQUEL relations.
    fn trace_walk(&mut self, text: impl Into<String>) {
        self.trace_verbose(format!("surtr: absolute walk: {}", text.into()));
    }

    /// The deep view of a scored result set: the top three Candidates with
    /// their ids, titles and scores, before the info-level decision line.
    fn trace_top_candidates(&mut self, query: &str, ranked: &[scorer::Ranked<'_>]) {
        let top = ranked
            .iter()
            .take(3)
            .map(|entry| {
                let title = candidate_title(entry.candidate, self.title_language).unwrap_or_default();
                format!(
                    "id={} title=\"{title}\" score={:.3}",
                    entry.candidate.id, entry.score
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        self.trace_verbose(format!("surtr: top Candidates for \"{query}\": [{top}]"));
    }

    /// The Candidate that won this search, with the scores that ranked it.
    fn trace_chosen(&mut self, candidate: &Candidate, scores: scorer::Scores) {
        let title = candidate_title(candidate, self.title_language).unwrap_or_default();
        self.trace(format!(
            "surtr: chose Candidate id={} title=\"{title}\" score={:.3} runner-up={:.3}",
            candidate.id, scores.best, scores.runner_up
        ));
    }

    /// The Match the Lookup settled on, with the Episode segment users see.
    fn trace_match(&mut self, metadata: &Metadata) {
        let episode = match &metadata.episode {
            Some(segment) => format!("\"{segment}\""),
            None => "none".to_owned(),
        };
        self.trace(format!(
            "surtr: Match id={} title=\"{}\" episode={episode}",
            metadata.id, metadata.title
        ));
    }

    /// The search for one file, in order: the cleaned title, then a Movie's
    /// tail (`Jujutsu Kaisen - Movie 01 - Jujutsu Kaisen 0` searches the full
    /// title, then `Jujutsu Kaisen 0`). The last entry is the folder's spelling
    /// of a title the basename already names, and it is asked for only when
    /// every earlier query was answered with zero Candidates.
    fn plan(parsed: &release::ParsedRelease) -> Vec<String> {
        let mut queries = vec![parsed.title.clone()];
        if let Some(tail) = &parsed.tail {
            if scorer::normalize(tail) != scorer::normalize(&parsed.title) {
                queries.push(tail.clone());
            }
        }
        let mut seen = std::collections::HashSet::new();
        queries.retain(|query| seen.insert(scorer::normalize(query)));
        if let Some(folder) = &parsed.folder {
            if seen.insert(scorer::normalize(folder)) {
                queries.push(folder.clone());
            }
        }
        queries
    }

    fn lookup_release(&mut self, path: &str) -> Lookup {
        let parsed = match release::parse(path) {
            Ok(parsed) => parsed,
            Err(rejected) => {
                return Lookup::Missed(Miss::Fatal {
                    reason: rejection_reason(rejected),
                });
            }
        };
        self.trace(format!(
            "surtr: parsed Release Filename: title=\"{}\", season={}, episode={}, movie={}, year={}",
            parsed.title,
            display_optional(parsed.season),
            display_optional(parsed.episode),
            parsed.movie,
            display_optional(parsed.year),
        ));
        // Non-episodic OVAs/specials carry no episode number and are not
        // Movies: out of scope, no Presence.
        if !parsed.movie && parsed.episode.is_none() {
            return Lookup::Missed(Miss::Fatal {
                reason: "non-episodic file (OVA or special)".to_owned(),
            });
        }
        let context = scorer::Context {
            year: parsed.year,
            movie: parsed.movie,
            episodic: parsed.episode.is_some(),
            season: parsed.season,
        };
        // The folder's spelling is the last query, and only a run of zero
        // results reaches it: a query that returned any Candidate is evidence
        // the title is right, and its loss is a score-off, not a zero.
        let queries = Self::plan(&parsed);
        let folder_only = parsed
            .folder
            .as_ref()
            .filter(|folder| queries.len() > 1 && queries.last() == Some(folder))
            .map(|_| queries.len() - 1);
        let mut miss_key = None;
        let mut answered = false;
        let mut zero = true;
        // The failures met before a Match attempt: folded by precedence, so
        // whatever the pass ends on is not downgraded by them (an earlier
        // throttle must not be silenced by a later Miss).
        let mut deferred: Option<Lookup> = None;
        for (index, query) in queries.iter().enumerate() {
            if Some(index) == folder_only && !zero {
                break;
            }
            miss_key = Some(query.clone());
            // An override is the user's assertion: skip search and fetch the
            // declared id. A dead id or a contradicted season is no Presence —
            // never a silent fallback to search.
            if let Some(id) = self.overrides.id(query) {
                let answer = match self.entry(id) {
                    Ok(candidate) => {
                        self.trace(format!("surtr: override \"{query}\" -> id {id}"));
                        self.resolve_override(&candidate, &parsed)
                    }
                    Err(SourceError::RateLimited(delay)) => Lookup::RateLimited(delay),
                    Err(SourceError::Unavailable) => Lookup::Missed(Miss::Fatal {
                        reason: format!("the override for \"{query}\" names no AniList entry"),
                    }),
                };
                return settle(deferred, answer);
            }
            let candidates = match self.search(query) {
                Ok(candidates) => candidates,
                Err(error) => {
                    // A throttle or failed request is no answer: the pass goes
                    // on (a later query may still answer) and the strongest
                    // failure so far is remembered.
                    zero = false;
                    deferred = Some(match deferred {
                        None => error.into(),
                        Some(previous) => stronger(previous, error.into()),
                    });
                    continue;
                }
            };
            self.trace(format!(
                "surtr: search \"{query}\" -> {} {}",
                candidates.len(),
                candidate_noun(candidates.len())
            ));
            answered = true;
            if candidates.is_empty() {
                continue;
            }
            zero = false;
            let ranked = scorer::ranked(query, &candidates, &context);
            self.trace_top_candidates(query, &ranked);
            match scorer::choose(&ranked, &context) {
                scorer::Selection::One(candidate, scores) => {
                    self.trace_chosen(candidate, scores);
                    return settle(deferred, self.resolve(candidate, &parsed));
                }
                // A tie no ordinal settled is the Root Season question: prove
                // it along the `SEQUEL` chain rather than guess.
                scorer::Selection::Tied(tied, scores) => match self.root_season(&tied) {
                    Ok(Some(root)) => {
                        self.trace_chosen(root, scores);
                        return settle(deferred, self.resolve(root, &parsed));
                    }
                    // A throttle or failed request is no answer, as anywhere
                    // else: the lookup carries it, not a Miss.
                    Err(error) => return settle(deferred, error.into()),
                    Ok(None) => {}
                },
                scorer::Selection::None => {}
            }
        }
        // A query that got no answer at all is a transport failure, not a
        // miss; only a definitive zero or a lost score-off is repairable by
        // one `overrides.json` line naming the searched key.
        if answered {
            let key = miss_key.expect("an answered pass searched at least one query");
            let missed = Lookup::Missed(Miss::Repairable {
                key: scorer::normalize(&key),
            });
            return settle(deferred, missed);
        }
        deferred.unwrap_or(Lookup::Unavailable)
    }

    /// Turns the winning Candidate into display metadata: season corroboration
    /// and contradiction first, then Absolute Episode remapping for an
    /// unmarked file.
    fn resolve(&mut self, candidate: &Candidate, parsed: &release::ParsedRelease) -> Lookup {
        self.resolve_with(candidate, parsed, true)
    }

    /// The override path: the user asserted the id, so the title need not
    /// corroborate an explicit season; only an impossible episode does.
    fn resolve_override(
        &mut self,
        candidate: &Candidate,
        parsed: &release::ParsedRelease,
    ) -> Lookup {
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
    ) -> Lookup {
        // The override key a user could add to repair this miss.
        let key = scorer::normalize(&parsed.title);
        if let Some(season) = parsed.season {
            // `Season N` was in the query; the entry must carry the same
            // ordinal in its titles or synonyms. No corroboration, no
            // Presence — never fall back to the base entry. An Override
            // asserts its id, so it skips this check and its trace.
            if corroborate && season > 1 {
                let found = corroborates(candidate, season);
                self.trace_verbose(format!(
                    "surtr: season marker {season} in titles or synonyms of id={}: {}",
                    candidate.id,
                    if found { "found" } else { "not found" }
                ));
                if !found {
                    return Lookup::Missed(Miss::Repairable { key });
                }
            }
            // An explicit marker is trusted, never walked forward from: an
            // episode beyond the entry's known total contradicts it.
            if let (Some(episode), Some(total)) = (parsed.episode, candidate.episodes) {
                if episode > total {
                    self.trace_verbose(format!(
                        "surtr: episode {episode} exceeds Total Episodes {total} in id={}: season marker {season} contradicted",
                        candidate.id
                    ));
                    return Lookup::Missed(Miss::Repairable { key });
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
                Lookup::Missed(Miss::Fatal {
                    reason: "absolute episode with no known Total Episodes".to_owned(),
                })
            }
            _ => self.matched(candidate, parsed),
        }
    }

    /// The Match was won but could not render: a console-only failure, since
    /// no override line can give the entry a title.
    fn matched(&self, candidate: &Candidate, parsed: &release::ParsedRelease) -> Lookup {
        match metadata(candidate, parsed, self.title_language) {
            Some(m) => Lookup::Matched(m),
            None => Lookup::Missed(Miss::Fatal {
                reason: "the matched entry carries no usable title".to_owned(),
            }),
        }
    }

    /// The Root Season for a tie no ordinal could settle: the tied Candidate
    /// whose `SEQUEL` chain reaches every other tied one. Two such Candidates
    /// (a cycle) are no answer.
    fn root_season<'a>(
        &mut self,
        tied: &[&'a Candidate],
    ) -> Result<Option<&'a Candidate>, SourceError> {
        let ids: HashSet<i64> = tied.iter().map(|candidate| candidate.id).collect();
        if ids.len() < 2 {
            return Ok(None);
        }
        let mut budget = MAX_RELATIONS_REQUESTS;
        let mut root = None;
        for candidate in tied {
            let chain = self.sequel_chain(candidate.id, &mut budget)?;
            if ids
                .iter()
                .all(|id| *id == candidate.id || chain.contains(id))
            {
                if root.is_some() {
                    return Ok(None);
                }
                root = Some(*candidate);
            }
        }
        Ok(root)
    }

    /// Every id reachable from `id` along a unique `SEQUEL` chain. A branch,
    /// a cycle or the request budget stops the walk: the ids gathered so far
    /// still prove the root when they cover every tied Candidate, and never
    /// guess beyond that.
    fn sequel_chain(&mut self, id: i64, budget: &mut usize) -> Result<Vec<i64>, SourceError> {
        let mut chain = Vec::new();
        let mut current = id;
        let mut visited = HashSet::from([id]);
        while *budget > 0 {
            *budget -= 1;
            let mut sequels = self.sequels(current)?;
            sequels.sort_by_key(|sequel| sequel.id);
            sequels.dedup_by_key(|sequel| sequel.id);
            if sequels.len() != 1 {
                break;
            }
            let next = sequels.remove(0);
            if !visited.insert(next.id) {
                break;
            }
            chain.push(next.id);
            current = next.id;
        }
        Ok(chain)
    }

    /// Walks `SEQUEL` relations from the Root Season along the unique chain,
    /// counting TV, ONA and TV_SHORT seasons and traversing movies, OVAs and
    /// specials without counting them, until a season contains the Absolute
    /// Episode. A season whose own total is unknown is the target once the
    /// chain ends (its episode renders without a denominator, the same as an
    /// unknown total on a direct match); only a second counted season *after*
    /// it blocks the walk, since the number could belong to either. Each way
    /// the walk can fail carries its own report: an ambiguous
    /// branch is repairable — one override naming a sequel could still
    /// produce a card — while an overrun and a chain leaving the seasons are
    /// console-only, and a failed request answers no Miss at all.
    fn remap(&mut self, root: &Candidate, episode: u32, key: &str) -> Lookup {
        // The caller's arm matched a known total above the episode; these two
        // guards are for the compiler and unreachable in practice. They keep
        // the old `?` semantics: a silent no-answer.
        let Some(total) = root.episodes else {
            return Lookup::Unavailable;
        };
        let Some(mut remaining) = episode.checked_sub(total) else {
            return Lookup::Unavailable;
        };
        let mut current = root.id;
        self.trace_walk(format!(
            "root id={} total={total} episode={episode} remaining={remaining}",
            root.id
        ));
        // A counted season whose own total is unknown, and the episode number
        // relative to it: the target when the chain ends, or when only movies
        // and other continuity entries follow it.
        let mut unknown: Option<(Candidate, u32)> = None;
        let mut visited = HashSet::from([root.id]);
        let mut spent = 0usize;
        loop {
            if spent == MAX_RELATIONS_REQUESTS {
                self.trace_walk(format!(
                    "stop: budget exhausted after {spent} relations requests"
                ));
                return Lookup::Missed(Miss::Fatal {
                    reason: "absolute episode sequel chain exceeds the relations budget".to_owned(),
                });
            }
            spent += 1;
            // A failed relations request is no answer, not the end of the
            // chain: no Presence and no report rather than guessing.
            let mut sequels = match self.sequels(current) {
                Ok(sequels) => sequels,
                Err(SourceError::RateLimited(delay)) => return Lookup::RateLimited(delay),
                Err(SourceError::Unavailable) => return Lookup::Unavailable,
            };
            // Dedupe a repeated edge, then require exactly one: no sequel
            // means the number overruns the known chain, several mean an
            // ambiguous branch. Neither yields a Presence.
            sequels.sort_by_key(|sequel| sequel.id);
            sequels.dedup_by_key(|sequel| sequel.id);
            if sequels.is_empty() {
                if let Some((season, episode)) = unknown {
                    self.trace_walk(format!(
                        "stop: unknown total in id={} at episode {episode} (budget={spent})",
                        season.id
                    ));
                    return self.matched_season(&season, episode);
                }
                self.trace_walk(format!(
                    "stop: overrun after id={current}: no SEQUEL (budget={spent})"
                ));
                return Lookup::Missed(Miss::Fatal {
                    reason: "absolute episode beyond the known sequel chain".to_owned(),
                });
            }
            if sequels.len() != 1 {
                self.trace_walk(format!(
                    "stop: branch at id={current} with {} sequels (budget={spent})",
                    sequels.len()
                ));
                return Lookup::Missed(Miss::Repairable {
                    key: key.to_owned(),
                });
            }
            let next = sequels.remove(0);
            if !visited.insert(next.id) {
                self.trace_walk(format!("stop: cycle at id={} (budget={spent})", next.id));
                return Lookup::Missed(Miss::Fatal {
                    reason: "the sequel chain cycles".to_owned(),
                });
            }
            if is_counted(next.format.as_deref()) {
                // A second counted season after an unknown total makes the
                // containment ambiguous: the number may belong to either.
                if unknown.is_some() {
                    self.trace_walk(format!(
                        "stop: unknown total ambiguity at id={} (budget={spent})",
                        next.id
                    ));
                    return Lookup::Missed(Miss::Fatal {
                        reason: "absolute episode across a season with no known Total Episodes"
                            .to_owned(),
                    });
                }
                match next.episodes {
                    Some(total) if remaining <= total => {
                        self.trace_walk(format!(
                            "stop: containment in id={} at episode {remaining}/{total} (budget={spent})",
                            next.id
                        ));
                        return self.matched_season(&next, remaining);
                    }
                    Some(total) => {
                        remaining -= total;
                        self.trace_walk(format!(
                            "counted season id={} total={total}: remaining={remaining} (budget={spent})",
                            next.id
                        ));
                    }
                    None => {
                        self.trace_walk(format!(
                            "counted season id={} total=unknown: remaining={remaining} (budget={spent})",
                            next.id
                        ));
                        unknown = Some((next.clone(), remaining));
                    }
                }
            } else if is_continuity(next.format.as_deref()) {
                // Continuity nodes are walked but never counted; name them so
                // no step of the chain is missing from the deep view.
                self.trace_walk(format!(
                    "visited continuity id={} format={} (budget={spent})",
                    next.id,
                    next.format.as_deref().unwrap_or("unknown")
                ));
            } else {
                // A SEQUEL to anything that is not a season (music, manga,
                // an unknown format) is not part of the tally.
                self.trace_walk(format!(
                    "stop: non-season sequel id={} (budget={spent})",
                    next.id
                ));
                return Lookup::Missed(Miss::Fatal {
                    reason: "the sequel chain leaves the counted seasons".to_owned(),
                });
            }
            current = next.id;
        }
    }

    /// The season an Absolute Episode landed in: a Match, or a console-only
    /// failure when it cannot render.
    fn matched_season(&self, season: &Candidate, episode: u32) -> Lookup {
        match season_metadata(season, episode, self.title_language) {
            Some(m) => Lookup::Matched(m),
            None => Lookup::Missed(Miss::Fatal {
                reason: "the matched entry carries no usable title".to_owned(),
            }),
        }
    }
}

/// An absent parsed field prints as `none`, never an empty slot.
fn display_optional<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "none".to_owned(), |value| value.to_string())
}

fn candidate_noun(count: usize) -> &'static str {
    if count == 1 {
        "Candidate"
    } else {
        "Candidates"
    }
}

impl<S: MediaSource> MetadataClient for Matcher<S> {
    fn lookup(&mut self, path: &str) -> Lookup {
        self.trace.clear();
        self.failure = None;
        let answer = self.lookup_release(path);
        // Only a Match that survived resolution gets the final line: a
        // contradicted season is still a Miss, even after a Candidate won.
        if let Lookup::Matched(metadata) = &answer {
            self.trace_match(metadata);
        }
        // A non-definitive answer says so on the console: one warning naming
        // the failed query or the Retry-After delay. A Miss keeps its own
        // report; a Match needs no warning.
        if matches!(answer, Lookup::RateLimited(_) | Lookup::Unavailable) {
            if let Some((error, label)) = self.failure.take() {
                self.trace.push(Self::failure_warning(error, &label));
            }
        }
        answer
    }

    fn take_trace(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.trace)
    }
}

/// Ends a lookup with `answer`: a Match wins outright — no earlier failure
/// downgrades it — and anything else folds with the lookup's earlier
/// failures by the Lookup Answer's precedence.
fn settle(deferred: Option<Lookup>, answer: Lookup) -> Lookup {
    match answer {
        Lookup::Matched(_) => answer,
        other => match deferred {
            Some(previous) => stronger(previous, other),
            None => other,
        },
    }
}

/// The stronger of two non-Match answers, by the Lookup Answer's precedence:
/// a rate limit outranks a Miss, a Miss outranks silence — and of two rate
/// limits the most recent delay is the one honoured. A Match never reaches
/// this fold (it returns before any fold), and `previous` never holds a
/// Miss (a Miss is terminal: it only ever arrives as `next`).
fn stronger(previous: Lookup, next: Lookup) -> Lookup {
    match (previous, next) {
        (Lookup::RateLimited(_), Lookup::RateLimited(delay)) => Lookup::RateLimited(delay),
        (limited @ Lookup::RateLimited(_), _) => limited,
        (_, Lookup::RateLimited(delay)) => Lookup::RateLimited(delay),
        (Lookup::Unavailable, missed @ Lookup::Missed(_)) => missed,
        (previous, _) => previous,
    }
}

/// The console reason for a parser rejection: no single `overrides.json`
/// line can fix a batch, an OVA, or a path with no title.
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
