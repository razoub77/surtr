//! mpv JSON IPC adapter tests against a real Windows named pipe.
//!
//! The fake mpv side is a real byte-mode listener: it accepts the daemon's
//! connection, reads every command, and writes scripted events/responses.
//! Nothing here touches Discord or the network.

#![cfg(windows)]

use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::{mpsc, oneshot};

use surtr::daemon::{Diagnostic, Level, LiveState, MpvEvent, Property};
use surtr::mpv::{run_session, Outbound, SessionEnd};

/// A listening pipe the daemon can connect to, mirroring mpv's byte-mode
/// listener.
struct Listener {
    name: String,
    server: NamedPipeServer,
}

impl Listener {
    fn new(tag: &str) -> Self {
        let name = format!(r"\\.\pipe\surtr-test-{tag}-{}", std::process::id());
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&name)
            .expect("CreateNamedPipeW failed");
        Self { name, server }
    }

    /// Waits for the daemon to connect; returns the mpv end of the pipe.
    async fn accept(self) -> NamedPipeServer {
        self.server
            .connect()
            .await
            .expect("ConnectNamedPipe failed");
        self.server
    }
}

/// Splits a connected pipe into a buffered reader and a writer.
fn split(
    server: NamedPipeServer,
) -> (
    BufReader<ReadHalf<NamedPipeServer>>,
    WriteHalf<NamedPipeServer>,
) {
    let (reader, writer) = tokio::io::split(server);
    (BufReader::new(reader), writer)
}

async fn next_command(reader: &mut BufReader<ReadHalf<NamedPipeServer>>) -> Option<Value> {
    let mut line = String::new();
    match tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut line)).await {
        Ok(Ok(0)) | Err(_) | Ok(Err(_)) => None,
        Ok(Ok(_)) => serde_json::from_str(line.trim()).ok(),
    }
}

async fn next_event(events: &mut mpsc::UnboundedReceiver<MpvEvent>) -> MpvEvent {
    tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("an event within 5 s")
        .expect("the event channel is open")
}

async fn reply(writer: &mut WriteHalf<NamedPipeServer>, command: &Value, data: Value) {
    let reply = serde_json::json!({
        "error": "success",
        "data": data,
        "request_id": command["request_id"],
    });
    writer
        .write_all(format!("{reply}\n").as_bytes())
        .await
        .unwrap();
}

