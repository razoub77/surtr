//! Runner tests: the production shell around the kernel. The loop here is the
//! one the daemon runs — one mpv event, or the timer at the wake the Runner
//! asked for — and the assertions are on the effects the host would apply, in
//! order.

use std::cell::Cell;
use std::collections::VecDeque;
use std::future::{ready, Future};
use std::pin::Pin;
use std::rc::Rc;

use surtr::daemon::{
    Effect, LiveState, Metadata, MetadataClient, MpvEvent, OwnerClaim, Policy, Presence, Property,
    Runner,
};

const T0: i64 = 1_700_000_000_000;

/// The process id every test Daemon runs as.
const PID: u32 = 4242;

/// In-memory `owner.json`: `None` is an absent file. Cloned so a test can
/// hand the claim to another window while this Daemon is mid-flight.
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

/// A scripted Match lookup: one `true` (a Match) or `false` (a miss) per
/// lookup, `false` once the script runs out.
#[derive(Default)]
struct Script {
    answers: VecDeque<bool>,
    asked: Vec<String>,
}

impl MetadataClient for Script {
    fn lookup(&mut self, path: &str) -> Option<Metadata> {
        self.asked.push(path.to_owned());
        self.answers.pop_front().filter(|matched| *matched).map(|_| Metadata {
            id: 1,
            title: "Frieren".to_owned(),
            episode: Some("Episode 1 of 12".to_owned()),
            hover: "Frieren".to_owned(),
            cover: None,
        })
    }
}

fn path(n: u8) -> String {
    format!(r"D:\Anime\Frieren - {n:02}.mkv")
}

fn live(path: &str, time_pos: f64) -> LiveState {
    LiveState {
        path: Some(path.to_owned()),
        time_pos: Some(time_pos),
        duration: Some(1440.0),
        ..LiveState::default()
    }
}

/// The host side of the Runner: the clock, the live player, and every effect
/// the Runner produced, in order.
struct Host {
    now: i64,
    live: LiveState,
    /// What mpv observed: everything about the loaded file except the
    /// position, which is only read when a publish is dispatched.
    observed: LiveState,
    effects: Vec<Effect>,
}

type BoxFuture<T> = Pin<Box<dyn Future<Output = T>>>;

impl Host {
    fn new(owning: bool) -> (Self, SharedClaim) {
        let mut claim = SharedClaim::default();
        if owning {
            claim.claim(PID);
        }
        (
            Self {
                now: T0,
                live: LiveState::default(),
                observed: LiveState::default(),
                effects: Vec::new(),
            },
            claim,
        )
    }

    /// The answer to a sample demand: the observed player, with the position
    /// read fresh at this instant.
    fn sampler(&self) -> impl FnMut() -> BoxFuture<LiveState> + '_ {
        move || {
            Box::pin(ready(LiveState {
                time_pos: self.live.time_pos,
                ..self.observed.clone()
            }))
        }
    }

    /// One turn of the daemon loop, then the timer at each wake the Runner
    /// asked for, until it is quiet or the turns run out.
    async fn run(&mut self, runner: &mut Runner<'_, SharedClaim>, event: MpvEvent) -> Vec<Effect> {
        let observed = match &event {
            MpvEvent::FileLoaded { live, .. } => live.clone(),
            _ => LiveState::default(),
        };
        let mut effects = runner
            .mpv(self.now, event, observed, &mut self.sampler())
            .await;
        // In the real daemon each turn is the sleep arm of the select: one
        // timer at the advertised wake. A claim poll re-arms itself, so the
        // host would just sleep again; the test takes a few turns and stops.
        // A 429 retry needs at most one poll turn before its own deadline.
        for _ in 0..4 {
            let Some(at) = runner.wake_at() else { break };
            if at <= self.now {
                break;
            }
            self.now = at;
            effects.extend(runner.timer(self.now, &mut self.sampler()).await);
            if runner.has_exited() {
                break;
            }
        }
        self.effects.extend(effects.iter().cloned());
        effects
    }

    /// Loads `file` with the player `at` seconds in. The load reports the
    /// snapshot it read, position included.
    async fn load(
        &mut self,
        runner: &mut Runner<'_, SharedClaim>,
        file: &str,
        at: f64,
    ) -> Vec<Effect> {
        self.live = live(file, at);
        self.observed = LiveState {
            time_pos: None,
            ..self.live.clone()
        };
        let event = MpvEvent::FileLoaded {
            path: file.to_owned(),
            live: self.live.clone(),
        };
        self.run(runner, event).await
    }

    fn publishes(&self) -> Vec<Presence> {
        self.effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Publish(presence) => Some(presence.clone()),
                _ => None,
            })
            .collect()
    }

    fn clears(&self) -> usize {
        self.effects
            .iter()
            .filter(|effect| matches!(effect, Effect::Clear))
            .count()
    }
}

#[tokio::test]
async fn a_load_publishes_from_the_sampled_player() {
    let mut script = Script {
        answers: VecDeque::from([true]),
        ..Script::default()
    };
    let (mut host, claim) = Host::new(true);
    let mut runner = Runner::new(&mut script, claim, PID, T0, Policy::default(), Vec::new());

    let file = path(1);
    let effects = host.load(&mut runner, &file, 120.0).await;

    assert_eq!(script.asked, vec![file]);
    assert!(
        !effects.contains(&Effect::Sample),
        "the demand is the Runner's to answer, not the host's to apply"
    );
    let publish = host.publishes().pop().expect("a publish");
    assert_eq!(publish.details, "Frieren");
    assert_eq!(publish.start_ms, Some(T0 - 120_000));
    assert_eq!(publish.end_ms, Some(T0 + 1_320_000));
}

