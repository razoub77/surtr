//! Discord IPC adapter tests against a real Windows named pipe.
//!
//! The fake Discord side is a real listener that reads the handshake and
//! answers with scripted frames: READY welcome, op 2 CLOSE refusal, or
//! nothing at all. The silent case is the one that used to hang the
//! publisher forever with nothing in any console (an unreplying client id).

#![cfg(windows)]

use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::mpsc;

use surtr::daemon::Presence;
use surtr::discord::{run_publisher, DiscordCmd};
use surtr::mpv::Outbound;

/// A listening pipe the daemon can connect to, mirroring Discord's listener.
struct Listener {
    name: String,
    server: NamedPipeServer,
}

impl Listener {
    fn new(tag: &str) -> Self {
        let name = format!(r"\\.\pipe\surtr-discord-test-{tag}-{}", std::process::id());
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&name)
            .expect("CreateNamedPipeW failed");
        Self { name, server }
    }

    /// Waits for the daemon to connect; returns the Discord end of the pipe.
    async fn accept(self) -> NamedPipeServer {
        self.server
            .connect()
            .await
            .expect("ConnectNamedPipe failed");
        self.server
    }
}

fn split(
    server: NamedPipeServer,
) -> (
    ReadHalf<NamedPipeServer>,
    WriteHalf<NamedPipeServer>,
) {
    tokio::io::split(server)
}

/// One inbound frame: opcode, payload. `None` on error or a closed pipe.
async fn read_frame(reader: &mut ReadHalf<NamedPipeServer>) -> Option<(u32, Value)> {
    let mut header = [0u8; 8];
    reader.read_exact(&mut header).await.ok()?;
    let op = u32::from_le_bytes(header[0..4].try_into().unwrap());
    let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await.ok()?;
    Some((op, serde_json::from_slice(&payload).ok()?))
}

async fn write_frame(writer: &mut WriteHalf<NamedPipeServer>, op: u32, payload: &Value) {
    let bytes = serde_json::to_vec(payload).expect("payload serializes");
    let mut frame = Vec::with_capacity(8 + bytes.len());
    frame.extend_from_slice(&op.to_le_bytes());
    frame.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    frame.extend_from_slice(&bytes);
    writer.write_all(&frame).await.unwrap();
    writer.flush().await.unwrap();
}

/// A publisher pointed only at `listener`'s pipe (the nine standard indexes
/// do not exist in the test environment and are skipped instantly).
fn spawn_publisher(
    listener: &Listener,
) -> (
    mpsc::UnboundedSender<DiscordCmd>,
    mpsc::UnboundedReceiver<Outbound>,
) {
    let (tx, rx) = mpsc::unbounded_channel();
    let (reconnect_tx, mut reconnect_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while reconnect_rx.recv().await.is_some() {}
    });
    let (log_tx, log_rx) = mpsc::unbounded_channel();
    tokio::spawn(run_publisher(
        rx,
        reconnect_tx,
        false,
        Some("1234567890123456789".to_owned()),
        Some(log_tx),
        vec![listener.name.clone()],
    ));
    (tx, log_rx)
}

/// No SET_ACTIVITY reaches the refused connection. A retrying publisher may
/// legitimately write fresh handshakes (op 0) here; anything else fails.
async fn assert_no_activity(reader: &mut ReadHalf<NamedPipeServer>) {
    let window = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some((op, _)) = read_frame(reader).await {
            assert_eq!(op, 0, "only handshake retries, never an activity frame");
        }
    })
    .await;
    // Ok: the window passed with no activity frame. Err: timeout, same.
    assert!(window.is_ok() || window.is_err(), "unreachable");
}

fn presence() -> Presence {
    Presence {
        id: 0,
        details: "Test Anime".to_owned(),
        hover: "TV • Finished • Fall 2019".to_owned(),
        state: Some("\u{25B6}\u{FE0F} Playing • Episode 1/12".to_owned()),
        cover: None,
        start_ms: None,
        end_ms: None,
    }
}

/// The first console line the publisher emits for a failed connection burst.
async fn first_log(mut log_rx: mpsc::UnboundedReceiver<Outbound>) -> String {
    let received = tokio::time::timeout(Duration::from_secs(15), log_rx.recv())
        .await
        .expect("a diagnostic within 15 s")
        .expect("the log channel is open");
    let Outbound::Log(text) = received else {
        panic!("a log line, not another outbound command");
    };
    text
}

