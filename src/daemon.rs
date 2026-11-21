//! Daemon Core State Machine: the pure domain logic of Surtr.
//!
//! Everything testable lives here, behind injected ports (`MetadataClient`,
//! `OwnerClaim`). The adapters in `mpv` and `discord` translate raw IPC into
//! the events this module consumes, so the core itself never touches sockets
//! or pipes.
//!
//! # Kernel: one step
//!
//! [`Step`] is the Daemon Core's single entry point: one [`Frame`] in (the
//! time the host read, plus one [`Input`] — the `Init` roots, an mpv event, a
//! timer, a reconnect, or the answer to a sample demand), one [`Outcome`] out
//! (ordered [`Effect`] values, the next wake deadline, the exit flag). The
//! kernel reads no clock and calls no sink; [`Policy`] holds the timing.
//!
//! The production event loop drives that step through [`Runner`], the shell
//! that owns the choreography every host call site would otherwise repeat:
//! the drain-until-quiet loop, the sample handshake, the claim carried by
//! value through each step, and the wake-deadline bookkeeping. The kernel's
//! raw surface ([`Step`], [`Frame`], [`Input`], [`Outcome`]) is crate-private:
//! only [`Runner`] and the kernel contract tests inside this module reach it.
//!
//! # The Presence lifecycle
//!
//! Presence is driven by one *desired* state, never a queue. A trigger only
//! marks the desired state; the kernel hands out one dispatch per token of
//! Discord's rate-limit bucket, and the Runner samples the live player at that
//! moment, so a coalesced seek never carries stale timestamps.
//!
//! Multi-window arbitration sits on top of that: only the Owner — the Daemon
//! whose window was last focused, recorded in `owner.json` — ever marks or
//! dispatches a publish. Every other Daemon observes mpv and resolves Matches
//! but stays dormant with no Discord connection.

use std::future::Future;

use crate::clock::Millis;

/// How long a terminated file waits for a successor before Presence is
/// blanked: back-to-back playlist episodes transition without a flicker.
/// The shipped [`Policy::grace_ms`].
pub const GRACE_MS: Millis = 500;

/// Discord's documented 5 updates per 20 seconds: five tokens, one refilled
/// every four seconds. The shipped [`Policy::bucket_capacity`] and
/// [`Policy::refill_ms`].
pub const BUCKET_CAPACITY: u32 = 5;
pub const REFILL_MS: Millis = 4_000;

/// After a window reports `focused = true`, wait this long for focus to
/// settle before claiming `owner.json`: rapid alt-tabbing claims once.
/// The shipped [`Policy::claim_debounce_ms`].
pub const CLAIM_DEBOUNCE_MS: Millis = 300;

/// After claiming, wait this long before publishing from the live snapshot, so
/// the previous Owner has had time to clear and disconnect.
/// The shipped [`Policy::claim_settle_ms`].
pub const CLAIM_SETTLE_MS: Millis = 400;

/// The Owner re-reads `owner.json` at this cadence while it has a card up, so
/// a handover clears the old card within one poll.
/// The shipped [`Policy::claim_poll_ms`].
pub const CLAIM_POLL_MS: Millis = 250;

/// How long a pause may last before Presence is cleared. Zero means no timeout.
/// The shipped [`Policy::pause_timeout_ms`].
pub const PAUSE_TIMEOUT_MS: Millis = 0;

/// Resolved display metadata for a played file: the body of a
/// [`Lookup::Matched`]. No Match means no Presence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    /// The AniList id of the Match.
    pub id: i64,
    /// The card's main title (`details`): the configured
    /// `details_title_lang` title, the entry's other language as fallback.
    pub title: String,
    /// The Episode segment of the state line (`Episode 3/12`, `Episode 1`,
    /// `Episode 5/?`); `None` when the file has no episode data to show.
    pub episode: Option<String>,
    /// The cover-art hover text (`assets.large_text`): the metadata line
    /// `Format • Status • Season • Genres`, the title as fallback when the
    /// entry carries none of the four.
    pub hover: String,
    /// The public AniList cover art URL.
    pub cover: Option<String>,
}

/// The one answer to one lookup of a Release Filename (the glossary's Lookup
/// Answer). One lookup can meet several fates — one query throttled, the
/// next a lost score-off — and answers with the strongest by precedence: a
/// [`Lookup::Matched`] wins outright (a card now outranks a stale throttle
/// signal), then [`Lookup::RateLimited`], then [`Lookup::Missed`], then
/// [`Lookup::Unavailable`] (the retry must not be swallowed by a report,
/// and the report not by silence). The client constructs the answer; the
/// core only interprets it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// A Match: Presence follows.
    Matched(Metadata),
    /// A definitive Miss: no Presence, and the report names why.
    Missed(Miss),
    /// AniList throttled a request: the lookup is deferred and retried once
    /// after this many milliseconds. Not a Miss — the file has no answer
    /// yet, so nothing is reported.
    RateLimited(Millis),
    /// A failed request (transport or an unusable response): no Presence
    /// and no report. The lookup has no answer and no reason to name.
    Unavailable,
}

impl Lookup {
    /// The Miss a definitive answer carries; a rate limit and a bare
    /// failure report nothing.
    fn into_miss(self) -> Option<Miss> {
        match self {
            Lookup::Missed(miss) => Some(miss),
            _ => None,
        }
    }
}

/// Looks up a played file. The production client queries AniList/cache;
/// tests inject recorded answers.
pub trait MetadataClient {
    fn lookup(&mut self, path: &str) -> Lookup;
}

/// Why a lookup found no Match. The rule of thumb: a repairable
/// miss is one an `overrides.json` line could plausibly fix, so it earns an
/// OSD; a fatal miss only gets a console line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Miss {
    /// The override key a user could add to `overrides.json`.
    Repairable { key: String },
    /// A short reason no single override line can fix this.
    Fatal { reason: String },
}

