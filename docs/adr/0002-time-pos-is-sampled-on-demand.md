# `time-pos` is sampled on demand, not observed

Supersedes part of ADR 0001, which listed `time-pos` among the properties the daemon observes itself. Observing `time-pos` means one JSON IPC event per frame, permanently, whether or not anything is being published. It also makes a coalesced update carry whatever position happened to arrive last, not the position at dispatch.

So `time-pos` is **not** observed. `path`, `pause`, `duration`, `speed`, `eof-reached`, `focused` and `user-data/surtr/*` still are, and they drive the event stream. The Daemon Core's kernel issues a sample demand whenever it admits a dispatch and the host answers it at that instant, so a publish always carries the live position at dispatch time.

## Consequences

- The IPC reader stays quiet while a file plays; the write half of the pipe is idle until the next dispatch.
- Reads and writes must both be genuinely async, or a pending read blocks the on-demand `get_property`. The adapter therefore uses overlapped named-pipe I/O (`tokio::net::windows::named_pipe`), not `tokio::fs::File`, whose blocking-threadpool I/O serialises a cancelled read with the next write.
- A `time-pos` reply that never arrives must not wedge playback or the core: the adapter answers with the last observed state after a short deadline.
