//! AniList GraphQL client: the read-only public search, the relations lookup
//! used by the Absolute Episode walk, and their payload shapes.

use crate::clock::Millis;
use crate::matcher::SourceError;

/// The public read-only endpoint; no authentication, no user credentials.
pub const ENDPOINT: &str = "https://graphql.anilist.co";

/// AniList's documented limit is 30 requests per minute. A 429 without a
/// usable `Retry-After` waits this long before its one deferred retry.
pub const DEFAULT_RETRY_AFTER_MS: Millis = 30_000;

/// The search: relevance-ordered page of 10 anime candidates. `type: ANIME`
/// is mandatory — an untyped search puts a manga or novel in the top 5 for 35
/// of the 49 corpus files. The cover URL and the hover-line fields (status,
/// airing season, genres) are fetched with the search so a Match can publish
/// them without a second request.
pub const SEARCH_QUERY: &str = "query ($search: String) { \
Page (perPage: 10) { media (search: $search, type: ANIME, sort: SEARCH_MATCH) { \
id format seasonYear season status episodes genres title { romaji english } synonyms \
coverImage { extraLarge } } } }";

/// The relation edges of one entry, with each node's full display metadata:
/// the absolute-episode walk uses the node directly as the Match, so its
/// title, episode total, cover and hover-line fields arrive in the same
/// request.
pub const RELATIONS_QUERY: &str = "query ($id: Int) { Media (id: $id) { \
relations { edges { relationType node { id format seasonYear season status episodes genres \
title { romaji english } synonyms coverImage { extraLarge } } } } } }";

/// One entry by id, with the same display metadata as a Candidate. An
/// `overrides.json` hit short-circuits search and fetches its entry this way.
pub const ENTRY_QUERY: &str = "query ($id: Int) { Media (id: $id) { \
id format seasonYear season status episodes genres title { romaji english } synonyms \
coverImage { extraLarge } } }";

/// One search result, as recorded from `POST https://graphql.anilist.co`.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Candidate {
    pub id: i64,
    pub format: Option<String>,
    #[serde(rename = "seasonYear")]
    pub season_year: Option<u32>,
    /// The airing season (`WINTER`/`SPRING`/`SUMMER`/`FALL`), null for
    /// entries without one (many movies, OVAs, pre-2014 entries).
    pub season: Option<String>,
    /// The release status (`FINISHED`, `RELEASING`, `NOT_YET_RELEASED`,
    /// `CANCELLED`, `HIATUS`).
    pub status: Option<String>,
    pub episodes: Option<u32>,
    /// The AniList genres, in the order the search returns them.
    #[serde(default)]
    pub genres: Vec<String>,
    pub title: Option<Titles>,
    #[serde(default)]
    pub synonyms: Vec<String>,
    #[serde(rename = "coverImage")]
    pub cover_image: Option<CoverImage>,
}

impl Candidate {
    /// The public cover art URL for `assets.large_image`.
    pub fn cover_url(&self) -> Option<&str> {
        self.cover_image
            .as_ref()
            .and_then(|cover| cover.extra_large.as_deref())
            .filter(|url| !url.is_empty())
    }
}

/// The cover art URLs of a Candidate.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct CoverImage {
    #[serde(rename = "extraLarge")]
    pub extra_large: Option<String>,
}

/// The searchable titles of a Candidate.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Titles {
    pub romaji: Option<String>,
    pub english: Option<String>,
}

/// The JSON request body for one search.
pub fn request_body(search: &str) -> serde_json::Value {
    serde_json::json!({
        "query": SEARCH_QUERY,
        "variables": { "search": search },
    })
}

/// One relation edge of an entry: `SEQUEL`, `PREQUEL`, `SIDE_STORY`, …
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct RelationEdge {
    #[serde(rename = "relationType")]
    pub relation_type: String,
    pub node: Candidate,
}

/// The JSON request body for one entry's relations.
pub fn relations_body(id: i64) -> serde_json::Value {
    serde_json::json!({
        "query": RELATIONS_QUERY,
        "variables": { "id": id },
    })
}

/// The JSON request body for one entry by id.
pub fn entry_body(id: i64) -> serde_json::Value {
    serde_json::json!({
        "query": ENTRY_QUERY,
        "variables": { "id": id },
    })
}

/// The Candidates from a GraphQL response body. `Some(vec![])` is a real
/// zero-result search; `None` is no answer (transport, GraphQL or parse
/// failure) and is never cached.
pub fn parse_response(body: &str) -> Option<Vec<Candidate>> {
    let response = serde_json::from_str::<Response>(body).ok()?;
    Some(response.data?.page?.media)
}

