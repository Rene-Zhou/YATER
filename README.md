# YATER

YATER (Yet Another Terminal Epub Reader) is a terminal-native EPUB reader written in Rust. It focuses on CJK-friendly sentence navigation, a minimal framed reading UI, inline terminal images, a Vim-style table of contents, and footnote/endnote overlays.

## Features

- EPUB-only reader with no GUI window.
- CJK-aware sentence segmentation, including quoted dialogue cases.
- Typewriter-style reading: the active sentence stays near the vertical center.
- Violet text-only focus highlight (`#a97df4`) without reverse video or bold.
- TOC sidebar opened with `Tab`, rendered inside the same reader frame.
- Footnote/endnote extraction from EPUB semantics, DPUB-ARIA, and EPUB2-style reciprocal links.
- Inline image support through Sixel, Kitty, iTerm2, halfblock fallback, or explicit off mode.
- Debounced progress persistence under `$XDG_DATA_HOME/yater/progress.json`.
- Non-fatal parsing/image issues logged under `$XDG_STATE_HOME/yater/yater.log`.

## Install

Build from source:

```bash
cargo build --release
```

The binary will be at:

```bash
target/release/yater
```

For development on Fedora with the distro Rust packages, install the formatter with:

```bash
sudo dnf install rustfmt
```

If system package installation is unavailable, install user-level Rust tooling:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
~/.cargo/bin/rustup component add rustfmt
```

## Usage

```bash
yater <file.epub> [--image-mode=sixel|halfblock|off]
```

Examples:

```bash
cargo run -- ~/books/book.epub
cargo run -- ~/books/book.epub --image-mode=halfblock
cargo run -- ~/books/book.epub --image-mode=off
```

When `--image-mode` is omitted, YATER auto-detects terminal graphics capability and may select Sixel, Kitty, iTerm2, halfblock, or off depending on support. Manual override currently accepts `sixel`, `halfblock`, or `off`.

## Keys

### Reading

| Key | Action |
| --- | --- |
| `j` / `Down` | Next sentence |
| `k` / `Up` | Previous sentence |
| `u` | Fast previous sentence |
| `n` | Fast next sentence |
| `h` | Previous paragraph/block |
| `l` | Next paragraph/block |
| `i` | Start of current chapter |
| `m` | End of current chapter |
| `;` | Open/cycle annotations for the current sentence |
| `Tab` | Open TOC |
| `q` | Quit |

### TOC

| Key | Action |
| --- | --- |
| `j` / `Down` | Next visible TOC row |
| `k` / `Up` | Previous visible TOC row |
| `l` / `Enter` | Expand collapsed row or jump to selected target |
| `h` | Collapse row or move to parent |
| `Tab` / `Esc` | Close TOC |

### Annotation

| Mode | Key | Action |
| --- | --- | --- |
| Compact overlay | `;` | Cycle annotations |
| Compact overlay | `Enter` | Enter immersed annotation view |
| Compact overlay | any other key | Close overlay |
| Immersed view | `j` / `Down` | Scroll annotation down |
| Immersed view | `k` / `Up` | Scroll annotation up |
| Immersed view | `Esc` | Return to compact overlay |

## Development

Run the main checks:

```bash
cargo fmt --check
cargo test --locked
cargo check --locked
cargo build --locked
git diff --check
```

The project uses a test-first workflow. Useful seams include:

- `src/sentence.rs` for sentence segmentation.
- `src/input.rs` for key mapping.
- `src/app.rs` for focus and navigation state.
- `src/render.rs` and `tests/runtime_ui.rs` for terminal UI snapshots.
- `src/epub.rs` and `tests/annotation_runtime.rs` for EPUB parsing and annotation behavior.

## Documentation

- [CONTEXT.md](CONTEXT.md) documents the current domain model and runtime behavior.
- [docs/PRD_v1.md](docs/PRD_v1.md) describes the v1 product scope.
- [docs/basic-epub-formatting-plan.md](docs/basic-epub-formatting-plan.md) defines the approved requirements and development plan for basic EPUB formatting in the TUI.
- [docs/adr/0001-flat-block-list.md](docs/adr/0001-flat-block-list.md) records the flat block list data model decision.

## Scope

YATER intentionally does not support PDF/MOBI, TTS, search, bookmarks, themes, mouse interaction, or multiple open books in v1.