/// What gets published to Discord for one activity card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presence {
    /// The AniList id, for the `details_url`/`large_url` links.
    pub id: i64,
    pub details: String,
    /// The cover-art hover text (`assets.large_text`).
    pub hover: String,
    /// The state line: `▶️ Playing`/`⏸️ Paused`, then the Episode segment.
    /// `None` when nothing renders (an unnumbered movie while playing).
    pub state: Option<String>,
    /// The AniList cover art URL (`assets.large_image`).
    pub cover: Option<String>,
    /// Timestamps for Discord's progress bar, epoch ms. Both are absent when
    /// paused or when the duration is unknown.
    pub start_ms: Option<Millis>,
    pub end_ms: Option<Millis>,
}

/// The single-owner claim in `%LOCALAPPDATA%\surtr\owner.json`: the process id
/// of the Daemon whose mpv window was last focused. Only that process holds a
/// Discord connection and publishes. Last write wins; no liveness check.
pub trait OwnerClaim {
    /// Writes `pid` as the Owner.
    fn claim(&mut self, pid: u32);
    /// The pid named by the file: another process id when a different Daemon
    /// holds or has taken the claim, `None` when the file is absent.
    fn owner(&self) -> Option<u32>;
}

// ---------------------------------------------------------------------------
// Kernel: Frame in, Outcome out
// ---------------------------------------------------------------------------
//
// One synchronous, run-to-completion step. The host reads the time once per
// frame, says what happened, and applies the ordered effects the step returns.
// The kernel never reads a clock and never touches a sink: time and the live
// player sample are inputs, and publishes, clears and diagnostics are values.
//
// Effect contract (the data channel carries what method calls would otherwise
// have to enforce by construction):
//
// - a clear precedes any publish in the same outcome;
// - a clear is emitted only when a card is actually up (nothing to blank needs
//   no `activity: null` and costs no token);
// - a publish and the matching sample demand never share an outcome: the
//   demand says what to read, and the next step's outcome is what it feeds;
// - effects are interpreted by the host in order, never inspected by the
//   kernel to decide what to do next.

/// For how long the Miss OSD stays on screen.
/// The shipped [`Policy::miss_osd_seconds`].
pub const MISS_OSD_SECONDS: u64 = 5;

/// The timing policy of one Daemon. `Default` reproduces the shipped
/// constants, so tuning a window is one value, not a hunt for literals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// [`GRACE_MS`]: how long a terminated file waits for a successor.
    pub grace_ms: Millis,
    /// [`BUCKET_CAPACITY`] and [`REFILL_MS`]: Discord's rate-limit bucket.
    pub bucket_capacity: u32,
    pub refill_ms: Millis,
    /// [`CLAIM_DEBOUNCE_MS`]: focus must settle this long before a claim.
    pub claim_debounce_ms: Millis,
    /// [`CLAIM_SETTLE_MS`]: wait this long after claiming before publishing.
    pub claim_settle_ms: Millis,
    /// [`CLAIM_POLL_MS`]: how often an Owner re-reads `owner.json`.
    pub claim_poll_ms: Millis,
    /// [`MISS_OSD_SECONDS`]: how long the Miss OSD stays on screen.
    pub miss_osd_seconds: u64,
    /// [`PAUSE_TIMEOUT_MS`]: milliseconds of continuous pause before Presence
    /// is cleared. Zero disables the timeout.
    pub pause_timeout_ms: Millis,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            grace_ms: GRACE_MS,
            bucket_capacity: BUCKET_CAPACITY,
            refill_ms: REFILL_MS,
            claim_debounce_ms: CLAIM_DEBOUNCE_MS,
            claim_settle_ms: CLAIM_SETTLE_MS,
            claim_poll_ms: CLAIM_POLL_MS,
            miss_osd_seconds: MISS_OSD_SECONDS,
            pause_timeout_ms: PAUSE_TIMEOUT_MS,
        }
    }
}

/// Everything one step consumes: the time the host read, the observed player
/// state as of that moment, and the one stimulus.
///
/// `observed` is the host's live snapshot (every property mpv reports except
/// `time-pos`, which is only read on demand). A sample demand is answered
/// against it, so a frame can never rebuild a card from a stale load.
///
/// Crate-private: production speaks to the kernel through [`Runner`] only.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Frame {
    pub(crate) now: Millis,
    pub(crate) observed: LiveState,
    pub(crate) input: Input,
}

impl Frame {
    /// A frame at `now` carrying no observed player state.
    pub(crate) fn new(now: Millis, input: Input) -> Self {
        Self {
            now,
            observed: LiveState::default(),
            input,
        }
    }

    /// Attaches the observed player state.
    pub(crate) fn with_observed(mut self, observed: LiveState) -> Self {
        self.observed = observed;
        self
    }
}

/// One stimulus. Everything the kernel reacts to is one of these.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Input {
    /// Installs the Anime Roots; the first frame of every Daemon, exactly once.
    Init {
        roots: Vec<String>,
    },
    Mpv(MpvEvent),
    /// The host's sleep expired.
    Timer,
    /// Discord reconnected.
    Reconnect,
    /// The answer to a [`Effect::Sample`] demand: the observed properties held
    /// by the host, with `time_pos` read fresh at that instant.
    Sampled(LiveState),
}

/// The severity of one console Diagnostic, and so the mpv script message the
/// launcher receives (`surtr-log` ... `surtr-log-verbose`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
    Verbose,
}

impl Level {
    /// The IPC script message for this level; the launcher maps it to `mp.msg`.
    pub fn script_message(self) -> &'static str {
        match self {
            Self::Info => "surtr-log",
            Self::Warn => "surtr-log-warn",
            Self::Error => "surtr-log-error",
            Self::Verbose => "surtr-log-verbose",
        }
    }
}

/// One console Diagnostic: the level selects the launcher's `mp.msg` function,
/// the text is what mpv prints after its `[surtr]` script prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub level: Level,
    pub text: String,
}

