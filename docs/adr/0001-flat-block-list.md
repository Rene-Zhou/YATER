# Flat block list instead of tree structure

The Document stores content as a flat `Vec<Block>` in spine order, not a tree mirroring the EPUB's HTML structure. Each `Block` carries a chapter index, and a separate `ChapterRange` maps chapter boundaries.

The EPUB's XHTML naturally forms a tree (chapters contain paragraphs, paragraphs contain inline elements). A tree would preserve this hierarchy, but it adds complexity everywhere: rendering must flatten the tree to draw a scrollable view, navigation must skip over non-leaf nodes, and progress persistence must encode a tree path instead of a simple index. Since the user only ever sees a linear reading flow — sentences in order, top to bottom — the flat list matches the actual UX. Chapter-level structure is recovered cheaply via `ChapterRange` without needing a tree.
