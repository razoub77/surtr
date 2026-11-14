Surtr: a Windows Rust daemon that observes mpv over JSON IPC and publishes anime Rich Presence to Discord. A small Lua launcher (`scripts/surtr/main.lua`) spawns the daemon per mpv window; per ADR 0001 all logic lives in Rust, Lua only launches and binds.

## Agent skills

### Issue tracker

Issues and specs live in GitHub Issues at `razoub77/surtr` (uses the `gh` CLI). See `docs/agents/issue-tracker.md`.

### Domain docs

Single-context: one `GLOSSARY.md` plus `docs/adr/` at the repo root. See `docs/agents/domain.md`. Read `GLOSSARY.md` and relevant ADRs before working in an area, and use the glossary's terms (e.g. Match, Presence, Anime Root — not "lookup", "status", "watch folder").

## Build, test, verify

- `cargo test` — the whole suite is offline; tests never touch the network or a live mpv/Discord.
- Single test file / test: `cargo test --test corpus`, or `cargo test --test corpus golden_corpus_matches_the_recorded_entries`.
- Package: `powershell -File scripts/package.ps1` (builds `target/release/surtr.exe`, writes `dist/surtr.zip`); verify with `powershell -File scripts/verify-package.ps1 -Zip dist/surtr.zip` (needs `lua` on PATH for launcher checks, or pass `-SkipLuaRun`).
- Installer tests: `powershell -File tests/install.tests.ps1` (dash-free, offline: dot-sources `scripts/install.ps1 -SelfTest` and checks `Merge-SurtrConf`). `scripts/install.ps1` is what users run and what the release workflow publishes.

## Gotchas

- The binary is `#![windows_subsystem = "windows"]` (`src/main.rs`): a release build has **no console output**. Debug with `surtr --dry-run` (prints activity payloads to stdout); normal diagnostics reach mpv's console over IPC. verify-package.ps1 enforces the GUI subsystem in the packaged exe — don't remove the attribute.
- The mpv IPC pipe is per-process (`\\.\pipe\surtr-<pid>`) and mpv creates it asynchronously after options are set, so the daemon must retry connecting for a few seconds (ADR 0001).
- Match fixtures (`tests/fixtures/anilist-corpus.json`) are recorded AniList searches keyed by the matcher's normalized query; regenerate with `node tests/fixtures/generate.mjs` only when the recorded searches change. `anilist-relations.json` is hand-mapped, not generated. `tests/corpus.rs` is a golden corpus: changing the scorer or normalization can legitimately break it — check the expected ids before "fixing" a row.
- `scripts/verify-package.ps1` asserts exact user-facing contracts: the shipped `script-opts/surtr.conf` keys and defaults, and the exact eight `scripts/surtr/README.md` sections in order. Editing either requires updating that script's expectations too.
- Config arrives verbatim from mpv's script-opts: `anime_roots` is one semicolon-separated string split in Rust (`surtr::cli`), and the client id falls back to the `SURTR_CLIENT_ID` env var.
- `scripts/install.ps1` owns the *shape* of `script-opts/surtr.conf` on every install/update: it regenerates the file from the shipped template and only carries over values of keys the template still has. Deleting a key from the shipped conf therefore deletes it from every user's file on the next update — see ADR 0003.
- PowerShell 5.1 gotcha in the installer and its tests: `-split "`r?`n"` is a *regex* on a double-quoted string, so `?` is literal — use `-split '\r?\n'`.
- Never check exit codes with `cmd /c "... & echo %ERRORLEVEL%"`: cmd expands `%ERRORLEVEL%` when it parses the whole line, so it prints the *pre-existing* value and a failing script looks like it exited 0. Read `$LASTEXITCODE` instead (or `call echo %^ERRORLEVEL%`).
