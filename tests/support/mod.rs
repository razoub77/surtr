//! The one harness for the Daemon Core scenarios: a scripted Match lookup, an
//! in-memory `owner.json`, and the Host — the one definition of "a turn of
//! the daemon". Time is a plain integer in milliseconds; effects are recorded
//! in order. No sockets, no pipes, no real time.

#![allow(dead_code)] // every test binary compiles its own copy; not all use everything

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::future::{ready, Future};
use std::pin::Pin;
use std::rc::Rc;

use surtr::anilist::{Candidate, Titles};
use surtr::daemon::{
    Effect, Level, LiveState, Lookup, Metadata, MetadataClient, MpvEvent, OwnerClaim, Presence,
    Runner, CLAIM_DEBOUNCE_MS, CLAIM_SETTLE_MS,
};

/// The instant every test Daemon starts at.
pub const T0: i64 = 1_700_000_000_000;

/// The process id every test Daemon runs as.
pub const PID: u32 = 4242;

/// In-memory `owner.json`: `None` is an absent file. Clones share the cell,
/// so a test can hand the claim to another window mid-flight.
#[derive(Clone, Default)]
pub struct SharedClaim(Rc<Cell<Option<u32>>>);

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
pub struct Script {
    script: VecDeque<Lookup>,
    /// Every path looked up, shared so a test can read it while the Runner
    /// still holds the client.
    lookups: Rc<RefCell<Vec<String>>>,
}

