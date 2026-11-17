//! The production loop of one Daemon: the channels it holds, the observed
//! player snapshot between frames, and the routing of the kernel's effects to
//! the adapters.
//!
//! [`run`] is the only place that composes a live Daemon: it sleeps until the
//! [`Runner`]'s next wake, selects over mpv events and Discord reconnections,
//! hands the kernel one stimulus per turn, and applies the effects that came
//! back. `main` supplies the wiring and spawns the adapters; nothing here
//! parses arguments or exits the process.
//!
//! The observed snapshot is the host's memory of the player between frames:
//! mpv's observed properties as the adapter last reported them. `time-pos` is
//! never part of it (ADR 0002): a dispatch reads it fresh through the sample
//! channel, and the [`Runner`] answers every demand before the next stimulus.

use std::future::Future;
use std::pin::Pin;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, WeakUnboundedSender};
use tokio::sync::oneshot;

use crate::clock::Clock;
use crate::daemon::{
    Effect, LiveState, MetadataClient, MpvEvent, OwnerClaim, Policy, Property, Runner,
};
use crate::discord::DiscordCmd;
use crate::mpv::Outbound;

/// The channel ends one Daemon's loop holds: the stimuli in and the sinks out.
/// `main` creates the channels, spawns the adapters on the far ends, and hands
/// these to [`run`].
pub struct Wiring {
    /// mpv events from the session; `None` means mpv is gone.
    pub events: UnboundedReceiver<MpvEvent>,
    /// "Discord is connected again" notifications from the publisher.
    pub reconnects: UnboundedReceiver<()>,
    /// On-demand live-state reads. Weak, so a finished session answers with the
    /// empty state instead of being kept alive by the loop.
    pub samples: WeakUnboundedSender<oneshot::Sender<LiveState>>,
    /// Presence writes for the publisher.
    pub presence: UnboundedSender<DiscordCmd>,
    /// Console and OSD effects for the mpv session.
    pub mpv: UnboundedSender<Outbound>,
}

/// A boxed future: the shape a call-site closure uses to answer a sample
/// demand without naming the concrete future type.
type BoxFuture<'f, T> = Pin<Box<dyn Future<Output = T> + 'f>>;

/// What woke the event loop.
enum Stimulus {
    Mpv(MpvEvent),
    Timer,
    Reconnected,
}

/// Runs the Daemon until mpv is gone or the kernel exits.
///
/// `clock` and `policy` are injected so a test can drive the loop at a fixed
/// instant; production passes [`crate::clock::SystemClock`] and
/// [`Policy::default`]. The sinks close when this returns, which is how the
/// publisher learns to flush its last payload.
pub async fn run<C: OwnerClaim>(
    wiring: Wiring,
    metadata: &mut dyn MetadataClient,
    claim: C,
    pid: u32,
    roots: Vec<String>,
    policy: Policy,
    clock: &dyn Clock,
) {
    let Wiring {
        mut events,
        mut reconnects,
        samples,
        presence,
        mpv,
    } = wiring;
    let mut runner = Runner::new(metadata, claim, pid, clock.now_millis(), policy, roots);
    let mut sampler = live_sampler(samples);
    let mut observed = LiveState::default();

    loop {
        // Sleep until the bucket refills, a grace window closes, a deferred
        // 429 retry comes due, or the claim needs re-checking.
        let sleep = async {
            match runner.wake_at() {
                Some(at) => {
                    let delay = (at - clock.now_millis()).max(0) as u64;
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                }
                None => std::future::pending::<()>().await,
            }
        };

        let stimulus = tokio::select! {
            event = events.recv() => match event {
                Some(event) => Stimulus::Mpv(event),
                None => break,
            },
            _ = sleep => Stimulus::Timer,
            Some(()) = reconnects.recv() => Stimulus::Reconnected,
        };

        // One stimulus in, the ordered effects out. The Runner answers any
        // sample demand from the live player at this instant.
        let now = clock.now_millis();
        let effects = match stimulus {
            Stimulus::Timer => runner.timer(now, &mut sampler).await,
            // Discord came back: republish from live state, no lookup.
            Stimulus::Reconnected => runner.reconnect(now, &mut sampler).await,
            Stimulus::Mpv(event) => {
                observe(&mut observed, &event);
                runner.mpv(now, event, observed.clone(), &mut sampler).await
            }
        };
        for effect in effects {
            apply(effect, &presence, &mpv);
        }
        if runner.has_exited() {
            break;
        }
    }
}

/// Mirrors one event into the observed player snapshot: the properties mpv
/// reported, never the position (ADR 0002).
fn observe(observed: &mut LiveState, event: &MpvEvent) {
    match event {
        MpvEvent::FileLoaded { live, .. } => {
            *observed = LiveState {
                time_pos: None,
                ..live.clone()
            };
        }
        MpvEvent::Property(Property::Pause(paused)) => observed.paused = *paused,
        MpvEvent::Property(Property::Speed(speed)) => observed.speed = *speed,
        MpvEvent::Property(Property::Duration(duration)) => observed.duration = *duration,
        MpvEvent::Property(Property::EofReached(eof)) => observed.eof_reached = *eof,
        MpvEvent::Property(Property::Enabled(enabled)) => observed.enabled = *enabled,
        _ => {}
    }
}

/// Applies one effect to the channel it belongs to. A closed Discord channel
/// or a dead mpv session drops the effect: both are already gone.
fn apply(effect: Effect, presence: &UnboundedSender<DiscordCmd>, mpv: &UnboundedSender<Outbound>) {
    match effect {
        Effect::Publish(presence_body) => {
            let _ = presence.send(DiscordCmd::Publish(presence_body));
        }
        Effect::Clear => {
            let _ = presence.send(DiscordCmd::Clear);
        }
        Effect::Log(text) => {
            let _ = mpv.send(Outbound::Log(text));
        }
        Effect::Osd { text, seconds } => {
            let _ = mpv.send(Outbound::ShowText { text, seconds });
        }
        // The Runner answers every demand itself; none reach the host.
        Effect::Sample => {}
    }
}

/// The Runner's handshake with the live player: one snapshot per demand, read
/// from mpv through the session's oneshot channel. The sender is weak, so it
/// cannot keep a finished session's queue alive.
fn live_sampler(
    samples: WeakUnboundedSender<oneshot::Sender<LiveState>>,
) -> impl FnMut() -> BoxFuture<'static, LiveState> {
    move || {
        let samples = samples.clone();
        Box::pin(async move {
            let (reply, rx) = oneshot::channel();
            // A dead session answers with the empty state, which the core
            // turns into an empty Presence.
            let Some(tx) = samples.upgrade() else {
                return LiveState::default();
            };
            if tx.send(reply).is_err() {
                return LiveState::default();
            }
            rx.await.unwrap_or_default()
        })
    }
}
