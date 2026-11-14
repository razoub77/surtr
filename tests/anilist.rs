//! AniList client tests: the GraphQL request shape and response parsing.
//! The HTTP POST itself is exercised only in production; these tests pin the
//! wire contract offline.

use surtr::anilist;

#[test]
fn a_429_retry_after_is_seconds_with_a_safe_fallback() {
    assert_eq!(anilist::retry_after_ms(Some("30")), 30_000);
    assert_eq!(anilist::retry_after_ms(Some(" 5 ")), 5_000);
    assert_eq!(
        anilist::retry_after_ms(None),
        anilist::DEFAULT_RETRY_AFTER_MS
    );
    assert_eq!(
        anilist::retry_after_ms(Some("Wed, 21 Oct 2015 07:28:00 GMT")),
        anilist::DEFAULT_RETRY_AFTER_MS,
        "an HTTP-date Retry-After is not guessed at"
    );
}

#[test]
fn the_search_is_a_page_of_ten_relevance_sorted_anime_without_auth() {
    let body = anilist::request_body("Frieren");
    let query = body["query"].as_str().expect("GraphQL query");

    assert_eq!(body["variables"]["search"], "Frieren");
    assert!(query.contains("perPage: 10"), "{query}");
    assert!(query.contains("type: ANIME"), "anime only: {query}");
    assert!(query.contains("sort: SEARCH_MATCH"), "{query}");
    assert!(
        query.contains("coverImage { extraLarge }"),
        "cover art: {query}"
    );
    assert!(
        query.contains("season status episodes genres"),
        "hover-line fields: {query}"
    );
    assert!(
        !body.to_string().to_lowercase().contains("authorization")
            && !query.to_lowercase().contains("token"),
        "no authentication in the read-only request"
    );
}

#[test]
fn a_recorded_response_yields_candidates() {
    let body = r#"{
        "data": {
            "Page": {
                "media": [
                    {
                        "id": 1723,
                        "format": "MOVIE",
                        "seasonYear": 2007,
                        "season": null,
                        "status": "FINISHED",
                        "episodes": 1,
                        "genres": ["Drama", "Romance"],
                        "title": {
                            "romaji": "Clannad Movie",
                            "english": "Clannad: The Motion Picture"
                        },
                        "synonyms": [],
                        "coverImage": {
                            "extraLarge": "https://s4.anilist.co/file/anilistcdn/media/anime/cover/large/1723.jpg"
                        }
                    },
                    {
                        "id": 2167,
                        "format": "TV",
                        "seasonYear": 2007,
                        "episodes": 23,
                        "title": { "romaji": "CLANNAD", "english": "Clannad" },
                        "synonyms": ["CLANNAD CLANNAD"],
                        "coverImage": { "extraLarge": null }
                    }
                ]
            }
        }
    }"#;

    let candidates = anilist::parse_response(body).expect("a recorded answer");
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].id, 1723);
    assert_eq!(candidates[0].format.as_deref(), Some("MOVIE"));
    assert_eq!(candidates[0].season_year, Some(2007));
    assert_eq!(candidates[0].season.as_deref(), None);
    assert_eq!(candidates[0].status.as_deref(), Some("FINISHED"));
    assert_eq!(candidates[0].genres, vec!["Drama", "Romance"]);
    assert_eq!(candidates[1].episodes, Some(23));
    assert_eq!(
        candidates[0].cover_url(),
        Some("https://s4.anilist.co/file/anilistcdn/media/anime/cover/large/1723.jpg")
    );
    assert_eq!(candidates[1].cover_url(), None);
}

/// A response recorded before the hover-line fields existed: the older
/// candidates still parse, with the new fields empty (a version-1 cache file
/// reads back the same way).
#[test]
fn a_response_without_the_hover_fields_defaults_them_empty() {
    let body = r#"{
        "data": { "Page": { "media": [ { "id": 19, "format": "TV" } ] } }
    }"#;
    let candidates = anilist::parse_response(body).expect("a recorded answer");
    assert_eq!(candidates[0].season, None);
    assert_eq!(candidates[0].status, None);
    assert!(candidates[0].genres.is_empty());
}

