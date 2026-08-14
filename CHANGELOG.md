# Changelog

All notable changes are recorded here. Versions follow [semantic versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] — 2026-08-14

First public build.

### Library and navigation
- Categorised library with compact, comfortable, cover-only and list layouts,
  nine sort orders in both directions, six tri-state filters, unread /
  downloaded / local / language badges, and multi-select bulk actions.
- Updates tab grouped by day, with a global library update that honours the
  configured restrictions.
- History tab with resume and per-entry removal.
- Browse: sources grouped by language with pinning, global search across pinned
  sources, and migration between sources carrying categories and read progress.
- Manga details with expandable description, clickable genres, private notes,
  and per-manga chapter sorting and filtering.

### Reader
- Left-to-right, right-to-left, vertical, webtoon, continuous vertical, and an
  infinite-scroll mode that appends the next chapter as you reach the end.
- Six scale modes, zoom and pan, optional double pages with spread detection,
  border cropping, page preloading, keyboard and click-zone navigation.
- Progress and history recording, skipped entirely in incognito mode.

### Sources
- MangaDex over the public API, registered once per translated language.
- AnimeSama (French), a native port of the Kotlin extension.
- Local source reading CBZ, ZIP and folders of images.
- Scripted extensions: JSON manifests describing endpoints and selectors,
  loaded at runtime with no recompilation. Supports HTML selectors and JSON
  pointers, regex post-processing, value composition across fields, fallbacks
  for missing translations, and item filtering.
- Mihon/Tachiyomi Android extension repositories are detected and refused with
  an explanation rather than a parse error.

### Downloads and data
- Bounded-concurrency queue with pause, reordering, retry and mirror fallback,
  storing chapters as CBZ archives readable by any comic viewer.
- Gzipped-JSON backup and restore of the library and settings.

### Platform
- System font fallbacks so Japanese, Korean, Chinese, Cyrillic and Greek titles
  render; the app reports which scripts it covered at startup.
- Panic handler writing a `crash.log` and showing a dialog, since a release
  build has no console.
- `MIHON_DATA_DIR` for a portable install or a second instance.
- A screenshot harness driven by environment variables, used to verify the
  interface without a human at the keyboard.

[Unreleased]: https://github.com/hashk014/MihonDesktop/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hashk014/MihonDesktop/releases/tag/v0.1.0
