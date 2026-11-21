//! Discord IPC adapter: connects to the local Discord client's named pipe and
//! publishes/clears the Rich Presence activity.
//!
//! Frame format: u32 opcode LE, u32 length LE, payload. Opcodes: 0 HANDSHAKE,
//! 1 FRAME, 2 CLOSE, 3 PING, 4 PONG. Commands are FRAME frames carrying
//! `{"cmd": "SET_ACTIVITY", "args": {..}, "nonce": ".."}`.

use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::ClientOptions;
use tokio::sync::mpsc;

use crate::daemon::{Diagnostic, Presence};
use crate::mpv::Outbound;

/// Commands from the core's PresenceSink.
#[derive(Debug, Clone)]
pub enum DiscordCmd {
    Publish(Presence),
    Clear,
}

/// Discord activity type 3 = "Watching".
const ACTIVITY_WATCHING: u8 = 3;

/// Discord status display type 2 = Details: the compact profile status line
/// renders this field after "Watching", so it reads `Watching {details}`
/// instead of `Watching {application name}`.
const STATUS_DISPLAY_DETAILS: u8 = 2;

/// The name Discord renders for the application in the views that show it —
/// the expanded card's header (`Watching Anime`) and any Name-based status
/// display. Sent in the payload, it replaces the registered application name
/// without a developer-portal rename (limusic's approach).
const ACTIVITY_NAME: &str = "Anime";

/// Frame opcode 2 CLOSE: after the handshake it means Discord refused the
/// connection (unknown client id).
const OP_CLOSE: u32 = 2;

/// The Surtr application registered at discord.com/developers. Its ID ships in
/// the binary so the install needs no portal setup: the Discord client renders
/// the payload's `name` field instead of the registered application name, so
/// the registered name is never shown anyway. A `client_id` in
/// `script-opts/surtr.conf` (or `SURTR_CLIENT_ID`, or `--client-id`) overrides
/// this for publishing through a different application.
const DEFAULT_CLIENT_ID: &str = "1570608302629458140";

/// How long a handshake may sit unanswered before that pipe is given up on.
/// Discord answers a refused id with a CLOSE frame within milliseconds; no
/// reply at all has been observed for ids it stalls on, and an unbounded wait
/// here would hang the publisher with nothing in any console.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Where the publisher starts backing off, doubling up to [`RECONNECT_BACKOFF_MAX`].
const RECONNECT_BACKOFF_MIN: Duration = Duration::from_millis(500);
const RECONNECT_BACKOFF_MAX: Duration = Duration::from_secs(60);

/// A failed connection burst reports itself at most once per this interval:
/// the first failure is visible, a long outage is not spam.
const REPORT_INTERVAL: Duration = Duration::from_secs(60);

/// Discord's pipe has ten fixed indexes (stable/PTB/canary installs).
pub fn standard_pipes() -> Vec<String> {
    (0..10).map(pipe_name).collect()
}

fn pipe_name(index: u32) -> String {
    format!(r"\\?\pipe\discord-ipc-{index}")
}

/// The per-field limit Discord validates client-side: an oversized `details`
/// (a 233-character romaji title does this) is answered with `evt: ERROR`
/// ("details length must be less than or equal to 128 characters long") and
/// the activity is never set. Every free-text field is therefore clipped here,
/// at the seam that speaks Discord's protocol — the Match keeps its full
/// title for the links and the cover hover.
const FIELD_LIMIT: usize = 128;

/// Clipped to [`FIELD_LIMIT`], ending on the last whole word and marked with
/// an ellipsis; a title with no spaces (CJK) is cut at a character boundary.
fn clip(text: &str) -> String {
    if text.chars().count() <= FIELD_LIMIT {
        return text.to_owned();
    }
    let clipped: String = text.chars().take(FIELD_LIMIT - 1).collect();
    match clipped.rfind(' ') {
        Some(cut) => format!("{}…", &clipped[..cut]),
        None => format!("{}…", clipped.chars().take(FIELD_LIMIT - 2).collect::<String>()),
    }
}