impl Diagnostic {
    pub fn new(level: Level, text: impl Into<String>) -> Self {
        Self {
            level,
            text: text.into(),
        }
    }

    pub fn info(text: impl Into<String>) -> Self {
        Self::new(Level::Info, text)
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self::new(Level::Error, text)
    }
}

/// One ordered command for the host to carry out.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Publish(Presence),
    Clear,
    /// One console Diagnostic, sent as a level-tagged script message.
    Log(Diagnostic),
    /// A transient mpv OSD toast, visible for `seconds`.
    Osd {
        text: String,
        seconds: u64,
    },
    /// Sample the live player now (read `time-pos` fresh) and re-enter the
    /// step with [`Input::Sampled`]. Never chains: nothing else may ask for a
    /// sample before the answer arrives.
    Sample,
}

/// What one step decided: the ordered effects, the next wake deadline (the
/// sleep question lives here, so it cannot drift from the dispatch), and the
/// exit flag.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Outcome {
    pub(crate) effects: Vec<Effect>,
    pub(crate) wake_at: Option<Millis>,
    pub(crate) exit: bool,
}

/// The player state sampled on demand, immediately before a dispatch.
///
/// `time-pos` is the only property that changes every frame and is therefore
/// never observed: the adapter reads it fresh here. The rest are observed
/// properties, mirrored here so a dispatch sees one coherent snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveState {
    pub path: Option<String>,
    pub time_pos: Option<f64>,
    pub duration: Option<f64>,
    pub paused: bool,
    pub speed: f64,
    pub eof_reached: bool,
    /// The session Toggle, mirrored from mpv's `user-data/surtr/enabled`.
    pub enabled: bool,
}

impl Default for LiveState {
    fn default() -> Self {
        Self {
            path: None,
            time_pos: None,
            duration: None,
            paused: false,
            speed: 1.0,
            eof_reached: false,
            enabled: true,
        }
    }
}

/// An observed property whose change re-cadences Presence.
#[derive(Debug, Clone, PartialEq)]
pub enum Property {
    Pause(bool),
    Speed(f64),
    Duration(Option<f64>),
    EofReached(bool),
    /// The session Toggle (`user-data/surtr/enabled`).
    Enabled(bool),
}

/// The inputs of the state machine: mpv events and observed property changes.
#[derive(Debug, Clone, PartialEq)]
pub enum MpvEvent {
    /// A new file finished loading; `path` is the file just loaded, and
    /// `live` the player snapshot the adapter read alongside it.
    FileLoaded {
        path: String,
        live: LiveState,
    },
    /// Start of playback or the settled end of a seek: recompute from live.
    PlaybackRestart,
    /// The current file ended. A short grace window precedes the clear so a
    /// playlist advance does not blink.
    EndFile,
    /// An observed property changed.
    Property(Property),
    /// mpv's `focused` changed. `true` starts claiming Discord ownership;
    /// `false` writes no claim, so an alt-tab to a browser keeps the Owner
    /// publishing. Ownership is never given up on losing focus.
    FocusChanged(bool),
    Shutdown,
}

/// One loaded file and the Match resolved for it.
struct CurrentMatch {
    path: String,
    metadata: Metadata,
}

/// The single deferred 429 retry for one file.
struct Retry {
    path: String,
    at: Millis,
}

/// Five tokens, one refilled every four seconds — at the configured capacity
/// and cadence.
struct Bucket {
    tokens: u32,
    last_refill: Millis,
    policy: Policy,
}

impl Bucket {
    fn new(now: Millis, policy: Policy) -> Self {
        Self {
            tokens: policy.bucket_capacity,
            last_refill: now,
            policy,
        }
    }

    fn refill(&mut self, now: Millis) {
        // Never count time backwards: a stale frame only costs a refill.
        if now <= self.last_refill || self.policy.refill_ms <= 0 {
            return;
        }
        let gained = ((now - self.last_refill) / self.policy.refill_ms) as u32;
        if gained > 0 {
            self.tokens = (self.tokens + gained).min(self.policy.bucket_capacity);
            self.last_refill += gained as Millis * self.policy.refill_ms;
        }
    }

    fn take(&mut self, now: Millis) -> bool {
        self.refill(now);
        if self.tokens > 0 {
            self.tokens -= 1;
            true
        } else {
            false
        }
    }

    fn next_token_at(&mut self, now: Millis) -> Millis {
        self.refill(now);
        if self.tokens > 0 {
            now
        } else {
            self.last_refill + self.policy.refill_ms
        }
    }
}

/// The kernel: one step in, one outcome out. No clock, no sinks — it borrows
/// only the Match lookup, and the claim arrives by value and leaves in the
/// outcome. The host owns everything else.
///
/// The state is the same Presence lifecycle the [`Core`] methods drive: one
/// latest-wins publish intent, a clear that always goes first, a single
/// deferred 429 retry, and the ownership protocol.
///
/// Crate-private: the [`Runner`] is the kernel's one seam.
pub(crate) struct Step<'a> {
    metadata: &'a mut dyn MetadataClient,
    policy: Policy,
    pid: u32,
    /// Set by the `Init` frame; empty means every local file is eligible.
    roots: Vec<String>,
    bucket: Bucket,
    current: Option<CurrentMatch>,
    desired_publish: bool,
    desired_clear: Option<Millis>,
    empty: bool,
    grace_until: Option<Millis>,
    retry: Option<Retry>,
    enabled: bool,
    eof_reached: bool,
    owning: bool,
    claim_pending: Option<Millis>,
    post_claim_at: Option<Millis>,
    reported_miss: Option<String>,
    /// An admitted publish whose sample answer has not landed yet.
    pending_sample: bool,
    /// When the current continuous pause expires, if the timeout is armed.
    pause_deadline: Option<Millis>,
    /// Set when a pause outlives the timeout; clears only on resume or load.
    pause_expired: bool,
}

/// Opens a [`Step`] with tuned timing (tests use literal values).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct StepBuilder {
    policy: Policy,
}

