//! Kernel tests: one Frame in, one Outcome out. The driving verbs are what
//! the user did (load, seek, toggle, focus, wait); the assertions are on the
//! ordered effects the host would apply. Time is a frame field, so every test
//! runs on a virtual timeline of literal milliseconds.

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;

use surtr::daemon::{
    Effect, Frame, Input, LiveState, Lookup, Metadata, MetadataClient, Miss, MpvEvent, Outcome,
    OwnerClaim, Policy, Presence, Property, Step, StepBuilder, BUCKET_CAPACITY, CLAIM_DEBOUNCE_MS,
    CLAIM_POLL_MS, CLAIM_SETTLE_MS, GRACE_MS, MISS_OSD_SECONDS, REFILL_MS,
};

const T0: i64 = 1_700_000_000_000;

/// The process id every test Daemon runs as.
const PID: u32 = 4242;

// ---------------------------------------------------------------------------
// Harness: a scripted Match lookup and an in-memory owner.json
// ---------------------------------------------------------------------------

/// In-memory `owner.json`: `None` is an absent file. Cloned so a test can take
/// the claim from another window mid-flight.
#[derive(Clone, Default)]
struct SharedClaim(Rc<Cell<Option<u32>>>);

impl OwnerClaim for SharedClaim {
    fn claim(&mut self, pid: u32) {
        self.0.set(Some(pid));
    }
    fn owner(&self) -> Option<u32> {
        self.0.get()
    }
}

/// A scripted Match lookup: one Lookup Answer per lookup, `Unavailable` once
/// the script runs out (no Presence, no report).
#[derive(Default)]
struct Script {
    answers: VecDeque<Lookup>,
    asked: Vec<String>,
}

impl MetadataClient for Script {
    fn lookup(&mut self, path: &str) -> Lookup {
        self.asked.push(path.to_owned());
        self.answers.pop_front().unwrap_or(Lookup::Unavailable)
    }
}

fn metadata(title: &str) -> Metadata {
    Metadata {
        id: 1,
        title: title.to_owned(),
        episode: Some("Episode 1 of 12".to_owned()),
        hover: title.to_owned(),
        cover: None,
    }
}

/// The scripted Match: one card, `Frieren`.
fn matched() -> Lookup {
    Lookup::Matched(metadata("Frieren"))
}

fn live(path: &str, time_pos: f64) -> LiveState {
    LiveState {
        path: Some(path.to_owned()),
        time_pos: Some(time_pos),
        duration: Some(1440.0),
        ..LiveState::default()
    }
}

fn path(n: u8) -> String {
    format!(r"D:\Anime\Frieren - {n:02}.mkv")
}

/// The host side of the kernel: a virtual clock, the live player, the owner
/// file, and every effect the kernel produced, in order.
struct Host {
    now: i64,
    /// The position, read fresh when the kernel asks.
    live: LiveState,
    /// The observed player the host holds between frames (no position).
    observed: LiveState,
    claim: SharedClaim,
    /// Where the kernel's effects go, one entry per step.
    outcomes: Vec<Outcome>,
}

impl Host {
    /// `owning` starts with this window already holding the claim, so most
    /// tests are about Presence, not arbitration.
    fn new(owning: bool) -> Self {
        let claim = SharedClaim::default();
        if owning {
            claim.0.set(Some(PID));
        }
        Self {
            now: T0,
            live: LiveState::default(),
            observed: LiveState::default(),
            claim,
            outcomes: Vec::new(),
        }
    }

    /// The answer to a sample demand: the observed player, with the position
    /// read fresh at this instant.
    fn answer(&self) -> Input {
        Input::Sampled(LiveState {
            time_pos: self.live.time_pos,
            ..self.observed.clone()
        })
    }

    /// One raw frame. The kernel's demand marks who answers.
    fn frame(&mut self, step: &mut Step, input: Input) -> Vec<Effect> {
        let mut claim = self.claim.clone();
        let outcome = step.step(
            Frame::new(self.now, input).with_observed(self.observed.clone()),
            &mut claim,
        );
        let effects = outcome.effects.clone();
        self.outcomes.push(outcome);
        effects
    }