/// Builds the exact JSON payload the daemon sends (or prints under `--dry-run`).
pub fn activity_payload(presence: Option<&Presence>, pid: u32, nonce: u64) -> Value {
    let activity = presence.map(|p| {
        // Clicking the title text and the cover art both open the anime's
        // AniList page. Older clients ignore the `*_url` fields, rendering
        // the elements plain.
        let anilist_url = format!("https://anilist.co/anime/{}", p.id);
        let mut activity = json!({
            "name": ACTIVITY_NAME,
            "type": ACTIVITY_WATCHING,
            "details": clip(&p.details),
            "status_display_type": STATUS_DISPLAY_DETAILS,
            "details_url": anilist_url.as_str(),
        });
        // Discord treats `state: null` as an update to a null value; the key
        // must be absent for a movie or an unknown episode total.
        if let Some(state) = &p.state {
            activity["state"] = json!(clip(state));
        }
        // Discord timestamps are Unix epoch *seconds*; the core works in ms.
        if let Some(start_ms) = p.start_ms {
            let mut timestamps = json!({ "start": start_ms / 1000 });
            if let Some(end_ms) = p.end_ms {
                timestamps["end"] = json!(end_ms / 1000);
            }
            activity["timestamps"] = timestamps;
        }
        // AniList's CDN URL does not go straight to Discord: route it through
        // the free WordPress Photon image proxy (`i0.wp.com`, `?resize=1024,1024`)
        // to resize and center-crop on the fly, no asset upload needed.
        // `large_text` is the hover text.
        if let Some(cover) = &p.cover {
            let proxy = format!(
                "https://i0.wp.com/{}?resize=1024,1024",
                cover.strip_prefix("https://").unwrap_or(cover)
            );
            activity["assets"] = json!({
                "large_image": proxy,
                "large_text": clip(&p.hover),
                "large_url": anilist_url,
            });
        }
        activity
    });
    json!({
        "cmd": "SET_ACTIVITY",
        "args": {
            "pid": pid,
            "activity": activity,
        },
        "nonce": nonce.to_string(),
    })
}

/// Runs the Discord publisher (or dry-run printer) until `rx` closes.
///
/// The pipe is only held while there is something to show: a clear sends
/// `activity: null` and closes the connection so another client can claim the
/// slot, and the next publish reconnects. A fresh connection never replays a
/// queued payload; it asks the core for a live-state republish (via
/// `reconnect`) instead. Reconnects back off from 500 ms to 60 s.
///
/// `diagnostics` receives one console line per failed connection burst
/// (throttled): a client id Discord refuses, or Discord not running, must be
/// visible in mpv's console instead of a silently absent card. `pipes` is the
/// scan list; production passes [`standard_pipes`], tests pass their own.
/// `client_id` is the configured application id (`--client-id`, fed from
/// `script-opts/surtr.conf`); `None` falls back to the `SURTR_CLIENT_ID`
/// environment variable, then to the application id built into the binary.
pub async fn run_publisher(
    mut rx: mpsc::UnboundedReceiver<DiscordCmd>,
    reconnect: mpsc::UnboundedSender<()>,
    dry_run: bool,
    client_id: Option<String>,
    diagnostics: Option<mpsc::UnboundedSender<Outbound>>,
    pipes: Vec<String>,
) {
    let client_id = client_id.unwrap_or_else(|| {
        std::env::var("SURTR_CLIENT_ID").unwrap_or_else(|_| DEFAULT_CLIENT_ID.to_owned())
    });
    let pid = std::process::id();
    let mut nonce: u64 = 0;
    let mut last_report: Option<tokio::time::Instant> = None;

    if dry_run {
        while let Some(cmd) = rx.recv().await {
            nonce += 1;
            match cmd {
                DiscordCmd::Publish(p) => {
                    println!("{}", activity_payload(Some(&p), pid, nonce));
                }
                DiscordCmd::Clear => {
                    println!("{}", activity_payload(None, pid, nonce));
                }
            }
        }
        return;
    }

    'disconnected: loop {
        // Nothing to show: stay off the pipe. A clear while disconnected is
        // already satisfied by the closed connection.
        loop {
            match rx.recv().await {
                None => return,
                Some(DiscordCmd::Publish(_)) => break,
                Some(DiscordCmd::Clear) => {}
            }
        }

        // Connect with backoff, still accepting commands while Discord is
        // unavailable: a clear sends us back to idle, a newer publish keeps
        // the backoff already earned so a rapid toggle cannot hammer the pipe.
        let mut backoff = RECONNECT_BACKOFF_MIN;
        let conn = loop {
            match connect(&client_id, &pipes).await {
                Some(conn) => break conn,
                None => {
                    let now = tokio::time::Instant::now();
                    if last_report.is_none_or(|at| now.duration_since(at) >= REPORT_INTERVAL) {
                        last_report = Some(now);
                        if let Some(tx) = &diagnostics {
                            let _ = tx.send(Outbound::Log(Diagnostic::error(
                                "Discord is not accepting a presence connection \
                                 (no answer or refusal on every pipe). Check that the \
                                 Discord app is running and signed in, and that \
                                 SURTR_CLIENT_ID names a registered application \
                                 (README §3 Configure)."
                                    .to_owned(),
                            )));
                        }
                    }
                    let delay = backoff;
                    backoff = (backoff * 2).min(RECONNECT_BACKOFF_MAX);
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        cmd = rx.recv() => {
                            match cmd {
                                None => return,
                                Some(DiscordCmd::Publish(_)) => {}
                                Some(DiscordCmd::Clear) => continue 'disconnected,
                            }
                        }
                    }
                }
            }
        };

        // The queued payload is never replayed: its timestamps went stale
        // while Discord was away. Ask the core to recompute from the live
        // player and send only what comes back (nothing, if the file changed
        // or ended meanwhile).
        let _ = reconnect.send(());

        let (mut reader, mut writer) = tokio::io::split(conn);
        loop {
            tokio::select! {
                biased;

                cmd = rx.recv() => {
                    let Some(cmd) = cmd else { return };
                    nonce += 1;
                    match cmd {
                        DiscordCmd::Publish(presence) => {
                            if send_activity(&mut writer, Some(&presence), pid, nonce).await.is_err() {
                                let _ = reconnect.send(());
                                continue 'disconnected;
                            }
                        }
                        DiscordCmd::Clear => {
                            // Release the activity slot, then close so a later
                            // client is not blocked by this connection.
                            let _ = send_activity(&mut writer, None, pid, nonce).await;
                            continue 'disconnected;
                        }
                    }
                }

                // Drain inbound frames so a closed pipe is noticed promptly,
                // and quote a rejected update: Discord validates text lengths
                // client-side and answers with an ERROR frame on this same
                // live connection, never a closed pipe. Without this the
                // absent card has no explanation in any console.
                read = read_frame(&mut reader) => {
                    match read {
                        Ok((_, payload)) => {
                            if let Some(message) = error_message(&payload) {
                                if let Some(tx) = &diagnostics {
                                    let _ = tx.send(Outbound::Log(Diagnostic::error(
                                        format!("Discord rejected the presence update: {message}")
                                    )));
                                }
                            }
                        }
                        Err(_) => {
                            let _ = reconnect.send(());
                            continue 'disconnected;
                        }
                    }
                }
            }
        }
    }
}

