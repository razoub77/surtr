# Surtr

Surtr shows what you are watching on your Discord profile, from mpv: the anime's
title and cover art, `▶️ Playing • Episode 3/12`, and a live progress bar — for
local files under the folders you name, and silence for everything else.

## 1. What it is

Surtr is a small plugin for [mpv](https://mpv.io) on Windows. When you play an
episode of anime from one of your folders, mpv tells Surtr which file that is;
Surtr looks the anime up on [AniList](https://anilist.co) and publishes a
"Watching" card to the Discord app on the same PC. It never changes anything in
your Discord account or your AniList lists; it only displays the card, the same
way the desktop app does when you listen to Spotify.

Before you start, you need:

- Windows 11, 64-bit.
- mpv 0.36 or newer.
- The Discord **desktop app** running and signed in.
- In Discord, **User Settings → Activity Privacy → Share your detected
  activities with others** turned on. With it off nothing appears, which is the
  most common surprise.

Surtr installs with one command. There is no installer app, no service, and no
Python.

## 2. Install

Run this in PowerShell (press `Win`, type "PowerShell", Enter, then paste this
and press Enter):

```powershell
irm https://github.com/razoub77/surtr/releases/latest/download/install.ps1 | iex
```

First it asks what to do: **Install / Update** or **Uninstall**. Move with
`Up`/`Down`, choose with `Enter`; you can also type `1` or `2`. It finds mpv's
configuration folders for you - `%APPDATA%\mpv` (a stock Windows install),
`portable_config` next to `mpv.exe` (a portable mpv), `MPV_HOME`, or Scoop's
folder. Tick the folders to install into with `Space`, then press `Enter`. If it
finds none, it asks you to paste a folder's path. If `surtr.zip` sits beside
`install.ps1`, it uses that file and never touches the network; otherwise it
downloads the latest release. It installs Surtr into each chosen folder:

```
scripts/surtr/main.lua
scripts/surtr/surtr.exe
scripts/surtr/README.md        (this file)
script-opts/surtr.conf         (created on first install, synced on update)
script-opts/surtr-overrides.json (created empty when you have no override file)
```

Do not rename or move the files out of `scripts/surtr`, and do not put this
file in the configuration root.

The first time mpv starts `surtr.exe`, Windows SmartScreen may warn that it is
an unrecognised app. Choose **More info → Run anyway**. Surtr's executable is
not code-signed; it contains no installer, and nothing is written outside mpv's
configuration folder and `%LOCALAPPDATA%\surtr`.

Start Discord, then mpv, and fill in your folders as shown next.

## 3. Configure

Surtr is configured in `script-opts/surtr.conf`, next to the `scripts` folder.
Open it in Notepad and tell Surtr which folders hold your anime.

```ini
anime_roots=D:\Anime;E:\Anime\Seasonal
```

- Separate folders with semicolons. Subfolders count, so `D:\Anime` covers
  everything inside it.
- Files must live **beneath** a folder, not be the folder itself. Folder names
  are compared whole, so `D:\Anime2` is not inside `D:\Anime`.
- Paths are absolute. Use the drive-letter form above; network shares
  (`\\server\share`) are not supported.
- Every file that is not under one of these folders produces no presence at
  all — this is what keeps the rest of your viewing private.

The other keys:

- `client_id`: the Application ID of the Discord application Surtr publishes
  through. Surtr ships with one, so this can stay empty. Set it only if you
  want the presence to come from your own application: create one at
  [discord.com/developers/applications](https://discord.com/developers/applications)
  ("New Application"), copy its Application ID, and paste it here:

  ```ini
  client_id=1234567890123456789
  ```

  A value here wins over the `SURTR_CLIENT_ID` environment variable, which in
  turn wins over the built-in application id.

- `binary_path` stays empty: it means `surtr.exe` next to `main.lua`. Set it
  only if you moved the executable; see [Troubleshooting](#5-troubleshooting).
- `overrides_path` is `~~/script-opts/surtr-overrides.json`: the override file
  beside this conf. Change it only if you keep that file elsewhere; the value
  must be absolute or start with `~~` (mpv's config folder) or `~~exe_dir`
  (mpv.exe's folder). See [Overrides](#8-overrides).
- `toggle_key` is `ctrl+shift+a`: press it to hide or show presence in this mpv
  window. Any mpv key name works (`alt+p`, `F8`, ...). It is not saved across
  mpv restarts.
- `details_title_lang` picks which AniList title leads the card's main line:
  `english` (the default) or `romaji`. When an entry has no title in the chosen
  language, the other one stands in. Case-insensitive; a typo keeps the
  default. Hovering the cover art always shows the entry's metadata line
  (`TV • Finished • Fall 2023 • Adventure, Drama`), so it has no language
  setting.

- `pause_timeout` is `0`: a paused card stays until you resume or load another
  file. Set whole seconds (for example `pause_timeout=300`) to clear the card
  after that long a continuous pause. Seeking while paused does not restart
  the countdown, and resuming brings the card back. A bad value is ignored
  with a message on mpv's console.

The configuration file is read once when mpv starts. **Restart mpv after
editing it.**

Leaving `anime_roots` empty is allowed — mpv then treats every local file as
eligible — but it defeats the privacy gate, so Surtr warns about it on every
start until you fill it in.

## 4. Verify

With Discord running and an episode under one of your folders ready:

1. Start mpv and play the episode. Keep the mpv window focused.
2. Click your own name and avatar in Discord, or open your profile in another
   client. After a few seconds a **Watching** card appears with the cover art,
   the title, `▶️ Playing • Episode 3/12` and a progress bar. Hovering the
   cover art shows the entry's metadata line (`TV • Finished • Fall 2023 •
   Adventure, Drama, Fantasy`). Pausing turns the state line to
   `⏸️ Paused • Episode 3/12`; a one-episode entry reads the bare
   `Episode 1`, and a movie with no episode number shows the play/pause word
   alone while paused and no state line while playing.
3. Press `Ctrl+Shift+A`. The card disappears and mpv shows
   `Surtr: presence off`; press it again and the card comes back
   (`Surtr: presence on`).
4. Quit mpv. The card disappears.

That is the whole walkthrough. If nothing appears, open mpv's console with the
backtick key (`` ` ``) and look for lines beginning `[surtr]`; each loaded file
prints its full path there, and [Troubleshooting](#5-troubleshooting) lists the
usual causes for error lines. Every mpv start also prints one
`Surtr: watching for playback` line, so an empty console is itself a clue:
Surtr never loaded.

## 5. Troubleshooting

Open mpv's console with `` ` `` and read the `[surtr]` lines first; most problems
are named there.

- **No `[surtr]` line at all, not even `Surtr: watching for playback`.** The
  plugin never loaded from `scripts/surtr/`; run the install command again and
  restart mpv. If even normal mpv output is missing, mpv is running with quiet
  logging.
- **No card at all, and no `[surtr]` line after that startup line, or a line
  about Discord not accepting a connection.** Check that Discord is running
  and signed in, and its **Share your detected activities with others**
  setting. Restart Discord and mpv after changing the setting. Make sure the
  episode started playing (a paused first frame is fine). A refusal from
  Discord can also mean the application id changed server-side; if you have
  published through your own application, check its ID against `client_id`
  in `script-opts/surtr.conf` (then restart mpv).
- **`Surtr: no Anime match for "<title>"`.** The file is under a folder, but
  AniList could not be matched confidently. The title is the exact key to
  paste into `overrides.json`; see [Overrides](#8-overrides).
- **`surtr: skipping non-local path ...`.** The file is a URL or a stream, or
  is not under any `anime_roots` folder. Check the folder list and remember
  `D:\Anime2` is not inside `D:\Anime`.
- **`surtr: no anime_roots configured`.** `script-opts/surtr.conf` is missing,
  has no `anime_roots` line, or every entry was blank. Fill it in and restart
  mpv. Until then every local file is eligible.
- **`Surtr needs mpv 0.36 or newer (found ...)`.** Your mpv is too old. Update
  mpv; Surtr stops by design rather than half-working.
- **`surtr: surtr.exe not found at "..."`.** Surtr was installed without its
  `scripts/surtr` shape, or `surtr.exe` was moved or deleted. Run the install
  command again. If you keep `surtr.exe` elsewhere deliberately, set
  `binary_path` in `script-opts/surtr.conf` to an absolute path or an mpv
  meta-path such as `~~exe_dir/surtr/surtr.exe`; a relative path is refused.
- **`surtr: binary_path "..." is not absolute`.** Same as above: use
  `D:\...`, `~~\...` or `~~exe_dir\...`.
- **Card appears but is wrong or missing the episode line.** The file matched a
  Movie or an entry whose episode count AniList does not know; Surtr never
  guesses a total — an entry with no known count shows `Episode 3/?`, and a
  movie with no episode number shows no state line while playing. If the title
  itself is wrong, pin it with an override.
- **The card stays after you quit mpv, or a stale card from another app
  remains.** Discord occasionally holds the activity slot of a client that died
  badly; restarting Discord clears it.
- **Two mpv windows open.** The card follows the mpv window you last clicked,
  one window at a time. Clicking a browser keeps the last mpv card.

## 6. Updating / uninstalling

**Update.** Run the same install command again:

```powershell
irm https://github.com/razoub77/surtr/releases/latest/download/install.ps1 | iex
```

Close mpv first. The new plugin files replace the old ones, and
`surtr.conf` is rewritten from the new version's shipped file: every value you
set is kept, any key that no longer exists is dropped, any new key arrives at
its default, and the comments always match the version you just installed. Your
`surtr-overrides.json` and cache are never overwritten. An old
`%LOCALAPPDATA%\surtr\overrides.json` is moved or merged into the configured
file by the daemon on the next mpv start. If mpv is still open, the installer
asks you to close it and press Enter, then retries.

**Uninstall.** Run the same command and choose **Uninstall** in the menu. To
skip the menu, set `$env:SURTR_UNINSTALL = '1'` before the one-liner, or run
`powershell -File install.ps1 -Uninstall` when you have the script:

```powershell
$env:SURTR_UNINSTALL = '1'
irm https://github.com/razoub77/surtr/releases/latest/download/install.ps1 | iex
```

Tick the folders to clean with `Space`, press `Enter`, then choose whether to
also delete `%LOCALAPPDATA%\surtr` (the cache and the window-ownership file).
Pressing `Enter` keeps it; choose **Delete** to remove it. The uninstall removes
`scripts/surtr` (the plugin and its executable) and `script-opts/surtr.conf`
from the ticked folders. `script-opts/surtr-overrides.json` is yours, so it is
left in place; delete it by hand if you no longer want it. Nothing else is
installed: no service, no registry entries, no scheduled task. Discord forgets
the card when mpv exits.

## 7. Known Limits

- **Local files only.** URLs and streams (HTTP, torrent streaming, network
  shares) never produce a card.
- **Anime only, as AniList lists it.** Western animation, manga and live-action
  titles are not matched.
- **No OVAs, specials, or multi-episode files.** Batch ranges (`01-12`), bundled
  episodes and OVA entries are skipped.
- **Absolute numbering needs known season counts.** A show whose first season
  has no episode count on AniList (for example, ongoing weekday anime numbered
  past 1000) has no range to count from, so the file is skipped. An unmarked
  episode that lands in a later season with an unknown count still shows, but
  without a `of Y` denominator.
- **Season and title are guessed from the filename.** Renamed files or odd
  release names may not match; [Overrides](#8-overrides) fix the ones that
  matter. When the filename's title finds nothing on AniList at all, Surtr
  also tries the name of the folder the file is in, so a folder named exactly
  as AniList spells the title (a space in `Dai Seijo` where the release glued
  it into `Daiseijo`) matches without an override.
- **One Discord card per user, from the last-clicked mpv window.** Selecting
  another mpv window hands the card over; a browser or other app does not take
  it away.
- **Configuration is read at startup.** Editing `surtr.conf` or
  `surtr-overrides.json` takes effect after restarting mpv.
- **Network hiccups are silent.** If AniList, Discord or the network is
  unavailable, the card simply does not update; playback is never affected.
- **mpv 0.36 or newer, Windows 11 64-bit only.**

## 8. Overrides

When a file under your folders does not match, Surtr shows
`Surtr: no Anime match for "<title>"` for five seconds and prints the same line
in mpv's console. The text in quotes is the override key: the exact query Surtr
searched, for example `honzuki no gekokujou` or `youjo senki season 2`.

Decide which AniList entry the file belongs to — its id is the number in the
URL, `https://anilist.co/anime/108268` — then edit
`script-opts/surtr-overrides.json` (create the file if it does not exist). A
different `overrides_path` in `surtr.conf` points somewhere else:

```json
{
  "honzuki no gekokujou": 108268,
  "youjo senki season 2": 135865
}
```

- Keys are compared ignoring capitalisation and extra spaces, so the quoted
  title can be pasted as-is.
- Values are AniList ids as plain numbers, not strings.
- An entry Surtr can use short-circuits the search for that key: that title is
  shown from the id you named. A wrong id means no card for those files, and
  one `[surtr]` console line says so.
- The rest of the file still works if one line is malformed; a malformed file as
  a whole is ignored with one console line.
- Restart mpv after editing. The installer creates the file empty when you have
  none. An old `%LOCALAPPDATA%\surtr\overrides.json` is moved into it on the next
  mpv start, or merged with it when both exist; the current file wins a
  conflicting id. After that migration, Surtr leaves the file alone.