    /// Runs frames until nothing is left to do now: every demand is answered
    /// from the live player and every due effect applied. The demand is a
    /// signal to the host, so the host answers before poking the kernel.
    fn drain(&mut self, step: &mut Step) {
        loop {
            if step.awaiting_sample() {
                let answer = self.answer();
                self.frame(step, answer);
            } else if !self.frame(step, Input::Timer).contains(&Effect::Sample) {
                break;
            }
        }
    }

    /// Waits `ms`, then drains (what the host's sleep does).
    fn advance(&mut self, step: &mut Step, ms: i64) {
        self.now += ms;
        self.drain(step);
    }

    fn send(&mut self, step: &mut Step, input: Input) {
        self.frame(step, input);
        self.drain(step);
    }

    fn load(&mut self, step: &mut Step, n: u8) {
        self.load_at(step, n, 0.0);
    }

    /// Loads file `n` with the player at `time_pos`; the host observes
    /// everything about the load except that position.
    fn load_at(&mut self, step: &mut Step, n: u8, time_pos: f64) {
        let path = path(n);
        self.live = live(&path, time_pos);
        self.observed = LiveState {
            time_pos: None,
            ..self.live.clone()
        };
        self.send(
            step,
            Input::Mpv(MpvEvent::FileLoaded {
                path,
                live: self.live.clone(),
            }),
        );
    }

    fn toggle(&mut self, step: &mut Step, on: bool) {
        self.live.enabled = on;
        self.observed.enabled = on;
        self.send(step, Input::Mpv(MpvEvent::Property(Property::Enabled(on))));
    }

    /// A seek: playback restarts with the position at `time_pos`.
    fn seek(&mut self, step: &mut Step, time_pos: f64) {
        self.live.time_pos = Some(time_pos);
        self.send(step, Input::Mpv(MpvEvent::PlaybackRestart));
    }

    /// Every effect of every step, in order.
    fn effects(&self) -> Vec<Effect> {
        self.outcomes
            .iter()
            .flat_map(|outcome| outcome.effects.iter().cloned())
            .collect()
    }

    fn publishes(&self) -> Vec<Presence> {
        self.effects()
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::Publish(presence) => Some(presence),
                _ => None,
            })
            .collect()
    }

    fn clears(&self) -> usize {
        self.effects()
            .into_iter()
            .filter(|effect| matches!(effect, Effect::Clear))
            .count()
    }

    fn samples(&self) -> usize {
        self.effects()
            .into_iter()
            .filter(|effect| matches!(effect, Effect::Sample))
            .count()
    }
}

/// Opens the kernel with the scripted lookup and installs the Anime Roots
/// through the `Init` frame.
fn open<'a>(script: &'a mut Script, host: &mut Host) -> Step<'a> {
    let mut step = Step::new(script, PID, T0);
    host.send(&mut step, Input::Init { roots: Vec::new() });
    step
}

// ---------------------------------------------------------------------------
// The effect contract
// ---------------------------------------------------------------------------

#[test]
fn a_frame_returns_ordered_effects_and_the_next_wake() {
    let mut script = Script {
        answers: VecDeque::from([matched()]),
        ..Script::default()
    };
    let mut host = Host::new(true);
    let mut step = open(&mut script, &mut host);

    let path = path(1);
    host.live = live(&path, 120.0);
    host.observed = LiveState {
        time_pos: None,
        ..host.live.clone()
    };
    let effects = host.frame(
        &mut step,
        Input::Mpv(MpvEvent::FileLoaded {
            path,
            live: host.live.clone(),
        }),
    );

    assert_eq!(effects, vec![Effect::Sample], "a load admits a sample");
    assert_eq!(
        host.outcomes.last().unwrap().wake_at,
        Some(T0 + CLAIM_POLL_MS),
        "an Owner re-checks its claim at the policy cadence"
    );
    let answered = host.answer();
    let answer = host.frame(&mut step, answered);
    assert!(matches!(answer.as_slice(), [Effect::Publish(_)]));
}

#[test]
fn a_clear_precedes_the_publish_of_a_new_file_in_one_outcome() {
    let mut script = Script {
        answers: VecDeque::from([matched(), matched()]),
        ..Script::default()
    };
    let mut host = Host::new(true);
    let mut step = open(&mut script, &mut host);
    host.load(&mut step, 1);

    // Two publishes pending in one outcome (a card up, then a new file): the
    // clear always goes first.
    let second = path(2);
    host.live = live(&second, 0.0);
    let effects = host.frame(
        &mut step,
        Input::Mpv(MpvEvent::FileLoaded {
            path: second,
            live: host.live.clone(),
        }),
    );
    assert_eq!(
        effects,
        vec![Effect::Clear, Effect::Sample],
        "the stale card is blanked in the same frame the new sample is asked"
    );

    let answered = host.frame(&mut step, Input::Sampled(host.live.clone()));
    assert!(matches!(answered.as_slice(), [Effect::Publish(_)]));
    assert_eq!(host.publishes().len(), 2);
}

