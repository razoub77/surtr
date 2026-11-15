# Changelog

## [0.1.2] — 2026-11-15

### Changed

- Discord cover art now routes through the free WordPress Photon image proxy
  (`https://i0.wp.com/<target-without-scheme>?resize=1024,1024`) instead of
  linking the raw AniList CDN URL directly. Photon resizes and center-crops
  covers on the fly and does not block `anilist.co`.

[0.1.2]: https://github.com/razoub77/surtr/compare/v0.1.1...v0.1.2
