# Context: YATER

## Glossary

### Document
The in-memory representation of an opened EPUB book. A flat list of `Block`s derived from the EPUB spine order. Each `Block` knows its source chapter. The TOC is a separate tree structure, not derived from the blocks.

### Block
A fundamental rendering unit in the document. Two variants: `TextBlock` (a paragraph of text) and `ImageBlock` (an inline image). Blocks appear in spine order.

### Annotation
A footnote or endnote extracted from the EPUB at parse time. Stored as plain text in an `AnnotationStore` (HashMap keyed by ID). Referenced from `TextBlock`s via `AnnotationRef`s that record the character offset of the anchor marker within the block's plain text.

### AnnotationRef
Metadata attached to a `TextBlock`. Points into the `AnnotationStore` by ID and records the character offset of the anchor within the block's plain text. Enables the renderer to highlight the anchor and look up the annotation text on `;`.

### AnnotationStore
A top-level `HashMap<String, String>` mapping annotation IDs to their pre-extracted plain text. Populated once at parse time.

### Focus
An enum representing which UI component currently owns keyboard input. Four variants: `Content` (reading), `Toc` (sidebar navigation), `AnnotationOverlay` (floating footnote), `AnnotationImmersed` (deep reading inside annotation). Transitions are explicit: `Tab` toggles `Content`/`Toc`, `;` enters `AnnotationOverlay` from `Content`, `Enter` deepens to `AnnotationImmersed`, `Esc` pops back one level.

### Parse strategy
Eager full-parse at file open. The entire EPUB is walked spine-order, all chapters converted to Blocks, all annotations extracted into the AnnotationStore, before any rendering begins.

### Sentence
A rendering-time concern, not a persisted data structure. Segmented on the fly from a `TextBlock` for the purpose of sentence-level highlighting. Segmentation follows the standard Chinese definition of 句子: boundaries only at `。`, `？`, `！`, and `……` (ellipsis). Commas (`，`), semicolons (`；`), enumeration marks (`、`) are clause-internal, not sentence boundaries. For English text, boundaries at `.`, `!`, `?`. Returns byte-offset ranges, not new strings.

### Module structure
Single crate. `app.rs` owns the main loop and Focus state machine. `epub/` handles parsing. `document/` holds data structures. `render/` does ratatui drawing. `input.rs` maps keys per Focus variant. `sentence.rs` segments text. `image.rs` handles terminal image rendering. Synchronous main loop, no event bus.

### Image rendering
Auto-detect terminal graphics capability via `ratatui-image`'s `Picker` at startup (Sixel, Kitty, iTerm2, or halfblock fallback). Store chosen protocol in `App` state. Optional CLI flag `--image-mode=sixel|halfblock|off` for manual override.

### Progress persistence
Save reading position to `$XDG_DATA_HOME/yater/progress.json`. Keyed by file path. Stores block index, sentence offset, and timestamp. Auto-save on navigation (debounced). Restore on open if progress exists for the given EPUB.

### TOC
A `Vec<TocNode>` tree parsed from the EPUB's navigation document. Each `TocNode` has title, target block index, and children. Rendered in a sidebar with indent guides (`│`, `└`, `├`), expand/collapse markers (`▸`/`▾`), and selection highlight. Inspired by neo-tree.nvim's component-composition pattern. `TocState` tracks expanded nodes (HashSet), selected row, and scroll offset.

### Annotation overlay
A floating window drawn on top of the content area. Bottom edge aligns with the top of the current highlighted sentence. Bordered `Paragraph` widget via ratatui's `Clear` + draw. Multiple annotations cycle with `;`, counter shown as `[2/3]`. If text overflows, `Enter` enters `AnnotationImmersed` for scroll. Drawn after content to render on top.

### CLI
`yater <file.epub> [--image-mode=sixel|halfblock|off]`. One required positional arg, one optional flag. No subcommands, no config file in v1.

### Error handling
Startup errors (file not found, corrupted EPUB): print to stderr, exit code 1. Runtime panics: catch at top of main loop, restore terminal, print error, exit. Non-fatal issues (bad image, malformed HTML): log to `$XDG_STATE_HOME/yater/yater.log`, show placeholder, continue.

### Keymap
Content: `j`/`k` sentence nav, `h`/`l` paragraph nav, `u`/`n` page up/down, `i`/`m` chapter start/end, `;` annotation, `Tab` TOC, `q` quit. TOC: `j`/`k` move, `l`/`Enter` expand/jump, `h` collapse/parent, `Tab`/`Esc` close. AnnotationOverlay: `;` cycle, `Enter` immerse, other key close. AnnotationImmersed: `j`/`k` scroll, `Esc` exit.

### ChapterRange
Maps chapter index to `(start_block, end_block)` range in the flat block list. Computed once at parse time. Each `Block` stores its chapter index. Enables `i`/`m` chapter navigation.

### Screen layout
Minimal. Top bar (1 row, centered chapter name) + content area. TOC sidebar (~30% width) overlays left side when open. No bottom bar, no line numbers, no progress percentage in v1.

## Reference

[lue](https://github.com/superstarryeyes/lue) — the inspiration project for YATER. Python-based terminal ebook reader with TTS. YATER's UI design and module structure draw from lue's architecture (separate parser, reader, input handler, UI modules).