#[test]
fn a_clear_is_emitted_only_when_a_card_is_up() {
    let mut script = Script {
        answers: VecDeque::from([Lookup::Unavailable]),
        ..Script::default()
    };
    let mut host = Host::new(true);
    let mut step = open(&mut script, &mut host);

    // Nothing was ever published: the miss needs no `activity: null`.
    host.load(&mut step, 1);
    assert_eq!(host.clears(), 0);

    host.send(&mut step, Input::Mpv(MpvEvent::Shutdown));
    assert_eq!(host.clears(), 0, "shutdown of an empty card clears nothing");
}

#[test]
fn a_sample_answer_that_arrives_after_a_shutdown_publishes_nothing() {
    let mut script = Script {
        answers: VecDeque::from([matched()]),
        ..Script::default()
    };
    let mut host = Host::new(true);
    let mut step = open(&mut script, &mut host);

    let path = path(1);
    host.live = live(&path, 0.0);
    host.observed = LiveState {
        time_pos: None,
        ..host.live.clone()
    };
    host.frame(
        &mut step,
        Input::Mpv(MpvEvent::FileLoaded {
            path,
            live: host.live.clone(),
        }),
    );
    let shutdown = host.frame(&mut step, Input::Mpv(MpvEvent::Shutdown));
    assert_eq!(
        shutdown,
        vec![Effect::Publish(host.publishes()[0].clone()), Effect::Clear],
        "the lapsed demand published from the observed player, then the clear"
    );
    let late = host.frame(&mut step, Input::Sampled(host.live.clone()));
    assert_eq!(
        late,
        Vec::new(),
        "the demand was already answered: a late answer publishes nothing"
    );
    let samples = host.samples();
    assert_eq!(samples, 1, "the demand never re-armed");
    assert_eq!(host.publishes().len(), 1);
}

// ---------------------------------------------------------------------------
// A file load leads to a publish
// ---------------------------------------------------------------------------

#[test]
fn a_file_load_publishes_the_match_from_the_sampled_player() {
    let mut script = Script {
        answers: VecDeque::from([matched()]),
        ..Script::default()
    };
    let mut host = Host::new(true);
    let mut step = open(&mut script, &mut host);

    host.load_at(&mut step, 1, 120.0);

    assert_eq!(script.asked, vec![path(1)]);
    let publish = host.publishes().pop().expect("a publish");
    assert_eq!(publish.details, "Frieren");
    assert_eq!(publish.start_ms, Some(T0 - 120_000));
    assert_eq!(publish.end_ms, Some(T0 + 1_320_000));
    assert_eq!(
        host.samples(),
        1,
        "the position is read at dispatch, not at load"
    );
}

#[test]
fn the_sample_answer_and_not_the_load_snapshot_feeds_the_publish() {
    let mut script = Script {
        answers: VecDeque::from([matched()]),
        ..Script::default()
    };
    let mut host = Host::new(true);
    let mut step = open(&mut script, &mut host);

    let file = path(1);
    host.live = live(&file, 0.0);
    host.observed = LiveState {
        time_pos: None,
        ..host.live.clone()
    };
    // A seek arrives after the load was admitted but before the answer.
    host.frame(
        &mut step,
        Input::Mpv(MpvEvent::FileLoaded {
            path: file,
            live: host.live.clone(),
        }),
    );
    host.live.time_pos = Some(600.0);
    host.now += 10_000;
    let answered = host.answer();
    host.frame(&mut step, answered);

    let publish = host.publishes().pop().expect("a publish");
    assert_eq!(
        publish.start_ms,
        Some(T0 + 10_000 - 600_000),
        "the card carries the position at dispatch"
    );
}

// ---------------------------------------------------------------------------
// Toggle-off leads to a clear
// ---------------------------------------------------------------------------