impl StepBuilder {
    pub(crate) fn policy(mut self, policy: Policy) -> Self {
        self.policy = policy;
        self
    }

    pub(crate) fn build<'a>(
        self,
        metadata: &'a mut dyn MetadataClient,
        pid: u32,
        now: Millis,
    ) -> Step<'a> {
        Step {
            metadata,
            policy: self.policy,
            pid,
            roots: Vec::new(),
            bucket: Bucket::new(now, self.policy),
            current: None,
            desired_publish: false,
            desired_clear: None,
            empty: true,
            grace_until: None,
            retry: None,
            enabled: true,
            eof_reached: false,
            owning: false,
            claim_pending: None,
            post_claim_at: None,
            reported_miss: None,
            pending_sample: false,
            pause_deadline: None,
            pause_expired: false,
        }
    }
}

impl<'a> Step<'a> {
    /// The single entry point: handles one Frame and returns what to do.
    pub(crate) fn step<C: OwnerClaim>(&mut self, frame: Frame, claim: &mut C) -> Outcome {
        let now = frame.now;
        let mut out = Outcome {
            effects: Vec::new(),
            wake_at: None,
            exit: false,
        };

        // An unanswered demand is answered before anything else, and its
        // answer is the whole outcome: a stale card must never be published
        // from an old playback position, and the demand never chains. A frame
        // that is not an answer lapses the demand against the observed player
        // and is still handled as the stimulus it is.
        if std::mem::take(&mut self.pending_sample) {
            match frame.input {
                Input::Sampled(live) => {
                    self.complete(now, &live, claim, &mut out);
                    self.bucket.refill(now);
                }
                other => {
                    let observed = frame.observed;
                    self.complete(now, &observed, claim, &mut out);
                    self.respond(now, other, claim, &mut out);
                    self.admit(now, claim, &mut out);
                }
            }
        } else {
            self.respond(now, frame.input, claim, &mut out);
            self.admit(now, claim, &mut out);
        }

        out.wake_at = if out.exit { None } else { self.next_wake(now) };
        out
    }

    /// Applies one stimulus that is not a sample answer.
    fn respond<C: OwnerClaim>(
        &mut self,
        now: Millis,
        input: Input,
        claim: &mut C,
        out: &mut Outcome,
    ) {
        match input {
            // Installs the configuration and nothing else: no claim is
            // adopted and no publish is marked before the first stimulus.
            Input::Init { roots } => {
                self.roots = roots;
                return;
            }
            Input::Mpv(event) => self.on_event(now, event, claim, out),
            Input::Timer => self.on_timer(now, claim, out),
            Input::Reconnect => {
                self.check_claim(claim, out);
                self.trigger_publish();
            }
            // Nothing was asked: an answer to no demand changes nothing.
            Input::Sampled(_) => {}
        }
    }

    /// Whether a [`Effect::Sample`] demand is waiting for its answer. The host
    /// should send [`Input::Sampled`] next: any other frame lapses the demand
    /// against the observed player, which is a clear when nothing was asked.
    pub(crate) fn awaiting_sample(&self) -> bool {
        self.pending_sample
    }

