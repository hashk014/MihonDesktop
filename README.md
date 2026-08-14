# Mihon Desktop

[![CI](https://github.com/hashk014/MihonDesktop/actions/workflows/ci.yml/badge.svg)](https://github.com/hashk014/MihonDesktop/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

A desktop manga reader written in Rust, rebuilding [Mihon](https://mihon.app/)'s
feature set and information architecture for a resizable window instead of a
phone: a categorised library, chapter updates, reading history, source browsing
with extensions, a paged/webtoon reader, a download queue and backups.

Everything is native Rust — no browser engine, no JVM, no bundled runtime. The
release build is a single 17 MB executable.

```
cargo run --release
```

Prebuilt Windows binaries are attached to each
[release](https://github.com/hashk014/MihonDesktop/releases); what changed in
each one is in [CHANGELOG.md](CHANGELOG.md).

## What it does

**Library** — grid (compact, comfortable, cover-only) or list, category tabs,
unread / downloaded / local / language badges, nine sort orders in both
directions, six tri-state filters, multi-select with bulk mark-read, download,
category assignment and removal, and a hover "continue reading" button.

**Updates** — recently fetched chapters of favourited entries, grouped by day,
with per-row download and read toggles, and a global library update that honours
the configured restrictions (skip completed, skip entries with unread chapters,
category scope).

**History** — recently read chapters with resume, grouped by day, searchable,
individually removable.

**Browse** — sources grouped by language with pinning, global search across
pinned sources, per-source Popular / Latest / Search with the source's own
filter list, and migration of a library entry from one source to another
(carrying categories and read progress).

**Manga details** — cover, metadata, expandable description, clickable genre
chips, private notes, chapter list with per-manga sort and tri-state filters,
multi-select bulk actions including "mark previous read", and per-chapter
download state.

**Reader** — left-to-right, right-to-left, vertical, webtoon, continuous
vertical, and an infinite-scroll mode that appends the next chapter as you
reach the end, so a series reads as one uninterrupted scroll (with a labelled
divider at each chapter, a chapter-local page counter, and chapters marked read
as you pass them); fit-screen / fit-width / fit-height / original / stretch / smart
scaling; zoom and pan; optional double pages with spread detection; border
cropping; page preloading; keyboard and click-zone navigation; chapter
transitions; progress and history recording (skipped in incognito mode).

**Downloads** — bounded-concurrency queue with pause/resume, reordering,
progress, retry with mirror fallback, and chapters stored as CBZ archives that
open in any comic reader.

**More** — download queue, categories, extensions, statistics, and a full
settings tree (appearance, library, reader, downloads, browse, data & storage,
about), plus "downloaded only" and "incognito" switches.

**Data** — gzipped-JSON backup and restore of the whole library and settings.

## Sources

Mihon's extensions are Android APKs containing compiled Kotlin, which a native
desktop binary cannot load. Content therefore comes from three places:

1. **MangaDex**, built in, registered once per translated language.
2. **AnimeSama** (French), built in — a native port of the Kotlin extension.
   Its chapter list is assembled by following several requests and replaying
   the page's own `panneauScan` / `creerListe` / `newSP` JavaScript, which no
   declarative manifest can express; sources shaped like this need a native
   module in `src/source/`.
3. **Local source** — your own files, laid out like Mihon's local source:

   ```
   local/
     Series Name/
       cover.jpg          (optional; otherwise the first page is used)
       details.json       (optional: title, author, artist, description, genre, status)
       Chapter 1.cbz      (or .zip, or a folder of images)
       Chapter 2/
         01.jpg
   ```

4. **Scripted extensions** — JSON files describing a site's endpoints and where
   each value sits in the response. Drop one into the `extensions/` folder (More
   → Extensions → Install from file, or add a repository URL) and it is loaded at
   startup without recompiling.

### Writing an extension

A manifest declares the requests and, for each field, a CSS selector (HTML) or a
JSON pointer (JSON), plus optional post-processing. See
[`examples/extensions/mangadex-scripted-en.json`](examples/extensions/mangadex-scripted-en.json)
— a complete, working reimplementation of the built-in MangaDex source as an
extension, covered by tests.

```jsonc
{
  "id": "example-en",
  "name": "Example",
  "lang": "en",
  "baseUrl": "https://example.com",
  "pageSize": 24,
  "rateLimit": { "permits": 2, "periodMs": 1000 },

  "popular": {
    "url": "{base}/popular?page={page}",
    "list": {
      "item": "div.manga-card",
      "title": "h3 a",
      "url": { "selector": "h3 a", "attr": "href" },
      "thumbnail": { "selector": "img", "attr": "data-src" }
    },
    "nextPage": "a.next-page"
  },

  "details": { "url": "{base}{url}", "fields": { "title": "h1.title" } },
  "chapters": {
    "url": "{base}{url}",
    "list": {
      "item": "li.chapter",
      "name": "a",
      "url": { "selector": "a", "attr": "href" },
      "date": { "selector": "span.date", "dateFormat": "%b %d, %Y" }
    }
  },
  "pages": { "url": "{base}{url}", "list": { "item": "div.reader img" } }
}
```

URL placeholders: `{base}`, `{page}`, `{page0}`, `{offset}`, `{limit}`,
`{query}`, `{url}`, `{id}`, `{filters}`.

Field options:

| Key | Meaning |
| --- | --- |
| `selector` | CSS selector, or JSON pointer when `"json": true` |
| `attr` | `text` (default), `html`, or an attribute name |
| `all` | collect every match (genre lists) |
| `regex` | `{ "pattern": …, "replace": … }`; without `replace`, capture group 1 |
| `prefix` / `suffix` | wrap the value |
| `map` | literal remapping, e.g. status labels |
| `default` | value when nothing matched |
| `dateFormat` | `chrono` format; ISO-8601, epochs and "3 days ago" are understood anyway |
| `find` / `equals` / `then` | JSON: pick the array element whose field matches, then read inside it |
| `format` / `parts` | build a value from several extractions (`{0}`, `{1}`, …) |
| `fromRoot` | resolve this part against the document root, not the current item |
| `alternatives` | specs tried in order when the primary yields nothing |
| `firstValue` | JSON: take an object's first value (language-keyed fields) |
| `skipIf` | on a list: drop items where this resolves (e.g. externally hosted chapters) |

## Building

Requires a Rust toolchain. On Windows, the MSVC target is expected — the GNU
target's bundled mingw lacks the assembler `dlltool` needs for `windows-sys`.

```
cargo build --release      # binary in target/release/
cargo test                 # unit tests, no network
cargo test -- --ignored    # live tests against MangaDex (network required)
```

The live tests exercise the real paths end to end: browsing, details, chapter
lists, page lists, image download and decode, the scripted extension against the
same API, and a full chapter download into a CBZ that is then read back.

Non-Latin titles need a system CJK font; the app picks up the usual Windows,
macOS and Linux ones automatically and logs which scripts it covered.

### Screenshots

The UI can screenshot itself, which is how it is verified:

```
MIHON_SCREENSHOT=out.png MIHON_SCREENSHOT_VIEW=library MIHON_SCREENSHOT_DELAY=8 mihon-desktop
```

`MIHON_SCREENSHOT_VIEW` accepts `library`, `updates`, `history`, `browse`,
`more`, `extensions`, `downloads`, `settings`, `statistics`, `source`, `manga`,
`reader`, `seed` (fills the library from a source first) and `glyphs` (a font
coverage probe).

## Where things live

| Path | Contents |
| --- | --- |
| `src/model.rs` | Domain types, mirroring Mihon's data classes and bit flags |
| `src/db.rs` | Embedded store (redb) with an in-memory cache and aggregates |
| `src/source/` | `Source` trait, MangaDex, local source, scripted extensions |
| `src/download.rs` | Download queue, workers, CBZ writing |
| `src/core.rs` | Services and every background task |
| `src/ui/` | Screens, theme, widgets, reader, screenshot harness |

Application data (database, covers, downloads, extensions, backups) lives under
the platform data directory — the exact paths are listed in Settings → About.

## Differences from Mihon

- Android extensions cannot be loaded: they are compiled Android apps. Adding
  a repository index from a Mihon extension repo is detected and refused with
  an explanation. A source is ported either as a scripted manifest (most sites)
  or as a native module (sites whose catalogue needs real control flow).
- Tracking services (MyAnimeList, AniList, …) are not connected. The data model
  carries tracks and the library can filter on them, but nothing syncs.
- Storage is redb with JSON records rather than SQLite/SQLDelight, and backups
  are gzipped JSON rather than protobuf `.tachibk`.

## Contributing

Issues and pull requests are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md).
Adding a source usually means writing a JSON manifest, not Rust.

## Licence

Apache-2.0, the same licence as the projects this one follows. See [LICENSE](LICENSE)
and [NOTICE](NOTICE) for attribution: the app is an independent reimplementation
of Mihon (no shared code), and the built-in AnimeSama source is a port of the
Kotlin extension from keiyoushi. Neither project is affiliated with this one.

This is a reader. It fetches what a browser would; it circumvents no paywall
and no DRM, and it ships no content.
