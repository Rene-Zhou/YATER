use std::collections::HashMap;

pub type AnnotationStore = HashMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationRef {
    pub id: String,
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextBlock {
    pub text: String,
    pub chapter_index: usize,
    pub annotations: Vec<AnnotationRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageBlock {
    pub alt_text: Option<String>,
    pub source_path: Option<String>,
    pub chapter_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Text(TextBlock),
    Image(ImageBlock),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocNode {
    pub title: String,
    pub target_block_index: usize,
    pub children: Vec<TocNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChapterRange {
    pub start_block: usize,
    pub end_block: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub blocks: Vec<Block>,
    pub toc: Vec<TocNode>,
    pub annotations: AnnotationStore,
    pub chapter_ranges: Vec<ChapterRange>,
}

impl Document {
    pub fn text_block(&self, block_index: usize) -> Option<&TextBlock> {
        match self.blocks.get(block_index) {
            Some(Block::Text(block)) => Some(block),
            _ => None,
        }
    }

    pub fn chapter_title_for_block(&self, block_index: usize) -> Option<&str> {
        self.toc
            .iter()
            .filter(|node| node.target_block_index <= block_index)
            .max_by_key(|node| node.target_block_index)
            .map(|node| node.title.as_str())
    }

    pub fn chapter_range_for_block(&self, block_index: usize) -> Option<ChapterRange> {
        self.chapter_ranges
            .iter()
            .copied()
            .find(|range| range.start_block <= block_index && block_index <= range.end_block)
    }

    pub fn annotation_text(&self, id: &str) -> Option<&str> {
        self.annotations.get(id).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::{AnnotationRef, AnnotationStore, Block, ChapterRange, Document, TextBlock, TocNode};

    #[test]
    fn exposes_current_chapter_title_and_text_annotations() {
        let document = Document {
            blocks: vec![Block::Text(TextBlock {
                text: "Text with note[1].".to_string(),
                chapter_index: 0,
                annotations: vec![AnnotationRef {
                    id: "note-1".to_string(),
                    offset: 14,
                }],
            })],
            toc: vec![TocNode {
                title: "Chapter One".to_string(),
                target_block_index: 0,
                children: Vec::new(),
            }],
            annotations: AnnotationStore::new(),
            chapter_ranges: Vec::new(),
        };

        assert_eq!(document.chapter_title_for_block(0), Some("Chapter One"));
        assert_eq!(
            document.text_block(0).map(|block| block.annotations.as_slice()),
            Some(
                [AnnotationRef {
                    id: "note-1".to_string(),
                    offset: 14,
                }]
                .as_slice()
            )
        );
    }

    #[test]
    fn finds_chapter_range_for_block() {
        let document = Document {
            blocks: vec![
                text_block("chapter one start", 0),
                text_block("chapter one end", 0),
                text_block("chapter two start", 1),
            ],
            toc: Vec::new(),
            annotations: AnnotationStore::new(),
            chapter_ranges: vec![
                ChapterRange {
                    start_block: 0,
                    end_block: 1,
                },
                ChapterRange {
                    start_block: 2,
                    end_block: 2,
                },
            ],
        };

        assert_eq!(
            document.chapter_range_for_block(1),
            Some(ChapterRange {
                start_block: 0,
                end_block: 1,
            })
        );
        assert_eq!(
            document.chapter_range_for_block(2),
            Some(ChapterRange {
                start_block: 2,
                end_block: 2,
            })
        );
    }

    fn text_block(text: &str, chapter_index: usize) -> Block {
        Block::Text(TextBlock {
            text: text.to_string(),
            chapter_index,
            annotations: Vec::new(),
        })
    }
}
