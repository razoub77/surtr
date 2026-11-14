# Install and update with a one-line PowerShell script that owns `surtr.conf`'s shape

The spec previously ruled installers out, leaving users to extract `surtr.zip` into the right mpv folder by hand and to merge it over their files on every update - the step where a stranger gets it wrong. Copy-paste of the file tree stays possible either way, so the cheapest fix that fully solves "one click, works on any machine, keeps my settings" is a PowerShell script fetched from the latest GitHub Release: it finds mpv's configuration folder (`MPV_HOME`, `%APPDATA%\mpv`, `portable_config`, Scoop), overwrites the plugin files, and regenerates `script-opts/surtr.conf` from the shipped template, carrying over the value of every key the template still has.

The alternative - merging the shipped conf into the user's in place - cannot refresh comments (they would keep describing deleted keys, which is exactly how `hover_title_lang` would linger) nor drop keys the new version no longer reads. Regenerating makes the shipped template the single source of the conf's shape, and makes "your values survive" a property of one pure function rather than of how careful the installer is.

## Consequences

- The installer, not mpv, is what decides which configuration folder Surtr lives in; a machine with several is a numbered prompt, and no folder found is a prompt for a path, never a guess.
- Deleting a key from the shipped `surtr.conf` is a breaking change to every user's file: their value for it disappears on the next update. Bump deliberately.
- Plugins files are replaced unconditionally, so there is no version check anywhere and no release tag to read; `releases/latest/download/` is the whole update mechanism.
- A locked `surtr.exe` (mpv running) prompts the user to close mpv and retry rather than being killed, so a running session is never silently broken.
- `scripts/install.ps1` and `surtr.zip` must be published together on each release, or the one-liner installs a version whose conf does not match its template.
- Nothing outside mpv's configuration folder and `%LOCALAPPDATA%\surtr` is written; `overrides.json`, `cache.json` and `owner.json` are never touched.

## Considered options

- **One-line `irm | iex` PowerShell script** - chosen: no download step, no SmartScreen prompt of its own, and `-Uninstall` reuses the same URL.
- **A double-clickable `install.cmd` inside the zip** - still two clicks and a manual download, and Explorer runs `.cmd` from a zip badly.
- **A real installer `.exe` (Inno Setup) on the release** - literally one click, but a new toolchain and an unsigned binary that trips SmartScreen.
- **Merge the shipped conf into the user's file in place** - keeps stale comments and dead keys forever; rejected in favour of regeneration.
