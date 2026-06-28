# Flat style ranges for EPUB formatting

Status: accepted

## Context

EPUB XHTML is a tree, while YATER intentionally stores reading content as a flat `Vec<Block>` (ADR-0001). Inline formatting can be nested and can overlap sentence and annotation boundaries. Navigation, annotation lookup, and progress persistence already address text through UTF-8 byte offsets in `TextBlock.text`.

Storing a DOM subtree or splitting text into independently owned styled strings would introduce a second addressing model and make sentence navigation depend on render structure.

## Decision

Keep `TextBlock.text` as the canonical normalized plain text and attach sorted, non-overlapping `TextStyleRange`s that address the same string with UTF-8 byte offsets. Each range stores the effective combination of supported inline modifiers.

Store block presentation separately as composable metadata: content role, blockquote depth, and optional list-item presentation. These properties are not a mutually exclusive enum because structures such as an ordered list inside a blockquote must remain representable.

Parser whitespace normalization remaps annotation offsets and style boundaries in one pass. Render-only decorations such as list markers, quote gutters, indentation, and heading spacing are not inserted into `TextBlock.text`.

## Consequences

- Sentence ranges, annotations, formatting, and progress continue to share one canonical text coordinate system.
- The renderer combines boundaries from sentences, style ranges, and annotation markers before emitting terminal spans.
- Nested XHTML styles are flattened during parsing; the renderer does not need the source DOM.
- Block decorations must be included in wrapping and row calculations even though they are absent from logical text.
- Adding or changing whitespace normalization requires tests proving that all stored byte offsets remain valid UTF-8 boundaries.
