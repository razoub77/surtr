//! mpv JSON IPC adapter: connects to the per-process pipe created by the Lua
//! launcher and translates raw messages into [`MpvEvent`]s for the core.
//!
//! `path`, `pause`, `duration`, `speed`, `eof-reached`, `focused` and the
//! session Toggle are observed once and drive updates. `time-pos` is
//! deliberately **not** observed: the kernel demands a sample immediately
//! before a dispatch, so a coalesced seek never carries a stale position.

use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use tokio::sync::{mpsc, oneshot};

use crate::daemon::{LiveState, MpvEvent, Property};

/// The daemon retries connecting every 50–100 ms for up to 5 s: mpv creates
/// its pipe listener asynchronously after `input-ipc-server` is set (ADR 0001).
const CONNECT_DEADLINE: Duration = Duration::from_secs(5);
const CONNECT_RETRY_MIN: Duration = Duration::from_millis(50);
const CONNECT_RETRY_MAX: Duration = Duration::from_millis(100);

/// Give up waiting for a `time-pos` reply after this and dispatch without it.
const SAMPLE_DEADLINE: Duration = Duration::from_secs(2);

/// The observed properties that trigger a Presence update, with their
/// `observe_property` ids. `time-pos` is absent on purpose.
const OBSERVED: [(&str, u64); 7] = [
    ("path", 1),
    ("pause", 2),
    ("duration", 3),
    ("speed", 4),
    ("eof-reached", 5),
    ("user-data/surtr/enabled", 6),
    ("focused", 7),
];

/// Why the session ended.
#[derive(Debug)]
pub enum SessionEnd {
    /// mpv is closing (shutdown event or the pipe broke): normal daemon life.
    Shutdown,
    /// Could not connect within the deadline: mpv never came up.
    ConnectTimeout,
}

/// One on-demand `time-pos` read waiting for mpv's reply.
struct PendingSample {
    request_id: u64,
    reply: oneshot::Sender<LiveState>,
    deadline: tokio::time::Instant,
}

/// One outbound command the session relays to mpv on behalf of the Daemon
/// Core's diagnostics effects ([`crate::daemon::Effect::Log`] and
/// [`crate::daemon::Effect::Osd`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outbound {
    /// A console line: `script-message surtr-log`, re-emitted by Lua as
    /// `mp.msg.error` so the console prefixes it with `[surtr]` (issue 07).
    Log(String),
    /// A transient OSD toast: `show-text`, visible for `seconds`.
    ShowText { text: String, seconds: u64 },
}