#[tokio::test]
async fn the_publish_carries_the_position_at_dispatch() {
    let mut script = Script {
        answers: VecDeque::from([true]),
        ..Script::default()
    };
    let (mut host, claim) = Host::new(true);
    let mut runner = Runner::new(&mut script, claim, PID, T0, Policy::default(), Vec::new());

    let file = path(1);
    host.load(&mut runner, &file, 0.0).await;
    assert_eq!(host.publishes().pop().unwrap().start_ms, Some(T0));

    // Seek 600 s in: the next dispatch samples the player where it is now.
    host.live.time_pos = Some(600.0);
    host.now = T0 + 10_000;
    host.run(&mut runner, MpvEvent::PlaybackRestart).await;

    let publish = host.publishes().pop().expect("a republish");
    assert_eq!(
        publish.start_ms,
        Some(T0 + 10_000 - 600_000),
        "the card carries the position at dispatch, not at the load"
    );
}

#[tokio::test]
async fn toggle_off_clears_within_the_same_turn() {
    let mut script = Script {
        answers: VecDeque::from([true]),
        ..Script::default()
    };
    let (mut host, claim) = Host::new(true);
    let mut runner = Runner::new(&mut script, claim, PID, T0, Policy::default(), Vec::new());

    host.load(&mut runner, &path(1), 0.0).await;
    assert_eq!(host.publishes().len(), 1);

    host.live.enabled = false;
    host.run(&mut runner, MpvEvent::Property(Property::Enabled(false)))
        .await;

    assert_eq!(host.clears(), 1, "a Toggle-off never waits for a token");
    assert_eq!(host.publishes().len(), 1);
}

#[tokio::test]
async fn shutdown_clears_then_stops_the_host() {
    let mut script = Script {
        answers: VecDeque::from([true]),
        ..Script::default()
    };
    let (mut host, claim) = Host::new(true);
    let mut runner = Runner::new(&mut script, claim, PID, T0, Policy::default(), Vec::new());

    host.load(&mut runner, &path(1), 0.0).await;
    assert!(!runner.has_exited());

    let effects = host.run(&mut runner, MpvEvent::Shutdown).await;

    assert_eq!(effects.last(), Some(&Effect::Clear));
    assert!(runner.has_exited(), "the host loop has nothing left to do");
}

#[tokio::test]
async fn a_deferred_retry_wakes_at_the_deadline_the_runner_asked_for() {
    /// A first lookup rate limited, a second one empty.
    #[derive(Default)]
    struct RateLimited {
        calls: Rc<Cell<usize>>,
    }

    impl MetadataClient for RateLimited {
        fn lookup(&mut self, _path: &str) -> Option<Metadata> {
            self.calls.set(self.calls.get() + 1);
            None
        }
        fn retry_after(&self) -> Option<i64> {
            (self.calls.get() == 1).then_some(30_000)
        }
    }

    let calls = Rc::new(Cell::new(0));
    let mut script = RateLimited {
        calls: calls.clone(),
    };
    let (mut host, claim) = Host::new(true);
    let mut runner = Runner::new(&mut script, claim, PID, T0, Policy::default(), Vec::new());

    host.load(&mut runner, &path(1), 0.0).await;
    assert_eq!(calls.get(), 1, "a 429 is not a Match");
    assert!(host.publishes().is_empty());

    // The Owner's claim poll comes first; keep taking the advertised wake
    // until the retry's own deadline arrives.
    while calls.get() == 1 {
        let Some(at) = runner.wake_at() else {
            panic!("a wake for the retry");
        };
        if at <= host.now {
            panic!("the retry wake already passed: {at} at {}", host.now);
        }
        host.now = at;
        let more = runner.timer(host.now, &mut host.sampler()).await;
        host.effects.extend(more);
    }

    assert_eq!(calls.get(), 2, "exactly one deferred retry");
    assert_eq!(host.now, T0 + 30_000, "at its Retry-After");
}

#[tokio::test]
async fn a_dormant_daemon_claims_on_focus_and_publishes_its_match() {
    let mut script = Script {
        answers: VecDeque::from([true]),
        ..Script::default()
    };
    let (mut host, claim) = Host::new(false);
    let mut runner = Runner::new(&mut script, claim, PID, T0, Policy::default(), Vec::new());

    host.load(&mut runner, &path(1), 0.0).await;
    assert!(host.effects.is_empty(), "a non-Owner publishes nothing");
    assert_eq!(
        runner.wake_at(),
        None,
        "a dormant Daemon has nothing to wake for"
    );

    host.run(&mut runner, MpvEvent::FocusChanged(true)).await;

    let claim = runner.into_claim();
    assert_eq!(claim.owner(), Some(PID), "this window wrote the claim");
    assert_eq!(host.publishes().len(), 1, "the Match is reused");
    assert_eq!(script.asked.len(), 1, "no second lookup");
}

