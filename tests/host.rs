//! Host tests: the production loop itself — one stimulus in, the routed effects
//! out on the adapters' channels, and the loop's own exit.
//!
//! The Runner's Presence behaviour is covered by `core.rs`; these tests only
//! prove that `main`'s wiring and the Host agree: events in, a sample demand
//! answered, effects on the right sink, and a clean exit when mpv is gone.

use surtr::clock::Clock;
use surtr::daemon::{LiveState, Lookup, Miss, MpvEvent, OwnerClaim, Policy};
use surtr::discord::DiscordCmd;
use surtr::host::{self, Wiring};
use surtr::mpv::Outbound;
use tokio::sync::mpsc;
use tokio::sync::oneshot;

mod support;

use support::{matched, Script, SharedClaim, PID, T0};

/// A frozen clock: the loop's wake deadlines stay put, so a test never waits on
/// a clock it did not set.
struct FixedClock;

impl Clock for FixedClock {
    fn now_millis(&self) -> i64 {
        T0
    }
}

/// One Host's channels: the wiring it runs on plus the far ends a test drives
/// or asserts on.
struct Rig {
    wiring: Wiring,
    events: mpsc::UnboundedSender<MpvEvent>,
    samples: mpsc::UnboundedReceiver<oneshot::Sender<LiveState>>,
    presence: mpsc::UnboundedReceiver<DiscordCmd>,
    mpv: mpsc::UnboundedReceiver<Outbound>,
    /// The strong end of the sample channel, alive as it is in `main`; the
    /// wiring holds only the weak end.
    _samples_tx: mpsc::UnboundedSender<oneshot::Sender<LiveState>>,
}

impl Rig {
    /// The reconnect sender is dropped here, so that stimulus never fires.
    fn new() -> Self {
        let (events, events_rx) = mpsc::unbounded_channel();
        let (presence, presence_rx) = mpsc::unbounded_channel();
        let (mpv, mpv_rx) = mpsc::unbounded_channel();
        let (samples_tx, samples_rx) = mpsc::unbounded_channel();
        let (_reconnects, reconnect_rx) = mpsc::unbounded_channel();
        let wiring = Wiring {
            events: events_rx,
            reconnects: reconnect_rx,
            samples: samples_tx.downgrade(),
            presence,
            mpv,
        };
        Self {
            wiring,
            events,
            samples: samples_rx,
            presence: presence_rx,
            mpv: mpv_rx,
            _samples_tx: samples_tx,
        }
    }
}

const FILE: &str = r"D:\Anime\Frieren\S01E03.mkv";

fn live() -> LiveState {
    LiveState {
        path: Some(FILE.into()),
        duration: Some(1440.0),
        ..LiveState::default()
    }
}

/// Runs one Host on the current thread: its lookup and claim are not `Send`,
/// so it lives on a `LocalSet` beside the test body.
#[tokio::test]
async fn a_load_is_sampled_then_published() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let mut rig = Rig::new();
            let mut script = Script::scripted([matched()]);
            let lookups = script.lookups();
            let mut claim = SharedClaim::default();
            claim.claim(PID);
            let host = tokio::task::spawn_local(async move {
                host::run(
                    rig.wiring,
                    &mut script,
                    claim,
                    PID,
                    Vec::new(),
                    Policy::default(),
                    &FixedClock,
                )
                .await;
            });

            rig.events
                .send(MpvEvent::FileLoaded {
                    path: FILE.into(),
                    live: live(),
                })
                .unwrap();

            // The dispatch waits for the live position: answer as the session
            // would, with `time-pos` read fresh.
            let reply = rig.samples.recv().await.expect("a sample demand");
            let _ = reply.send(LiveState {
                time_pos: Some(120.0),
                ..live()
            });

            match rig.presence.recv().await.expect("a publish") {
                DiscordCmd::Publish(card) => assert_eq!(card.details, "Frieren"),
                DiscordCmd::Clear => panic!("the first card is a publish, not a clear"),
            }
            assert_eq!(
                lookups.borrow().as_slice(),
                [FILE],
                "the load was looked up"
            );

            rig.events.send(MpvEvent::Shutdown).unwrap();
            host.await.unwrap();
            assert!(
                matches!(rig.presence.recv().await, Some(DiscordCmd::Clear)),
                "shutdown clears the card before the loop ends"
            );
            assert!(
                rig.presence.recv().await.is_none(),
                "the sinks close when the Host ends"
            );
        })
        .await;
}

#[tokio::test]
async fn a_miss_is_logged_to_mpv_and_publishes_nothing() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let mut rig = Rig::new();
            let mut script = Script::scripted([Lookup::Missed(Miss::Fatal {
                reason: "no candidate".into(),
            })]);
            let mut claim = SharedClaim::default();
            claim.claim(PID);
            let host = tokio::task::spawn_local(async move {
                host::run(
                    rig.wiring,
                    &mut script,
                    claim,
                    PID,
                    Vec::new(),
                    Policy::default(),
                    &FixedClock,
                )
                .await;
            });

            rig.events
                .send(MpvEvent::FileLoaded {
                    path: FILE.into(),
                    live: live(),
                })
                .unwrap();

            match rig.mpv.recv().await.expect("a console line") {
                Outbound::Log(text) => assert!(text.contains("no Anime match"), "{text}"),
                Outbound::ShowText { .. } => panic!("a fatal miss is console-only"),
            }
            assert!(rig.presence.try_recv().is_err(), "a Miss publishes nothing");

            rig.events.send(MpvEvent::Shutdown).unwrap();
            host.await.unwrap();
        })
        .await;
}

#[tokio::test]
async fn the_loop_ends_when_mpv_is_gone() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let mut rig = Rig::new();
            let mut script = Script::scripted([]);
            let host = tokio::task::spawn_local(async move {
                host::run(
                    rig.wiring,
                    &mut script,
                    SharedClaim::default(),
                    PID,
                    Vec::new(),
                    Policy::default(),
                    &FixedClock,
                )
                .await;
            });

            // The session ends: its sender for mpv events is the loop's stop.
            drop(rig.events);
            host.await.unwrap();
            assert!(rig.presence.recv().await.is_none());
        })
        .await;
}