/// Runs one mpv session to completion.
///
/// - `events`: domain events handed to the core.
/// - `outbound`: the core's diagnostics, sent as `script-message surtr-log`
///   (console, through Lua) or `show-text` (OSD).
/// - `samples`: on-demand live-state reads. The Runner answers each sample
///   demand the kernel makes immediately before a dispatch; the adapter reads
///   `time-pos` fresh and answers with the observed properties around it.
pub async fn run_session(
    pipe: &str,
    events: mpsc::UnboundedSender<MpvEvent>,
    mut outbound: mpsc::UnboundedReceiver<Outbound>,
    mut samples: mpsc::UnboundedReceiver<oneshot::Sender<LiveState>>,
) -> SessionEnd {
    let Some(pipe) = connect(pipe).await else {
        return SessionEnd::ConnectTimeout;
    };

    // Real overlapped pipe I/O: a pending read must never stall a write (the
    // core can ask for `time-pos` while the read half is idle).
    let (reader, mut writer) = tokio::io::split(pipe);
    let mut reader = BufReader::new(reader);

    for (name, id) in OBSERVED {
        let command = json!({"command": ["observe_property", id, name]});
        if write_line(&mut writer, &command).await.is_err() {
            return pipe_broken(&events);
        }
    }

    let mut observed = LiveState::default();
    // Observer ids occupy 1..=7; command request ids start above them.
    let mut request_id: u64 = OBSERVED.len() as u64;

    // A daemon started behind a playing file behaves like any other start:
    // read the current path once and let the core resolve it.
    let initial_id = next_request(&mut request_id);
    let mut pending_initial_path = Some(initial_id);
    let command = json!({"command": ["get_property", "path"], "request_id": initial_id});
    if write_line(&mut writer, &command).await.is_err() {
        return pipe_broken(&events);
    }

    let mut pending_file_loaded: Option<u64> = None;
    let mut pending_sample: Option<PendingSample> = None;
    let mut line = String::new();

    loop {
        line.clear();
        let sample_deadline = pending_sample.as_ref().map(|sample| sample.deadline);
        tokio::select! {
            biased;

            // Console diagnostics and OSD toasts from the core.
            Some(outbound) = outbound.recv() => {
                let command = match outbound {
                    Outbound::Log(text) => {
                        json!({"command": ["script-message", "surtr-log", text]})
                    }
                    // mpv's `show-text` duration is milliseconds.
                    Outbound::ShowText { text, seconds } => {
                        json!({"command": ["show-text", text, seconds * 1000]})
                    }
                };
                if write_line(&mut writer, &command).await.is_err() {
                    return pipe_broken(&events);
                }
            }

            // A slow property read must never wedge playback or the core.
            _ = async {
                match sample_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }
            }, if pending_sample.is_some() => {
                let PendingSample { reply, .. } = pending_sample.take().unwrap();
                let _ = reply.send(observed.clone());
            }

            // A dispatch is ready: read `time-pos` fresh for it.
            Some(reply) = samples.recv() => {
                // A newer dispatch supersedes an unanswered read.
                if let Some(PendingSample { reply: old, .. }) = pending_sample.take() {
                    let _ = old.send(observed.clone());
                }
                let id = next_request(&mut request_id);
                pending_sample = Some(PendingSample {
                    request_id: id,
                    reply,
                    deadline: tokio::time::Instant::now() + SAMPLE_DEADLINE,
                });
                let command = json!({"command": ["get_property", "time-pos"], "request_id": id});
                if write_line(&mut writer, &command).await.is_err() {
                    return pipe_broken(&events);
                }
            }

            read = reader.read_line(&mut line) => {
                match read {
                    Ok(0) | Err(_) => return pipe_broken(&events),
                    Ok(_) => {
                        let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else {
                            continue;
                        };
                        if let Some(id) = msg.get("request_id").and_then(Value::as_u64) {
                            if absorb_reply(
                                id,
                                &msg,
                                &mut observed,
                                &mut pending_sample,
                                &mut pending_initial_path,
                                &mut pending_file_loaded,
                                &events,
                            ) == Reply::Exit
                            {
                                return SessionEnd::Shutdown;
                            }
                            continue;
                        }
                        match msg.get("event").and_then(Value::as_str) {
                            Some("property-change") => {
                                if observe_property(&msg, &mut observed, &events) == Reply::Exit {
                                    return SessionEnd::Shutdown;
                                }
                            }
                            Some("file-loaded") => {
                                // Read the path fresh: the property-change may
                                // not have arrived before the event.
                                observed.eof_reached = false;
                                let id = next_request(&mut request_id);
                                pending_file_loaded = Some(id);
                                let command =
                                    json!({"command": ["get_property", "path"], "request_id": id});
                                if write_line(&mut writer, &command).await.is_err() {
                                    return pipe_broken(&events);
                                }
                            }
                            Some("playback-restart") => {
                                if events.send(MpvEvent::PlaybackRestart).is_err() {
                                    return SessionEnd::Shutdown;
                                }
                            }
                            Some("end-file") => {
                                if events.send(MpvEvent::EndFile).is_err() {
                                    return SessionEnd::Shutdown;
                                }
                            }
                            Some("shutdown") => {
                                let _ = events.send(MpvEvent::Shutdown);
                                return SessionEnd::Shutdown;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

/// Whether handling a message asked the session to stop.
#[derive(PartialEq, Eq)]
enum Reply {
    Continue,
    Exit,
}

/// Routes one `request_id` reply to the request that issued it.
#[allow(clippy::too_many_arguments)]
fn absorb_reply(
    id: u64,
    msg: &Value,
    observed: &mut LiveState,
    pending_sample: &mut Option<PendingSample>,
    pending_initial_path: &mut Option<u64>,
    pending_file_loaded: &mut Option<u64>,
    events: &mpsc::UnboundedSender<MpvEvent>,
) -> Reply {
    let success = msg.get("error").and_then(Value::as_str) == Some("success");
    let data = msg.get("data");

    if pending_sample
        .as_ref()
        .is_some_and(|sample| sample.request_id == id)
    {
        if success {
            observed.time_pos = data.and_then(Value::as_f64);
        }
        let PendingSample { reply, .. } = pending_sample.take().unwrap();
        let _ = reply.send(observed.clone());
        return Reply::Continue;
    }

    if *pending_initial_path == Some(id) || *pending_file_loaded == Some(id) {
        if *pending_initial_path == Some(id) {
            *pending_initial_path = None;
        }
        if *pending_file_loaded == Some(id) {
            *pending_file_loaded = None;
        }
        if success {
            if let Some(path) = data.and_then(Value::as_str) {
                observed.path = Some(path.to_owned());
                if events
                    .send(MpvEvent::FileLoaded {
                        path: path.to_owned(),
                        // The Toggle and the other observed properties at the
                        // moment this file was reported: mpv can deliver a
                        // keypress change after the load, and the file that is
                        // already playing must not flicker into view.
                        live: observed.clone(),
                    })
                    .is_err()
                {
                    return Reply::Exit;
                }
            }
        }
    }
    Reply::Continue
}

/// Mirrors one observed property into the live snapshot and emits its trigger.
fn observe_property(
    msg: &Value,
    observed: &mut LiveState,
    events: &mpsc::UnboundedSender<MpvEvent>,
) -> Reply {
    let data = msg.get("data");
    let property = match msg.get("name").and_then(Value::as_str) {
        Some("path") => {
            observed.path = data.and_then(Value::as_str).map(str::to_owned);
            None
        }
        Some("pause") => {
            observed.paused = data.and_then(Value::as_bool).unwrap_or(false);
            Some(Property::Pause(observed.paused))
        }
        Some("speed") => {
            observed.speed = data.and_then(Value::as_f64).unwrap_or(1.0);
            Some(Property::Speed(observed.speed))
        }
        Some("duration") => {
            observed.duration = data.and_then(Value::as_f64);
            Some(Property::Duration(observed.duration))
        }
        Some("eof-reached") => {
            observed.eof_reached = data.and_then(Value::as_bool).unwrap_or(false);
            Some(Property::EofReached(observed.eof_reached))
        }
        Some("user-data/surtr/enabled") => {
            observed.enabled = data.and_then(Value::as_bool).unwrap_or(true);
            Some(Property::Enabled(observed.enabled))
        }
        Some("focused") => {
            // A focus change is not a Presence trigger by itself: focus in
            // only starts claiming ownership, focus out writes nothing.
            if events
                .send(MpvEvent::FocusChanged(
                    data.and_then(Value::as_bool).unwrap_or(false),
                ))
                .is_err()
            {
                return Reply::Exit;
            }
            None
        }
        _ => None,
    };
    if let Some(property) = property {
        if events.send(MpvEvent::Property(property)).is_err() {
            return Reply::Exit;
        }
    }
    Reply::Continue
}

fn next_request(counter: &mut u64) -> u64 {
    *counter += 1;
    *counter
}

/// Emits a shutdown first: the pipe breaking is how "mpv died" reaches the core.
fn pipe_broken(events: &mpsc::UnboundedSender<MpvEvent>) -> SessionEnd {
    let _ = events.send(MpvEvent::Shutdown);
    SessionEnd::Shutdown
}

async fn write_line<W: AsyncWrite + Unpin>(writer: &mut W, value: &Value) -> std::io::Result<()> {
    let mut line = value.to_string().into_bytes();
    line.push(b'\n');
    writer.write_all(&line).await?;
    writer.flush().await
}

/// Opens mpv's pipe as an overlapped client. mpv owns the listener, so the
/// daemon is the client; both directions are truly async.
async fn connect(pipe: &str) -> Option<NamedPipeClient> {
    let start = std::time::Instant::now();
    let mut retry = CONNECT_RETRY_MIN;
    loop {
        match ClientOptions::new().open(pipe) {
            Ok(client) => return Some(client),
            Err(err) if is_retryable(&err) => {}
            Err(_) => return None,
        }
        if start.elapsed() >= CONNECT_DEADLINE {
            return None;
        }
        tokio::time::sleep(retry).await;
        retry = (retry * 2).min(CONNECT_RETRY_MAX);
    }
}

/// NotFound: the listener does not exist yet. WouldBlock: ERROR_PIPE_BUSY.
/// Both mean "mpv has not finished creating the listener" or "transient".
fn is_retryable(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::WouldBlock
    )
}
