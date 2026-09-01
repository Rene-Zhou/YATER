# YATER

**YATER** (Yet Another Terminal Epub Reader) is a terminal-native EPUB reader. No GUI, no distractions — just your book, in the terminal you already live in.

It was built for comfortable long-form reading, with first-class support for both English and CJK text.

## Highlights

- **Typewriter-style reading** — the sentence you're reading stays centered on the screen; the text scrolls around you.
- **Sentence-level navigation** — move through a book one sentence at a time, with CJK-aware segmentation that understands Chinese and Japanese punctuation and quoted dialogue.
- **Gentle focus highlight** — the current sentence is tinted, never inverted or bolded.
- **Faithful formatting** — bold, italic, underline, strikethrough, headings, blockquotes, and nested lists carry over from the EPUB.
- **Footnotes without leaving the page** — endnotes and footnotes open in a floating overlay, expandable to a scrollable view.
- **Table of contents sidebar** — a Vim-navigable tree that opens beside your text.
- **Inline images** — rendered right in the terminal via Sixel, Kitty, or iTerm2 graphics, with an automatic halfblock fallback.
- **Remembers where you stopped** — reading progress is saved per book and restored on the next open.

## Install

Build from source (requires a recent Rust toolchain):

```bash
git clone https://github.com/Rene-Zhou/YATER.git
cd YATER
cargo build --release
```

The binary is at `target/release/yater`. Copy it somewhere on your `PATH`, e.g.:

```bash
ln -s "$PWD/target/release/yater" ~/.local/bin/yater
```

## Usage

```bash
yater <file.epub|file.txt>
```

Plain text files are supported alongside EPUB: encoding is detected automatically (UTF-8, UTF-16 with BOM, or GBK/GB18030), each non-empty line becomes a paragraph, and the whole file is presented as a single chapter without a table of contents.

Images are auto-detected by default. To override:

```bash
yater book.epub --image-mode=sixel      # force Sixel
yater book.epub --image-mode=halfblock  # force Unicode halfblock
yater book.epub --image-mode=off        # disable images
```

## Keys

### Reading

| Key | Action |
| --- | --- |
| `j` / `k` | Next / previous sentence |
| `u` / `n` | Fast sentence jump |
| `h` / `l` | Previous / next paragraph |
| `i` / `m` | Start / end of chapter |
| `;` | Open footnotes for the current sentence |
| `Tab` | Table of contents |
| `q` | Quit |

### Table of contents

| Key | Action |
| --- | --- |
| `j` / `k` | Move selection |
| `l` / `Enter` | Expand or jump to chapter |
| `h` | Collapse or move to parent |
| `Tab` / `Esc` | Close |

### Footnotes

| Key | Action |
| --- | --- |
| `;` | Cycle through the sentence's notes |
| `Enter` | Expand to full scrollable view |
| `j` / `k` | Scroll (in full view) |
| `Esc` | Step back / close |

## Scope

YATER is intentionally focused: it reads EPUB and TXT files, in the terminal, one book at a time. It does not aim to support PDF/MOBI, search, bookmarks, themes, or mouse interaction.

## License

MIT
