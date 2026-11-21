# Spec: Surtr — anime Discord Rich Presence for mpv

Status: ready-for-agent

## Problem Statement

When watching anime locally in mpv on Windows, Discord only displays a generic "mpv" process status or nothing at all. Existing Discord Rich Presence solutions for mpv either show raw messy filenames, scrape uncurated media tags, lack anime-specific season awareness, fail on absolute numbering, spam Discord's rate limits while seeking, leak private viewing habits without a clear session toggle, or require complex Python runtimes and external package managers.

A viewer wants their Discord profile to automatically showcase the canonical anime title, official cover art, correct season-relative episode progress, and a live timestamp progress bar without manual intervention, while remaining silent on non-anime media, respectful of privacy, and effortless to install.

## Solution

Surtr is a Windows-only mpv plugin pairing a lightweight Lua launcher with a self-contained Rust Daemon. The Lua script launches alongside mpv, registers an on/off Toggle shortcut, sets up a per-process named pipe, and fire-and-forget spawns the Daemon. The Daemon connects back to mpv over JSON IPC, observes player state and playback events, filters files against configured Anime Roots, and resolves clean metadata via the read-only AniList GraphQL API and a local cache.

When a valid anime file plays, the Daemon resolves the exact franchise season, remaps absolute episode numbers, and publishes a Rich Presence card containing the cover artwork, the configured title, `▶️ Playing • Episode x/y` state, a metadata hover line, and an interpolated timestamp progress bar to Discord IPC. Seeking coalesces through a token-bucket rate limiter; pausing freezes and strips timestamps cleanly; window focus changes smoothly transfer Discord ownership between mpv windows; and a simple keybind instantly clears presence. An overrides file gives viewers a clean escape hatch for edge cases, and all installation fits in a single merged config directory tree.

## User Stories

1. As an anime viewer, I want my Discord profile to show canonical romanized anime titles, so that my friends recognize what I am watching without deciphering release-group filenames.
2. As an anime viewer, I want the official AniList cover art displayed on my Discord activity card, so that my profile has rich visuals matching the anime.
3. As an anime viewer, I want episodic files to show my progress as `Episode x/y`, so that friends can see how far along I am in the season.
4. As an anime viewer, I want episodic anime with an unannounced or unknown total episode count to show `Episode x/?`, so that presence still renders cleanly without an arbitrary denominator.
5. As an anime viewer, I want an anime movie to show its title and cover art with the episode line it earns: a bare `Episode 1` when the entry counts one, no episode line at all when the file is unnumbered.
6. As an anime viewer, I want a live elapsed-and-remaining timestamp progress bar on Discord for episodic anime, so that friends see live progress through the episode.
7. As an anime viewer, I want the progress bar to calculate from playback speed, so that watching at 1.25x or 1.5x speed does not cause the Discord bar to drift out of sync.
8. As an anime viewer, I want seeking or scrubbing through an episode to update the progress bar to the new position once playback resumes, so that Discord accurately reflects my current position.
9. As an anime viewer, I want frequent seeking to coalesce into rate-limited updates, so that Discord does not rate-limit or drop my presence updates.
10. As an anime viewer, I want pausing playback to drop the progress bar and lead the state with `⏸️ Paused`, so that friends know I have stepped away and Discord does not keep running an inaccurate progress bar.
11. As an anime viewer, I want resuming playback after a pause to re-publish with fresh timestamps immediately, so that the live progress bar resumes accurately without waiting for a periodic poll.
12. As an anime viewer, I want long pauses to retain the paused presence card indefinitely by default, and to clear it after a configurable `pause_timeout` when I choose, so that leaving an episode on pause does not prematurely blank my Discord activity unless I ask it to.
13. As an anime viewer, I want files with `--keep-open` enabled to clear presence upon reaching the end of the file, so that a finished episode does not linger indefinitely as a paused card.
14. As an anime viewer, I want sequential episodes playing back-to-back in a playlist to transition smoothly within a short grace window, so that my Discord profile does not blink blank between episodes.
15. As an anime viewer, I want stopping playback or closing mpv to clear my presence promptly, so that Discord does not display a stale watching status when I am done.
16. As an anime viewer, I want only files located inside my configured Anime Roots to activate presence, so that home videos, western movies, and casual media in other directories remain completely private and ignored.
17. As an anime viewer, I want to toggle presence off with a single keyboard shortcut (`ctrl+shift+a`), so that I can immediately conceal my watch session for privacy without closing the player.
18. As an anime viewer, I want toggling presence back on with the keyboard shortcut to republish the current episode immediately, so that presence restores seamlessly without restarting playback.
19. As an anime viewer, I want multi-season anime with distinct subtitle titles or roman numerals (e.g., `2nd Season`, `II`) to resolve to their specific season entry, so that artwork, title, and episode progress match the actual season on screen.
20. As an anime viewer, I want absolute-numbered episodes (e.g., `Episode 50` across multiple 24-episode seasons) to automatically remap to their containing season (e.g., Season 3 Episode 2 of 12), so that long-running franchise releases display intuitive season-relative numbering.
21. As an anime viewer, I want title-less release basenames inside a named season directory (e.g., `Show S2/S02E05.mkv`) to inherit the show title from the parent directory, so that standard automated downloader layouts match reliably.
22. As an anime viewer, I want repeated queries and relation walks to be cached locally on disk for 30 days, so that offline play works smoothly and AniList API rate limits are conserved.
23. As an anime viewer, I want an anime title with no automatic AniList match to display a helpful mpv OSD message containing the exact key to copy, so that I know why presence did not appear and can easily fix it.
24. As an anime viewer, I want to add an override in `overrides.json` mapping a title key directly to an AniList ID, so that rare naming quirks or split titles can be permanently resolved.
25. As an anime viewer, I want overrides to short-circuit search entirely, so that user-specified IDs are unconditionally trusted and skip external search queries.
26. As an anime viewer, I want multiple simultaneous mpv windows to hand off Discord presence to whichever window was last focused, so that multiple open players do not fight over the single Discord user activity slot.
27. As an anime viewer, I want losing focus from mpv to an external application (like a web browser) to keep the active window's presence playing, so that multitasking does not clear my Discord status.
28. As an anime viewer, I want network failures, API 429 rate limits, and Misses to be named on the mpv console without blocking video playback, so that my viewing experience is never interrupted by networking glitches and I can tell why the card did not appear.
29. As an anime viewer, I want to install or update the plugin by running one PowerShell command that finds my mpv configuration folder by itself, so that setup takes less than two minutes without technical tooling or file merges.
30. As an anime viewer, I want uninstalling to be as simple as deleting the plugin directory, config file, and appdata cache folder, so that no residual services or registry entries remain on my system.

