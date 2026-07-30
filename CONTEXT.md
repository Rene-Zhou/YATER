# Context: YATER

## Glossary

### Document
The in-memory representation of an opened EPUB book. A flat list of `Block`s derived from the EPUB spine order. Each `Block` knows its source chapter. The TOC is a separate tree structure, not derived from the blocks.

### Block
A fundamental rendering unit in the document. Two variants: `TextBlock` (normalized text with optional style ranges and block presentation) and `ImageBlock` (an inline image). Blocks appear in spine order.

### TextStyleRange
A sorted, non-overlapping UTF-8 byte range into `TextBlock.text`. It stores the effective combination of bold, italic, underline, and strikethrough modifiers derived from semantic XHTML tags. Nested tags are flattened during eager parsing. Style ranges, sentences, and annotations all address the same normalized text.

### TextBlockPresentation
Composable block-level metadata attached to a `TextBlock`: paragraph or heading role, blockquote depth, and optional list-item marker/depth/continuation. Render-only heading spacing, quote gutters, list markers, and hanging indentation are not inserted into `TextBlock.text`, so navigation and persisted progress offsets remain stable. See `docs/adr/0002-flat-style-ranges.md`.

### Annotation
A footnote or endnote extracted from the EPUB at parse time. Stored as plain text in an `AnnotationStore` (HashMap keyed by ID). Referenced from `TextBlock`s via `AnnotationRef`s that record the character offset of the anchor marker within the block's plain text.

Annotation discovery supports EPUB structural semantics (`epub:type="footnote|endnote"`), DPUB-ARIA roles (`doc-footnote`, `doc-endnote`, and entries under `doc-endnotes`), and EPUB2-style reciprocal fragment links where a note marker links to a note block whose leading link returns to the source marker. `doc-backlink` links and reciprocal return markers are excluded from displayed annotation text.

Parsed annotation markers remain in the reading text and are rendered bold and underlined so numeric markers such as `205`, bracketed markers such as `[1]`, superscript digits, and symbol markers are visibly distinguishable from ordinary text. The current sentence's violet text highlight composes with the marker style.

### AnnotationRef
Metadata attached to a `TextBlock`. Points into the `AnnotationStore` by ID and records the character offset of the anchor within the block's plain text. Enables the renderer to highlight the anchor and look up the annotation text on `;`.

### AnnotationStore
A top-level `HashMap<String, String>` mapping document-qualified annotation IDs (normalized XHTML path plus fragment) to their pre-extracted plain text. EPUB fragment IDs are local to an XHTML document, so qualification prevents same-named notes in different files from colliding. Populated once at parse time.

### Focus
An enum representing which UI component currently owns keyboard input. Four variants: `Content` (reading), `Toc` (table-of-contents navigation), `AnnotationOverlay` (floating footnote), `AnnotationImmersed` (deep reading inside annotation). Transitions are explicit: `Tab` toggles `Content`/`Toc`, `;` enters `AnnotationOverlay` from `Content`, `Enter` deepens to `AnnotationImmersed`, `Esc` pops back one level.

### Parse strategy
Eager full-parse at file open. The entire EPUB is walked spine-order, all chapters converted to Blocks, all annotations extracted into the AnnotationStore, before any rendering begins.

### Basic EPUB formatting
Semantic `<strong>/<b>`, `<em>/<i>`, `<u>/<ins>`, and `<s>/<strike>/<del>` tags map to terminal modifiers. `h1` renders bold and underlined; `h2`–`h6` render bold, with a blank row before and after headings. Blockquotes use a dark `│ ` gutter on every wrapped line. Unordered and ordered lists preserve nesting, `<ol start>`, and hanging indentation; later paragraphs in one item do not repeat the marker. Full CSS, author colors, fonts, and annotation rich text are not parsed.

### Sentence
A rendering-time concern, not a persisted data structure. Segmented on the fly from a `TextBlock` for the purpose of sentence-level highlighting. Segmentation follows the standard Chinese definition of 句子: boundaries only at `。`, `？`, `！`, and `……` (ellipsis). Commas (`，`), semicolons (`；`), enumeration marks (`、`) are clause-internal, not sentence boundaries. Quoted Chinese dialogue keeps inner terminal punctuation together until the closing quote and may include a following attribution phrase. For English text, boundaries at `.`, `!`, `?`. Returns byte-offset ranges, not new strings.

