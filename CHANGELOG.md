# Changelog

## [0.3.0] — 2026-11-21

### Added

- Level-tagged console Diagnostics: a loaded file prints its full path, each
  search prints its cleaned query and Candidate count, and the chosen Candidate
  and settled Match print their AniList ids; verbose (`--msg-level=all=v`) adds
  Candidate scores, cache hits and Season Resolution detail. A failed or
  throttled AniList request now warns with its query or retry delay instead of
  being silent, without touching playback or Presence.
- New `pause_timeout` setting in `script-opts/surtr.conf` clears the Presence card after a whole number of paused seconds. `0` (the default) keeps the paused card until resume or another file loads.

### Changed

- Console Diagnostics no longer repeat `surtr:` in their text; mpv's `[surtr]`
  prefix is the only tag. A loaded file prints `playing: <full path>`. OSD
  notifications keep `Surtr:` because they have no mpv prefix.
- Console Diagnostics now carry a level from the Daemon to mpv: four IPC script
  messages (`surtr-log`, `surtr-log-warn`, `surtr-log-error`,
  `surtr-log-verbose`) map to the matching `mp.msg` functions. Every previous
  line stays unchanged at error level; verbose needs `--msg-level=all=v`.
  No compatibility shim: daemon and launcher ship together.
- Manual Overrides moved from `%LOCALAPPDATA%\surtr\overrides.json` to
  `script-opts/surtr-overrides.json`, beside `surtr.conf`. `overrides_path` in
  the conf points at a different file; the default is the file beside the conf.
  On first start the old file is copied to the configured path, or merged when
  both exist, and then removed. A failed move or invalid data keeps the old file.

### Fixed

- A parsed release year — on the basename, or the nearest ancestor folder when the basename has none — now rules out Candidates with a different known AniList `seasonYear`. Remakes no longer displace the original: `Fruits Basket (2019)` matches the 2019 season. Candidates with no known year stay eligible; the year never enters the search query or cache key.
- `DVD480p`-style resolution tags no longer leak into the parsed title, so `Pandora Hearts_-_01_(10bit_DVD480p_x265).mkv` matches Pandora Hearts.

## [0.2.0] — 2026-11-17

### Added

- The installer opens with a menu: **Install / Update** or **Uninstall**.
  Arrow keys move, `Enter` chooses, `Esc` exits; a console that cannot read
  keys gets the same choices as numbered prompts.
- Every detected mpv Configuration Folder is offered as a tick box, so one
  install can cover `%APPDATA%\mpv`, a portable `portable_config`, `MPV_HOME`
  and Scoop at once. Nothing detected still means typing a path.
- A `surtr.zip` beside `install.ps1` is installed from the file and the
  network is never touched, so an offline install is two downloaded files in
  one folder. A bad local archive fails naming the missing files rather than
  silently downloading.
- Uninstall ticks the folders to clean and asks before deleting
  `%LOCALAPPDATA%\surtr`; `Enter` keeps it.

### Changed

- When a season-1 or unmarked file's candidates tie at the best score, the
  matcher now proves the Root Season from the SEQUEL chains instead of
  guessing: a tied Candidate is the Root Season only when its unique chain
  reaches every other tied id. Two such Candidates (a cycle) stay a repairable
  Miss, and a throttled or failed relations request records no Miss at all.
  This tie-break needs no extra request while the disk cache is warm.
- The lookup's answer now carries `RateLimited`/`Unavailable` and the Miss,
  instead of a retry flag on the source and a separate Miss protocol.
- The daemon core is one seam with one harness, and the production loop is a
  `Host` module. No user-facing behaviour change.

### Fixed

- A season-1 file whose candidates tie no longer picks the sequel: Honzuki S1
  now matches its own entry (AniList id 108268) rather than the later season.

## [0.1.2] — 2026-11-15

### Changed

- Discord cover art now routes through the free WordPress Photon image proxy
  (`https://i0.wp.com/<target-without-scheme>?resize=1024,1024`) instead of
  linking the raw AniList CDN URL directly. Photon resizes and center-crops
  covers on the fly and does not block `anilist.co`.

[0.3.0]: https://github.com/razoub77/surtr/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/razoub77/surtr/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/razoub77/surtr/compare/v0.1.1...v0.1.2