#[test]
fn an_error_response_yields_no_candidates() {
    assert_eq!(
        anilist::parse_response(r#"{"errors":[{"message":"Not Found."}]}"#),
        None
    );
    assert_eq!(anilist::parse_response("not json"), None);
    assert_eq!(anilist::parse_response(r#"{"data":null}"#), None);
    assert_eq!(
        anilist::parse_response(r#"{"data":{"Page":{"media":[]}}}"#),
        Some(Vec::new()),
        "a real zero-result search is an answer, not a failure"
    );
}

#[test]
fn the_relations_request_asks_for_sequel_nodes_with_display_metadata() {
    let body = anilist::relations_body(161645);
    let query = body["query"].as_str().expect("GraphQL query");

    assert_eq!(body["variables"]["id"], 161645);
    assert!(query.contains("relationType"), "{query}");
    assert!(query.contains("Media (id: $id)"), "{query}");
    assert!(query.contains("episodes"), "{query}");
    assert!(query.contains("coverImage { extraLarge }"), "{query}");
    assert!(query.contains("season"), "airing season: {query}");
    assert!(query.contains("status"), "release status: {query}");
    assert!(query.contains("genres"), "genres: {query}");
}

#[test]
fn a_recorded_relations_response_yields_edges() {
    let body = r#"{
        "data": {
            "Media": {
                "relations": {
                    "edges": [
                        {
                            "relationType": "SEQUEL",
                            "node": {
                                "id": 176301,
                                "format": "TV",
                                "seasonYear": 2025,
                                "episodes": 24,
                                "title": {
                                    "romaji": "Kusuriya no Hitorigoto 2nd Season",
                                    "english": "The Apothecary Diaries Season 2"
                                },
                                "synonyms": [],
                                "coverImage": { "extraLarge": null }
                            }
                        },
                        {
                            "relationType": "PREQUEL",
                            "node": {
                                "id": 161645,
                                "format": "TV",
                                "seasonYear": 2023,
                                "episodes": 24,
                                "title": { "romaji": "Kusuriya no Hitorigoto", "english": null },
                                "synonyms": [],
                                "coverImage": { "extraLarge": null }
                            }
                        }
                    ]
                }
            }
        }
    }"#;

    let edges = anilist::parse_relations_response(body).expect("a recorded answer");
    assert_eq!(edges.len(), 2);
    assert_eq!(edges[0].relation_type, "SEQUEL");
    assert_eq!(edges[0].node.id, 176301);
    assert_eq!(edges[0].node.episodes, Some(24));
    assert_eq!(edges[1].relation_type, "PREQUEL");
}

#[test]
fn an_error_relations_response_yields_no_edges() {
    assert_eq!(
        anilist::parse_relations_response(r#"{"errors":[{"message":"Not Found."}]}"#),
        None
    );
    assert_eq!(
        anilist::parse_relations_response(r#"{"data":{"Media":null}}"#),
        None
    );
    assert_eq!(
        anilist::parse_relations_response(r#"{"data":{"Media":{"relations":null}}}"#),
        Some(Vec::new()),
        "an entry with no relations is an empty answer"
    );
}

#[test]
fn an_entry_by_id_returns_the_same_display_metadata() {
    let body = r#"{
        "data": {
            "Media": {
                "id": 108268,
                "format": "TV",
                "seasonYear": 2019,
                "episodes": 14,
                "title": { "romaji": "Honzuki no Gekokujou", "english": null },
                "synonyms": [],
                "coverImage": { "extraLarge": "https://example.test/cover.jpg" }
            }
        }
    }"#;

    let candidate = anilist::parse_entry_response(body).expect("a recorded entry");
    assert_eq!(candidate.id, 108268);
    assert_eq!(candidate.episodes, Some(14));
    assert_eq!(
        candidate.cover_url(),
        Some("https://example.test/cover.jpg")
    );
    assert_eq!(
        anilist::parse_entry_response(r#"{"data":{"Media":null}}"#),
        None,
        "a dead id is no answer"
    );
}

#[test]
fn the_entry_request_asks_for_the_full_display_metadata_by_id() {
    let body = anilist::entry_body(108268);
    let query = body["query"].as_str().expect("GraphQL query");

    assert_eq!(body["variables"]["id"], 108268);
    assert!(query.contains("Media (id: $id)"), "{query}");
    assert!(query.contains("episodes"), "{query}");
    assert!(query.contains("title { romaji english }"), "{query}");
    assert!(query.contains("coverImage { extraLarge }"), "{query}");
    assert!(query.contains("season"), "airing season: {query}");
    assert!(query.contains("status"), "release status: {query}");
    assert!(query.contains("genres"), "genres: {query}");
}
