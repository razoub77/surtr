# Overrides live beside `surtr.conf`; internal state stays in `%LOCALAPPDATA%`

ADR 0003 made one config folder tree the install unit, but Overrides still
lived in `%LOCALAPPDATA%\surtr`, so users had to look in two places for the
settings they edit. Putting them in `scripts/surtr` is worse: the installer
overwrites that folder on every update. The single contract is now that
everything a user edits lives in mpv's configuration folder, while files the
daemon owns (`cache.json`, `owner.json`) stay in `%LOCALAPPDATA%\surtr`.

The default file is `script-opts/surtr-overrides.json`; `surtr.conf` ships an
explicit `overrides_path=~~/script-opts/surtr-overrides.json`, and a blank value
means the same default. A custom value must be absolute or use mpv's `~~` /
`~~exe_dir`, validated and expanded by the Lua launcher like `binary_path`.
A bad path disables Overrides only, not presence.

## Consequences

- The installer creates an empty override file only when the user has neither
  the new file nor the legacy `%LOCALAPPDATA%\surtr\overrides.json`. Uninstall
  leaves the override file: it holds user assertions, not plugin files.
- The daemon moves the legacy file once at startup. An absent target gets an
  exact byte copy; both present means a merge under normalized keys with the
  target winning conflicts. The legacy file is deleted only after a successful
  write. Invalid JSON, an invalid target, or a write failure keeps both files
  with one console line; a failed move still reads the legacy file that run.
- Multiple mpv Configuration Folders each get their own override file unless
  their confs point at a shared one.
- ADR 0003's consequence about Overrides in the data folder is superseded; the
  "one-line installer regenerates the conf" decision is unchanged.

## Considered options

- **Keep Overrides in `%LOCALAPPDATA%`** — no migration, but user-facing config
  stays split from `surtr.conf`, which is the problem being solved.
- **Put Overrides in `scripts/surtr`** — one folder, but updates overwrite it;
  losing user assertions by accident is worse than leaving a cache behind.
- **A path option with no migration** — simpler, but silently loses existing
  Overrides on upgrade.
