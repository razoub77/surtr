//! The Surtr daemon binary: selects over mpv events, the Runner's timer and
//! Discord reconnections, applies the effects each Runner call returns to the
//! mpv and Discord adapters, and runs until mpv closes or the pipe breaks.

// The daemon is spawned detached by mpv; a console window would flash behind
// the player. Diagnostics reach mpv's console over IPC, and argument errors
// go through `report_usage`.
#![windows_subsystem = "windows"]

use std::future::Future;
use std::pin::Pin;

use surtr::anilist::AniList;
use surtr::cache::{Cache, Cached};
use surtr::clock::{Clock, SystemClock};
use surtr::daemon::{Effect, LiveState, MpvEvent, Policy, Runner};
use surtr::matcher::Matcher;
use surtr::overrides::Overrides;
use surtr::owner::OwnerFile;
use surtr::{discord, mpv, paths};

const USAGE: &str = "\
surtr — anime Discord Rich Presence daemon for mpv

USAGE:
    surtr --mpv-pipe <pipe> [--dry-run] [--client-id <id>] [--anime-roots <roots>]
         [--details-title-lang <lang>]

OPTIONS:
    --mpv-pipe <pipe>    mpv JSON IPC pipe name, e.g. \\\\.\\pipe\\surtr-1234
    --dry-run            print activity payloads to stdout instead of
                         connecting to Discord
    --client-id <id>     Discord application client id (real publishing only;
                         overrides SURTR_CLIENT_ID)
    --anime-roots <roots>
                         semicolon-separated folders eligible for presence
                         (files outside every root produce no presence)
    --details-title-lang <lang>
                         title language of the card's main line: english or
                         romaji (default english; the other stands in)
";

struct Args {
    mpv_pipe: String,
    dry_run: bool,
    client_id: Option<String>,
    anime_roots: Option<String>,
    details_title_lang: Option<String>,
}

impl Args {
    /// The Anime Roots as separate folders; an empty list means "no filter".
    fn roots(&self) -> Vec<String> {
        surtr::cli::anime_roots(self.anime_roots.as_deref())
    }
}

#[cfg(windows)]
#[link(name = "user32", kind = "raw-dylib")]
extern "system" {
    fn MessageBoxA(hwnd: *mut u8, text: *const u8, caption: *const u8, kind: u32) -> i32;
}

/// Surtr's own diagnostics reach mpv's console over IPC, but argument errors
/// happen before mpv is involved and the release binary has no console to
/// print to (the `windows_subsystem` attribute above). A developer running
/// `--help` from a terminal still sees stderr; anyone else gets a dialog
/// instead of a silent nothing.
fn report_usage(text: &str) {
    use std::io::Write;
    let _ = std::io::stderr().write_all(text.as_bytes());
    #[cfg(windows)]
    {
        let text = format!("{text}\0");
        let caption = "Surtr\0";
        // MB_OK | MB_ICONERROR
        unsafe { MessageBoxA(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), 0x10) };
    }
}

fn parse_args() -> Option<Args> {
    let mut args = std::env::args().skip(1);
    let mut parsed = Args {
        mpv_pipe: String::new(),
        dry_run: false,
        client_id: None,
        anime_roots: None,
        details_title_lang: None,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mpv-pipe" => parsed.mpv_pipe = args.next()?,
            "--dry-run" => parsed.dry_run = true,
            "--client-id" => parsed.client_id = args.next(),
            "--anime-roots" => parsed.anime_roots = args.next(),
            "--details-title-lang" => parsed.details_title_lang = args.next(),
            "--help" | "-h" => {
                report_usage(USAGE);
                return None;
            }
            _ => {
                report_usage(&format!("unknown argument: {arg}\n\n{USAGE}"));
                std::process::exit(2);
            }
        }
    }
    if parsed.mpv_pipe.is_empty() {
        report_usage(&format!("missing --mpv-pipe\n\n{USAGE}"));
        std::process::exit(2);
    }
    Some(parsed)
}

use tokio::sync::mpsc::{UnboundedSender, WeakUnboundedSender};
use tokio::sync::oneshot;

/// A boxed future: the shape a call-site closure uses to answer a sample
/// demand without naming the concrete future type.
type BoxFuture<'f, T> = Pin<Box<dyn Future<Output = T> + 'f>>;