#[tokio::test]
async fn observed_properties_drive_events_and_time_pos_is_read_on_demand() {
    let listener = Listener::new("snapshot");
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let (_log_tx, log_rx) = mpsc::unbounded_channel();
    let (samples_tx, samples_rx) = mpsc::unbounded_channel();
    let pipe = listener.name.clone();
    let session =
        tokio::spawn(async move { run_session(&pipe, events_tx, log_rx, samples_rx).await });

    let mpv = listener.accept().await;
    let (mut reader, mut writer) = split(mpv);

    // Every trigger is observed exactly once; `time-pos` is absent.
    let observed = [
        ("path", 1),
        ("pause", 2),
        ("duration", 3),
        ("speed", 4),
        ("eof-reached", 5),
        ("user-data/surtr/enabled", 6),
        ("focused", 7),
    ];
    for (name, id) in observed {
        let command = next_command(&mut reader)
            .await
            .expect("daemon should observe a property");
        assert_eq!(
            command["command"],
            serde_json::json!(["observe_property", id, name])
        );
    }

    // A daemon started behind a playing file reads the current path once.
    let command = next_command(&mut reader)
        .await
        .expect("daemon should read the current path");
    assert_eq!(
        command["command"],
        serde_json::json!(["get_property", "path"])
    );
    let path = r"D:\Anime\Frieren - 01.mkv";
    reply(&mut writer, &command, Value::String(path.into())).await;
    // `file-loaded` carries the snapshot read with the path: the Toggle at
    // the moment the file was reported, so a hidden session stays hidden.
    assert_eq!(
        next_event(&mut events_rx).await,
        MpvEvent::FileLoaded {
            path: path.into(),
            live: LiveState {
                path: Some(path.into()),
                enabled: true,
                ..LiveState::default()
            },
        }
    );

    // Observed property changes become triggers.
    let properties = [
        ("pause", Value::Bool(true)),
        ("duration", Value::from(1440.0)),
        ("speed", Value::from(1.5)),
        ("eof-reached", Value::Bool(false)),
        ("user-data/surtr/enabled", Value::Bool(true)),
    ];
    for (name, data) in properties {
        let message = serde_json::json!({
            "event": "property-change",
            "name": name,
            "data": data,
        });
        writer
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
    }
    assert_eq!(
        next_event(&mut events_rx).await,
        MpvEvent::Property(Property::Pause(true))
    );
    assert_eq!(
        next_event(&mut events_rx).await,
        MpvEvent::Property(Property::Duration(Some(1440.0)))
    );
    assert_eq!(
        next_event(&mut events_rx).await,
        MpvEvent::Property(Property::Speed(1.5))
    );
    assert_eq!(
        next_event(&mut events_rx).await,
        MpvEvent::Property(Property::EofReached(false))
    );
    assert_eq!(
        next_event(&mut events_rx).await,
        MpvEvent::Property(Property::Enabled(true))
    );

    // Focus is its own event, not a Presence trigger: only `focused = true`
    // starts an ownership claim, `false` (an alt-tab to a browser) does
    // nothing.
    for focused in [Value::Bool(true), Value::Bool(false)] {
        let message = serde_json::json!({
            "event": "property-change",
            "name": "focused",
            "data": focused,
        });
        writer
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
    }
    assert_eq!(
        next_event(&mut events_rx).await,
        MpvEvent::FocusChanged(true)
    );
    assert_eq!(
        next_event(&mut events_rx).await,
        MpvEvent::FocusChanged(false)
    );

    // `file-loaded` reads the path fresh instead of trusting the last change.
    writer
        .write_all(b"{\"event\":\"file-loaded\"}\n")
        .await
        .unwrap();
    let command = next_command(&mut reader)
        .await
        .expect("daemon should read the loaded path");
    assert_eq!(
        command["command"],
        serde_json::json!(["get_property", "path"])
    );
    let next_path = r"D:\Anime\Frieren - 02.mkv";
    reply(&mut writer, &command, Value::String(next_path.into())).await;
    assert_eq!(
        next_event(&mut events_rx).await,
        MpvEvent::FileLoaded {
            path: next_path.into(),
            live: LiveState {
                path: Some(next_path.into()),
                paused: true,
                speed: 1.5,
                duration: Some(1440.0),
                ..LiveState::default()
            },
        }
    );

    // A dispatch asks for `time-pos` immediately before publishing.
    let (reply_tx, reply_rx) = oneshot::channel();
    samples_tx.send(reply_tx).unwrap();
    let command = next_command(&mut reader)
        .await
        .expect("daemon should read time-pos");
    assert_eq!(
        command["command"],
        serde_json::json!(["get_property", "time-pos"])
    );
    reply(&mut writer, &command, Value::from(123.5)).await;
    let live: LiveState = tokio::time::timeout(Duration::from_secs(5), reply_rx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        live,
        LiveState {
            path: Some(next_path.into()),
            time_pos: Some(123.5),
            duration: Some(1440.0),
            paused: true,
            speed: 1.5,
            eof_reached: false,
            enabled: true,
        }
    );

    // Seek and end-of-file events pass straight through.
    writer
        .write_all(b"{\"event\":\"playback-restart\"}\n")
        .await
        .unwrap();
    assert_eq!(next_event(&mut events_rx).await, MpvEvent::PlaybackRestart);
    writer
        .write_all(b"{\"event\":\"end-file\"}\n")
        .await
        .unwrap();
    assert_eq!(next_event(&mut events_rx).await, MpvEvent::EndFile);

    // mpv closing the pipe ends the session as a clean shutdown (ADR 0001).
    drop(writer);
    drop(reader);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), session)
            .await
            .unwrap()
            .unwrap(),
        SessionEnd::Shutdown
    ));
}

