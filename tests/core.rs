//! Daemon Core behavior tests: scenarios drive the [`Runner`] with the verbs a
//! user applies (load a file, seek, pause, Toggle, end-file, focus the window,
//! advance the clock, another window takes the claim), and the assertions are
//! on recorded effects — Presence payload fields, clears, exact console and OSD
//! text. The harness — the scripted lookup, the in-memory `owner.json`, and the
//! one definition of “a turn of the daemon” — lives in `support`; the kernel's
//! raw Frame-level surface is tested inside `daemon.rs`.

use std::cell::RefCell;
use std::rc::Rc;

use surtr::daemon::{
    Effect, LiveState, Lookup, Metadata, MetadataClient, Miss, MpvEvent, OwnerClaim, Policy,
    Property, Runner, CLAIM_DEBOUNCE_MS, CLAIM_POLL_MS, CLAIM_SETTLE_MS, GRACE_MS,
};

mod support;

use support::{matched, metadata, Host, Script, PID, T0};

/// Hermetic paths: no real directory decides eligibility in the tests. The
/// shipped policy is the default; timing tests use their own literals.
const ANIME_ROOT: &str = r"D:\Anime";

fn policy() -> Policy {
    Policy::default()
}

fn episode(title: &str, episode_segment: &str) -> Metadata {
    Metadata {
        episode: Some(episode_segment.to_owned()),
        ..metadata(title)
    }
}

fn movie() -> Metadata {
    Metadata {
        id: 1723,
        title: "Clannad Movie".into(),
        ..metadata("Clannad Movie")
    }
}

fn live(file: &str, time_pos: f64, duration: f64) -> LiveState {
    LiveState {
        path: Some(file.to_owned()),
        time_pos: Some(time_pos),
        duration: Some(duration),
        ..LiveState::default()
    }
}

// ---------------------------------------------------------------------------
// Publishing and timestamps
// ---------------------------------------------------------------------------

#[tokio::test]
async fn file_loaded_publishes_watching_presence_with_live_timestamps() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    let file = r"D:\Anime\Frieren - 01.mkv";
    host.observed = live(file, 120.0, 1440.0);
    host.load_as_observed(&mut runner, file, 120.0).await;

    let publish = host.last_publish();
    assert_eq!(publish.details, "Frieren");
    assert_eq!(publish.state, None);
    // start = now - pos, end = now + (duration - pos), speed 1.
    assert_eq!(publish.start_ms, Some(T0 - 120_000));
    assert_eq!(publish.end_ms, Some(T0 + 1_320_000));
}

#[tokio::test]
async fn timestamps_scale_with_playback_speed() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    let file = r"D:\Anime\Frieren - 01.mkv";
    host.observed = LiveState {
        speed: 2.0,
        ..live(file, 120.0, 1440.0)
    };
    host.load_as_observed(&mut runner, file, 120.0).await;

    let publish = host.last_publish();
    // At 2x: 120 s watched = 60 s elapsed, remaining 1320 s = 660 s.
    assert_eq!(publish.start_ms, Some(T0 - 60_000));
    assert_eq!(publish.end_ms, Some(T0 + 660_000));
}

#[tokio::test]
async fn a_speed_change_republishes_from_the_live_position() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    let file = r"D:\Anime\Frieren - 01.mkv";
    host.observed = live(file, 120.0, 1440.0);
    host.load_as_observed(&mut runner, file, 120.0).await;
    assert_eq!(host.last_publish().end_ms, Some(T0 + 1_320_000));

    // 1.25x plays 1320 s of media in 1056 s of wall time.
    host.observed.speed = 1.25;
    host.trigger(&mut runner, MpvEvent::Property(Property::Speed(1.25)))
        .await;
    let fast = host.last_publish();
    assert_eq!(fast.start_ms, Some(T0 - 96_000));
    assert_eq!(fast.end_ms, Some(T0 + 1_056_000));

    host.observed.speed = 1.0;
    host.trigger(&mut runner, MpvEvent::Property(Property::Speed(1.0)))
        .await;
    let normal = host.last_publish();
    assert_eq!(normal.start_ms, Some(T0 - 120_000));
    assert_eq!(normal.end_ms, Some(T0 + 1_320_000));
}

#[tokio::test]
async fn missing_duration_publishes_without_timestamps() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Short clip"))]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    let file = r"D:\Anime\clip.webm";
    host.observed = LiveState {
        duration: None,
        ..live(file, 5.0, 0.0)
    };
    host.load_as_observed(&mut runner, file, 5.0).await;

    let publish = host.last_publish();
    assert_eq!(publish.start_ms, None, "no duration means no bar");
    assert_eq!(publish.end_ms, None);
}