## Implementation Decisions

### Architectural Split and IPC Transport
- System divided into two components: a minimal Lua launcher (`main.lua`) running inside mpv and a compiled Rust Daemon process (`surtr.exe`).
- Lua is an un-opinionated launcher and gateway: upon mpv startup, it checks for mpv capability (`user-data`), creates a unique per-process named pipe path (`\\.\pipe\surtr-<pid>`), passes the pipe path and configuration options as CLI arguments when spawning the Daemon in detached mode, binds the session Toggle shortcut, and relays IPC log messages to mpv's console.
- The Daemon is the sole owner of all application logic: mpv JSON IPC observation, AniList GraphQL queries, candidate scoring, season resolution, relation traversal, disk caching, Discord IPC transport, token-bucket rate limiting, and presence lifecycle management.
- JSON IPC connection uses exponential retry (50–100 ms intervals up to a 5-second deadline) to accommodate mpv's asynchronous pipe listener creation.
- Lifetime binding: the Daemon exits cleanly when the mpv IPC pipe breaks or the parent mpv process terminates. There is no background lingering and no orphan daemon process.

### Configuration and Discovery
- Configuration lives in mpv's standard `script-opts/surtr.conf`, holding `anime_roots`, `client_id`, optional `binary_path`, `overrides_path`, `details_title_lang`, `pause_timeout`, and `toggle_key`.
- `anime_roots` is a semicolon-separated list of absolute Windows folder paths. Each path is canonicalized and compared case-insensitively per component to guard against prefix bleed (e.g. `D:\Anime2` will never match root `D:\Anime`).
- Binary discovery defaults to `surtr.exe` situated directly alongside `main.lua` in the script directory. An optional `binary_path` override in `surtr.conf` is expanded via mpv's `expand-path` command to support portable meta-paths (`~~/`, `~~exe_dir/`). Relative paths and working directory lookups are disallowed.
- If mpv lacks `user-data` property support (mpv < 0.36) or the Daemon executable cannot be located, Lua emits a single diagnostic error to the console and halts without binding shortcuts.
- If `anime_roots` is empty or unconfigured, Lua emits a single startup notice to the console, but the Daemon still launches and shortcut binds remain active so unmounted drives or later configured paths can function without restarting mpv.