    /// The earliest moment this Daemon has work for the host. Pure: adopts
    /// nothing and delivers nothing.
    fn next_wake(&mut self, now: Millis) -> Option<Millis> {
        let clear_at = self
            .desired_clear
            .map(|not_before| not_before.max(self.bucket.next_token_at(now)));
        let pause_at = self.pause_deadline;
        // A dormant Daemon holds no intent: it wakes only for ownership.
        let publish_at = self.desired_publish.then(|| self.bucket.next_token_at(now));
        let retry_at = self.retry.as_ref().map(|retry| retry.at);
        // An Owner re-checks `owner.json` on a short cadence, so a handover
        // clears the old card within one poll.
        let poll_at = self
            .owning
            .then(|| now.saturating_add(self.policy.claim_poll_ms));
        let settle_at = [self.claim_pending, self.post_claim_at]
            .into_iter()
            .flatten()
            .min();
        let earliest = |a: Option<Millis>, b: Option<Millis>| match (a, b) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(at), None) | (None, Some(at)) => Some(at),
            (None, None) => None,
        };

        // A pending clear blocks a publish: while one is queued only the
        // clear, a deferred retry, the claim steps and the poll can wake us.
        match clear_at {
            Some(clear_at) => earliest(
                Some(clear_at),
                earliest(pause_at, earliest(retry_at, earliest(poll_at, settle_at))),
            ),
            None => earliest(
                earliest(earliest(publish_at, pause_at), retry_at),
                earliest(poll_at, settle_at),
            ),
        }
    }

    fn on_event<C: OwnerClaim>(
        &mut self,
        now: Millis,
        event: MpvEvent,
        claim: &mut C,
        out: &mut Outcome,
    ) {
        match event {
            MpvEvent::FileLoaded { path, live } => self.on_file_loaded(now, path, &live, out),
            MpvEvent::PlaybackRestart => self.trigger_publish(),
            MpvEvent::EndFile => self.on_end_file(now),
            MpvEvent::Property(Property::Pause(paused)) => self.on_pause(now, paused),
            MpvEvent::Property(Property::Speed(_)) | MpvEvent::Property(Property::Duration(_)) => {
                self.trigger_publish()
            }
            MpvEvent::Property(Property::EofReached(reached)) => {
                self.eof_reached = reached;
                if reached {
                    self.retry = None;
                    self.request_clear(now);
                } else {
                    self.trigger_publish();
                }
            }
            MpvEvent::Property(Property::Enabled(enabled)) => {
                self.enabled = enabled;
                if enabled {
                    self.trigger_publish();
                } else {
                    // Toggle-off hides the session immediately, whatever the
                    // lookups in flight and whether or not we own Discord.
                    self.retry = None;
                    self.request_clear(now);
                }
            }
            MpvEvent::FocusChanged(true) => {
                if !self.owning && self.claim_pending.is_none() {
                    self.claim_pending = Some(now.saturating_add(self.policy.claim_debounce_ms));
                }
            }
            // Losing focus writes nothing: the last Owner keeps publishing
            // while the user is in another application.
            MpvEvent::FocusChanged(false) => {}
            MpvEvent::Shutdown => {
                self.desired_publish = false;
                self.desired_clear = None;
                self.retry = None;
                self.clear(out);
                out.exit = true;
            }
        }
        // Every non-shutdown event re-checks the claim: an event racing a
        // handover must not keep this window's card alive.
        if !out.exit {
            self.check_claim(claim, out);
        }
    }

    fn on_timer<C: OwnerClaim>(&mut self, now: Millis, claim: &mut C, out: &mut Outcome) {
        if self.pause_deadline.is_some_and(|at| now >= at) {
            self.pause_deadline = None;
            self.pause_expired = true;
            // The normal clear queue: one token, only if a card is actually up.
            self.request_clear(now);
        }

        if self.retry.as_ref().is_some_and(|retry| now >= retry.at) {
            self.fire_retry(now, out);
        }

        if self.claim_pending.is_some_and(|at| now >= at) {
            self.claim_pending = None;
            // Writing the claim is the whole acquisition: this Daemon now owns
            // Discord, but holds off publishing until the previous Owner has
            // had time to clear and disconnect.
            claim.claim(self.pid);
            self.owning = true;
            self.post_claim_at = Some(now.saturating_add(self.policy.claim_settle_ms));
        }

        if self.post_claim_at.is_some_and(|at| now >= at) {
            self.post_claim_at = None;
            // Still the claim holder? If a newer window took over meanwhile,
            // fall dormant without a card.
            self.check_claim(claim, out);
            self.trigger_publish();
        }

        if self.claim_pending.is_none() && self.post_claim_at.is_none() {
            self.check_claim(claim, out);
        }
    }

    fn fire_retry(&mut self, now: Millis, out: &mut Outcome) {
        let Some(retry) = self.retry.take() else {
            return;
        };
        if self.enabled && !self.eof_reached && self.current.is_none() {
            let enabled = self.enabled;
            self.lookup(retry.path, true, now, enabled, out);
        }
    }

    /// A pause arms the timeout; resuming cancels it and lets Presence back.
    fn on_pause(&mut self, now: Millis, paused: bool) {
        if paused {
            if !self.pause_expired {
                self.pause_deadline = (self.policy.pause_timeout_ms > 0)
                    .then(|| now.saturating_add(self.policy.pause_timeout_ms));
                self.trigger_publish();
            }
        } else {
            self.pause_deadline = None;
            self.pause_expired = false;
            self.trigger_publish();
        }
    }

    /// A file finished loading; `live` is the snapshot the adapter read with
    /// it (including the session Toggle at that moment).
    fn on_file_loaded(&mut self, now: Millis, path: String, live: &LiveState, out: &mut Outcome) {
        // A playlist advance inside the grace window replaces the old card;
        // any other file switch clears it before the lookup completes.
        let seamless = self.grace_until.is_some_and(|until| now < until);
        self.grace_until = None;
        self.eof_reached = false;

        if self.retry.as_ref().is_some_and(|retry| retry.path != path) {
            self.retry = None;
        }

        // The adapter can announce the same file twice (startup read plus
        // `file-loaded`); never look the same path up twice in a row.
        if self
            .current
            .as_ref()
            .is_some_and(|current| current.path == path)
        {
            self.trigger_publish();
            return;
        }

        // A real file change starts a fresh pause window, even if the new
        // file's first frame is already paused.
        self.pause_expired = false;
        self.pause_deadline = (self.policy.pause_timeout_ms > 0 && live.paused)
            .then(|| now.saturating_add(self.policy.pause_timeout_ms));

        if seamless {
            // The grace clear is for a file that did not arrive; a successor
            // replaces the old card instead of blanking. A Toggle-off clear
            // is not the grace clear and must still run.
            if self.enabled {
                self.desired_clear = None;
            }
        } else {
            self.request_clear(now);
        }
        self.desired_publish = false;
        self.current = None;

        // The one routine console line: mpv's `[surtr]` prefix plus the full
        // path tells the user the Daemon saw this file at all.
        out.effects.push(Effect::Log(Diagnostic::info(path.clone())));

        // Local files only (spec: Out of Scope — URL/stream playback), and
        // only inside an Anime Root: everything else yields no Presence and
        // triggers no lookup, with a console line for the user.
        if path.contains("://") || !self.eligible(&path) {
            out.effects.push(Effect::Log(Diagnostic::error(format!(
                "surtr: skipping non-local path {path}"
            ))));
            self.request_clear(now);
            return;
        }

        // The file's own snapshot decides the Toggle: mpv may deliver the
        // keypress change after the load, and a file loaded while hidden must
        // resolve its Match without ever showing it.
        self.lookup(path, false, now, live.enabled, out);
    }

    fn lookup(
        &mut self,
        path: String,
        is_retry: bool,
        now: Millis,
        enabled: bool,
        out: &mut Outcome,
    ) {
        match self.metadata.lookup(&path) {
            Lookup::Matched(metadata) => {
                self.retry = None;
                self.current = Some(CurrentMatch { path, metadata });
                self.enabled = enabled;
                self.trigger_publish();
            }
            // A 429 is not a definitive miss: one deferred retry honours
            // Retry-After, and the report waits for the retry that actually
            // answers.
            Lookup::RateLimited(delay) if !is_retry => {
                self.current = None;
                self.request_clear(now);
                self.retry = Some(Retry {
                    path,
                    at: now.saturating_add(delay.max(0)),
                });
            }
            answer => {
                self.current = None;
                self.request_clear(now);
                // A 429 on an already-spent retry is still no answer: no
                // Presence, and nothing to report. A second failure is no
                // Presence either.
                self.report_miss(&path, enabled, answer.into_miss(), out);
            }
        }
    }

    /// The one failure report per loaded file: a repairable miss
    /// is a console line then a 5 s OSD naming the override key, a fatal miss
    /// is a console line only, and a file loaded behind the Toggle-off stays
    /// silent. An answer with no Miss (a failed request, or a throttle whose
    /// retry was already spent) burns the report but prints nothing. Seek,
    /// pause and a Toggle-on never re-report it.
    fn report_miss(&mut self, path: &str, enabled: bool, miss: Option<Miss>, out: &mut Outcome) {
        if self.reported_miss.as_deref() == Some(path) {
            return;
        }
        // The report is spent even when suppressed: a re-announce of the same
        // file (startup read plus `file-loaded`) must not nag later.
        self.reported_miss = Some(path.to_owned());
        let Some(miss) = miss else {
            return;
        };
        if !enabled {
            return;
        }
        match miss {
            Miss::Repairable { key } => {
                let text = format!("Surtr: no Anime match for \"{key}\"");
                out.effects
                    .push(Effect::Log(Diagnostic::error(text.clone())));
                out.effects.push(Effect::Osd {
                    text,
                    seconds: self.policy.miss_osd_seconds,
                });
            }
            Miss::Fatal { reason } => {
                out.effects.push(Effect::Log(Diagnostic::error(format!(
                    "surtr: no Anime match ({reason}): {path}"
                ))));
            }
        }
    }

    fn on_end_file(&mut self, now: Millis) {
        let until = now.saturating_add(self.policy.grace_ms);
        self.grace_until = Some(until);
        self.current = None;
        self.retry = None;
        self.reported_miss = None;
        self.desired_publish = false;
        self.pause_deadline = None;
        self.request_clear(until);
    }

    /// Admits at most one piece of work per frame: a clear first (it takes its
    /// own token), then one publish per token. A Daemon that is not the Owner
    /// never publishes. An admitted publish becomes a sample demand.
    fn admit<C: OwnerClaim>(&mut self, now: Millis, claim: &mut C, out: &mut Outcome) {
        self.adopt(claim);
        self.flush_clear(now, out);
        if self.desired_clear.is_some() || out.exit {
            return;
        }
        // Never publish into another window's claim: a case this Daemon did
        // not write means someone else owns Discord now.
        if claim.owner().is_some_and(|owner| owner != self.pid) {
            self.check_claim(claim, out);
        }
        if self.desired_publish && self.owning && self.bucket.take(now) {
            self.desired_publish = false;
            self.post_claim_at = None;
            self.pending_sample = true;
            out.effects.push(Effect::Sample);
        }
    }

    /// Completes the publish the sample demand asked for. If the file changed,
    /// ended, the Toggle went off, or the claim was lost while waiting, the
    /// correct result is an empty Presence.
    fn complete<C: OwnerClaim>(
        &mut self,
        now: Millis,
        live: &LiveState,
        claim: &mut C,
        out: &mut Outcome,
    ) {
        // The claim is checked at the last moment so an event racing a
        // handover cannot republish the old window's card.
        self.check_claim(claim, out);
        let Some(current) = &self.current else {
            self.clear(out);
            return;
        };
        if self.pause_expired
            || !self.owning
            || !live.enabled
            || live.eof_reached
            || live.path.as_deref() != Some(current.path.as_str())
        {
            self.clear(out);
            return;
        }
        let metadata = current.metadata.clone();

        let presence = build_presence(&metadata, live, now);
        out.effects.push(Effect::Publish(presence));
        self.empty = false;
    }

    /// Re-reads the claim. The Owner keeps the card only while the file names
    /// its own process id; a different pid means another window has taken
    /// over, so the card is cleared.
    fn check_claim<C: OwnerClaim>(&mut self, claim: &mut C, out: &mut Outcome) {
        match claim.owner() {
            Some(owner) if owner == self.pid => self.owning = true,
            // A claim this Daemon wrote can disappear (the file was removed):
            // only another pid, or a vanished file, means "not the Owner".
            _ => {
                if self.owning {
                    self.owning = false;
                    self.desired_publish = false;
                    self.post_claim_at = None;
                    self.clear(out);
                }
            }
        }
    }

    /// Adopts a claim the Daemon did not write itself: restarted over a held
    /// claim, or handed ownership before it connected.
    fn adopt<C: OwnerClaim>(&mut self, claim: &mut C) {
        if !self.owning && claim.owner() == Some(self.pid) {
            self.owning = true;
            self.trigger_publish();
        }
    }

    /// Mark the current Match for a fresh publish. Clears are not cancelled:
    /// a pending clear always goes first. A non-Owner keeps observing and
    /// resolving, but never marks anything for dispatch.
    fn trigger_publish(&mut self) {
        if !self.pause_expired
            && self.owning
            && self.enabled
            && !self.eof_reached
            && self.current.is_some()
        {
            self.desired_publish = true;
        }
    }

    fn request_clear(&mut self, not_before: Millis) {
        self.desired_publish = false;
        self.desired_clear = Some(not_before);
    }

    /// Deliver a due clear. Clears are ordinary updates and take a token; an
    /// already-empty Presence needs neither a token nor another null.
    fn flush_clear(&mut self, now: Millis, out: &mut Outcome) {
        let Some(not_before) = self.desired_clear else {
            return;
        };
        if now < not_before {
            return;
        }
        if !self.empty && !self.bucket.take(now) {
            return;
        }
        self.desired_clear = None;
        self.clear(out);
    }

    /// Emits a clear only when a card is actually up (or a publish in this
    /// same outcome would put one up): a clear precedes any publish.
    fn clear(&mut self, out: &mut Outcome) {
        let publishing = out
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::Publish(_)));
        if !self.empty || publishing {
            out.effects.push(Effect::Clear);
            self.empty = true;
        }
    }

    /// Whether the file may produce a Presence: local, and inside an Anime
    /// Root when any are configured.
    fn eligible(&self, path: &str) -> bool {
        if path.contains("://") {
            return false;
        }
        self.roots.is_empty() || crate::release::within_roots(path, &self.roots)
    }
}

