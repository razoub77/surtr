# Rust observes mpv over JSON IPC; the Lua script only launches it

mpv's Lua API can spawn a process but cannot hold a channel to one: there are no sockets or named pipes, `subprocess` writes stdin only at spawn (documented as unreliable on win32), and stdout is returned only when the process exits. A "Lua observes playback and pushes events to Rust" design is therefore impossible, so the boundary is inverted: the Lua script sets a per-process `input-ipc-server` pipe, fire-and-forget spawns the Rust daemon, forwards config as CLI args and binds the toggle, while the daemon observes `path`, `time-pos`, `duration`, `pause`, `focused` and `user-data/surtr/*` itself and owns AniList lookups, matching, caching and Discord IPC.

## Consequences

- All testable logic sits in Rust; Lua stays roughly 60 lines with nothing worth unit-testing.
- The daemon can drive mpv over the same pipe later (seek, next episode, OSD messages) without a new channel.
- Installation requires mpv ≥ 0.36, and the pipe must be named per process (`\\.\pipe\surtr-<pid>`) because mpv creates its listener with `FILE_FLAG_FIRST_PIPE_INSTANCE` — a shared name makes a second mpv window fail to listen.
- The pipe is created asynchronously after the option is set, so the daemon must retry connecting for a few seconds.

## Considered options

- **Lua pushes events to Rust over a socket or pipe** — not possible with stock mpv Lua.
- **Lua writes a state file, Rust tails it** — works, but 0.1–1 s latency, disk churn, and no back-channel.
- **One-shot process per event** — 10–50 ms spawn per event; fine for load/seek, useless for a progress ticker.
- **Rust launches mpv with `--input-ipc-client`** — cleanest channel, but it stops being an mpv plugin and requires controlling mpv's startup.

## Amended

ADR 0002 drops `time-pos` from the observed set: it is read on demand, only when a Presence update is about to be sent.

ADR 0004 adds one launcher duty: `overrides_path` is validated and expanded with mpv's `expand-path`, because only mpv can resolve `~~` / `~~exe_dir`. The decision logic stays in Rust; Lua still only launches, forwards config and binds keys.