#[test]
fn toggle_off_clears_the_card_at_once() {
    let mut script = Script {
        answers: VecDeque::from([matched()]),
        ..Script::default()
    };
    let mut host = Host::new(true);
    let mut step = open(&mut script, &mut host);
    host.load(&mut step, 1);
    assert_eq!(host.publishes().len(), 1);

    host.toggle(&mut step, false);

    assert_eq!(host.clears(), 1);
    assert_eq!(host.publishes().len(), 1, "nothing is published while off");

    // Toggle-on brings the same Match back from memory, no second lookup.
    host.toggle(&mut step, true);
    let published = host.publishes().len();
    let asked = script.asked.len();
    assert_eq!(published, 2);
    assert_eq!(asked, 1);
}

#[test]
fn toggle_off_behind_a_dormant_daemon_emits_nothing() {
    let mut script = Script {
        answers: VecDeque::from([matched()]),
        ..Script::default()
    };
    let mut host = Host::new(false);
    let mut step = open(&mut script, &mut host);
    host.load(&mut step, 1);
    host.toggle(&mut step, false);

    assert_eq!(host.effects(), Vec::new(), "no card, no clear, no claim");
}

// ---------------------------------------------------------------------------
// Shutdown leads to a clear, then an exit
// ---------------------------------------------------------------------------

#[test]
fn shutdown_clears_then_exits() {
    let mut script = Script {
        answers: VecDeque::from([matched()]),
        ..Script::default()
    };
    let mut host = Host::new(true);
    let mut step = open(&mut script, &mut host);
    host.load(&mut step, 1);

    let mut claim = host.claim.clone();
    let done = step.step(
        Frame::new(host.now, Input::Mpv(MpvEvent::Shutdown)),
        &mut claim,
    );

    assert!(done.exit, "shutdown terminates the host loop");
    assert_eq!(done.effects, vec![Effect::Clear]);
    assert_eq!(
        done.wake_at, None,
        "an exited host has nothing left to wake for"
    );
}

#[test]
fn an_unowned_shutdown_exits_without_a_clear() {
    let mut script = Script {
        answers: VecDeque::from([matched()]),
        ..Script::default()
    };
    let mut host = Host::new(false);
    let mut step = open(&mut script, &mut host);
    host.load(&mut step, 1);
    let mut claim = host.claim.clone();
    let done = step.step(
        Frame::new(host.now, Input::Mpv(MpvEvent::Shutdown)),
        &mut claim,
    );

    assert!(done.exit);
    assert_eq!(done.effects, Vec::new());
}

// ---------------------------------------------------------------------------
// The sample demand: one answer, no chaining
// ---------------------------------------------------------------------------

#[test]
fn one_step_emits_at_most_one_sample_demand() {
    let mut script = Script {
        answers: VecDeque::from([matched()]),
        ..Script::default()
    };
    let mut host = Host::new(true);
    let mut step = open(&mut script, &mut host);
    host.load(&mut step, 1);

    // A scrub burst before the core gets a chance to dispatch: ten seeks,
    // each its own frame, and no more than one demand in any of them.
    for position in 1..=10 {
        host.live.time_pos = Some(position as f64);
        let effects = host.frame(&mut step, Input::Mpv(MpvEvent::PlaybackRestart));
        let demands = effects.iter().filter(|e| **e == Effect::Sample).count();
        assert!(demands <= 1, "a frame emits at most one demand");
    }
    host.drain(&mut step);
    host.advance(&mut step, BUCKET_CAPACITY as i64 * REFILL_MS);
    // The last seek of the burst is the one whose position the card carries:
    // playback restarts once, sampled at that position.
    host.seek(&mut step, 10.0);

    let publishes = host.publishes();
    let publish = publishes.last().expect("a publish");
    assert_eq!(
        publish.start_ms,
        Some(host.now - 10_000),
        "the coalesced publish resamples the final position"
    );
    assert!(
        publishes.len() < 10,
        "the burst is one latest-wins update, not ten: got {}",
        publishes.len()
    );
}

