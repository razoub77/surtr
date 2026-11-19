# Install and update with a one-line PowerShell script that owns `surtr.conf`'s shape

The spec previously ruled installers out, leaving users to extract `surtr.zip` into the right mpv folder by hand and to merge it over their files on every update - the step where a stranger gets it wrong. Copy-paste of the file tree stays possible either way, so the cheapest fix that fully solves "one click, works on any machine, keeps my settings" is a PowerShell script fetched from the latest GitHub Release: opening it asks for **Install / Update** or **Uninstall** with arrow keys, it finds mpv's configuration folders (`MPV_HOME`, `%APPDATA%\mpv`, `portable_config`, Scoop) and lets the user tick which ones to work on, it overwrites plugin files, and it regenerates `script-opts/surtr.conf` from the shipped template, carrying over the value of every key the template still has. A `surtr.zip` beside the script is used instead of the release download, which makes an offline install of the two downloaded files possible without a second code path.

The alternative - merging the shipped conf into the user's in place - cannot refresh comments (they would keep describing deleted keys, which is exactly how `hover_title_lang` would linger) nor drop keys the new version no longer reads. Regenerating makes the shipped template the single source of the conf's shape, and makes "your values survive" a property of one pure function rather than of how careful the installer is.

## Consequences

- The Installer, not mpv, decides which mpv Configuration Folders Surtr lives in; a machine with several offers them for ticking, and no folder found is a prompt for a path, never a guess. Opening the script offers install/update or uninstall; without a console that can read keys, the same choices become numbered prompts.
- Deleting a key from the shipped `surtr.conf` is a breaking change to every user's file: their value for it disappears on the next update. Bump deliberately.
- Plugin files are replaced unconditionally, so there is no version check anywhere and no release tag to read; `releases/latest/download/` is the whole update mechanism.
- A locked `surtr.exe` (mpv running) prompts the user to close mpv and retry rather than being killed, so a running session is never silently broken.
- `scripts/install.ps1` and `surtr.zip` must be published together on each release, or the one-liner installs a version whose conf does not match its template.
- The installer writes nothing outside mpv's configuration folder and `%LOCALAPPDATA%\surtr`. An existing override file is never overwritten; an empty `script-opts/surtr-overrides.json` is created only when the user has no override file in either place. `cache.json` and `owner.json` are never touched unasked. Uninstall asks before deleting that data folder, and Enter keeps it; it leaves the user's override file in `script-opts` alone. ADR 0004 moved Overrides out of the data folder; an explicit user `overrides_path` can point the daemon's one-time migration outside those two folders, which is the deliberate exception to this boundary.
- Offline install works when the two release files are in one folder: a `surtr.zip` beside the script wins over the network. A bad local zip hard-fails, naming the missing files; no silent fallback to a download, because that would break the offline promise.

## Considered options

- **One-line `irm | iex` PowerShell script** - chosen: no download step, no SmartScreen prompt of its own, and `-Uninstall` reuses the same URL.
- **A double-clickable `install.cmd` inside the zip** - still two clicks and a manual download, and Explorer runs `.cmd` from a zip badly.
- **A real installer `.exe` (Inno Setup) on the release** - literally one click, but a new toolchain and an unsigned binary that trips SmartScreen.
- **Merge the shipped conf into the user's file in place** - keeps stale comments and dead keys forever; rejected in favour of regeneration.