#[tokio::test]
async fn a_movie_publishes_a_progress_bar_without_an_episode_line() {
    let mut script = Script::scripted([Lookup::Matched(movie())]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    let file = r"D:\Anime\Clannad Movie.mkv";
    host.observed = LiveState {
        duration: Some(4500.0),
        ..live(file, 300.0, 4500.0)
    };
    host.load_as_observed(&mut runner, file, 300.0).await;

    let publish = host.last_publish();
    assert_eq!(publish.state, None);
    assert_eq!(publish.start_ms, Some(T0 - 300_000));
    assert_eq!(publish.end_ms, Some(T0 + 4_200_000));
}

#[tokio::test]
async fn shutdown_clears_presence_and_exits() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    host.load(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;
    host.trigger(&mut runner, MpvEvent::Shutdown).await;

    assert_eq!(host.last_effect(), Some(&Effect::Clear));
    assert!(runner.has_exited(), "shutdown terminates the loop");
}

#[tokio::test]
async fn no_match_clears_instead_of_publishing() {
    let mut script = Script::scripted([Lookup::Unavailable]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    host.load(&mut runner, r"D:\HomeVideos\birthday.mp4").await;

    assert!(host.publishes().is_empty());
    assert_eq!(host.clears(), 0, "nothing was on screen to clear");
}

#[tokio::test]
async fn non_local_path_is_skipped_with_a_console_line() {
    let mut script = Script::scripted([Lookup::Matched(metadata("ignored"))]);
    let mut host = Host::new(true);
    let lookups = script.lookups();
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    host.load(&mut runner, "https://example.com/stream.m3u8").await;

    assert!(host.publishes().is_empty());
    assert!(host.osds().is_empty(), "a URL never shows an OSD");
    assert_eq!(
        host.logs(),
        vec!["surtr: skipping non-local path https://example.com/stream.m3u8".to_owned()]
    );
    assert!(lookups.borrow().is_empty(), "a URL is never looked up");
}

#[tokio::test]
async fn files_outside_the_anime_roots_produce_no_presence_and_no_lookup() {
    let mut script = Script::scripted([
        Lookup::Matched(metadata("Frieren")),
        Lookup::Matched(metadata("ignored")),
    ]);
    let mut host = Host::new(true);
    let lookups = script.lookups();
    let mut runner = Runner::new(
        &mut script,
        host.claim(),
        PID,
        T0,
        policy(),
        vec![ANIME_ROOT.to_owned()],
    );

    // A file inside the Root is looked up and published.
    host.load(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;
    assert_eq!(host.publishes().len(), 1);

    let file = r"D:\Anime2\Show - 01.mkv";
    host.load(&mut runner, file).await;

    assert_eq!(host.publishes().len(), 1, "no presence outside a root");
    assert_eq!(host.clears(), 1, "the old card is blanked");
    assert_eq!(
        *lookups.borrow(),
        vec![r"D:\Anime\Frieren - 01.mkv".to_owned()],
        "no lookup outside a root"
    );
    assert!(host.osds().is_empty(), "out-of-root never shows an OSD");
    assert_eq!(
        host.logs(),
        vec![format!("surtr: skipping non-local path {file}")],
        "the user still gets the console line"
    );
}

#[tokio::test]
async fn init_installs_the_anime_roots_for_every_eligible_file() {
    let mut script = Script::scripted([matched()]);
    let mut host = Host::new(true);
    let lookups = script.lookups();
    let mut runner = Runner::new(
        &mut script,
        host.claim(),
        PID,
        T0,
        policy(),
        vec![ANIME_ROOT.to_owned()],
    );

    host.load(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;

    assert_eq!(
        *lookups.borrow(),
        vec![r"D:\Anime\Frieren - 01.mkv".to_owned()],
        "a file inside the Root is looked up"
    );
    assert_eq!(host.publishes().len(), 1);
}

#[tokio::test]
async fn a_new_path_clears_the_old_card_before_the_lookup() {
    let mut script = Script::scripted([
        Lookup::Matched(metadata("First")),
        Lookup::Matched(metadata("Second")),
    ]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    host.load(&mut runner, r"D:\Anime\First - 01.mkv").await;
    assert_eq!(host.card_sequence(), "P");

    // The event itself delivers the clear; the new card follows once the
    // lookup completes.
    let second = r"D:\Anime\Second - 01.mkv";
    host.play(second, 0.0, Some(1440.0));
    let events = host
        .run(
            &mut runner,
            MpvEvent::FileLoaded {
                path: second.to_owned(),
                live: host.observed.clone(),
            },
        )
        .await;

    let cards: String = events
        .iter()
        .filter_map(|effect| match effect {
            Effect::Publish(_) => Some('P'),
            Effect::Clear => Some('-'),
            _ => None,
        })
        .collect();
    assert_eq!(
        cards, "-P",
        "the stale card is gone before the new one appears"
    );
    assert_eq!(host.last_publish().details, "Second");
}

#[tokio::test]
async fn a_repeated_file_loaded_does_not_lookup_twice() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(true);
    let lookups = script.lookups();
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    let file = r"D:\Anime\Frieren - 01.mkv";
    host.load(&mut runner, file).await;
    host.trigger(
        &mut runner,
        MpvEvent::FileLoaded {
            path: file.to_owned(),
            live: host.observed.clone(),
        },
    )
    .await;

    assert_eq!(*lookups.borrow(), vec![file.to_owned()]);
    assert_eq!(host.publishes().len(), 2);
    assert_eq!(host.clears(), 0, "a repeat must not blink");
}

// ---------------------------------------------------------------------------
// Seek, pause, and speed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn seek_republishes_from_the_new_position() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    host.load(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;

    host.time_pos = Some(600.0);
    host.trigger(&mut runner, MpvEvent::PlaybackRestart).await;

    let publish = host.last_publish();
    assert_eq!(publish.start_ms, Some(T0 - 600_000));
    assert_eq!(publish.end_ms, Some(T0 + 840_000));
}

#[tokio::test]
async fn a_scrub_burst_coalesces_to_one_update() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    host.load(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;
    assert_eq!(host.publishes().len(), 1);

    // Empty the bucket so the burst cannot be dispatched while it arrives.
    host.advance(20_000);
    for _ in 0..5 {
        host.turn_and_record(&mut runner, MpvEvent::PlaybackRestart)
            .await;
    }
    let during_burst = host.publishes().len();

    // Ten seeks land before a token returns: each only marks the latest-wins
    // intent, and the player keeps moving.
    for position in 1..=10 {
        host.time_pos = Some(position as f64);
        host.turn_and_record(&mut runner, MpvEvent::PlaybackRestart)
            .await;
    }
    assert_eq!(
        host.publishes().len(),
        during_burst,
        "nothing dispatched mid-burst"
    );

    // The next token admits one publish, sampled where the player is now.
    host.advance(4_000);
    host.timer(&mut runner).await;
    assert_eq!(
        host.publishes().len(),
        during_burst + 1,
        "the burst is one latest-wins update, not ten"
    );
    assert_eq!(
        host.last_publish().start_ms,
        Some(T0 + 24_000 - 10_000),
        "the dispatch resampled the final live position"
    );
}

#[tokio::test]
async fn the_token_bucket_allows_five_updates_then_one_every_four_seconds() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    host.load(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;

    // The bucket refills to its five-token capacity.
    host.advance(20_000);
    for _ in 0..6 {
        host.trigger(&mut runner, MpvEvent::PlaybackRestart).await;
    }
    assert_eq!(
        host.publishes().len(),
        6,
        "one initial publish plus a full bucket of five"
    );

    // The sixth update is still pending; one token arrives after 4 s.
    host.advance(4_000);
    host.timer(&mut runner).await;
    assert_eq!(host.publishes().len(), 7);
}

#[tokio::test]
async fn pause_drops_timestamps_and_leads_the_state_with_paused() {
    let mut script =
        Script::scripted([Lookup::Matched(episode("Frieren", "Episode 5 of 12"))]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    let file = r"D:\Anime\Frieren - 05.mkv";
    host.observed = live(file, 120.0, 1440.0);
    host.load_as_observed(&mut runner, file, 120.0).await;
    assert_eq!(
        host.last_publish().state.as_deref(),
        Some("\u{25B6}\u{FE0F} Playing • Episode 5 of 12")
    );

    host.observed.paused = true;
    host.trigger(&mut runner, MpvEvent::Property(Property::Pause(true)))
        .await;

    let publish = host.last_publish();
    assert_eq!(
        publish.state.as_deref(),
        Some("\u{23F8}\u{FE0F} Paused • Episode 5 of 12")
    );
    assert_eq!(publish.start_ms, None, "a paused card has no bar");
    assert_eq!(publish.end_ms, None);
}

#[tokio::test]
async fn unpause_republishes_with_fresh_timestamps() {
    let mut script =
        Script::scripted([Lookup::Matched(episode("Frieren", "Episode 5 of 12"))]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    let file = r"D:\Anime\Frieren - 05.mkv";
    host.observed = live(file, 120.0, 1440.0);
    host.load_as_observed(&mut runner, file, 120.0).await;

    host.observed.paused = true;
    host.trigger(&mut runner, MpvEvent::Property(Property::Pause(true)))
        .await;
    host.observed.paused = false;
    host.time_pos = Some(300.0);
    host.trigger(&mut runner, MpvEvent::Property(Property::Pause(false)))
        .await;

    let publish = host.last_publish();
    assert_eq!(
        publish.state.as_deref(),
        Some("\u{25B6}\u{FE0F} Playing • Episode 5 of 12")
    );
    assert_eq!(publish.start_ms, Some(T0 - 300_000));
    assert_eq!(publish.end_ms, Some(T0 + 1_140_000));
}

#[tokio::test]
async fn a_paused_movie_gets_a_paused_state() {
    let mut script = Script::scripted([Lookup::Matched(movie())]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    let file = r"D:\Anime\Clannad Movie.mkv";
    host.observed = live(file, 300.0, 4500.0);
    host.load_as_observed(&mut runner, file, 300.0).await;

    host.observed.paused = true;
    host.trigger(&mut runner, MpvEvent::Property(Property::Pause(true)))
        .await;

    let publish = host.last_publish();
    assert_eq!(
        publish.state.as_deref(),
        Some("\u{23F8}\u{FE0F} Paused")
    );
    assert_eq!(publish.start_ms, None);
    assert_eq!(publish.end_ms, None);
}

#[tokio::test]
async fn a_long_pause_never_times_out() {
    let mut script =
        Script::scripted([Lookup::Matched(episode("Frieren", "Episode 5 of 12"))]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    let file = r"D:\Anime\Frieren - 05.mkv";
    host.observed = live(file, 120.0, 1440.0);
    host.load_as_observed(&mut runner, file, 120.0).await;
    host.observed.paused = true;
    host.trigger(&mut runner, MpvEvent::Property(Property::Pause(true)))
        .await;

    // An hour later the paused card is still there; no idle timer exists.
    host.advance(3_600_000);
    host.timer(&mut runner).await;

    assert_eq!(host.clears(), 0);
    assert_eq!(
        host.last_publish().state.as_deref(),
        Some("\u{23F8}\u{FE0F} Paused • Episode 5 of 12")
    );
}

// ---------------------------------------------------------------------------
// End of file, grace window, and the Toggle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn end_file_clears_after_the_grace_window() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    host.load(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;

    host.trigger(&mut runner, MpvEvent::EndFile).await;
    assert_eq!(host.clears(), 0, "the grace window holds the card");

    host.advance(GRACE_MS - 1);
    host.timer(&mut runner).await;
    assert_eq!(host.clears(), 0);

    host.advance(1);
    host.timer(&mut runner).await;
    assert_eq!(host.clears(), 1);
    assert_eq!(host.last_effect(), Some(&Effect::Clear));
}

#[tokio::test]
async fn the_grace_window_is_the_policy_value() {
    let mut script = Script::scripted([matched()]);
    let mut host = Host::new(true);
    let policy = Policy {
        grace_ms: 1_000,
        ..Policy::default()
    };
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy, Vec::new());
    host.load(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;

    host.trigger(&mut runner, MpvEvent::EndFile).await;
    host.advance(999);
    host.timer(&mut runner).await;
    assert_eq!(host.clears(), 0, "the long grace holds the card");
    host.advance(1);
    host.timer(&mut runner).await;
    assert_eq!(host.clears(), 1);
}

#[tokio::test]
async fn a_playlist_advance_inside_the_grace_window_does_not_blank() {
    let mut script = Script::scripted([
        Lookup::Matched(metadata("Frieren")),
        Lookup::Matched(metadata("Frieren")),
    ]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    host.load(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;
    host.trigger(&mut runner, MpvEvent::EndFile).await;

    host.advance(GRACE_MS - 100);
    host.load(&mut runner, r"D:\Anime\Frieren - 02.mkv").await;

    assert_eq!(host.clears(), 0, "no blank between episodes");
    assert_eq!(host.publishes().len(), 2);
}

#[tokio::test]
async fn eof_reached_clears_and_a_seek_back_republishes() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    host.load(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;

    // `--keep-open` never fires `end-file`; eof-reached is the end signal.
    host.observed.eof_reached = true;
    host.trigger(&mut runner, MpvEvent::Property(Property::EofReached(true)))
        .await;
    assert_eq!(host.clears(), 1);

    host.observed.eof_reached = false;
    host.time_pos = Some(1200.0);
    host.trigger(&mut runner, MpvEvent::Property(Property::EofReached(false)))
        .await;
    assert_eq!(host.publishes().len(), 2);
}

#[tokio::test]
async fn a_clear_beats_a_publish_when_both_are_pending() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    host.load(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;
    // Refill the bucket so the Toggle-off clear and the Toggle-on publish can
    // both be admitted in one drain.
    host.advance(4_000);

    // Toggle off then on before the kernel can dispatch: the clear still goes
    // first, then the fresh publish.
    let mut events = runner
        .mpv(
            host.now,
            MpvEvent::Property(Property::Enabled(false)),
            LiveState {
                enabled: false,
                ..host.observed.clone()
            },
            &mut host.sampler(),
        )
        .await;
    host.observed.enabled = true;
    events.extend(
        runner
            .mpv(
                host.now,
                MpvEvent::Property(Property::Enabled(true)),
                host.observed.clone(),
                &mut host.sampler(),
            )
            .await,
    );
    host.effects.extend(events.iter().cloned());
    let cards: String = events
        .iter()
        .filter_map(|effect| match effect {
            Effect::Publish(_) => Some('P'),
            Effect::Clear => Some('-'),
            _ => None,
        })
        .collect();
    assert_eq!(cards, "-P", "clear first, then the fresh publish");
}

// ---------------------------------------------------------------------------
// Discord ownership: claiming, handover, dormancy
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_dormant_daemon_resolves_but_publishes_nothing() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(false);
    let lookups = script.lookups();
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    host.load(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;

    assert!(host.effects.is_empty());
    assert_eq!(lookups.borrow().len(), 1, "it keeps observing");
    assert_eq!(
        runner.wake_at(),
        None,
        "a dormant Daemon with no claim has nothing to wake for"
    );
}

#[tokio::test]
async fn focused_true_claims_ownership_after_the_debounce() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(false);
    let claim = host.claim();
    let seen = claim.clone();
    let mut runner = Runner::new(&mut script, claim, PID, T0, policy(), Vec::new());

    host.turn_and_record(&mut runner, MpvEvent::FocusChanged(true))
        .await;
    assert_eq!(seen.owner(), None, "focus has not settled yet");
    host.advance(CLAIM_DEBOUNCE_MS - 1);
    host.timer(&mut runner).await;
    assert_eq!(seen.owner(), None);

    host.advance(1);
    host.timer(&mut runner).await;
    assert_eq!(seen.owner(), Some(PID));
}

#[tokio::test]
async fn the_fresh_owner_publishes_live_state_after_the_settle() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(false);
    let claim = host.claim();
    let seen = claim.clone();
    let lookups = script.lookups();
    let mut runner = Runner::new(&mut script, claim, PID, T0, policy(), Vec::new());
    let file = r"D:\Anime\Frieren - 05.mkv";

    // Loaded and playing while the window is unfocused: silent.
    host.observed = live(file, 600.0, 1440.0);
    host.load_as_observed(&mut runner, file, 600.0).await;
    assert!(host.effects.is_empty(), "a non-Owner publishes nothing");

    host.turn_and_record(&mut runner, MpvEvent::FocusChanged(true))
        .await;
    host.advance(CLAIM_DEBOUNCE_MS);
    host.timer(&mut runner).await;
    assert_eq!(seen.owner(), Some(PID));
    assert!(
        host.effects.is_empty(),
        "the previous Owner needs the settle to clear"
    );
    let wake = runner.wake_at().expect("a settle wake");
    assert!(
        wake <= T0 + CLAIM_DEBOUNCE_MS + CLAIM_SETTLE_MS,
        "the settle lands within the policy window, got {wake}"
    );

    host.advance(CLAIM_SETTLE_MS);
    host.timer(&mut runner).await;

    let publish = host.last_publish();
    assert_eq!(publish.details, "Frieren");
    // The clock ran through the debounce and the settle (300 + 400 ms) before
    // the dispatch, which reads the live position then: 600 s into 1440 s.
    let now = T0 + CLAIM_DEBOUNCE_MS + CLAIM_SETTLE_MS;
    assert_eq!(
        publish.start_ms,
        Some(now - 600_000),
        "the bar comes from the live time-pos, not the load moment"
    );
    assert_eq!(lookups.borrow().len(), 1, "the Match is reused");
}

#[tokio::test]
async fn losing_focus_to_another_application_keeps_the_card() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());
    host.own(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;
    assert_eq!(host.publishes().len(), 1);

    // Alt-tab to a browser: mpv reports focus lost, but no other window
    // claims, so the Owner keeps publishing.
    host.trigger(&mut runner, MpvEvent::FocusChanged(false)).await;
    let now = T0 + CLAIM_DEBOUNCE_MS + CLAIM_SETTLE_MS;
    host.advance(10_000);
    host.timer(&mut runner).await;
    host.time_pos = Some(200.0);
    host.trigger(&mut runner, MpvEvent::PlaybackRestart).await;

    assert_eq!(host.clears(), 0);
    assert_eq!(host.publishes().len(), 2);
    assert_eq!(
        host.last_publish().start_ms,
        Some(now + 10_000 - 200_000)
    );
}

#[tokio::test]
async fn refocusing_an_owner_writes_no_new_claim_and_keeps_the_card() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());
    host.own(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;

    host.trigger(&mut runner, MpvEvent::FocusChanged(false)).await;
    host.focus_in(&mut runner).await;

    assert_eq!(host.clears(), 0);
    assert_eq!(host.publishes().len(), 1);
}

#[tokio::test]
async fn a_new_claim_makes_the_previous_owner_clear_within_one_poll() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(true);
    let claim = host.claim();
    let mut other_window = claim.clone();
    let mut runner = Runner::new(&mut script, claim, PID, T0, policy(), Vec::new());

    host.own(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;
    assert_eq!(host.publishes().len(), 1);

    // A second mpv window takes focus and claims.
    other_window.claim(PID + 1);

    // Within one poll the old Owner lets go.
    host.advance(CLAIM_POLL_MS);
    host.timer(&mut runner).await;
    assert_eq!(
        host.last_effect(),
        Some(&Effect::Clear),
        "the handover is a clear, which closes the connection"
    );
    assert_eq!(host.publishes().len(), 1);
}

#[tokio::test]
async fn an_owner_repolls_the_claim_on_the_policy_cadence() {
    let mut script = Script::scripted([matched()]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());
    host.load(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;

    let wake = runner.wake_at().expect("a poll");
    assert_eq!(wake, host.now + CLAIM_POLL_MS);
}

#[tokio::test]
async fn a_preempted_daemon_keeps_observing_but_never_publishes_again() {
    let mut script = Script::scripted([
        Lookup::Matched(metadata("Frieren")),
        Lookup::Matched(metadata("Frieren")),
    ]);
    let mut host = Host::new(true);
    let claim = host.claim();
    let mut other_window = claim.clone();
    let lookups = script.lookups();
    let mut runner = Runner::new(&mut script, claim, PID, T0, policy(), Vec::new());

    host.own(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;
    assert_eq!(host.publishes().len(), 1);

    // Another window claims focus. Within one poll the old Owner clears and
    // stops publishing, but it keeps observing mpv.
    other_window.claim(PID + 1);
    host.advance(CLAIM_POLL_MS);
    host.timer(&mut runner).await;
    assert_eq!(host.last_effect(), Some(&Effect::Clear));

    host.observed = live(r"D:\Anime\Frieren - 02.mkv", 600.0, 1440.0);
    host.time_pos = Some(600.0);
    host.load(&mut runner, r"D:\Anime\Frieren - 02.mkv").await;
    host.observed.paused = true;
    host.trigger(&mut runner, MpvEvent::Property(Property::Pause(true)))
        .await;
    host.time_pos = Some(700.0);
    host.trigger(&mut runner, MpvEvent::PlaybackRestart).await;
    let effects = runner.reconnect(host.now, &mut host.sampler()).await;
    host.effects.extend(effects);

    assert_eq!(lookups.borrow().len(), 2, "it keeps observing");
    assert_eq!(
        host.publishes().len(),
        1,
        "the loser publishes nothing after the handover"
    );
    assert_eq!(host.clears(), 1, "and fights for no slot");
    assert_eq!(other_window.owner(), Some(PID + 1));
}

/// KERNEL TEST: the clear the raced publish leaves behind reaches the host.
///
/// Same interleaving as above, played through the Runner once the kernel has
/// already admitted a card: the handover between one timer and the next is
/// what the Runner's own loop would apply.
#[tokio::test]
async fn a_handover_after_the_admission_clears_the_old_card() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(true);
    let claim = host.claim();
    let mut other_window = claim.clone();
    let mut runner = Runner::new(&mut script, claim, PID, T0, policy(), Vec::new());

    host.own(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;
    assert_eq!(host.publishes().len(), 1, "the card is up");

    // The other window claims; the very next step sees it before dispatch.
    other_window.claim(PID + 1);
    host.time_pos = Some(300.0);
    host.trigger(&mut runner, MpvEvent::PlaybackRestart).await;

    assert_eq!(host.publishes().len(), 1, "the racing republish is dropped");
    assert_eq!(host.last_effect(), Some(&Effect::Clear));
}

#[tokio::test]
async fn a_claim_lost_during_the_settle_publishes_nothing() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(false);
    let claim = host.claim();
    let mut other_window = claim.clone();
    let mut runner = Runner::new(&mut script, claim, PID, T0, policy(), Vec::new());
    host.load(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;

    // Focus is reported; the debounce has not elapsed yet, so nothing is
    // written and the host has not slept.
    let mut effects = host.turn(&mut runner, MpvEvent::FocusChanged(true)).await;
    host.advance(CLAIM_DEBOUNCE_MS);
    effects.extend(
        runner
            .timer(host.now, &mut host.sampler())
            .await,
    );
    host.effects.extend(effects);
    // Another window claims before this one settles.
    other_window.claim(PID + 1);

    host.advance(CLAIM_SETTLE_MS);
    host.timer(&mut runner).await;

    assert!(
        host.effects.is_empty(),
        "a lost claim neither publishes nor clears"
    );
}

#[tokio::test]
async fn toggle_off_during_a_lookup_stays_hidden_and_toggle_on_republishes_the_match() {
    let mut script = Script::scripted([
        Lookup::RateLimited(30_000),
        Lookup::Matched(metadata("Frieren")),
    ]);
    let mut host = Host::new(false);
    let claim = host.claim();
    let seen = claim.clone();
    let lookups = script.lookups();
    let mut runner = Runner::new(&mut script, claim, PID, T0, policy(), Vec::new());

    // The window is focused and owns Discord before anything loads.
    host.focus_in(&mut runner).await;
    assert_eq!(seen.owner(), Some(PID));

    // Privacy first: the Toggle goes off before the file is known.
    host.observed.enabled = false;
    host.trigger(&mut runner, MpvEvent::Property(Property::Enabled(false)))
        .await;

    // The file loads behind the hidden session: it is looked up, but the 429
    // answer means no Match yet.
    let file = r"D:\Anime\Frieren - 01.mkv";
    host.observed = LiveState {
        enabled: false,
        ..live(file, 0.0, 1440.0)
    };
    host.load_as_observed(&mut runner, file, 0.0).await;
    assert_eq!(lookups.borrow().len(), 1);
    assert!(host.effects.is_empty());

    // Toggling back on before the retry comes due republishes nothing (no
    // Match), but the retry is still coming.
    host.observed.enabled = true;
    host.trigger(&mut runner, MpvEvent::Property(Property::Enabled(true)))
        .await;
    assert!(host.effects.is_empty());

    // The retry resolves the Match at its due time and shows it.
    host.advance(30_000);
    host.timer(&mut runner).await;
    assert_eq!(lookups.borrow().len(), 2, "one deferred retry");
    assert_eq!(host.last_publish().details, "Frieren");
    let now = T0 + CLAIM_DEBOUNCE_MS + CLAIM_SETTLE_MS + 30_000;
    assert_eq!(host.last_publish().start_ms, Some(now));
}

#[tokio::test]
async fn toggle_off_hides_the_live_match_and_toggle_on_brings_it_back_from_memory() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(true);
    let lookups = script.lookups();
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());
    host.own(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;
    assert_eq!(host.publishes().len(), 1);

    // Off: the card is gone at once, the file stays loaded.
    host.observed.enabled = false;
    host.trigger(&mut runner, MpvEvent::Property(Property::Enabled(false)))
        .await;
    assert_eq!(host.last_effect(), Some(&Effect::Clear));

    // On: the same Match returns from live state, with no new lookup.
    host.observed.enabled = true;
    host.time_pos = Some(60.0);
    host.trigger(&mut runner, MpvEvent::Property(Property::Enabled(true)))
        .await;
    assert_eq!(host.last_publish().details, "Frieren");
    let now = T0 + CLAIM_DEBOUNCE_MS + CLAIM_SETTLE_MS;
    assert_eq!(host.last_publish().start_ms, Some(now - 60_000));
    assert_eq!(lookups.borrow().len(), 1);
}

#[tokio::test]
async fn toggle_off_during_a_pending_publish_clears_at_once() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());
    host.own(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;

    // Take the seek's publish intent out of reach: drain the bucket so no
    // token is left for it, then let the Toggle go off.
    host.advance(20_000);
    for _ in 0..5 {
        host.turn_and_record(&mut runner, MpvEvent::PlaybackRestart)
            .await;
    }
    let published_before = host.publishes().len();
    host.turn_and_record(&mut runner, MpvEvent::PlaybackRestart)
        .await;
    assert_eq!(
        host.publishes().len(),
        published_before,
        "the bucket is empty, so the seek's publish waits for a token"
    );

    // The Toggle goes off while that publish is still pending.
    host.observed.enabled = false;
    host.trigger(&mut runner, MpvEvent::Property(Property::Enabled(false)))
        .await;
    assert_eq!(
        host.publishes().len(),
        published_before,
        "the pending publish is dropped, not flushed with the clear"
    );

    // Clears are ordinary updates: the next token delivers the clear at once,
    // with no further wait beyond the token.
    host.advance(4_000);
    host.timer(&mut runner).await;
    assert_eq!(host.clears(), 1, "the clear is never deferred");
    assert_eq!(host.last_effect(), Some(&Effect::Clear));
}

#[tokio::test]
async fn toggle_on_cannot_claim_discord_for_a_dormant_daemon() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(false);
    let claim = host.claim();
    let seen = claim.clone();
    let mut runner = Runner::new(&mut script, claim, PID, T0, policy(), Vec::new());

    host.observed.enabled = false;
    host.trigger(&mut runner, MpvEvent::Property(Property::Enabled(false)))
        .await;
    host.observed.enabled = true;
    host.load(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;
    host.trigger(&mut runner, MpvEvent::Property(Property::Enabled(true)))
        .await;

    assert!(host.effects.is_empty(), "only focus hands over ownership");
    assert_eq!(seen.owner(), None);
}

#[tokio::test]
async fn toggle_off_behind_a_dormant_daemon_emits_nothing() {
    let mut script = Script::scripted([matched()]);
    let mut host = Host::new(false);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());
    host.load(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;
    host.observed.enabled = false;
    host.trigger(&mut runner, MpvEvent::Property(Property::Enabled(false)))
        .await;

    assert!(host.effects.is_empty(), "no card, no clear, no claim");
}

// ---------------------------------------------------------------------------
// 429 retry and Discord reconnect
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_429_schedules_one_deferred_retry() {
    let mut script = Script::scripted([
        Lookup::RateLimited(30_000),
        Lookup::Matched(metadata("Frieren")),
    ]);
    let mut host = Host::new(true);
    let lookups = script.lookups();
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    let file = r"D:\Anime\Frieren - 01.mkv";
    host.load(&mut runner, file).await;

    assert_eq!(
        runner.wake_at(),
        Some(T0 + CLAIM_POLL_MS),
        "a 429 does not beat the Owner's claim poll"
    );
    assert_eq!(*lookups.borrow(), vec![file.to_owned()]);
    assert!(host.publishes().is_empty(), "429 is not a Match");

    host.advance(29_999);
    host.timer(&mut runner).await;
    assert_eq!(lookups.borrow().len(), 1, "not due yet");

    host.advance(1);
    host.timer(&mut runner).await;
    assert_eq!(lookups.borrow().len(), 2, "one retry");
    assert_eq!(host.publishes().len(), 1);
}

/// PINS TODAY'S BEHAVIOR for the 429 console line outside every Anime Root.
///
/// An out-of-Root file is rejected in `on_file_loaded` before any lookup: the
/// user gets one console line and nothing else. The client's `Retry-After` is
/// only consulted in `lookup`'s "no Match" arm, which the skip never reaches,
/// so no deferred retry is ever scheduled for that file — and therefore no
/// retry line can vanish. What the user sees is a single line at the load;
/// the console then stays silent forever.
///
/// This test changes nothing. When the follow-up fix lands it must be updated
/// deliberately, so the console output for a skipped file can neither grow a
/// retry line nor lose its skip line unnoticed.
#[tokio::test]
async fn an_out_of_root_file_is_reported_once_and_never_retried() {
    /// A client mid-back-off: every lookup would answer `RateLimited`, but a
    /// skipped file is never looked up, so the answer is never consumed.
    #[derive(Default)]
    struct RateLimited {
        /// Every path the kernel looked up, shared for inspection.
        lookups: Rc<RefCell<Vec<String>>>,
    }

    impl MetadataClient for RateLimited {
        fn lookup(&mut self, path: &str) -> Lookup {
            self.lookups.borrow_mut().push(path.to_owned());
            Lookup::RateLimited(30_000)
        }
    }

    let mut client = RateLimited::default();
    let lookups = client.lookups.clone();
    let mut host = Host::new(true);
    let mut runner = Runner::new(
        &mut client,
        host.claim(),
        PID,
        T0,
        policy(),
        vec![ANIME_ROOT.to_owned()],
    );

    // `D:\Anime2` is a sibling of the Root, not inside it (README: "D:\Anime2
    // is not inside D:\Anime"): the load is skipped, says so, and looks
    // nothing up.
    let file = r"D:\Anime2\Show - 01.mkv";
    host.load(&mut runner, file).await;
    let skip_line = format!("surtr: skipping non-local path {file}");
    assert_eq!(
        host.logs(),
        vec![skip_line.clone()],
        "the load tells the user why nothing is shown"
    );
    assert!(lookups.borrow().is_empty(), "no lookup outside a Root");
    assert_eq!(
        runner.wake_at(),
        Some(T0 + CLAIM_POLL_MS),
        "the only pending work is the Owner's claim poll: no retry was scheduled"
    );

    // Well past any Retry-After the client could have reported: the timer
    // finds nothing to do, so no lookup and no further console output.
    host.advance(60_000);
    host.timer(&mut runner).await;
    assert!(
        lookups.borrow().is_empty(),
        "a skipped file is never looked up, so it is never retried"
    );
    assert_eq!(
        host.logs(),
        vec![skip_line],
        "DEFECT PINNED: the skip says its one line and the console stays silent"
    );
    assert!(host.osds().is_empty(), "a skipped file never shows an OSD");
    assert!(host.publishes().is_empty());
    assert_eq!(host.clears(), 0, "nothing was on screen to clear");
}

#[tokio::test]
async fn a_file_change_cancels_the_deferred_retry() {
    let mut script = Script::scripted([
        Lookup::RateLimited(30_000),
        Lookup::Matched(metadata("Second")),
    ]);
    let mut host = Host::new(true);
    let lookups = script.lookups();
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    let first = r"D:\Anime\First - 01.mkv";
    let second = r"D:\Anime\Second - 01.mkv";
    host.load(&mut runner, first).await;

    host.advance(100);
    host.load(&mut runner, second).await;

    host.advance(60_000);
    host.timer(&mut runner).await;

    assert_eq!(
        *lookups.borrow(),
        vec![first.to_owned(), second.to_owned()],
        "the old file's retry was cancelled"
    );
}

#[tokio::test]
async fn reconnect_republishes_from_live_state_without_a_lookup() {
    let mut script = Script::scripted([Lookup::Matched(metadata("Frieren"))]);
    let mut host = Host::new(true);
    let lookups = script.lookups();
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    host.load(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;

    // Time passes and the player has moved on; the reconnect republishes from
    // that live position without asking AniList again.
    host.advance(10_000);
    host.time_pos = Some(400.0);
    let before = host.publishes().len();
    let effects = runner.reconnect(host.now, &mut host.sampler()).await;
    host.effects.extend(effects);

    assert_eq!(host.publishes().len(), before + 1);
    assert_eq!(lookups.borrow().len(), 1, "no AniList re-query");
    assert_eq!(host.last_publish().start_ms, Some(T0 + 10_000 - 400_000));
}

// ---------------------------------------------------------------------------
// Match-miss diagnostics: console and OSD
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_repairable_miss_shows_one_osd_and_one_console_line_per_file() {
    let mut script =
        Script::scripted([Lookup::Missed(Miss::Repairable {
        key: "honzuki no gekokujou".to_owned(),
    })]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    host.load(&mut runner, r"D:\Anime\Honzuki no Gekokujou S01E01.mkv")
        .await;

    // The exact override key, copy-pasteable, quotes included, for 5 s.
    let text = r#"Surtr: no Anime match for "honzuki no gekokujou""#;
    assert_eq!(host.osds(), vec![(text.to_owned(), 5)]);
    assert_eq!(host.logs(), vec![text.to_owned()]);

    // Seek, pause and a Toggle-on never repeat it while the file is loaded.
    host.time_pos = Some(600.0);
    host.trigger(&mut runner, MpvEvent::PlaybackRestart).await;
    host.observed.paused = true;
    host.trigger(&mut runner, MpvEvent::Property(Property::Pause(true)))
        .await;
    host.observed.paused = false;
    host.trigger(&mut runner, MpvEvent::Property(Property::Pause(false)))
        .await;
    assert_eq!(host.osds().len(), 1, "never repeated on seek/pause");
    assert_eq!(host.logs().len(), 1);
}

#[tokio::test]
async fn the_miss_osd_duration_is_the_policy_value() {
    let mut script = Script::scripted([Lookup::Missed(Miss::Repairable {
        key: "frieren".to_owned(),
    })]);
    let mut host = Host::new(true);
    let policy = Policy {
        miss_osd_seconds: 7,
        ..Policy::default()
    };
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy, Vec::new());
    host.load(&mut runner, r"D:\Anime\Frieren - 01.mkv").await;

    assert_eq!(
        host.logs(),
        vec![r#"Surtr: no Anime match for "frieren""#.to_owned()],
        "a repairable miss is a console line, then the OSD"
    );
    assert_eq!(
        host.osds(),
        vec![(r#"Surtr: no Anime match for "frieren""#.to_owned(), 7)]
    );
}

#[tokio::test]
async fn a_miss_is_silent_when_the_file_loads_behind_the_toggle_off() {
    let mut script =
        Script::scripted([Lookup::Missed(Miss::Repairable {
        key: "honzuki no gekokujou".to_owned(),
    })]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    let file = r"D:\Anime\Honzuki no Gekokujou S01E01.mkv";
    host.observed = LiveState {
        enabled: false,
        ..live(file, 0.0, 1440.0)
    };
    host.load_as_observed(&mut runner, file, 0.0).await;

    assert!(host.osds().is_empty(), "Toggle off means the user asked for silence");
    assert!(host.logs().is_empty());

    // Toggling back on does not conjure the missed report either.
    host.observed.enabled = true;
    host.trigger(&mut runner, MpvEvent::Property(Property::Enabled(true)))
        .await;
    assert!(host.osds().is_empty());
    assert!(host.logs().is_empty());
}

#[tokio::test]
async fn a_fatal_miss_logs_a_console_line_but_never_an_osd() {
    let mut script =
        Script::scripted([Lookup::Missed(Miss::Fatal {
        reason: "batch range or multi-episode file".to_owned(),
    })]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    let file = r"D:\Anime\Show - 01-12.mkv";
    host.load(&mut runner, file).await;

    assert!(host.osds().is_empty(), "no single override line can fix a batch");
    assert_eq!(
        host.logs(),
        vec![format!(
            "surtr: no Anime match (batch range or multi-episode file): {file}"
        )]
    );
}

#[tokio::test]
async fn a_silent_miss_produces_no_console_line_and_no_osd() {
    let mut script = Script::scripted([Lookup::Unavailable]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    host.load(&mut runner, r"D:\HomeVideos\birthday.mp4").await;

    assert!(host.osds().is_empty());
    assert!(host.logs().is_empty());
}

#[tokio::test]
async fn a_rate_limited_lookup_stays_silent_until_the_retry_fails() {
    let mut script = Script::scripted([
        Lookup::RateLimited(30_000),
        Lookup::Missed(Miss::Repairable {
        key: "honzuki no gekokujou".to_owned(),
    }),
    ]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    host.load(&mut runner, r"D:\Anime\Honzuki no Gekokujou S01E01.mkv")
        .await;

    assert!(
        host.osds().is_empty(),
        "a scheduled retry reports nothing yet"
    );
    assert!(host.logs().is_empty());

    // The retry actually fails: now the report fires, once.
    host.advance(30_000);
    host.timer(&mut runner).await;
    assert_eq!(host.osds().len(), 1);
    assert_eq!(host.logs().len(), 1);
}

#[tokio::test]
async fn a_repeated_announce_of_the_same_missed_file_does_not_repeat_the_osd() {
    let mut script = Script::scripted([
        Lookup::Missed(Miss::Repairable {
        key: "honzuki no gekokujou".to_owned(),
    }),
        Lookup::Missed(Miss::Repairable {
        key: "honzuki no gekokujou".to_owned(),
    }),
    ]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    // The adapter can announce the same file twice (startup read plus
    // `file-loaded`); the user sees the miss once.
    let file = r"D:\Anime\Honzuki no Gekokujou S01E01.mkv";
    host.load(&mut runner, file).await;
    host.load(&mut runner, file).await;

    assert_eq!(host.osds().len(), 1);
    assert_eq!(host.logs().len(), 1);
}

#[tokio::test]
async fn a_missed_file_reports_again_when_reloaded_after_end_file() {
    let mut script = Script::scripted([
        Lookup::Missed(Miss::Repairable {
        key: "honzuki no gekokujou".to_owned(),
    }),
        Lookup::Missed(Miss::Repairable {
        key: "honzuki no gekokujou".to_owned(),
    }),
    ]);
    let mut host = Host::new(true);
    let mut runner = Runner::new(&mut script, host.claim(), PID, T0, policy(), Vec::new());

    let file = r"D:\Anime\Honzuki no Gekokujou S01E01.mkv";
    host.load(&mut runner, file).await;
    assert_eq!(host.osds().len(), 1);

    host.trigger(&mut runner, MpvEvent::EndFile).await;
    host.advance(GRACE_MS);
    host.load(&mut runner, file).await;

    assert_eq!(host.osds().len(), 2, "a new load of the file may fire again");
}
