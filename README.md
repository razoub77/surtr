# Surtr

Anime Discord Rich Presence for [mpv](https://mpv.io), on Windows.

Surtr watches what mpv plays, matches the file against
[AniList](https://anilist.co), and publishes a Discord activity with the
canonical title, the official cover art, correct season-relative episode
progress, and a live progress bar. Files outside your configured anime folders
produce nothing at all, and one keybind hides the card instantly.

## Install

In PowerShell:

```powershell
irm https://github.com/razoub77/surtr/releases/latest/download/install.ps1 | iex
```

Opening it shows two choices, **Install / Update** and **Uninstall**: use
`Up`/`Down` and `Enter` (or type `1`/`2`). The installer finds mpv's
configuration folders, lets you tick which ones to install into or clean, and
keeps your settings when you run it again to update. If `surtr.zip` sits beside
`install.ps1`, it installs from that file with no network access; otherwise it
downloads the latest release. Uninstall asks before deleting `%LOCALAPPDATA%\surtr`
and keeps it by default.

Point Surtr at your anime folders in `script-opts/surtr.conf`:

```ini
anime_roots=D:\Anime;E:\Anime\Seasonal
```

Manual Overrides go beside it in `script-opts/surtr-overrides.json`; the full
guide explains the format and the optional `overrides_path` key.

## Requirements

- Windows 11, 64-bit
- mpv 0.36 or newer
- The Discord desktop app, signed in, with
  **User Settings → Activity Privacy → Share your detected activities with others** on

## Documentation

The full user guide — configuration, verification steps, troubleshooting,
known limits and overrides — ships inside the plugin as
[`scripts/surtr/README.md`](scripts/surtr/README.md), and is installed next to
it so it travels with the release.

## Building

```powershell
cargo test                                    # offline test suite
powershell -File scripts/package.ps1          # -> dist/surtr.zip
powershell -File scripts/verify-package.ps1 -Zip dist/surtr.zip
powershell -File tests/install.tests.ps1      # installer conf-sync tests
```

Releases are cut by pushing a `v*` tag; the workflow builds, verifies and
attaches `surtr.zip` and `install.ps1` to the release.

## License

MIT — see [LICENSE](LICENSE).
