//! The Surtr daemon binary: parses the command line, constructs the mpv and
//! Discord adapters, and hands their channels to the [`surtr::host`] loop.

// The daemon is spawned detached by mpv; a console window would flash behind
// the player. Diagnostics reach mpv's console over IPC, and argument errors
// go through `report_usage`.
#![windows_subsystem = "windows"]

use surtr::anilist::AniList;
use surtr::cache::{Cache, Cached};
use surtr::clock::SystemClock;
use surtr::daemon::{Diagnostic, Policy};
use surtr::host;
use surtr::matcher::Matcher;
use surtr::overrides::Overrides;
use surtr::owner::OwnerFile;
use surtr::{discord, mpv, paths};

const USAGE: &str = "\
surtr — anime Discord Rich Presence daemon for mpv

USAGE:
    surtr --mpv-pipe <pipe> [--dry-run] [--client-id <id>] [--anime-roots <roots>]
         [--details-title-lang <lang>] [--overrides <path>]
         [--pause-timeout <seconds>]

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
    --overrides <path>   override file path (the launcher expands
                         script-opts/surtr-overrides.json by default)
    --pause-timeout <seconds>
                         clear Presence after this much continuous pause;
                         whole seconds, 0 (default) disables
";

struct Args {
    mpv_pipe: String,
    dry_run: bool,
    client_id: Option<String>,
    anime_roots: Option<String>,
    details_title_lang: Option<String>,
    overrides: Option<std::path::PathBuf>,
    pause_timeout: Option<String>,
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
        overrides: None,
        pause_timeout: None,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mpv-pipe" => parsed.mpv_pipe = args.next()?,
            "--dry-run" => parsed.dry_run = true,
            "--client-id" => parsed.client_id = args.next(),
            "--anime-roots" => parsed.anime_roots = args.next(),
            "--details-title-lang" => parsed.details_title_lang = args.next(),
            "--overrides" => parsed.overrides = args.next().map(std::path::PathBuf::from),
            "--pause-timeout" => parsed.pause_timeout = args.next(),
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

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let Some(args) = parse_args() else { return };

    let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel();
    let (presence_tx, presence_rx) = tokio::sync::mpsc::unbounded_channel();
    let (mpv_tx, mpv_rx) = tokio::sync::mpsc::unbounded_channel::<mpv::Outbound>();
    let (samples_tx, samples_rx) = tokio::sync::mpsc::unbounded_channel();
    let (reconnect_tx, reconnect_rx) = tokio::sync::mpsc::unbounded_channel();

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

    // The override file and `cache.json` are read once, at startup: editing
    // either means restarting mpv. Diagnostics go through the same route to
    // mpv as the core's, buffered until the session connects.
    let (overrides, override_messages) = Overrides::load_with_migration(
        args.overrides.as_deref(),
        paths::legacy_overrides_file().as_deref(),
    );
    for text in override_messages {
        let _ = mpv_tx.send(mpv::Outbound::Log(Diagnostic::error(text)));
    }
    // A bad pause_timeout never stops the daemon: report it and run with the
    // timeout off.
    let pause_timeout_ms = match surtr::cli::pause_timeout_ms(args.pause_timeout.as_deref()) {
        Ok(millis) => millis,
        Err(()) => {
            let _ = mpv_tx.send(mpv::Outbound::Log(Diagnostic::error(
                "surtr: invalid pause_timeout; using 0 (no timeout)".to_owned(),
            )));
            0
        }
    };
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
        host::run(
            host::Wiring {
                events: events_rx,
                reconnects: reconnect_rx,
                samples,
                presence: presence_tx,
                mpv: mpv_tx,
            },
            &mut metadata,
            claim,
            std::process::id(),
            roots,
            Policy {
                pause_timeout_ms,
                ..Policy::default()
            },
            &clock,
        )
        .await;
    }

    // The Host (and its Match lookup) is gone with the block above: the sink
    // senders dropped with it, so the publisher observes channel EOF and
    // flushes its final payload before the process exits.
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