### AniList Candidate Matching and Scoring
- Queries use the public AniList GraphQL endpoint (`POST https://graphql.anilist.co`) read-only with no user credentials.
- Pre-search query cleanup is mandatory: release group brackets, resolution tags, audio codecs, hashes, episode suffixes, and season-1 identifiers are stripped. Parent folder fallback provides the title if the basename is stripped of all title tokens.
- AniList queries explicitly enforce `Page(perPage: 10, search, sort: SEARCH_MATCH)` with `type: ANIME` to prevent manga and light novel results from polluting search candidates.
- Candidates are evaluated using a strict scoring system:
  - Exact normalized title match: 1.000.
  - Token containment (all query tokens contained within candidate title): 0.850.
  - Fuzzy string similarity below containment.
  - Bonus modifiers for release format (TV vs MOVIE vs OVA) and release year.
- A release year on the basename, or on the nearest ancestor folder when the basename has none, rules out Candidates with a different known `seasonYear`. Candidates without a known year stay eligible; year never enters the AniList query, cache key or Override key.
- Candidate acceptance requires the highest score to achieve at least 0.850 **and** exceed the runner-up score by a margin of at least 0.100.
- Synonyms are evaluated as a fallback only if no candidate's primary titles are an exact normalized match, preventing secondary synonym noise from triggering false positives while still letting a long official title (`Romelia Senki: Maou wo Taoshita nochi mo …`) be found by the short name it carries as a synonym.
- A score tie between candidates is never broken arbitrarily: ties result in no Match (and therefore no Presence), unless it is a Season 1 or unmarked file that fits within the Root Season's known episode count.

### Season Resolution and Absolute Numbering
- Matching strictly follows Season Resolution, never precedes it.
- Season markers from the release filename (or parent directory) guide query construction: explicit Season `N > 1` appends `Season N` (or normalized ordinals `2nd Season`, `II`) to the search query.
- When a parsed episode exceeds the Root Season's total episodes, the Daemon identifies an Absolute Episode and initiates a `SEQUEL` relation walk via GraphQL:
  - Traverses `SEQUEL` edges across TV, ONA, and TV_SHORT formats.
  - Movies, specials, and OVAs are traversed for continuity but omitted from the cumulative season episode tally.
  - Locates the unique containing season entry and remaps the absolute episode to its season-relative number (`Season Episode`) and season total (`Total Episodes`).
  - If cumulative episode counts are unknown, if relation chains branch ambiguously, or if the episode exceeds known franchise totals, the resolution yields no Match and presence is suppressed.

### Presence Layout, Lifecycle, and Rate Limiting
- Published Rich Presence payload follows Discord's Activity structure:
  - `details`: The card's main title (the configured `details_title_lang` title, the other language as fallback).
  - `state`: `▶️ Playing • Episode x/y` while playing, `⏸️ Paused • Episode x/y` while paused (`x/?` when the total is unknown or smaller than the episode; a one-episode entry shows the bare `Episode 1`; a file with no episode data — an unnumbered movie or OVA — shows the play/pause word alone while paused and no `state` while playing).
  - `assets.large_image`: AniList cover art URL routed through the free WordPress Photon image proxy (`https://i0.wp.com/<target-without-scheme>?resize=1024,1024`), which resizes and center-crops on the fly and does not block `anilist.co`.
  - `assets.large_text`: The metadata line `{Format} • {Status} • {Season} • {Genres}` (each segment omitted when the entry lacks it; the title stands in when none of the four render), instead of a second title.
  - `timestamps`: `start = now - (pos / speed)` and `end = now + ((duration - pos) / speed)`.
