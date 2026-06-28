# PRD: YATER — Yet Another Terminal Epub Reader

Reference project: [lue](https://github.com/superstarryeyes/lue) — the inspiration for YATER. Python-based terminal ebook reader with TTS. YATER draws from lue's modular architecture (separate parser, reader, input handler, UI modules) but is a Rust rewrite focused exclusively on EPUB.

## Problem Statement

Developers and terminal-dwellers want to read EPUB books without leaving the terminal. Existing solutions either require GUI windows (visible to others), support too many formats poorly, or lack proper CJK text support. There is no terminal EPUB reader that combines Vim-style navigation, inline image rendering, and floating annotations in a single, minimal package.

## Solution

YATER is a terminal-native EPUB reader built in Rust. It renders EPUB content directly in the terminal with sentence-level highlighting, inline image display (Sixel, Kitty, iTerm2, or halfblock fallback), a Vim-style TOC sidebar, and floating footnote annotations — all without opening a single GUI window. The UI is deliberately minimal: one outer reader frame contains the chapter title, reading content, focus-specific shortcut footer, TOC sidebar, and annotation views.

## User Stories

1. As a terminal user, I want to open an EPUB file with `yater book.epub`, so that I can start reading immediately without GUI windows
2. As a reader, I want sentence-level highlighting of the current sentence, so that I always know where I am in the text
3. As a CJK reader, I want sentence segmentation to follow standard Chinese rules (boundaries at `。`, `？`, `！`, `……` only), so that sentences are not incorrectly split at commas or semicolons
4. As a reader, I want to navigate sentences with `j`/`k`, so that I can read at my own pace with Vim-style keys
5. As a reader, I want to jump between paragraphs with `h`/`l`, so that I can skip through content quickly
6. As a reader, I want faster sentence navigation with `u`/`n`, so that I can move through long chapters efficiently while preserving the typewriter-style focused sentence
7. As a reader, I want to jump to the start/end of the current chapter with `i`/`m`, so that I can quickly re-read or skip ahead within a chapter
8. As a reader, I want to open a TOC sidebar with `Tab`, so that I can see the book's structure and navigate to any chapter
9. As a TOC user, I want Vim-style navigation (`j`/`k` to move, `l`/`Enter` to expand or jump, `h` to collapse), so that the TOC feels native to my workflow
10. As a TOC user, I want visual indent guides (`│`, `└`, `├`) and expand/collapse markers (`▸`/`▾`), so that I can see the tree structure at a glance
11. As a reader, I want inline images rendered directly in the terminal, so that I never need an external image viewer
12. As a reader on a modern terminal, I want high-fidelity Sixel image rendering, so that images look sharp
13. As a reader on a basic terminal (e.g., WSL), I want automatic fallback to halfblock character rendering, so that images are still visible
14. As a reader, I want to override image rendering with `--image-mode=sixel|halfblock|off`, so that I can control behavior for my specific terminal
15. As a reader, I want annotation markers (like `[1]`, `*`, `¹⁴`) preserved in the text, so that I know footnotes exist
16. As a reader, I want to press `;` to see a floating annotation overlay above the current sentence, so that I can read footnotes without losing my place
17. As a reader with multiple annotations in one sentence, I want `;` to cycle through them with a `[2/3]` counter, so that I can access all annotations
18. As a reader, I want to press `Enter` to immerse in a long annotation (scrolling with `j`/`k`), so that I can read lengthy footnotes comfortably
19. As a reader, I want `Esc` to close the annotation overlay and return to content, so that navigation stays simple
20. As a reader, I want my reading position saved automatically, so that I can close and reopen the book without losing my place
21. As a reader, I want progress restored when I open a previously-read book, so that I pick up exactly where I left off
22. As a reader, I want the top border to show the application and current chapter, so that I retain context without a heavy UI
23. As a reader, I want the reading area to use the available terminal width while keeping mode-specific controls in the frame footer, so that I get maximum reading space without losing key hints
24. As a user, I want startup errors (file not found, corrupted EPUB) shown clearly on stderr, so that I know what went wrong
25. As a user, I want runtime errors handled gracefully (terminal restored, error logged), so that a crash never leaves my terminal in a broken state
26. As a user, I want non-fatal issues (bad image, malformed HTML) logged to a file and shown as placeholders, so that reading continues uninterrupted
27. As a user, I want `--help` and `--version` flags, so that I can get basic info without reading docs
28. As a reader, I want the terminal to handle resize events smoothly, so that the layout adapts when I change window size
29. As a reader on a small terminal, I want a "Terminal too small" message, so that I know to resize before continuing
30. As a reader, I want semantic EPUB emphasis, headings, blockquotes, and lists rendered in the TUI, so that the book's basic structure remains readable without browser-level layout

## Implementation Decisions

### Module structure

Single Rust crate with the following top-level modules:

- `main.rs` — entry point, CLI arg parsing, terminal init/restore
- `app.rs` — main loop, Focus state machine, event dispatch
- `epub/` — EPUB file parsing (open, extract spine/TOC/images/annotations)
- `document/` — Document, Block, AnnotationStore, ChapterRange data structures
- `render/` — ratatui rendering (content view, TOC sidebar, annotation overlay, top bar)
- `input.rs` — key mapping per Focus variant, produces Actions
- `sentence.rs` — sentence segmentation (CJK-aware)
- `image.rs` — terminal image rendering (Sixel/halfblock via ratatui-image)

### Data model

- **Document**: flat `Vec<Block>` in spine order. Each Block knows its chapter index. See [ADR-0001](adr/0001-flat-block-list.md).
- **Block**: two variants — `TextBlock(normalized_text, Vec<TextStyleRange>, TextBlockPresentation, Vec<AnnotationRef>)` and `ImageBlock(image_data)`.
- **TextStyleRange**: a sorted UTF-8 byte range carrying the effective bold, italic, underline, and strikethrough modifiers. See [ADR-0002](adr/0002-flat-style-ranges.md).
- **TextBlockPresentation**: composable heading role, blockquote depth, and optional list-item marker/depth/continuation metadata.
- **AnnotationStore**: `HashMap<String, String>` mapping annotation IDs to pre-extracted plain text.
- **ChapterRange**: `Vec<(start_block, end_block)>` mapping chapter index to block range. Computed at parse time.
- **TOC**: `Vec<TocNode>` tree. Each node has title, target block index, and children.

### Parse strategy

Eager full-parse at file open. Walk spine order, convert each XHTML chapter to Blocks, extract annotations into AnnotationStore, compute ChapterRanges. No lazy loading.

### HTML-to-Block conversion

Walk the DOM depth-first. Each block-level element (`<p>`, `<h1>`–`<h6>`, `<div>`, `<figure>`, `<blockquote>`, `<li>`) produces one or more Blocks. Inline `<img>` splits a paragraph into TextBlock + ImageBlock + TextBlock. Semantic emphasis tags become flat style ranges; headings, blockquote depth, and nested ordered/unordered lists become block presentation metadata. Annotation links (`<a href="#id">`) are extracted as AnnotationRefs with byte offsets into the same normalized text. CSS is not parsed.

### Sentence segmentation

A rendering-time function, not a persisted data structure. Follows standard Chinese definition of 句子: boundaries only at `。`, `？`, `！`, and `……`; quoted Chinese dialogue keeps inner terminal punctuation together until the closing quote and may include a following attribution phrase. For English: `.`, `!`, `?`. Returns byte-offset ranges into the original text.

### Focus state machine

Four states: `Content`, `Toc`, `AnnotationOverlay`, `AnnotationImmersed`. Transitions:
- `Tab`: Content ↔ Toc
- `;`: Content → AnnotationOverlay
- `Enter` (in overlay): AnnotationOverlay → AnnotationImmersed
- `Esc`: pops back one level (`;` in overlay also closes)

### Keymap

**Content mode:**
| Key | Action |
|---|---|
| `j` / `↓` | Next sentence |
| `k` / `↑` | Previous sentence |
| `l` | Next paragraph (next Block) |
| `h` | Previous paragraph (previous Block) |
| `u` | Fast previous sentence |
| `n` | Fast next sentence |
| `i` | Jump to first sentence of current chapter |
| `m` | Jump to last sentence of current chapter |
| `;` | Toggle annotation overlay |
| `Tab` | Open TOC sidebar |
| `q` | Quit |

**TOC mode:**
| Key | Action |
|---|---|
| `j` / `↓` | Next item |
| `k` / `↑` | Previous item |
| `l` / `Enter` | Expand or jump to chapter |
| `h` | Collapse or go to parent |
| `Tab` / `Esc` | Close TOC, return to Content |

**AnnotationOverlay mode:**
| Key | Action |
|---|---|
| `;` | Cycle to next annotation |
| `Enter` | Enter AnnotationImmersed |
| Any other key | Close overlay, return to Content |

**AnnotationImmersed mode:**
| Key | Action |
|---|---|
| `j` / `↓` | Scroll annotation down |
| `k` / `↑` | Scroll annotation up |
| `Esc` | Exit immersion, return to Content |

### Screen layout

```
┌  YATER | Chapter Name ─────────────────────┐
│                                             │
│  Content area: text + images                │
│  Current sentence kept near center          │
│                                             │
└  READ j/k sentence | ... ──────────────────┘
```

TOC mode splits the framed content area into a left sidebar and a right reading context pane separated by a dark divider. Annotation immersion reuses the same outer frame. Compact annotations are the only inner floating window. No line numbers or progress percentage in v1.

### Image rendering

Auto-detect terminal graphics capability via `ratatui-image`'s `Picker` at startup (Sixel, Kitty, iTerm2, or halfblock fallback). Store chosen protocol in App state. Optional CLI flag `--image-mode=sixel|halfblock|off` for manual override.

### TOC rendering

Inspired by neo-tree.nvim. Each row assembled from indent guides (`│`, `└`, `├`) + expand/collapse markers (`▸`/`▾`) + title. `TocState` tracks expanded nodes (HashSet), selected row, and scroll offset. Component-composition pattern for row rendering.

### Annotation overlay

Floating window drawn on top of content area via ratatui's `Clear` + bordered `Paragraph`. Bottom edge sits above the current highlighted sentence. Multiple annotations cycle with `;`, counter shown as `[2/3]`. Short-to-medium notes wrap and expand up to a capped compact height; overflow triggers `Enter` to immerse. Drawn after content to render on top.

### Progress persistence

Save to `$XDG_DATA_HOME/yater/progress.json`. Keyed by file path. Stores block index, sentence offset, and timestamp. Auto-save on navigation (debounced). Restore on open if progress exists for the given EPUB.

### Error handling

- **Startup errors** (file not found, corrupted EPUB): print to stderr, exit code 1
- **Runtime panics**: catch at top of main loop, restore terminal (crossterm disable_raw_mode, show cursor), print error, exit
- **Non-fatal issues** (bad image, malformed HTML): log to `$XDG_STATE_HOME/yater/yater.log`, show placeholder, continue

### CLI

```
yater <file.epub> [--image-mode=sixel|halfblock|off]
```

One required positional arg (EPUB path), one optional flag. `--help` and `--version` supported. No subcommands, no config file in v1.

### Tech stack

| Component | Crate | Purpose |
|---|---|---|
| TUI framework | `ratatui` | Immediate-mode terminal UI rendering |
| Terminal backend | `crossterm` | Cross-platform terminal I/O, resize, keyboard |
| Image rendering | `ratatui-image` | Terminal-native image display (Sixel/halfblock) |
| Image decoding | `image` | Decode PNG/JPG/WebP from EPUB |
| EPUB parsing | `roxmltree` + `zip` | Extract spine, TOC, HTML, images |
| HTML extraction | `roxmltree` | XHTML to normalized text, semantic styles, block presentation, images, and annotation refs |
| Serialization | `serde` + `serde_json` | Progress persistence |
| CLI | `clap` | Argument parsing |

## Testing Decisions

### Testing seams

1. **EPUB parser seam**: `epub_file_path -> Document + AnnotationStore + Vec<TocNode> + Vec<ChapterRange>` — test with fixture EPUBs
2. **Sentence segmenter seam**: `&str -> Vec<(start, end)>` — pure function, unit test with CJK and English examples
3. **Input handler seam**: `(Focus, KeyEvent) -> Action` — pure mapping, testable without UI
4. **Renderer seam**: `(AppState) -> ratatui::Frame` — given state, verify layout structure
5. **Progress seam**: `save(Progress) / load(path) -> Option<Progress>` — file I/O, test round-trip

### Test fixture

- `tests/fixtures/basic-formatting.epub` — a tracked synthetic EPUB covering semantic formatting.
- `test-fixtures/DragonLance.epub` — an optional real EPUB corpus input (excluded from git via `.gitignore`).

### Testing principles

- Test external behavior, not implementation details
- Pure functions (segmenter, input handler) get thorough unit tests
- Parser and renderer get integration tests with the fixture EPUB
- Focus state machine transitions are tested as a state table

## Out of Scope

- PDF, MOBI, or any non-EPUB format support
- Text-to-speech (TTS)
- Bookmarks or highlights (beyond reading progress)
- Search / full-text query
- Theming or color customization
- Configuration file support
- Mouse interaction
- Multiple open books / tabs
- Font or font-size control
- Full CSS cascade, author colors, or browser-equivalent EPUB layout
- Line numbers or progress percentage display

## Further Notes

- The project has a working Rust implementation. This PRD defines the v1 product scope and should be kept aligned with `CONTEXT.md` as behavior evolves.
- ADR-0001 documents the flat block list decision; ADR-0002 documents flat style ranges and composable block presentation.
- The keymap is intentionally dense (single-key actions) for speed. No Ctrl/Alt modifiers in v1.
