//! Discord IPC payload shape: what `--dry-run` prints and what is sent.

use surtr::daemon::Presence;
use surtr::discord::activity_payload;

#[test]
fn watching_activity_carries_details_state_and_epoch_second_timestamps() {
    let presence = Presence {
        id: 154587,
        details: "Sousou no Frieren".into(),
        hover: "TV • Ongoing • Fall 2023 • Adventure, Drama, Fantasy".into(),
        state: Some("▶️ Playing • Episode 3/28".into()),
        cover: Some("https://s4.anilist.co/file/anilistcdn/media/anime/cover/large/154587.jpg".into()),
        start_ms: Some(1_700_000_000_000),
        end_ms: Some(1_700_000_600_000),
    };

    let payload = activity_payload(Some(&presence), 4242, 1);

    assert_eq!(payload["cmd"], "SET_ACTIVITY");
    assert_eq!(payload["nonce"], "1");
    assert_eq!(payload["args"]["pid"], 4242);
    let activity = &payload["args"]["activity"];
    assert_eq!(activity["type"], 3, "type 3 is Watching");
    // The payload's name replaces the registered application name where
    // Discord shows one (the expanded card header), no portal rename needed.
    assert_eq!(activity["name"], "Anime");
    assert_eq!(activity["details"], "Sousou no Frieren");
    // Display type 2 (Details): the compact status line reads `Watching {details}`.
    assert_eq!(activity["status_display_type"], 2);
    assert_eq!(activity["state"], "\u{25B6}\u{FE0F} Playing • Episode 3/28");
    assert_eq!(activity["details_url"], "https://anilist.co/anime/154587");
    // Discord timestamps are Unix epoch seconds.
    assert_eq!(activity["timestamps"]["start"], 1_700_000_000);
    assert_eq!(activity["timestamps"]["end"], 1_700_000_600);
    // Hovering the cover shows the metadata line, not another title.
    assert_eq!(
        activity["assets"]["large_text"],
        "TV • Ongoing • Fall 2023 • Adventure, Drama, Fantasy"
    );
}

#[test]
fn missing_state_and_end_timestamp_are_omitted() {
    let presence = Presence {
        id: 30864,
        details: "Koe no Katachi".into(),
        hover: "Movie • 2016".into(),
        state: None,
        cover: None,
        start_ms: Some(1_700_000_000_000),
        end_ms: None,
    };

    let activity = activity_payload(Some(&presence), 1, 2);
    let activity = &activity["args"]["activity"];

    assert!(activity.get("state").is_none());
    assert_eq!(activity["timestamps"]["start"], 1_700_000_000);
    assert!(activity["timestamps"].get("end").is_none());
    // The title links to AniList even without cover art.
    assert_eq!(activity["details_url"], "https://anilist.co/anime/30864");
    // No cover: no assets, so no hover text or cover link either.
    assert!(activity.get("assets").is_none());
}

#[test]
fn a_movie_activity_carries_the_cover_art_and_a_bar_without_an_episode_line() {
    let presence = Presence {
        id: 97962,
        details: "Kimi no Na wa.".into(),
        hover: "Movie • Finished • 2016 • Romance, Supernatural".into(),
        state: None,
        cover: Some("https://s4.anilist.co/file/anilistcdn/media/anime/cover/large/97962.jpg".into()),
        start_ms: Some(1_700_000_000_000),
        end_ms: Some(1_700_000_600_000),
    };

    let activity = activity_payload(Some(&presence), 1, 3);
    let activity = &activity["args"]["activity"];

    // No `Episode X` line, but the bar renders.
    assert!(activity.get("state").is_none());
    assert_eq!(activity["timestamps"]["start"], 1_700_000_000);
    assert_eq!(activity["timestamps"]["end"], 1_700_000_600);
    assert_eq!(
        activity["assets"]["large_image"],
        "https://s4.anilist.co/file/anilistcdn/media/anime/cover/large/97962.jpg"
    );
    // Hovering the cover shows the metadata line; clicking it opens AniList.
    assert_eq!(
        activity["assets"]["large_text"],
        "Movie • Finished • 2016 • Romance, Supernatural"
    );
    assert_eq!(activity["assets"]["large_url"], "https://anilist.co/anime/97962");
}

/// The failure measured live: AniList's romaji title for Chii Fuyo is 233
/// characters, and Discord answers an oversized `details` with an ERROR frame
/// ("details length must be less than or equal to 128 characters long"),
/// setting nothing. The card never appears and the publisher said nothing.
#[test]
fn an_oversized_title_is_clipped_to_discords_field_limit() {
    let details = "Tsuihou Sareta Cheat Fuyo Majutsushi wa Kimama na Second Life wo Ouka Suru.: Ore wa Buki dake ja Naku, Arayuru Mono ni \"Kyouka Point\" wo Fuyo Dekiru shi, Ore no Ishi de Itsudemo Kouka wo Kaijo Dekiru kedo, Nokotta Hitotachi Daijoubu?";
    let presence = Presence {
        id: 207329,
        details: details.to_owned(),
        hover: "The Laid-Off Cheat-Granting Mage Enjoys a Second Lease on Life".into(),
        state: Some("▶️ Playing • Episode 2".into()),
        cover: Some("https://s4.anilist.co/file/anilistcdn/media/anime/cover/medium/b207329-6VPeZIDfF4Sr.png".into()),
        start_ms: Some(1_700_000_000_000),
        end_ms: Some(1_700_000_600_000),
    };

    let activity = &activity_payload(Some(&presence), 1, 1)["args"]["activity"];

    let sent = activity["details"].as_str().expect("details is text");
    assert!(
        sent.chars().count() <= 128,
        "the limit Discord quoted: {sent}"
    );
    assert!(sent.ends_with('…'), "a cut is marked as one: {sent}");
    let kept: String = sent.chars().take(sent.chars().count() - 1).collect();
    assert!(
        details.starts_with(&kept),
        "the sent text is the title's head, cut whole: {sent}"
    );
    // A short field is untouched: clipping only rescues an oversized one.
    assert_eq!(activity["state"], "\u{25B6}\u{FE0F} Playing • Episode 2");
    assert_eq!(
        activity["assets"]["large_text"],
        "The Laid-Off Cheat-Granting Mage Enjoys a Second Lease on Life"
    );
}

#[test]
fn clear_sends_a_null_activity() {
    let payload = activity_payload(None, 7, 9);

    assert_eq!(payload["cmd"], "SET_ACTIVITY");
    assert_eq!(payload["args"]["pid"], 7);
    assert!(payload["args"]["activity"].is_null());
}