impl Script {
    pub fn scripted(script: impl IntoIterator<Item = Lookup>) -> Self {
        Self {
            script: script.into_iter().collect(),
            lookups: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// A handle on every path looked up so far.
    pub fn lookups(&self) -> Rc<RefCell<Vec<String>>> {
        self.lookups.clone()
    }
}

impl MetadataClient for Script {
    fn lookup(&mut self, path: &str) -> Lookup {
        self.lookups.borrow_mut().push(path.to_owned());
        self.script.pop_front().unwrap_or(Lookup::Unavailable)
    }
}

/// The Metadata of a Match: one card, the title given.
pub fn metadata(title: &str) -> Metadata {
    Metadata {
        id: 1,
        title: title.to_owned(),
        episode: None,
        hover: title.to_owned(),
        cover: None,
    }
}

/// The scripted Match: one card, `Frieren`.
pub fn matched() -> Lookup {
    Lookup::Matched(metadata("Frieren"))
}

/// A TV Candidate with a romaji title: the shape a MediaSource fake hands out.
pub fn candidate(id: i64, romaji: &str) -> Candidate {
    Candidate {
        id,
        format: Some("TV".into()),
        season_year: None,
        season: None,
        status: None,
        episodes: Some(12),
        genres: Vec::new(),
        title: Some(Titles {
            romaji: Some(romaji.into()),
            english: None,
        }),
        synonyms: Vec::new(),
        cover_image: None,
    }
}

type BoxFuture<T> = Pin<Box<dyn Future<Output = T>>>;

/// The host side of the Runner: the clock, the player as the host holds it,
/// the owner file, and every effect the Runner produced, in order.
///
/// The loop here is the one the daemon runs: one stimulus, then the timer at
/// each wake the Runner asked for. A claim poll re-arms itself, so the host
/// would just sleep again; a few turns and the test stops.
pub struct Host {
    pub now: i64,
    /// The player as mpv reports it, position excluded: position is only read
    /// when a publish is dispatched.
    pub observed: LiveState,
    /// The position `time-pos` would report right now.
    pub time_pos: Option<f64>,
    /// Everything the Runner handed back, in order.
    pub effects: Vec<Effect>,
    /// Whether this window started out holding the claim.
    owning: bool,
}

impl Host {
    /// Most tests are about Presence, not arbitration: `owning` starts with
    /// this window already holding the claim, so the Daemon publishes as the
    /// Owner. `false` is dormant — no `owner.json`.
    pub fn new(owning: bool) -> Self {
        Self {
            now: T0,
            observed: LiveState::default(),
            time_pos: None,
            effects: Vec::new(),
            owning,
        }
    }

    /// The claim this Daemon carries: named after it when it owns Discord,
    /// absent when dormant. Clones share the file's cell.
    pub fn claim(&self) -> SharedClaim {
        let claim = SharedClaim::default();
        if self.owning {
            claim.0.set(Some(PID));
        }
        claim
    }

    /// The answer to a sample demand: the observed player, with the position
    /// read fresh at this instant.
    pub fn sampler(&self) -> impl FnMut() -> BoxFuture<LiveState> + '_ {
        move || {
            Box::pin(ready(LiveState {
                time_pos: self.time_pos,
                ..self.observed.clone()
            }))
        }
    }

    pub fn advance(&mut self, millis: i64) {
        self.now += millis;
    }

    /// One stimulus, then the timer at each wake the Runner asked for, until
    /// it is quiet or the turns run out. Time moves only to a wake the Runner
    /// advertised — the sleep ends at the wake, never before — or to an
    /// explicit `advance`.
    pub async fn run(
        &mut self,
        runner: &mut Runner<'_, SharedClaim>,
        event: MpvEvent,
    ) -> Vec<Effect> {
        let mut effects = self.turn(runner, event).await;
        for _ in 0..4 {
            let Some(at) = runner.wake_at() else {
                break;
            };
            if at > self.now {
                self.now = at;
            }
            effects.extend(runner.timer(self.now, &mut self.sampler()).await);
            if runner.has_exited() {
                break;
            }
        }
        self.effects.extend(effects.iter().cloned());
        effects
    }

    /// Exactly one stimulus, no follow-up timers: the host has not slept yet,
    /// so anything waiting for its wake is still waiting.
    pub async fn turn(
        &mut self,
        runner: &mut Runner<'_, SharedClaim>,
        event: MpvEvent,
    ) -> Vec<Effect> {
        // The adapter reports the player it observed with the event; every
        // other event is observed through the properties mpv already told us.
        let observed = match &event {
            MpvEvent::FileLoaded { live, .. } => {
                self.observed = live.clone();
                self.time_pos = live.time_pos;
                live.clone()
            }
            _ => self.observed.clone(),
        };
        runner
            .mpv(self.now, event, observed, &mut self.sampler())
            .await
    }

    pub async fn turn_and_record(
        &mut self,
        runner: &mut Runner<'_, SharedClaim>,
        event: MpvEvent,
    ) {
        let effects = self.turn(runner, event).await;
        self.effects.extend(effects);
    }

    pub async fn trigger(&mut self, runner: &mut Runner<'_, SharedClaim>, event: MpvEvent) {
        self.turn_and_record(runner, event).await;
    }

    /// The host's sleep expired: the time moved and nothing else happened.
    pub async fn timer(&mut self, runner: &mut Runner<'_, SharedClaim>) {
        let effects = runner.timer(self.now, &mut self.sampler()).await;
        self.effects.extend(effects);
    }

    /// The player starts playing `file` at `at` seconds: the observed
    /// properties change, the position follows. A duration of `None` is an
    /// unknown length (no progress bar).
    pub fn play(&mut self, file: &str, at: f64, duration: Option<f64>) {
        self.observed.path = Some(file.to_owned());
        self.observed.duration = duration;
        self.time_pos = Some(at);
    }

    /// Loads `file`. The load reports the snapshot it read, position included.
    pub async fn load(&mut self, runner: &mut Runner<'_, SharedClaim>, file: &str) {
        self.play(file, 0.0, Some(1440.0));
        let event = MpvEvent::FileLoaded {
            path: file.to_owned(),
            live: self.observed.clone(),
        };
        self.turn_and_record(runner, event).await;
    }

    /// Loads `file` with the observed player the test set up already
    /// (duration, speed, pause, Toggle), starting `at` seconds in.
    pub async fn load_as_observed(
        &mut self,
        runner: &mut Runner<'_, SharedClaim>,
        file: &str,
        at: f64,
    ) {
        self.observed.path = Some(file.to_owned());
        self.time_pos = Some(at);
        let event = MpvEvent::FileLoaded {
            path: file.to_owned(),
            live: self.observed.clone(),
        };
        self.turn_and_record(runner, event).await;
    }

    /// The whole claim sequence: focus (debounce, the claim write, the
    /// settle) then the publish from the live snapshot. Each step is one turn
    /// so the clock is the test's, not the drain loop's.
    pub async fn focus_in(&mut self, runner: &mut Runner<'_, SharedClaim>) {
        self.turn_and_record(runner, MpvEvent::FocusChanged(true))
            .await;
        self.advance(CLAIM_DEBOUNCE_MS);
        self.timer(runner).await;
        self.advance(CLAIM_SETTLE_MS);
        self.timer(runner).await;
    }

    /// Focus in, then load and play the file as the Owner.
    pub async fn own(&mut self, runner: &mut Runner<'_, SharedClaim>, file: &str) {
        self.focus_in(runner).await;
        self.load(runner, file).await;
    }

    pub fn publishes(&self) -> Vec<Presence> {
        self.effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Publish(presence) => Some(presence.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn last_publish(&self) -> Presence {
        self.publishes().pop().expect("a publish")
    }

    pub fn clears(&self) -> usize {
        self.effects
            .iter()
            .filter(|effect| matches!(effect, Effect::Clear))
            .count()
    }

    pub fn last_effect(&self) -> Option<&Effect> {
        self.effects.last()
    }

    /// `P` for a published card, `-` for a clear: the exact sequence of
    /// Presence writes the host applied.
    pub fn card_sequence(&self) -> String {
        self.effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Publish(_) => Some('P'),
                Effect::Clear => Some('-'),
                _ => None,
            })
            .collect()
    }

    /// Console lines at one level; the default error view is `logs()`.
    pub fn logs_at(&self, level: Level) -> Vec<String> {
        self.effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Log(log) if log.level == level => Some(log.text.clone()),
                _ => None,
            })
            .collect()
    }

    /// Failure lines only: routine info lines must not disturb these vectors.
    pub fn logs(&self) -> Vec<String> {
        self.logs_at(Level::Error)
    }

    pub fn infos(&self) -> Vec<String> {
        self.logs_at(Level::Info)
    }

    /// No Presence, OSD or sample traffic; routine console Diagnostics don't
    /// count as activity for the silence assertions.
    pub fn no_activity(&self) -> bool {
        !self.effects.iter().any(|effect| {
            matches!(
                effect,
                Effect::Publish(_) | Effect::Clear | Effect::Osd { .. } | Effect::Sample
            )
        })
    }

    pub fn osds(&self) -> Vec<(String, u64)> {
        self.effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Osd { text, seconds } => Some((text.clone(), *seconds)),
                _ => None,
            })
            .collect()
    }
}
