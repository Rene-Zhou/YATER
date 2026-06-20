# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

YATER (Yet Another Terminal Epub Reader) is a terminal-native EPUB reader written in Rust using `ratatui` + `crossterm`. It provides Vim-style navigation, typewriter-style sentence highlighting, inline image rendering (Sixel, Kitty, iTerm2, or halfblock), a TOC sidebar, and floating footnote annotations.

The project has a working Rust implementation. Keep this file, `CONTEXT.md`, and `docs/PRD_v1.md` aligned when behavior changes.

## Development Method: TDD

All code must be written Test-First. Strict Red-Green-Refactor:

1. **Red** — write a failing test that defines the desired behavior
2. **Green** — write the minimum code to make the test pass
3. **Refactor** — clean up while keeping tests green

Never write production code without a failing test driving it. Use the five testing seams defined in the PRD (parser, segmenter, input handler, renderer, progress) as entry points. Pure functions (`sentence.rs`, `input.rs`) get thorough unit tests; modules with I/O (`epub/`, `render/`) get integration tests with the fixture EPUB.

## Build / Test / Run

```
cargo build
cargo test
cargo run -- <file.epub> [--image-mode=sixel|halfblock|off]
cargo fmt --check
```

Test fixture: `test-fixtures/DragonLance.epub` (gitignored, must be obtained separately).

## Architecture

Single Rust crate, synchronous main loop (no event bus). Modules:

| Module | Role |
|---|---|
| `main.rs` | Entry point, CLI (`clap`), terminal init/restore (`crossterm`) |
| `app.rs` | Main loop, Focus state machine, event dispatch |
| `epub/` | EPUB parsing: spine, TOC, images, annotations |
| `document/` | Data structures: Document, Block, AnnotationStore, ChapterRange |
| `render/` | ratatui drawing: content, TOC sidebar, annotation overlay, top bar |
| `input.rs` | Key mapping per Focus variant → Actions |
| `sentence.rs` | CJK-aware sentence segmentation (pure function) |
| `image.rs` | Terminal image rendering via `ratatui-image` |

## Data Model

**Flat block list** (not a tree — see `docs/adr/0001-flat-block-list.md`):

- `Document` = `Vec<Block>` in spine order; each Block carries a chapter index
- `Block` = `TextBlock(plain_text, Vec<AnnotationRef>)` | `ImageBlock(image_data)`
- `AnnotationStore` = `HashMap<String, String>` (ID → plain text)
- `ChapterRange` = `Vec<(start_block, end_block)>` computed at parse time
- `TOC` = `Vec<TocNode>` tree (title, target block index, children)

## Focus State Machine

Four states: `Content`, `Toc`, `AnnotationOverlay`, `AnnotationImmersed`.

- `Tab`: Content ↔ Toc
- `;`: Content → AnnotationOverlay
- `Enter` (in overlay): AnnotationOverlay → AnnotationImmersed
- `Esc` / `;` (in overlay): pops back one level

## Keymap

**Content**: `j`/`k` sentence nav, `h`/`l` paragraph nav, `u`/`n` fast sentence nav, `i`/`m` chapter start/end, `;` annotation, `Tab` TOC, `q` quit.
**TOC**: `j`/`k` move, `l`/`Enter` expand/jump, `h` collapse/parent, `Tab`/`Esc` close.
**AnnotationOverlay**: `;` cycle, `Enter` immerse, other key close.
**AnnotationImmersed**: `j`/`k` scroll, `Esc` exit.

## Key Design Decisions

- **Eager full-parse**: entire EPUB walked at open, no lazy loading
- **Sentence segmentation** is rendering-time only, not persisted. CJK boundaries: `。`, `？`, `！`, `……`, with special handling for quoted dialogue. English: `.`, `!`, `?`
- **Image rendering**: auto-detect via `ratatui-image` Picker (Sixel → Kitty → iTerm2 → halfblock fallback)
- **Progress**: `$XDG_DATA_HOME/yater/progress.json`, keyed by file path, debounced auto-save
- **Errors**: startup → stderr + exit 1; runtime panic → catch, restore terminal, print; non-fatal → log to `$XDG_STATE_HOME/yater/yater.log` + placeholder in UI
- **Reader UI**: one outer frame with chapter title and focus-specific footer; TOC is a left sidebar inside that frame; compact annotations are the only floating inner window.

## Testing Seams

1. **Parser**: `epub_file_path → Document + AnnotationStore + Vec<TocNode> + Vec<ChapterRange>`
2. **Segmenter**: `&str → Vec<(start, end)>` — pure function, unit test with CJK and English
3. **Input handler**: `(Focus, KeyEvent) → Action` — pure mapping
4. **Renderer**: `(AppState) → ratatui::Frame`
5. **Progress**: `save(Progress)` / `load(path) → Option<Progress>` — round-trip

## Documentation

- `CONTEXT.md` — domain glossary and definitions
- `docs/PRD_v1.md` — full PRD with user stories, implementation decisions, keymaps, tech stack
- `docs/adr/0001-flat-block-list.md` — ADR for flat block list data model