/// Opens the first pipe that completes a handshake, or `None`.
///
/// A refused id arrives as an op 2 CLOSE frame carrying `{"code": ...}`:
/// that pipe is refused, not connected. Silence is bounded by
/// [`HANDSHAKE_TIMEOUT`] so a stalled peer can never hang the publisher.
async fn connect(
    client_id: &str,
    pipes: &[String],
) -> Option<tokio::net::windows::named_pipe::NamedPipeClient> {
    for pipe in pipes {
        let Ok(client) = ClientOptions::new().open(pipe) else {
            continue;
        };
        let handshake = json!({ "v": 1, "client_id": client_id });
        let mut conn = client;
        if write_frame(&mut conn, 0, &handshake).await.is_err() {
            continue;
        }
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, read_frame(&mut conn)).await {
            // A refused id: op 2 CLOSE, payload `{"code": 4000, ...}`.
            Ok(Ok((op, _))) if op == OP_CLOSE => continue,
            // READY (or any welcome) means the connection is live.
            Ok(Ok(_)) => return Some(conn),
            Ok(Err(_)) | Err(_) => continue,
        }
    }
    None
}

async fn write_frame<W>(writer: &mut W, op: u32, payload: &Value) -> std::io::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let payload = serde_json::to_vec(payload).expect("activity payload serializes");
    writer.write_all(&op.to_le_bytes()).await?;
    writer
        .write_all(&(payload.len() as u32).to_le_bytes())
        .await?;
    writer.write_all(&payload).await?;
    writer.flush().await
}

/// The message of an ERROR frame: Discord's rejection reason for the last
/// SET_ACTIVITY (`evt: ERROR`, `data.code`/`data.message`). Any other frame
/// is a normal reply and carries nothing worth a console line.
fn error_message(payload: &Value) -> Option<String> {
    if payload.get("evt").and_then(Value::as_str) != Some("ERROR") {
        return None;
    }
    let message = payload
        .get("data")
        .and_then(|data| data.get("message"))
        .and_then(Value::as_str)?;
    (!message.is_empty()).then(|| message.to_owned())
}

async fn read_frame<R>(reader: &mut R) -> std::io::Result<(u32, Value)>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut header = [0u8; 8];
    reader.read_exact(&mut header).await?;
    let op = u32::from_le_bytes(header[0..4].try_into().unwrap());
    let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    Ok((op, serde_json::from_slice(&payload)?))
}

async fn send_activity<W>(
    writer: &mut W,
    presence: Option<&Presence>,
    pid: u32,
    nonce: u64,
) -> std::io::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    write_frame(writer, 1, &activity_payload(presence, pid, nonce)).await
}
