# Surtr

The vocabulary of the mpv-to-Discord bridge: how a played file becomes a published activity.

## Language

**Anime Root**:
A configured directory; only files beneath one are eligible for presence.
_Avoid_: library, watch folder

**Release Filename**:
The basename of the played file: the primary evidence for title, season and episode. When the basename carries no title (`S02E05-Episode Title.mkv`), the parent folder supplies it. A basename that *does* name the show leaves the folder as a second spelling, searched only when the basename's title returns zero Candidates (a release's glued `Daiseijo` where AniList spells `Dai Seijo`).
_Avoid_: filename, video name

**Parsed Episode**:
The episode number written in the Release Filename, before season resolution. For an Absolute Episode it is the franchise-wide number.
_Avoid_: current episode, ep

**Absolute Episode**:
A Parsed Episode that continues across seasons instead of restarting at 1.
_Avoid_: cumulative episode, global episode

**Season Episode**:
The episode number within the resolved season. Equal to the Parsed Episode unless the file is absolute-numbered. Authoritative for display.
_Avoid_: current episode, local episode

**Total Episodes**:
The episode count of the matched AniList entry. The display denominator; the totals of preceding seasons also locate an Absolute Episode.
_Avoid_: episode count, length

**Candidate**:
An AniList entry returned by search, before scoring. The same payload shape
carries a `SEQUEL` relation node fetched by id during an Absolute Episode walk.
_Avoid_: result, hit

**Match**:
The AniList entry selected for the file, whether by scoring a Candidate or by Season Resolution. Without a Match there is no Presence.
_Avoid_: identification, lookup

**Miss**:
The reason a loaded file resolved to no Match. Repairable — one Override line could plausibly produce a card — earns the OSD and a console line; fatal misses stay console-only, and a failed request reports nothing.
_Avoid_: lookup failure, no result

**Lookup Answer**:
What one lookup of a Release Filename returns: a Match, a Miss, a rate-limit delay (the lookup is deferred and retried once after that delay), or no answer (a failed request — no Presence, no report). Exactly one of these, per lookup.
_Avoid_: lookup result

**Override**:
A line in `overrides.json` mapping a normalized cleaned query string to an AniList id the user asserts. An Override short-circuits search and is fetched by id; a dead id or a contradicted season means no Presence.
_Avoid_: manual match, mapping entry

**Season Resolution**:
Choosing the AniList entry for the season named or implied by the Release Filename, including following SEQUEL relations when the Parsed Episode is an Absolute Episode.
_Avoid_: season detection, lookup

**Root Season**:
The first entry in a franchise's SEQUEL chain; the default for a season-1 or unmarked file when scores tie.
_Avoid_: first season, original

**Format**:
The AniList entry's media format (TV, Movie, OVA, …); the leading segment of the hover line.
_Avoid_: type, kind

**Status**:
The AniList entry's release status (Finished, Ongoing, Hiatus); the second segment of the hover line. An unreleased or cancelled entry renders nothing.
_Avoid_: state, airing status

**Airing Season**:
The AniList entry's broadcast season and year (`Spring 2013`); the third segment of the hover line. An entry with a year but no season shows the year alone.
_Avoid_: season (when the filename's Season Marker is meant), release season

**Genres**:
The AniList entry's genre list, in AniList's own order, capped at the first four; the last segment of the hover line.
_Avoid_: tags, themes

**Presence**:
The Discord Rich Presence activity Surtr publishes.
_Avoid_: status, RP

**Daemon**:
The Rust process that observes mpv over JSON IPC and owns the Discord connection.
_Avoid_: backend, server, agent

**Host**:
The production loop of one Daemon: the channels it holds, the observed player snapshot between frames, and the routing of the kernel's effects to the mpv and Discord adapters. `main` builds it; tests drive the same entry point.
_Avoid_: event loop, runtime, scheduler

**Owner**:
The Daemon whose mpv window was last focused; the only Daemon allowed to publish Presence.
_Avoid_: master, primary, active

**Toggle**:
The session-scoped on/off switch for a single mpv window.
_Avoid_: enable flag, pause