/// A silent handshake must time out and be reported, not hang forever. This
/// is the shipped failure: an id Discord stalls on gets no reply at all.
#[tokio::test]
async fn silent_handshake_is_reported_and_never_connects() {
    let listener = Listener::new("silent");
    let (tx, log_rx) = spawn_publisher(&listener);
    tx.send(DiscordCmd::Publish(presence())).unwrap();

    let server = listener.accept().await;
    let (mut reader, mut _writer) = split(server);
    let (op, handshake) =
        tokio::time::timeout(Duration::from_secs(5), read_frame(&mut reader))
            .await
            .expect("handshake within 5 s")
            .expect("frame present");
    assert_eq!(op, 0, "opcode HANDSHAKE");
    assert_eq!(handshake["v"], 1);
    assert!(handshake["client_id"].is_string(), "client id is a string");

    // No READY: silence. The publisher must give up on this pipe, report the
    // failure to the console, and never send an activity.
    let text = first_log(log_rx).await;
    assert!(
        text.contains("Discord"),
        "the diagnostic names Discord: {text}"
    );
    assert_no_activity(&mut reader).await;
}

/// A CLOSE refusal (unknown client id) is a refusal, not a connection: the
/// publisher must not treat the pipe as live and send an activity into it.
#[tokio::test]
async fn close_refusal_is_reported_and_never_connects() {
    let listener = Listener::new("refused");
    let (tx, log_rx) = spawn_publisher(&listener);
    tx.send(DiscordCmd::Publish(presence())).unwrap();

    let server = listener.accept().await;
    let (mut reader, mut writer) = split(server);
    let _handshake = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut reader))
        .await
        .expect("handshake within 5 s")
        .expect("frame present");
    write_frame(
        &mut writer,
        2,
        &json!({"code": 4000, "message": "Invalid Client ID"}),
    )
    .await;

    let text = first_log(log_rx).await;
    assert!(text.contains("Discord"), "the diagnostic names Discord: {text}");
    assert_no_activity(&mut reader).await;
}

/// A SET_ACTIVITY rejected by Discord's client-side validation arrives as an
/// ERROR frame on the live connection, not a closed pipe: it must reach the
/// console, or an absent card has no explanation anywhere.
#[tokio::test]
async fn an_error_frame_reply_is_reported_to_the_console() {
    let listener = Listener::new("error");
    let (tx, log_rx) = spawn_publisher(&listener);
    tx.send(DiscordCmd::Publish(presence())).unwrap();

    let server = listener.accept().await;
    let (mut reader, mut writer) = split(server);
    let _handshake = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut reader))
        .await
        .expect("handshake within 5 s")
        .expect("frame present");
    write_frame(&mut writer, 0, &json!({"user": {"id": "1"}})).await;
    tx.send(DiscordCmd::Publish(presence())).unwrap();
    let (op, activity) = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut reader))
        .await
        .expect("activity within 5 s")
        .expect("frame present");
    assert_eq!(op, 1, "opcode FRAME");
    assert_eq!(activity["cmd"], "SET_ACTIVITY");

    // Discord's answer for an oversized field (the measured payload).
    write_frame(
        &mut writer,
        1,
        &json!({
            "cmd": "SET_ACTIVITY",
            "data": {"code": 4000, "message": "child \"activity\" fails because [child \"details\" fails because [\"details\" length must be less than or equal to 128 characters long]]"},
            "evt": "ERROR",
            "nonce": "x",
        }),
    )
    .await;

    let text = first_log(log_rx).await;
    assert!(text.contains("Discord"), "the diagnostic names Discord: {text}");
    assert!(
        text.contains("128 characters"),
        "the rejection is quoted, not just counted: {text}"
    );
}

/// A READY welcome connects the publisher, and the core's republish reaches
/// Discord as one SET_ACTIVITY frame.
#[tokio::test]
async fn ready_welcome_connects_and_publishes() {
    let listener = Listener::new("ready");
    let (tx, _log_rx) = spawn_publisher(&listener);
    tx.send(DiscordCmd::Publish(presence())).unwrap();

    let server = listener.accept().await;
    let (mut reader, mut writer) = split(server);
    let _handshake = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut reader))
        .await
        .expect("handshake within 5 s")
        .expect("frame present");
    write_frame(&mut writer, 0, &json!({"user": {"id": "1"}})).await;

    // A fresh connection never replays the queued payload: the core is asked
    // to recompute and sends the publish it wants on the live state.
    tx.send(DiscordCmd::Publish(presence())).unwrap();

    let (op, activity) = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut reader))
        .await
        .expect("activity within 5 s")
        .expect("frame present");
    assert_eq!(op, 1, "opcode FRAME");
    assert_eq!(activity["cmd"], "SET_ACTIVITY");
    assert_eq!(activity["args"]["activity"]["details"], "Test Anime");
    assert_eq!(
        activity["args"]["activity"]["state"],
        "\u{25B6}\u{FE0F} Playing • Episode 1/12"
    );
}
