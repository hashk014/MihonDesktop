# Contributing

Thanks for taking a look. Issues and pull requests are welcome.

## Getting set up

You need a Rust toolchain. On Windows the MSVC target is expected — the GNU
target's bundled mingw lacks the assembler that `windows-sys` needs.

```
cargo run            # debug build
cargo test           # unit tests, no network
cargo test -- --ignored   # live tests against real sources (network required)
```

Before opening a pull request:

```
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs exactly those three on Windows.

## Adding a source

Most sites can be described declaratively — no Rust, no rebuild. Write a JSON
manifest and drop it into the app's `extensions/` folder (More → Extensions →
Install from file). See [`examples/extensions/mangadex-scripted-en.json`](examples/extensions/mangadex-scripted-en.json)
for a complete, working example and the README for the full field reference.

A site needs a native module in `src/source/` only when its catalogue cannot be
expressed as fixed selectors — for instance when building the chapter list means
chaining several requests or interpreting the page's own JavaScript.
`src/source/animesama.rs` is the worked example.

Whichever route you take, please add tests:

- parsing tests against a recorded snippet of the real HTML or JSON, so the
  logic is pinned without needing the network;
- an `#[ignore]`d live test that walks listing → details → chapters → pages and
  actually downloads and decodes one image.

The live tests are the ones that catch the interesting bugs. Two real examples
from this codebase: a chapter list silently truncated because a title was
trimmed and the site's API keys on it byte for byte, and webtoon strips being
crushed to a quarter of their width by a texture-size cap.

## Style

- Comments explain *why*, not *what*. If something looks odd, say what forced it.
- Prefer fixing a cause over guarding a symptom.
- Keep behaviour close to Mihon's where it exists; where this project diverges,
  say so in the README's "Differences from Mihon" section.

## Scope

This is a reader. Sources fetch what a browser would; nothing here circumvents
paywalls or DRM. Please do not send patches that do.