#[tokio::test]
async fn outbound_diagnostics_reach_mpv_as_script_message_and_show_text() {
    let listener = Listener::new("outbound");
    let (events_tx, _events_rx) = mpsc::unbounded_channel();
    let (outbound_tx, outbound_rx) = mpsc::unbounded_channel::<Outbound>();
    let (_samples_tx, samples_rx) = mpsc::unbounded_channel();
    let pipe = listener.name.clone();
    let session =
        tokio::spawn(async move { run_session(&pipe, events_tx, outbound_rx, samples_rx).await });

    let mpv = listener.accept().await;
    let (mut reader, writer) = split(mpv);

    // The session opens with its property observations and the initial path
    // read; drain them before the core's diagnostics arrive.
    for _ in 0..8 {
        assert!(
            next_command(&mut reader).await.is_some(),
            "a startup command"
        );
    }

    let text = "skipping non-local path https://x";
    for (level, message) in [
        (Level::Info, "surtr-log"),
        (Level::Warn, "surtr-log-warn"),
        (Level::Error, "surtr-log-error"),
        (Level::Verbose, "surtr-log-verbose"),
    ] {
        outbound_tx
            .send(Outbound::Log(Diagnostic::new(level, text)))
            .unwrap();
        let command = next_command(&mut reader)
            .await
            .expect("the console line on the wire");
        assert_eq!(
            command["command"],
            serde_json::json!(["script-message", message, text])
        );
    }

    outbound_tx
        .send(Outbound::ShowText {
            text: r#"Surtr: no Anime match for "honzuki no gekokujou""#.into(),
            seconds: 5,
        })
        .unwrap();
    let command = next_command(&mut reader)
        .await
        .expect("the OSD on the wire");
    assert_eq!(
        command["command"],
        serde_json::json!([
            "show-text",
            r#"Surtr: no Anime match for "honzuki no gekokujou""#,
            5_000,
        ]),
        "mpv's show-text duration is milliseconds"
    );

    // mpv closing the pipe ends the session as a clean shutdown (ADR 0001).
    drop(outbound_tx);
    drop(writer);
    drop(reader);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), session)
            .await
            .unwrap()
            .unwrap(),
        SessionEnd::Shutdown
    ));
}

#[tokio::test]
async fn shutdown_event_ends_the_session() {
    let listener = Listener::new("shutdown");
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let (_log_tx, log_rx) = mpsc::unbounded_channel();
    let (_samples_tx, samples_rx) = mpsc::unbounded_channel();
    let pipe = listener.name.clone();
    let session =
        tokio::spawn(async move { run_session(&pipe, events_tx, log_rx, samples_rx).await });

    let mpv = listener.accept().await;
    let (_reader, mut writer) = split(mpv);
    writer
        .write_all(b"{\"event\":\"shutdown\"}\n")
        .await
        .unwrap();

    assert_eq!(next_event(&mut events_rx).await, MpvEvent::Shutdown);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), session)
            .await
            .unwrap()
            .unwrap(),
        SessionEnd::Shutdown
    ));
}

#[tokio::test]
async fn connect_times_out_when_no_pipe_appears() {
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let (_log_tx, log_rx) = mpsc::unbounded_channel();
    let (_samples_tx, samples_rx) = mpsc::unbounded_channel();
    let missing = format!(r"\\.\pipe\surtr-test-missing-{}", std::process::id());

    let started = std::time::Instant::now();
    let end = run_session(&missing, events_tx, log_rx, samples_rx).await;
    let elapsed = started.elapsed();

    assert!(matches!(end, SessionEnd::ConnectTimeout));
    // Retries every 50–100 ms for up to 5 s, never much longer.
    assert!(
        elapsed >= Duration::from_secs(4),
        "gave up too early: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(8),
        "gave up too late: {elapsed:?}"
    );
    assert!(
        events_rx.try_recv().is_err(),
        "no events without a connection"
    );
}