### Module structure
Single crate. `app.rs` owns the main loop and Focus state machine. `epub/` handles parsing. `document/` holds data structures. `render/` does ratatui drawing. `input.rs` maps keys per Focus variant. `sentence.rs` segments text. `image.rs` handles terminal image rendering. Synchronous main loop, no event bus.

### Image rendering
Auto-detect terminal graphics capability via `ratatui-image`'s `Picker` at startup (Sixel, Kitty, iTerm2, or halfblock fallback). Store chosen protocol in `App` state. Optional CLI flag `--image-mode=sixel|halfblock|off` for manual override.

### Progress persistence
Save reading position to `$XDG_DATA_HOME/yater/progress.json`. Keyed by file path. Stores block index, sentence offset, and timestamp. Auto-save on navigation (debounced). Restore on open if progress exists for the given EPUB.

### Reader viewport
Text reading is framed by a full-screen `Block` with the current chapter in the top border and focus-specific shortcut hints in the bottom border. The top title and footer are left-biased with a small inset instead of centered. The footer changes for content, TOC, compact annotation, and immersed annotation focus. Text reading uses a typewriter-style viewport: the highlighted sentence line is kept on the vertical center row of the framed content area. The active focus target uses `#a97df4` violet text only, without adding bold, background tint, or terminal reverse video; EPUB modifiers and annotation-marker modifiers remain composed with that foreground color. When TOC has focus, the right reading pane is contextual and does not keep the sentence highlight, but EPUB formatting remains. The renderer adds virtual top/bottom padding so the first and last text lines can also be centered instead of being clamped to the screen edges.

### TOC
A `Vec<TocNode>` tree parsed from the EPUB's navigation document. Each `TocNode` has title, target block index, and children. Rendered as a left sidebar inside the same outer reader frame, with the reading context reflowed in the remaining right pane and separated by a dark divider. TOC rows use indent guides (`│`, `└`, `├`), expand/collapse markers (`▸`/`▾`), and the same violet selection highlight used by the reader focus sentence. `App` tracks collapsed paths, the selected row, and a persistent scroll offset. Scrolling uses each title's rendered wrapped height: moving down scrolls only when the selection would leave the bottom, while moving up moves the selection within the current viewport until it reaches the top and only then scrolls.

### Annotation overlay
A floating window drawn on top of the content area. Bottom edge aligns above the current highlighted sentence. Bordered `Paragraph` widget via ratatui's `Clear` + draw. The compact overlay wraps text and grows with short-to-medium notes up to a capped height while preserving reading context. Multiple annotations cycle with `;`, counter shown as `[2/3]`. If text still overflows, `Enter` enters `AnnotationImmersed` for scroll; immersed annotation uses the outer reader frame as its border to preserve vertical space. Drawn after content to render on top.

### CLI
`yater <file.epub> [--image-mode=sixel|halfblock|off]`. One required positional arg, one optional flag. No subcommands, no config file in v1.

### Error handling
Startup errors (file not found, corrupted EPUB): print to stderr, exit code 1. Runtime panics: catch at top of main loop, restore terminal, print error, exit. Non-fatal issues (bad image, malformed HTML): log to `$XDG_STATE_HOME/yater/yater.log`, show placeholder, continue.

### Keymap
Content: `j`/`k` sentence nav, `h`/`l` paragraph nav, `u`/`n` fast sentence nav, `i`/`m` chapter start/end, `;` annotation, `Tab` TOC, `q` quit. TOC: `j`/`k` move, `l`/`Enter` expand/jump, `h` collapse/parent, `Tab`/`Esc` close. AnnotationOverlay: `;` cycle, `Enter` immerse, other key close. AnnotationImmersed: `j`/`k` scroll, `Esc` exit.

### ChapterRange
Maps chapter index to `(start_block, end_block)` range in the flat block list. Computed once at parse time. Each `Block` stores its chapter index. Enables `i`/`m` chapter navigation.

### Screen layout
Minimal. A single full-screen outer frame contains the top title, content area, and bottom shortcut footer. Reading, TOC, and immersed annotation modes reuse that frame instead of introducing separate chrome. TOC mode splits the framed content area into a left sidebar and right reading pane; compact annotations are the only inner floating window. No line numbers or progress percentage in v1.

## Reference

[lue](https://github.com/superstarryeyes/lue) — the inspiration project for YATER. Python-based terminal ebook reader with TTS. YATER's UI design and module structure draw from lue's architecture (separate parser, reader, input handler, UI modules).