// ---------------------------------------------------------------------------
// The Runner: the shell the host drives
// ---------------------------------------------------------------------------
//
// Everything the production loop (and, later, the test suite) needs around the
// kernel: one entry point per thing that happened, the drain-until-quiet loop
// that follows every entry, the sample handshake, and the claim carried by
// value through each step. The Runner never reads a clock and never touches a
// sink: it takes the time, answers each sample demand with the closure the
// host supplies, and hands the ordered effects back.

/// How the host answers a [`Effect::Sample`] demand: read `time-pos` fresh
/// now and return the observed player around it. A dead mpv session answers
/// with the empty [`LiveState`], which the kernel turns into an empty
/// Presence.
///
/// The closure owns whatever it needs (channels, a pipe handle) and is
/// `async`: a closure returning `Box::pin(...)` satisfies it, so the Runner
/// names no channel of its own.
pub trait Greeting {
    fn greet(&mut self) -> impl Future<Output = LiveState>;
}

impl<G, F> Greeting for G
where
    G: FnMut() -> F,
    F: Future<Output = LiveState>,
{
    fn greet(&mut self) -> impl Future<Output = LiveState> {
        self()
    }
}

/// The shell around one Daemon's [`Step`]: one entry point per stimulus, each
/// draining until the kernel is quiet and returning the effects in order.
pub struct Runner<'a, C: OwnerClaim> {
    step: Step<'a>,
    claim: C,
    /// The latest wake deadline the kernel asked for; `None` is "sleep until
    /// something happens".
    wake_at: Option<Millis>,
    /// The kernel asked the host to stop (mpv is closing).
    exited: bool,
}