/// Applies one effect to the channel it belongs to. A closed Discord channel
/// or a dead mpv session drops the effect: both are already gone.
fn apply(
    effect: Effect,
    presence: &UnboundedSender<discord::DiscordCmd>,
    mpv: &UnboundedSender<mpv::Outbound>,
) {
    match effect {
        Effect::Publish(presence_body) => {
            let _ = presence.send(discord::DiscordCmd::Publish(presence_body));
        }
        Effect::Clear => {
            let _ = presence.send(discord::DiscordCmd::Clear);
        }
        Effect::Log(text) => {
            let _ = mpv.send(mpv::Outbound::Log(text));
        }
        Effect::Osd { text, seconds } => {
            let _ = mpv.send(mpv::Outbound::ShowText { text, seconds });
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

/// What woke the event loop.
enum Stimulus {
    Mpv(MpvEvent),
    Timer,
    Reconnected,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let Some(args) = parse_args() else { return };

    let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
    let (presence_tx, presence_rx) = tokio::sync::mpsc::unbounded_channel();
    let (mpv_tx, mpv_rx) = tokio::sync::mpsc::unbounded_channel::<mpv::Outbound>();
    let (samples_tx, samples_rx) = tokio::sync::mpsc::unbounded_channel();
    let (reconnect_tx, mut reconnect_rx) = tokio::sync::mpsc::unbounded_channel();

    // The publisher owns the Discord connection (or the dry-run printer).
    // Diagnostics ride the mpv channel so a refused connection is visible in
    // mpv's console instead of being a silently absent card.
    let publisher = tokio::spawn(discord::run_publisher(
        presence_rx,
        reconnect_tx,
        args.dry_run,
        surtr::cli::client_id(args.client_id.as_deref()),
        Some(mpv_tx.clone()),
        discord::standard_pipes(),
    ));
    // The mpv session owns the JSON IPC connection. The sampler holds only a
    // weak handle, so when the session ends a sample demand is simply an
    // empty answer.
    let samples = samples_tx.downgrade();
    let roots = args.roots();
    let pipe = args.mpv_pipe;
    let session =
        tokio::spawn(async move { mpv::run_session(&pipe, events_tx, mpv_rx, samples_rx).await });

    // `overrides.json` and `cache.json` are read once, at startup: editing
    // either means restarting mpv. Diagnostics go through the same route to
    // mpv as the core's, buffered until the session connects.
    let (overrides, override_messages) = match paths::overrides_file() {
        Some(path) => Overrides::load(&path),
        None => (Overrides::none(), Vec::new()),
    };
    for text in override_messages {
        apply(Effect::Log(text), &presence_tx, &mpv_tx);
    }
    let cache = match paths::cache_file() {
        Some(path) => Cache::load(path),
        None => Cache::disabled(),
    };
    // `owner.json` is the arbitration channel between mpv windows: only the
    // Daemon it names holds a Discord connection. The Runner carries it.
    let claim = OwnerFile::new(paths::owner_file());

    {
        let clock = SystemClock;
        let title_language = surtr::cli::title_language(args.details_title_lang.as_deref());
        let mut metadata = Matcher::new(Cached::new(AniList::new(), cache, SystemClock))
            .with_overrides(overrides)
            .with_title_language(title_language);
        let mut runner = Runner::new(
            &mut metadata,
            claim,
            std::process::id(),
            clock.now_millis(),
            Policy::default(),
            roots,
        );
        // The live player, read only when the kernel demands a sample.
        let mut sampler = live_sampler(samples);

        loop {
            // Sleep until the bucket refills, a grace window closes, a
            // deferred 429 retry comes due, or the claim needs re-checking.
            let wake = runner.wake_at();
            let sleep = async {
                match wake {
                    Some(at) => {
                        let delay = (at - clock.now_millis()).max(0) as u64;
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    }
                    None => std::future::pending::<()>().await,
                }
            };

            let stimulus = tokio::select! {
                event = events_rx.recv() => match event {
                    Some(event) => Stimulus::Mpv(event),
                    None => break,
                },
                _ = sleep => Stimulus::Timer,
                Some(()) = reconnect_rx.recv() => Stimulus::Reconnected,
            };

            // One stimulus in, the ordered effects out. The Runner answers
            // any sample demand from the live player at this instant.
            let now = clock.now_millis();
            let effects = match stimulus {
                Stimulus::Timer => runner.timer(now, &mut sampler).await,
                // Discord came back: republish from live state, no lookup.
                Stimulus::Reconnected => runner.reconnect(now, &mut sampler).await,
                Stimulus::Mpv(event) => {
                    // Every event reports the player it observed; the
                    // position is only read when a publish is dispatched.
                    let observed = match &event {
                        MpvEvent::FileLoaded { live, .. } => live.clone(),
                        _ => LiveState::default(),
                    };
                    runner.mpv(now, event, observed, &mut sampler).await
                }
            };
            for effect in effects {
                apply(effect, &presence_tx, &mpv_tx);
            }
            if runner.has_exited() {
                break;
            }
        }
    }

    // The Runner (and its Match lookup) is gone with the block above; dropping
    // the last senders lets the publisher observe channel EOF and flush its
    // final payload before the process exits.
    drop(presence_tx);
    drop(mpv_tx);
    let _ = publisher.await;

    let exit_code = match session.await {
        Ok(mpv::SessionEnd::Shutdown) => 0,
        Ok(mpv::SessionEnd::ConnectTimeout) => {
            report_usage("surtr: mpv IPC pipe did not appear within 5 s; exiting\n");
            1
        }
        Err(err) => {
            report_usage(&format!("surtr: mpv session failed: {err}\n"));
            1
        }
    };
    std::process::exit(exit_code);
}