#[test]
fn the_demand_never_chains() {
    let mut script = Script {
        answers: VecDeque::from([matched()]),
        ..Script::default()
    };
    let mut host = Host::new(true);
    let mut step = open(&mut script, &mut host);
    host.load(&mut step, 1);

    host.frame(&mut step, Input::Mpv(MpvEvent::PlaybackRestart));
    assert!(step.awaiting_sample(), "the seek admits one demand");
    let answered = host.frame(&mut step, Input::Sampled(host.live.clone()));
    assert!(matches!(answered.as_slice(), [Effect::Publish(_)]));
    assert!(
        !step.awaiting_sample(),
        "the answer feeds a publish, never another demand"
    );

    // An answer to no demand changes nothing.
    let surplus = host.frame(&mut step, Input::Sampled(host.live.clone()));
    assert_eq!(surplus, Vec::new());
}

// ---------------------------------------------------------------------------
// Timing policy
// ---------------------------------------------------------------------------

#[test]
fn the_policy_defaults_reproduce_the_shipped_timing() {
    let policy = Policy::default();
    assert_eq!(policy.grace_ms, GRACE_MS);
    assert_eq!(policy.bucket_capacity, BUCKET_CAPACITY);
    assert_eq!(policy.refill_ms, REFILL_MS);
    assert_eq!(policy.claim_debounce_ms, CLAIM_DEBOUNCE_MS);
    assert_eq!(policy.claim_settle_ms, CLAIM_SETTLE_MS);
    assert_eq!(policy.claim_poll_ms, CLAIM_POLL_MS);
    assert_eq!(policy.miss_osd_seconds, MISS_OSD_SECONDS);
    assert_eq!((GRACE_MS, REFILL_MS), (500, 4_000));
    assert_eq!((BUCKET_CAPACITY, CLAIM_POLL_MS), (5, 250));
    assert_eq!((CLAIM_DEBOUNCE_MS, CLAIM_SETTLE_MS), (300, 400));
    assert_eq!(MISS_OSD_SECONDS, 5);
}

#[test]
fn the_grace_window_is_the_policy_value() {
    let mut script = Script {
        answers: VecDeque::from([matched()]),
        ..Script::default()
    };
    let policy = Policy {
        grace_ms: 1_000,
        ..Policy::default()
    };
    let mut host = Host::new(true);
    let mut step = StepBuilder::default()
        .policy(policy)
        .build(&mut script, PID, T0);
    host.send(&mut step, Input::Init { roots: Vec::new() });
    host.load(&mut step, 1);

    host.send(&mut step, Input::Mpv(MpvEvent::EndFile));
    host.advance(&mut step, 999);
    assert_eq!(host.clears(), 0, "the long grace holds the card");
    host.advance(&mut step, 1);
    assert_eq!(host.clears(), 1);
}