impl<'a, C: OwnerClaim> Runner<'a, C> {
    /// Opens the kernel at `now` with the policy, the claim it will carry
    /// through every step, and the Anime Roots.
    pub fn new(
        metadata: &'a mut dyn MetadataClient,
        claim: C,
        pid: u32,
        now: Millis,
        policy: Policy,
        roots: Vec<String>,
    ) -> Self {
        let mut runner = Self {
            step: StepBuilder::default()
                .policy(policy)
                .build(metadata, pid, now),
            claim,
            wake_at: None,
            exited: false,
        };
        // The `Init` frame is the kernel's first: it installs the roots and
        // nothing else (no demand, no effect, no wake), so it needs no
        // sampler and the host never sees it.
        let outcome = runner
            .step
            .step(Frame::new(now, Input::Init { roots }), &mut runner.claim);
        debug_assert!(outcome.effects.is_empty() && !outcome.exit);
        runner
    }

    /// One mpv event, plus the observed player the host holds at this moment.
    pub async fn mpv<G: Greeting>(
        &mut self,
        now: Millis,
        event: MpvEvent,
        observed: LiveState,
        sampling: &mut G,
    ) -> Vec<Effect> {
        self.handle(
            Frame::new(now, Input::Mpv(event)).with_observed(observed),
            sampling,
        )
        .await
    }

    /// The host's sleep expired: the one stimulus that carries no news of
    /// its own, only the time.
    pub async fn timer<G: Greeting>(&mut self, now: Millis, sampling: &mut G) -> Vec<Effect> {
        self.handle(Frame::new(now, Input::Timer), sampling).await
    }

    /// Discord reconnected: republish from live state, no lookup.
    pub async fn reconnect<G: Greeting>(
        &mut self,
        now: Millis,
        sampling: &mut G,
    ) -> Vec<Effect> {
        self.handle(Frame::new(now, Input::Reconnect), sampling)
            .await
    }

    /// The earliest moment the kernel asked to be woken, as of the last step.
    pub fn wake_at(&self) -> Option<Millis> {
        self.wake_at
    }

    /// Whether the kernel asked the host to stop: no further entry point has
    /// anything to do.
    pub fn has_exited(&self) -> bool {
        self.exited
    }

    /// Steps until the kernel is quiet, answering every sample demand as it
    /// comes. Effects are returned in the order they were produced, across
    /// every step the drain took.
    async fn handle<G: Greeting>(
        &mut self,
        mut frame: Frame,
        sampling: &mut G,
    ) -> Vec<Effect> {
        let mut effects = Vec::new();
        loop {
            let now = frame.now;
            let outcome = self.step.step(frame, &mut self.claim);
            self.wake_at = outcome.wake_at;
            let demanded = outcome.effects.contains(&Effect::Sample);
            // The sample demand is the Runner's to answer, not the host's to
            // apply: it never leaves this shell.
            effects.extend(
                outcome
                    .effects
                    .into_iter()
                    .filter(|effect| *effect != Effect::Sample),
            );
            if outcome.exit {
                self.exited = true;
                return effects;
            }
            // The demand is answered from the live player at this instant,
            // and the answer is what the next step publishes or clears from.
            // A demand the kernel lapsed in the same step (the frame was not
            // an answer) is not waiting any more and is never chained here.
            if demanded && self.step.awaiting_sample() {
                let live = sampling.greet().await;
                frame = Frame::new(now, Input::Sampled(live));
                continue;
            }
            // Everything else left is the host's to wait for, not to answer.
            return effects;
        }
    }
}