- Playing state publishes timestamps; paused state omits `timestamps` completely and leads `state` with `⏸️ Paused` instead of `▶️ Playing` to prevent the Discord client from rendering an inaccurate advancing progress bar.
- `time-pos` is never continuously observed over IPC; it is sampled on demand immediately prior to publishing an update.
- Rate limiting uses a token bucket with 5 tokens refilling at 1 token every 4 seconds (Discord's 5 updates per 20 seconds policy).
- `pause_timeout` is a whole number of seconds, `0` (the default) disabling it. A pause arms a deadline; seeking while paused does not restart it. On expiry the normal clear queue emits one clear; the card stays off through reconnects, claims and Toggles until playback resumes or another file loads. The owner claim is kept and the daemon does not exit.
- Exactly one desired Presence state is maintained in memory (latest wins). Queued updates re-sample live player properties (`time-pos`, `pause`, `speed`) at the actual moment of dispatch to eliminate stale timestamp payloads.
- Playlist advancement uses a 500 ms grace window on file termination (`end-file`): loading a subsequent file within the window transitions presence seamlessly without blanking.
- End-of-file conditions under `--keep-open` (`eof-reached = true`) are treated as playback completion and clear presence immediately.
- When presence is cleared or empty, the Daemon sends `activity: null` and closes the Discord IPC connection to release the slot for other applications.

### Multi-Window Ownership and Focus Handover
- Multi-instance coordination uses an ownership claim file stored at `%LOCALAPPDATA%\surtr\owner.json`.
- The Daemon corresponding to the most recently focused mpv window claims ownership by writing its process ID to `owner.json`.
- Only the active Owner maintains an active Discord IPC socket connection and publishes Presence.
- A previously active Owner detects when `owner.json` reflects a different process ID, whereupon it clears presence, terminates its Discord IPC connection, and enters a dormant observation mode.
- Losing focus to an external desktop application (e.g., browser or text editor) preserves the current Owner's status.

### Manual Overrides and Visibility
- Manual overrides live in `script-opts/surtr-overrides.json` beside `surtr.conf`, mapping normalized cleaned query strings (e.g., `"honzuki no gekokujou"` or `"youjo senki season 2"`) to integer AniList IDs. The `overrides_path` key in `surtr.conf` points elsewhere when set; its default and blank value both mean the file beside the conf.
- The legacy `%LOCALAPPDATA%\surtr\overrides.json` is moved to the configured path once at startup: copied byte-for-byte when the target is absent, merged with normalized-key conflicts won by the target when both exist. Invalid legacy data, an invalid target, or a failed write keeps both files and names the failure on mpv's console; a failed move still reads the legacy file for that run.
- Overrides short-circuit search completely: when a match exists in `overrides.json`, external search is bypassed, and metadata is fetched directly by ID.
- Overrides are parsed once at Daemon startup; modifications require restarting mpv.
- If an anime file within an Anime Root cannot be matched automatically, the Daemon surfaces a 5-second mpv OSD notification: `Surtr: no Anime match for "<key>"`, printing the exact string required for `overrides.json`. A repairable Miss earns the OSD and an error Diagnostic; a fatal Miss earns an error Diagnostic only; a failed AniList request or a 429 earns a warning Diagnostic naming the query or the Retry-After delay: no OSD, and playback is never affected.
- Daemon Diagnostics reach mpv's console as four level-tagged IPC script messages: `surtr-log` (info), `surtr-log-warn`, `surtr-log-error` and `surtr-log-verbose`. The Lua script routes each to the matching `mp.msg` function; mpv adds its `[surtr]` prefix. Diagnostic text has no extra `surtr:` prefix. Info names the loaded file's full path as `playing: <full path>`, each search's cleaned query and Candidate count, the chosen Candidate's id and score, and the settled Match's id, title and episode. Warning names a failed AniList request or a 429 with its retry delay. Error names a Miss, a skipped path, or a Discord connection or rejection failure. Verbose carries the top-three Candidates with scores, cache hits and live fetches, Season Resolution corroboration and contradiction, and each step of an Absolute Episode walk; it needs `--msg-level=all=v` and never reaches the OSD. The Daemon and launcher ship in the same zip, so there is no compatibility shim for the old error-only contract.

### Local Disk Cache
- Cache resides at `%LOCALAPPDATA%\surtr\cache.json`.
- Stores normalized search queries and full AniList media payloads (including relation edges) with a 30-day expiration TTL.
- Cache writes use atomic file replacement (write to temp file, rename) to prevent corruption during unexpected shutdowns.

## Testing Decisions

### Good Test Philosophy
- Automated tests must verify external behavior against domain contracts, never internal implementation details.
- Tests assert state transformations, emitted IPC commands, rate-limiting coalescing, and Presence payloads generated in response to recorded player events and API responses.
- Real network connections, operating system named pipes, and live Discord clients are excluded from automated test suites to maintain zero-flakiness and sub-second execution speeds.

### Target Module and Seam: The Daemon Core
Testing centers on one high-level architectural seam: the **Daemon Core State Machine**, plus one launcher smoke test for the four IPC log routes.

- **Inputs**: A scripted sequence of mpv JSON IPC events (`file-loaded`, `playback-restart`, `end-file`), property changes (`pause`, `speed`, `eof-reached`, `user-data/surtr/enabled`), and canned property responses (`path`, `time-pos`, `duration`).
- **Ports & Mocks**:
  - `MetadataClient`: Mocked using pre-recorded JSON fixture responses from AniList GraphQL queries.
  - `PresenceSink`: In-memory recording sink verifying the sequence of `publish(Activity)` and `clear()` payloads.
  - `Clock`: Controllable virtual clock for deterministic validation of the token-bucket rate limiter, 500 ms playlist grace window, 30-day cache TTL, and 429 backoff intervals.
  - `MpvSink`: Captures outgoing IPC commands (`show-text` for OSD and one of four `script-message surtr-log[-level]` commands for console Diagnostics).
- **Tested Behaviors**:
  - Full evaluation of the 51-file golden corpus verifying correct title, season resolution, absolute episode remapping, and candidate acceptance against recorded AniList payloads.
  - Rate-limiter token-bucket refill and seek-scrub coalescing with live timestamp re-sampling.
  - Pausing, unpausing, and speed modifications correctly altering or stripping Discord timestamp bars.
  - Grace window mechanics preventing presence flickering across consecutive playlist files.
  - Owner arbitration and Discord pipe connection closure upon empty presence or claim preemption.
  - Overrides file key normalization, search short-circuiting, and invalid JSON error reporting.

### Prior Art
- Pre-recorded offline fixtures and scoring test harnesses developed during early research (now `tests/fixtures/` and the scorer and corpus test suites).
- Standard deterministic state-machine and adapter testing patterns commonly utilized for protocol clients and event-driven daemons.

## Out of Scope

- Support for URL, HTTP, or streaming playback (filenames and paths are strictly required).
- AniList write-back functionality: OAuth authentication, watching status updates, score syncing, or user list modifications.
- Unresolvable absolute episode numbering without known franchise episode counts (e.g. `ONE PIECE - 1071`).
- Non-episodic OVA/specials lacking discrete episode counts, multi-episode bundled video files, and batch range filenames (`01-12`).
- Cross-platform operating system support (Linux and macOS named pipes/transports are excluded).
- MSI packages, Windows service registration, and CI/CD self-updating mechanisms: the Daemon never checks for or downloads updates itself, and nothing is registered with the OS. (A one-line install/update script is in scope; see ADR 0003.)
- Dynamic live configuration reloading while mpv is running without a restart.
- Discord Rich Presence custom buttons, small overlay icon artwork, and per-episode screenshot thumbnails.
- Automatic anime detection heuristics outside of the explicit Anime Root directory boundary.

## Further Notes

- Installation is a one-line PowerShell script (`scripts/install.ps1`, published with each release; see ADR 0003); the `surtr.zip` layout it installs is unchanged, and extracting the archive by hand still works.
- Installation uses a single archive and a single layout:
  ```
  surtr.zip
  ├── scripts/surtr/main.lua
  ├── scripts/surtr/surtr.exe
  ├── scripts/surtr/README.md
  └── script-opts/surtr.conf
  ```
  The installer also creates an empty `script-opts/surtr-overrides.json` only when neither an override file there nor the legacy appdata file exists; an existing override file is never overwritten.
  `scripts/install.ps1` (published beside it on each release) asks whether to install/update or uninstall, asks which detected mpv configuration folders (`%APPDATA%\mpv`, `portable_config`, …) to work on, and uses a `surtr.zip` beside it when present, otherwise downloads the archive. A console that cannot read keys gets the same choices as a numbered prompt.
- The README contains eight explicit user-facing sections: What it is, Install, Configure, Verify, Troubleshooting, Updating/Uninstalling, Known Limits, and Overrides.