/// The relation edges of a relations response; a missing entry or error is
/// no answer, while an entry with no relation edges is an empty answer.
pub fn parse_relations_response(body: &str) -> Option<Vec<RelationEdge>> {
    let response = serde_json::from_str::<RelationsResponse>(body).ok()?;
    let media = response.data?.media?;
    Some(media.relations.map(|edges| edges.edges).unwrap_or_default())
}

/// One entry from an id lookup; `None` means a dead id or a failed request.
pub fn parse_entry_response(body: &str) -> Option<Candidate> {
    serde_json::from_str::<EntryResponse>(body)
        .ok()?
        .data?
        .media
}

#[derive(serde::Deserialize)]
struct Response {
    #[serde(default)]
    data: Option<Page>,
}

#[derive(serde::Deserialize)]
struct Page {
    #[serde(rename = "Page")]
    page: Option<MediaPage>,
}

#[derive(serde::Deserialize)]
struct MediaPage {
    #[serde(default)]
    media: Vec<Candidate>,
}

#[derive(serde::Deserialize)]
struct RelationsResponse {
    #[serde(default)]
    data: Option<MediaRelations>,
}

#[derive(serde::Deserialize)]
struct MediaRelations {
    #[serde(rename = "Media")]
    media: Option<EntryRelations>,
}

#[derive(serde::Deserialize)]
struct EntryRelations {
    relations: Option<RelationEdges>,
}

#[derive(serde::Deserialize)]
struct RelationEdges {
    #[serde(default)]
    edges: Vec<RelationEdge>,
}

#[derive(serde::Deserialize)]
struct EntryResponse {
    #[serde(default)]
    data: Option<EntryData>,
}

#[derive(serde::Deserialize)]
struct EntryData {
    #[serde(rename = "Media")]
    media: Option<Candidate>,
}

/// The `Retry-After` delay from a 429 response, in milliseconds. AniList
/// sends whole seconds; anything unparseable falls back to a safe wait.
pub fn retry_after_ms(header: Option<&str>) -> Millis {
    header
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|seconds| (seconds as Millis).saturating_mul(1_000))
        .unwrap_or(DEFAULT_RETRY_AFTER_MS)
}

/// The live client: one blocking HTTPS POST per query. The daemon calls this
/// from the core loop; mpv runs in its own process, so playback never blocks,
/// and every failure path degrades to no answer (no Match, no Presence). A
/// 429 answers `RateLimited` with its `Retry-After` for the core's one
/// deferred retry; the disk cache wraps this in [`crate::cache::Cached`].
pub struct AniList {
    agent: ureq::Agent,
}

impl Default for AniList {
    fn default() -> Self {
        Self::new()
    }
}

impl AniList {
    pub fn new() -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(std::time::Duration::from_secs(5))
                .timeout(std::time::Duration::from_secs(15))
                .build(),
        }
    }

    /// One read-only GraphQL POST. A 429 answers `RateLimited` with the
    /// parsed `Retry-After`; every other failure (transport, an unreadable
    /// body) answers `Unavailable`: no Match, no Presence, playback
    /// untouched.
    fn post(&mut self, body: serde_json::Value) -> Result<String, SourceError> {
        match self
            .agent
            .post(ENDPOINT)
            .set("accept", "application/json")
            .send_json(body)
        {
            Ok(response) => response.into_string().map_err(|_| SourceError::Unavailable),
            Err(ureq::Error::Status(429, response)) => Err(SourceError::RateLimited(
                retry_after_ms(response.header("retry-after")),
            )),
            Err(_) => Err(SourceError::Unavailable),
        }
    }
}

impl crate::matcher::MediaSource for AniList {
    fn search(&mut self, query: &str) -> Result<Vec<Candidate>, SourceError> {
        let text = self.post(request_body(query))?;
        parse_response(&text).ok_or(SourceError::Unavailable)
    }

    fn sequels(&mut self, id: i64) -> Result<Vec<Candidate>, SourceError> {
        // No answer ends the walk with no Presence, rather than a wrong season.
        let text = self.post(relations_body(id))?;
        parse_relations_response(&text)
            .ok_or(SourceError::Unavailable)
            .map(|edges| {
                edges
                    .into_iter()
                    .filter(|edge| edge.relation_type == "SEQUEL")
                    .map(|edge| edge.node)
                    .collect()
            })
    }

    fn entry(&mut self, id: i64) -> Result<Candidate, SourceError> {
        let text = self.post(entry_body(id))?;
        parse_entry_response(&text).ok_or(SourceError::Unavailable)
    }
}