/// The Discord card for one dispatch.
///
/// Playing timestamps incorporate playback speed: at 2× the client-side bar
/// must run twice as fast. Pausing drops `timestamps` entirely — Discord keeps
/// advancing a bar whenever both ends are present, and there is no way to
/// freeze it — and leads the state with `⏸️ Paused` instead of `▶️ Playing`.
/// A state with no Episode segment (a movie, an unnumbered file) shows the
/// play/pause word alone while paused and nothing while playing.
fn build_presence(metadata: &Metadata, live: &LiveState, now: Millis) -> Presence {
    // U+25B6/U+23F8 plus the VS-16 presentation selector: colored emoji on
    // every platform Discord renders rich presence on.
    const PLAYING: &str = "\u{25B6}\u{FE0F} Playing";
    const PAUSED: &str = "\u{23F8}\u{FE0F} Paused";
    let action = if live.paused { PAUSED } else { PLAYING };
    let state = metadata
        .episode
        .as_ref()
        .map(|episode| format!("{action} • {episode}"))
        .or_else(|| live.paused.then(|| action.to_owned()));

    let (start_ms, end_ms) = if live.paused {
        (None, None)
    } else {
        match (live.time_pos, live.duration) {
            (Some(position), Some(duration)) => {
                let speed = if live.speed > 0.0 { live.speed } else { 1.0 };
                let start = now - (position / speed * 1000.0) as Millis;
                let end = now + ((duration - position).max(0.0) / speed * 1000.0) as Millis;
                (Some(start), Some(end))
            }
            // No duration means no bar: never guess an end.
            _ => (None, None),
        }
    };

    Presence {
        id: metadata.id,
        details: metadata.title.clone(),
        hover: metadata.hover.clone(),
        state,
        cover: metadata.cover.clone(),
        start_ms,
        end_ms,
    }
}

// ---------------------------------------------------------------------------
// Kernel contract tests
// ---------------------------------------------------------------------------
//
// The tests the [`Runner`] cannot express: one [`Frame`] in, one [`Outcome`]
// out, with the wake deadline and the exit flag read per frame. The driving
// verbs are what the user did (load, seek, focus, wait); the assertions are
// on the ordered effects the host would apply. Time is a frame field, so
// every test runs on a virtual timeline of literal milliseconds.

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    use super::*;

    const T0: i64 = 1_700_000_000_000;

    /// The process id every test Daemon runs as.
    const PID: u32 = 4242;

    /// In-memory `owner.json`: `None` is an absent file. Cloned so a test can
    /// take the claim from another window mid-flight.
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

    /// A scripted Match lookup: one Lookup Answer per lookup, `Unavailable`
    /// once the script runs out (no Presence, no report).
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

    /// The host side of the kernel: a virtual clock, the live player, the
    /// owner file, and every outcome the kernel produced, in order.
    struct Host {
        now: i64,
        /// The position, read fresh when the kernel asks.
        live: LiveState,
        /// The observed player the host holds between frames (no position).
        observed: LiveState,
        claim: SharedClaim,
        /// Every outcome, one entry per frame.
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

        /// The answer to a sample demand: the observed player, with the
        /// position read fresh at this instant.
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

        /// Runs frames until nothing is left to do now: every demand is
        /// answered from the live player and every due effect applied. The
        /// demand is a signal to the host, so the host answers before poking
        /// the kernel.
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

        /// A seek: playback restarts with the position at `time_pos`.
        fn seek(&mut self, step: &mut Step, time_pos: f64) {
            self.live.time_pos = Some(time_pos);
            self.send(step, Input::Mpv(MpvEvent::PlaybackRestart));
        }

        /// Every effect of every frame, in order.
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
        let mut step = StepBuilder::default().build(script, PID, T0);
        host.send(&mut step, Input::Init { roots: Vec::new() });
        step
    }

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

        assert_eq!(
            effects,
            vec![
                Effect::Log(Diagnostic::info(r"D:\Anime\Frieren - 01.mkv")),
                Effect::Sample
            ],
            "a load reports the file and admits a sample"
        );
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
            vec![
                Effect::Log(Diagnostic::info(r"D:\Anime\Frieren - 02.mkv")),
                Effect::Clear,
                Effect::Sample
            ],
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
        assert_eq!(policy.pause_timeout_ms, PAUSE_TIMEOUT_MS);
        assert_eq!((GRACE_MS, REFILL_MS), (500, 4_000));
        assert_eq!((BUCKET_CAPACITY, CLAIM_POLL_MS), (5, 250));
        assert_eq!((CLAIM_DEBOUNCE_MS, CLAIM_SETTLE_MS), (300, 400));
        assert_eq!(MISS_OSD_SECONDS, 5);
    }

    /// KERNEL TEST: a raw single step, for the race a user verb cannot express.
    ///
    /// The publish is admitted and its sample demanded in one step; the claim
    /// moves to the other window in that gap; the answer lands on a Daemon that
    /// no longer owns Discord. Only stepping by hand can interleave those three.
    #[test]
    fn a_publish_racing_a_handover_is_dropped() {
        let mut script = Script {
            answers: VecDeque::from([matched()]),
            ..Script::default()
        };
        let mut claim = SharedClaim::default();
        claim.claim(PID);

        let file = path(1);
        let mut step = StepBuilder::default().build(&mut script, PID, T0);
        // The owner claim is adopted through the first step that reaches
        // `admit`; a `FileLoaded` arriving while dormant marks no publish
        // intent.
        step.step(Frame::new(T0, Input::Timer), &mut claim);

        // The load resolves the Match, clears the (empty) card first, and
        // admits the publish: the sample is demanded in the same step.
        let opened = step.step(
            Frame::new(
                T0,
                Input::Mpv(MpvEvent::FileLoaded {
                    path: file.to_owned(),
                    live: live(&file, 0.0),
                }),
            ),
            &mut claim,
        );
        assert!(
            opened.effects.contains(&Effect::Sample),
            "the publish is admitted and the sample demanded: got {:?}",
            opened.effects
        );
        assert!(step.awaiting_sample());

        // The other window takes the claim before the answer arrives.
        claim.claim(PID + 1);

        let answered = step.step(
            Frame::new(T0, Input::Sampled(live(&file, 0.0))),
            &mut claim,
        );
        assert!(
            !answered
                .effects
                .iter()
                .any(|effect| matches!(effect, Effect::Publish(_))),
            "the racing publish is dropped: got {:?}",
            answered.effects
        );
    }
}