#[test]
fn the_miss_osd_duration_is_the_policy_value() {
    let mut script = Script {
        answers: VecDeque::from([Lookup::Missed(Miss::Repairable {
        key: "frieren".to_owned(),
    })]),
        ..Script::default()
    };
    let policy = Policy {
        miss_osd_seconds: 7,
        ..Policy::default()
    };
    let mut host = Host::new(true);
    let mut step = StepBuilder::default()
        .policy(policy)
        .build(&mut script, PID, T0);
    host.send(&mut step, Input::Init { roots: Vec::new() });
    host.load(&mut step, 1);

    assert_eq!(
        host.effects(),
        vec![
            Effect::Log(r#"Surtr: no Anime match for "frieren""#.to_owned()),
            Effect::Osd {
                text: r#"Surtr: no Anime match for "frieren""#.to_owned(),
                seconds: 7,
            },
        ],
        "a repairable miss is a console line, then the OSD"
    );
}

// ---------------------------------------------------------------------------
// Init installs the Anime Roots, exactly once
// ---------------------------------------------------------------------------

#[test]
fn init_installs_the_anime_roots_for_every_eligible_file() {
    let mut script = Script {
        answers: VecDeque::from([matched()]),
        ..Script::default()
    };
    let mut host = Host::new(true);
    let mut step = Step::new(&mut script, PID, T0);
    host.send(
        &mut step,
        Input::Init {
            roots: vec![r"D:\Anime".to_owned()],
        },
    );

    host.load(&mut step, 1);
    assert_eq!(script.asked, vec![path(1)]);
    assert_eq!(host.publishes().len(), 1);
}

#[test]
fn a_file_outside_every_anime_root_is_skipped_with_a_console_line() {
    let mut script = Script {
        answers: VecDeque::from([matched()]),
        ..Script::default()
    };
    let mut host = Host::new(true);
    let mut step = Step::new(&mut script, PID, T0);
    host.send(
        &mut step,
        Input::Init {
            roots: vec![r"D:\Anime".to_owned()],
        },
    );
    host.load(&mut step, 1);

    let outside = r"D:\Anime2\Show - 01.mkv";
    host.live = live(outside, 0.0);
    host.send(
        &mut step,
        Input::Mpv(MpvEvent::FileLoaded {
            path: outside.to_owned(),
            live: host.live.clone(),
        }),
    );

    assert_eq!(script.asked, vec![path(1)], "no lookup outside a root");
    assert_eq!(host.clears(), 1, "the old card is blanked");
    assert_eq!(host.publishes().len(), 1);
    // The user still gets the skip console line.
    assert_eq!(
        host.effects()
            .iter()
            .filter(|effect| matches!(effect, Effect::Log(_)))
            .count(),
        1,
        "an ineligible file is still reported on the console"
    );
}

// ---------------------------------------------------------------------------
// The claim and the timing the outcome schedules
// ---------------------------------------------------------------------------

#[test]
fn a_dormant_daemon_wakes_only_for_its_own_focus_and_settles_before_publishing() {
    let mut script = Script {
        answers: VecDeque::from([matched()]),
        ..Script::default()
    };
    let mut host = Host::new(false);
    let mut step = open(&mut script, &mut host);
    host.load(&mut step, 1);

    assert_eq!(
        host.outcomes.last().unwrap().wake_at,
        None,
        "a dormant Daemon with no claim has nothing to wake for"
    );

    host.send(&mut step, Input::Mpv(MpvEvent::FocusChanged(true)));
    assert_eq!(
        host.outcomes.last().unwrap().wake_at,
        Some(T0 + CLAIM_DEBOUNCE_MS),
        "the debounce travels in the outcome"
    );
    assert_eq!(host.claim.owner(), None);

    host.advance(&mut step, CLAIM_DEBOUNCE_MS);
    assert_eq!(host.claim.owner(), Some(PID));
    assert_eq!(host.publishes().len(), 0, "the settle is still running");
    let wake = host.outcomes.last().unwrap().wake_at.expect("a settle");
    assert!(
        wake <= T0 + CLAIM_DEBOUNCE_MS + CLAIM_SETTLE_MS,
        "the settle lands within the policy window, got {wake}"
    );

    host.advance(&mut step, CLAIM_SETTLE_MS);
    assert_eq!(host.publishes().len(), 1);
    let asked = script.asked.len();
    assert_eq!(asked, 1, "the Match is reused");
}

#[test]
fn an_owner_repolls_the_claim_on_the_policy_cadence() {
    let mut script = Script {
        answers: VecDeque::from([matched()]),
        ..Script::default()
    };
    let mut host = Host::new(true);
    let mut step = open(&mut script, &mut host);
    host.load(&mut step, 1);

    let wake = host.outcomes.last().unwrap().wake_at.expect("a poll");
    assert_eq!(wake, host.now + CLAIM_POLL_MS);
}

#[test]
fn a_handover_clears_the_old_card_within_one_poll() {
    let mut script = Script {
        answers: VecDeque::from([matched()]),
        ..Script::default()
    };
    let mut host = Host::new(true);
    let mut step = open(&mut script, &mut host);
    host.load(&mut step, 1);

    // Another mpv window takes focus and claims.
    host.claim.claim(PID + 1);
    host.advance(&mut step, CLAIM_POLL_MS);

    assert_eq!(host.clears(), 1);
    assert_eq!(host.publishes().len(), 1);
}

#[test]
fn a_deferred_429_retry_wakes_at_its_due_time_and_publishes_once() {
    let mut script = Script {
        answers: VecDeque::from([Lookup::RateLimited(30_000), matched()]),
        ..Script::default()
    };
    let mut host = Host::new(true);
    let mut step = open(&mut script, &mut host);
    host.load(&mut step, 1);

    assert_eq!(
        host.outcomes.last().unwrap().wake_at,
        Some(T0 + CLAIM_POLL_MS),
        "a 429 does not beat the Owner's claim poll"
    );
    assert_eq!(host.publishes().len(), 0, "429 is not a Match");

    host.advance(&mut step, 29_999);
    host.advance(&mut step, 1);
    let asked = script.asked.len();

    assert_eq!(asked, 2, "exactly one deferred retry, at its due time");
    assert_eq!(host.publishes().len(), 1);
}
