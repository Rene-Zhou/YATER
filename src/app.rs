use std::collections::HashSet;

use crate::document::Block;
use crate::document::ChapterRange;
use crate::document::Document;
use crate::document::TocNode;
use crate::image::SelectedImageMode;
use crate::input::{Action, Focus};
use crate::progress::Progress;
use crate::sentence::segment_sentences;

const PAGE_SENTENCE_COUNT: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadingPosition {
    pub block_index: usize,
    pub sentence_offset: usize,
}

#[derive(Debug)]
pub struct App {
    document: Document,
    position: ReadingPosition,
    focus: Focus,
    selected_toc_row: usize,
    collapsed_toc_paths: HashSet<Vec<usize>>,
    image_mode: SelectedImageMode,
    selected_annotation_index: usize,
    annotation_scroll: usize,
}

impl App {
    pub fn new(document: Document) -> Self {
        Self {
            document,
            position: ReadingPosition {
                block_index: 0,
                sentence_offset: 0,
            },
            focus: Focus::Content,
            selected_toc_row: 0,
            collapsed_toc_paths: HashSet::new(),
            image_mode: SelectedImageMode::Halfblock,
            selected_annotation_index: 0,
            annotation_scroll: 0,
        }
    }

    pub fn with_position(document: Document, position: ReadingPosition) -> Self {
        Self::with_position_and_image_mode(document, position, SelectedImageMode::Halfblock)
    }

    pub fn with_image_mode(document: Document, image_mode: SelectedImageMode) -> Self {
        Self::with_position_and_image_mode(
            document,
            ReadingPosition {
                block_index: 0,
                sentence_offset: 0,
            },
            image_mode,
        )
    }

    pub fn with_position_and_image_mode(
        document: Document,
        position: ReadingPosition,
        image_mode: SelectedImageMode,
    ) -> Self {
        Self {
            document,
            position,
            focus: Focus::Content,
            selected_toc_row: 0,
            collapsed_toc_paths: HashSet::new(),
            image_mode,
            selected_annotation_index: 0,
            annotation_scroll: 0,
        }
    }

    pub fn with_restored_progress(document: Document, progress: Option<Progress>) -> Self {
        let position = progress
            .filter(|progress| {
                Self::is_valid_reading_position(
                    &document,
                    progress.block_index,
                    progress.sentence_offset,
                )
            })
            .map(|progress| ReadingPosition {
                block_index: progress.block_index,
                sentence_offset: progress.sentence_offset,
            })
            .unwrap_or(ReadingPosition {
                block_index: 0,
                sentence_offset: 0,
            });

        Self::with_position(document, position)
    }

    fn is_valid_reading_position(
        document: &Document,
        block_index: usize,
        sentence_offset: usize,
    ) -> bool {
        match document.blocks.get(block_index) {
            Some(Block::Text(block)) => segment_sentences(&block.text)
                .into_iter()
                .any(|range| range.0 == sentence_offset),
            Some(Block::Image(_)) => sentence_offset == 0,
            None => false,
        }
    }

    pub fn position(&self) -> ReadingPosition {
        self.position
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    pub fn selected_toc_row(&self) -> usize {
        self.selected_toc_row
    }

    pub fn selected_annotation_index(&self) -> usize {
        self.selected_annotation_index
    }

    pub fn annotation_scroll(&self) -> usize {
        self.annotation_scroll
    }

    pub fn image_mode(&self) -> SelectedImageMode {
        self.image_mode
    }

    pub fn set_image_mode(&mut self, image_mode: SelectedImageMode) {
        self.image_mode = image_mode;
    }

    pub fn is_toc_path_collapsed(&self, path: &[usize]) -> bool {
        self.collapsed_toc_paths.iter().any(|collapsed_path| collapsed_path == path)
    }

    pub fn progress(&self, timestamp: impl Into<String>) -> Progress {
        Progress {
            block_index: self.position.block_index,
            sentence_offset: self.position.sentence_offset,
            timestamp: timestamp.into(),
        }
    }

    pub fn apply(&mut self, action: Action) {
        match action {
            Action::NextSentence => self.next_sentence(),
            Action::PreviousSentence => self.previous_sentence(),
            Action::NextParagraph => self.advance_to_next_reading_block(),
            Action::PreviousParagraph => self.retreat_to_previous_reading_block(),
            Action::PageDown => self.page_down(),
            Action::PageUp => self.page_up(),
            Action::JumpToChapterStart => self.jump_to_chapter_start(),
            Action::JumpToChapterEnd => self.jump_to_chapter_end(),
            Action::OpenToc => {
                self.focus = Focus::Toc;
                self.selected_toc_row = self
                    .selected_toc_row
                    .min(self.visible_toc_targets().len().saturating_sub(1));
            }
            Action::CloseToc => self.focus = Focus::Content,
            Action::NextTocItem => self.next_toc_item(),
            Action::PreviousTocItem => self.previous_toc_item(),
            Action::ExpandOrJumpToc => self.activate_selected_toc_item(),
            Action::CollapseOrParentToc => self.collapse_or_select_parent_toc_item(),
            Action::OpenAnnotationOverlay => {
                if self.current_sentence_annotation_count() > 0 {
                    self.focus = Focus::AnnotationOverlay;
                    self.selected_annotation_index = 0;
                    self.annotation_scroll = 0;
                }
            }
            Action::CycleAnnotation => self.cycle_annotation(),
            Action::ImmerseAnnotation => {
                if self.focus == Focus::AnnotationOverlay
                    && self.current_sentence_annotation_count() > 0
                {
                    self.focus = Focus::AnnotationImmersed;
                    self.annotation_scroll = 0;
                }
            }
            Action::ExitAnnotationImmersion => {
                self.focus = Focus::Content;
                self.annotation_scroll = 0;
            }
            Action::CloseAnnotationOverlay => {
                self.focus = Focus::Content;
                self.annotation_scroll = 0;
            }
            Action::ScrollAnnotationDown => self.scroll_annotation_down(),
            Action::ScrollAnnotationUp => {
                self.annotation_scroll = self.annotation_scroll.saturating_sub(1);
            }
            _ => {}
        }
    }

    fn next_sentence(&mut self) {
        let Some(block) = self.document.text_block(self.position.block_index) else {
            self.advance_to_next_reading_block();
            return;
        };

        let ranges = segment_sentences(&block.text);
        let Some(current_sentence_index) = ranges
            .iter()
            .position(|range| range.0 == self.position.sentence_offset)
        else {
            return;
        };

        if let Some(next_range) = ranges.get(current_sentence_index + 1) {
            self.position.sentence_offset = next_range.0;
        } else {
            self.advance_to_next_reading_block();
        }
    }

    fn advance_to_next_reading_block(&mut self) {
        let next_block_index = (self.position.block_index + 1 < self.document.blocks.len())
            .then_some(self.position.block_index + 1);

        if let Some(block_index) = next_block_index {
            self.position = ReadingPosition {
                block_index,
                sentence_offset: 0,
            };
        }
    }

    fn previous_sentence(&mut self) {
        let Some(block) = self.document.text_block(self.position.block_index) else {
            self.retreat_to_previous_reading_block();
            return;
        };

        let ranges = segment_sentences(&block.text);
        let current_sentence_index = ranges
            .iter()
            .position(|range| range.0 == self.position.sentence_offset)
            .unwrap_or(0);

        if current_sentence_index > 0 {
            self.position.sentence_offset = ranges[current_sentence_index - 1].0;
        } else {
            self.retreat_to_previous_reading_block();
        }
    }

    fn page_down(&mut self) {
        for _ in 0..PAGE_SENTENCE_COUNT {
            self.next_sentence();
        }
    }

    fn page_up(&mut self) {
        for _ in 0..PAGE_SENTENCE_COUNT {
            self.previous_sentence();
        }
    }

    fn retreat_to_previous_reading_block(&mut self) {
        let Some(block_index) = self.position.block_index.checked_sub(1) else {
            return;
        };
        let sentence_offset = match &self.document.blocks[block_index] {
            Block::Text(block) => segment_sentences(&block.text)
                .last()
                .map(|range| range.0)
                .unwrap_or(0),
            Block::Image(_) => 0,
        };

        self.position = ReadingPosition {
            block_index,
            sentence_offset,
        }
    }

    fn jump_to_chapter_start(&mut self) {
        let Some(range) = self
            .document
            .chapter_range_for_block(self.position.block_index)
        else {
            return;
        };

        if let Some(block_index) = self.first_reading_block_in_range(range) {
            self.position = ReadingPosition {
                block_index,
                sentence_offset: 0,
            };
        }
    }

    fn jump_to_chapter_end(&mut self) {
        let Some(range) = self
            .document
            .chapter_range_for_block(self.position.block_index)
        else {
            return;
        };

        if let Some((block_index, sentence_offset)) = self.last_reading_position_in_range(range) {
            self.position = ReadingPosition {
                block_index,
                sentence_offset,
            };
        }
    }

    fn first_reading_block_in_range(&self, range: ChapterRange) -> Option<usize> {
        (range.start_block < self.document.blocks.len()
            && range.start_block <= range.end_block)
            .then_some(range.start_block)
    }

    fn last_reading_position_in_range(&self, range: ChapterRange) -> Option<(usize, usize)> {
        if range.start_block > range.end_block || range.end_block >= self.document.blocks.len() {
            return None;
        }

        let sentence_offset = match &self.document.blocks[range.end_block] {
            Block::Text(block) => segment_sentences(&block.text)
                .last()
                .map(|sentence| sentence.0)
                .unwrap_or(0),
            Block::Image(_) => 0,
        };

        Some((range.end_block, sentence_offset))
    }

    fn next_toc_item(&mut self) {
        let last_row = self.visible_toc_targets().len().saturating_sub(1);
        self.selected_toc_row = (self.selected_toc_row + 1).min(last_row);
    }

    fn previous_toc_item(&mut self) {
        self.selected_toc_row = self.selected_toc_row.saturating_sub(1);
    }

    fn activate_selected_toc_item(&mut self) {
        let Some(row) = self.visible_toc_rows().get(self.selected_toc_row).cloned() else {
            return;
        };

        if row.has_children && self.collapsed_toc_paths.remove(&row.path) {
            return;
        }

        self.position = ReadingPosition {
            block_index: row.target_block_index,
            sentence_offset: 0,
        };
        self.focus = Focus::Content;
    }

    fn collapse_or_select_parent_toc_item(&mut self) {
        let Some(row) = self.visible_toc_rows().get(self.selected_toc_row).cloned() else {
            return;
        };

        if row.has_children && !self.collapsed_toc_paths.contains(&row.path) {
            self.collapsed_toc_paths.insert(row.path);
            self.selected_toc_row = self
                .selected_toc_row
                .min(self.visible_toc_rows().len().saturating_sub(1));
            return;
        }

        let Some(parent_path) = row.path.get(..row.path.len().saturating_sub(1)) else {
            return;
        };
        if parent_path.is_empty() {
            return;
        }

        if let Some(parent_row) = self
            .visible_toc_rows()
            .iter()
            .position(|visible_row| visible_row.path == parent_path)
        {
            self.selected_toc_row = parent_row;
        }
    }

    fn visible_toc_targets(&self) -> Vec<usize> {
        self.visible_toc_rows()
            .into_iter()
            .map(|row| row.target_block_index)
            .collect()
    }

    fn visible_toc_rows(&self) -> Vec<VisibleTocRow> {
        let mut rows = Vec::new();

        for (index, node) in self.document.toc.iter().enumerate() {
            append_toc_rows(
                node,
                vec![index],
                &self.collapsed_toc_paths,
                &mut rows,
            );
        }

        rows
    }

    fn cycle_annotation(&mut self) {
        let annotation_count = self.current_sentence_annotation_count();

        if annotation_count > 0 {
            self.selected_annotation_index =
                (self.selected_annotation_index + 1) % annotation_count;
            self.annotation_scroll = 0;
        }
    }

    fn scroll_annotation_down(&mut self) {
        let max_scroll = self.current_annotation_line_count().saturating_sub(1);
        self.annotation_scroll = (self.annotation_scroll + 1).min(max_scroll);
    }

    fn current_annotation_line_count(&self) -> usize {
        self.current_annotation_text()
            .map(|text| text.lines().count().max(1))
            .unwrap_or(0)
    }

    fn current_annotation_text(&self) -> Option<&str> {
        let block = self.document.text_block(self.position.block_index)?;
        let sentence_range = segment_sentences(&block.text)
            .into_iter()
            .find(|range| range.0 == self.position.sentence_offset)?;
        let annotation_ref = block
            .annotations
            .iter()
            .filter(|annotation_ref| {
                sentence_range.0 <= annotation_ref.offset && annotation_ref.offset < sentence_range.1
            })
            .nth(self.selected_annotation_index)?;

        self.document.annotation_text(&annotation_ref.id)
    }

    fn current_sentence_annotation_count(&self) -> usize {
        let Some(block) = self.document.text_block(self.position.block_index) else {
            return 0;
        };
        let Some(sentence_range) = segment_sentences(&block.text)
            .into_iter()
            .find(|range| range.0 == self.position.sentence_offset)
        else {
            return 0;
        };

        block
            .annotations
            .iter()
            .filter(|annotation_ref| {
                sentence_range.0 <= annotation_ref.offset && annotation_ref.offset < sentence_range.1
            })
            .count()
    }
}

#[derive(Clone)]
struct VisibleTocRow {
    path: Vec<usize>,
    target_block_index: usize,
    has_children: bool,
}

fn append_toc_rows(
    node: &TocNode,
    path: Vec<usize>,
    collapsed_paths: &HashSet<Vec<usize>>,
    rows: &mut Vec<VisibleTocRow>,
) {
    rows.push(VisibleTocRow {
        path: path.clone(),
        target_block_index: node.target_block_index,
        has_children: !node.children.is_empty(),
    });

    if collapsed_paths.contains(&path) {
        return;
    }

    for (index, child) in node.children.iter().enumerate() {
        let mut child_path = path.clone();
        child_path.push(index);
        append_toc_rows(child, child_path, collapsed_paths, rows);
    }
}

#[cfg(test)]
mod tests {
    use crate::document::{
        AnnotationRef, Block, ChapterRange, Document, ImageBlock, TextBlock, TocNode,
    };
    use crate::progress::Progress;
    use std::collections::HashMap;
    use crate::input::{Action, Focus};

    use super::{App, ReadingPosition};

    #[test]
    fn next_sentence_moves_within_and_across_text_blocks() {
        let mut app = App::new(Document {
            blocks: vec![text_block("First. Second?"), text_block("Third!")],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        });

        app.apply(Action::NextSentence);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 0,
                sentence_offset: "First.".len()
            }
        );

        app.apply(Action::NextSentence);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 1,
                sentence_offset: 0
            }
        );
    }

    #[test]
    fn previous_sentence_moves_within_and_across_text_blocks() {
        let mut app = App::with_position(
            Document {
                blocks: vec![text_block("First. Second?"), text_block("Third!")],
                toc: Vec::new(),
                annotations: HashMap::new(),
                chapter_ranges: Vec::new(),
            },
            ReadingPosition {
                block_index: 1,
                sentence_offset: 0,
            },
        );

        app.apply(Action::PreviousSentence);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 0,
                sentence_offset: "First.".len()
            }
        );

        app.apply(Action::PreviousSentence);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 0,
                sentence_offset: 0
            }
        );
    }

    #[test]
    fn paragraph_navigation_moves_between_reading_blocks() {
        let document = Document {
            blocks: vec![
                text_block("First paragraph."),
                image_block(),
                text_block("Second paragraph."),
            ],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        };
        let mut app = App::new(document);

        app.apply(Action::NextParagraph);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 1,
                sentence_offset: 0
            }
        );

        app.apply(Action::NextParagraph);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 2,
                sentence_offset: 0
            }
        );

        app.apply(Action::PreviousParagraph);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 1,
                sentence_offset: 0
            }
        );

        app.apply(Action::PreviousParagraph);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 0,
                sentence_offset: 0
            }
        );
    }

    #[test]
    fn sentence_navigation_visits_inline_image_blocks() {
        let document = Document {
            blocks: vec![text_block("Before."), image_block(), text_block("After.")],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        };
        let mut app = App::new(document);

        app.apply(Action::NextSentence);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 1,
                sentence_offset: 0
            }
        );

        app.apply(Action::NextSentence);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 2,
                sentence_offset: 0
            }
        );

        app.apply(Action::PreviousSentence);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 1,
                sentence_offset: 0
            }
        );
    }

    #[test]
    fn page_navigation_moves_by_ten_sentences() {
        let mut app = App::new(Document {
            blocks: vec![text_block(
                "One. Two. Three. Four. Five. Six. Seven. Eight. Nine. Ten. Eleven. Twelve.",
            )],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        });

        app.apply(Action::PageDown);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 0,
                sentence_offset:
                    "One. Two. Three. Four. Five. Six. Seven. Eight. Nine. Ten.".len(),
            }
        );

        app.apply(Action::PageUp);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 0,
                sentence_offset: 0,
            }
        );
    }

    #[test]
    fn restores_and_exports_progress_position() {
        let document = Document {
            blocks: vec![text_block("First. Second.")],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        };
        let mut app = App::with_restored_progress(
            document,
            Some(Progress {
                block_index: 0,
                sentence_offset: "First.".len(),
                timestamp: "2026-06-03T12:00:00Z".to_string(),
            }),
        );

        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 0,
                sentence_offset: "First.".len(),
            }
        );

        app.apply(Action::PreviousSentence);
        assert_eq!(
            app.progress("2026-06-03T12:01:00Z"),
            Progress {
                block_index: 0,
                sentence_offset: 0,
                timestamp: "2026-06-03T12:01:00Z".to_string(),
            }
        );
    }

    #[test]
    fn stale_sentence_offsets_restore_to_book_start() {
        let document = Document {
            blocks: vec![text_block("First. Second.")],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        };
        let app = App::with_restored_progress(
            document,
            Some(Progress {
                block_index: 0,
                sentence_offset: 1,
                timestamp: "2026-06-03T12:00:00Z".to_string(),
            }),
        );

        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 0,
                sentence_offset: 0,
            }
        );
    }

    #[test]
    fn chapter_navigation_jumps_to_start_and_end_of_current_chapter() {
        let mut app = App::with_position(
            Document {
                blocks: vec![
                    text_block("Chapter one."),
                    text_block("Chapter two first. Chapter two second?"),
                    text_block("Chapter two final!"),
                ],
                toc: Vec::new(),
                annotations: HashMap::new(),
                chapter_ranges: vec![
                    ChapterRange {
                        start_block: 0,
                        end_block: 0,
                    },
                    ChapterRange {
                        start_block: 1,
                        end_block: 2,
                    },
                ],
            },
            ReadingPosition {
                block_index: 1,
                sentence_offset: "Chapter two first.".len(),
            },
        );

        app.apply(Action::JumpToChapterEnd);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 2,
                sentence_offset: 0
            }
        );

        app.apply(Action::JumpToChapterStart);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 1,
                sentence_offset: 0
            }
        );
    }

    #[test]
    fn chapter_navigation_jumps_to_image_edge_blocks() {
        let mut app = App::with_position(
            Document {
                blocks: vec![
                    text_block("Previous chapter."),
                    image_block(),
                    text_block("Middle."),
                    image_block(),
                ],
                toc: Vec::new(),
                annotations: HashMap::new(),
                chapter_ranges: vec![
                    ChapterRange {
                        start_block: 0,
                        end_block: 0,
                    },
                    ChapterRange {
                        start_block: 1,
                        end_block: 3,
                    },
                ],
            },
            ReadingPosition {
                block_index: 2,
                sentence_offset: 0,
            },
        );

        app.apply(Action::JumpToChapterStart);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 1,
                sentence_offset: 0
            }
        );

        app.apply(Action::JumpToChapterEnd);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 3,
                sentence_offset: 0
            }
        );
    }

    #[test]
    fn focus_transitions_follow_reader_toc_and_annotation_actions() {
        let mut app = App::new(Document {
            blocks: vec![annotated_text_block("First [1].", "note-1", "First ".len())],
            toc: Vec::new(),
            annotations: annotation_map("note-1", "Note."),
            chapter_ranges: Vec::new(),
        });

        assert_eq!(app.focus(), Focus::Content);

        app.apply(Action::OpenToc);
        assert_eq!(app.focus(), Focus::Toc);

        app.apply(Action::CloseToc);
        assert_eq!(app.focus(), Focus::Content);

        app.apply(Action::OpenAnnotationOverlay);
        assert_eq!(app.focus(), Focus::AnnotationOverlay);

        app.apply(Action::ImmerseAnnotation);
        assert_eq!(app.focus(), Focus::AnnotationImmersed);

        app.apply(Action::ExitAnnotationImmersion);
        assert_eq!(app.focus(), Focus::Content);
    }

    #[test]
    fn annotation_overlay_only_opens_when_current_sentence_has_annotation() {
        let mut app = App::new(Document {
            blocks: vec![text_block("First.")],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        });

        app.apply(Action::OpenAnnotationOverlay);

        assert_eq!(app.focus(), Focus::Content);
    }

    #[test]
    fn annotation_immersion_only_enters_from_open_overlay() {
        let mut app = App::new(Document {
            blocks: vec![annotated_text_block("First [1].", "note-1", "First ".len())],
            toc: Vec::new(),
            annotations: annotation_map("note-1", "Note."),
            chapter_ranges: Vec::new(),
        });

        app.apply(Action::ImmerseAnnotation);

        assert_eq!(app.focus(), Focus::Content);
    }

    #[test]
    fn toc_navigation_selects_and_jumps_to_visible_item() {
        let mut app = App::new(Document {
            blocks: vec![text_block("Chapter one."), text_block("Chapter two.")],
            toc: vec![
                TocNode {
                    title: "Chapter One".to_string(),
                    target_block_index: 0,
                    children: Vec::new(),
                },
                TocNode {
                    title: "Chapter Two".to_string(),
                    target_block_index: 1,
                    children: Vec::new(),
                },
            ],
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        });

        app.apply(Action::OpenToc);
        assert_eq!(app.selected_toc_row(), 0);

        app.apply(Action::NextTocItem);
        assert_eq!(app.selected_toc_row(), 1);

        app.apply(Action::ExpandOrJumpToc);
        assert_eq!(app.focus(), Focus::Content);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 1,
                sentence_offset: 0,
            }
        );
    }

    #[test]
    fn toc_collapse_hides_children_and_expand_restores_navigation() {
        let mut app = App::new(Document {
            blocks: vec![
                text_block("Chapter one."),
                text_block("Section one."),
                text_block("Chapter two."),
            ],
            toc: vec![
                TocNode {
                    title: "Chapter One".to_string(),
                    target_block_index: 0,
                    children: vec![TocNode {
                        title: "Section One".to_string(),
                        target_block_index: 1,
                        children: Vec::new(),
                    }],
                },
                TocNode {
                    title: "Chapter Two".to_string(),
                    target_block_index: 2,
                    children: Vec::new(),
                },
            ],
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        });

        app.apply(Action::OpenToc);
        app.apply(Action::CollapseOrParentToc);
        app.apply(Action::NextTocItem);
        app.apply(Action::ExpandOrJumpToc);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 2,
                sentence_offset: 0,
            }
        );

        app.apply(Action::OpenToc);
        app.apply(Action::PreviousTocItem);
        app.apply(Action::ExpandOrJumpToc);
        app.apply(Action::NextTocItem);
        app.apply(Action::ExpandOrJumpToc);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 1,
                sentence_offset: 0,
            }
        );
    }

    #[test]
    fn app_tracks_selected_image_mode() {
        let app = App::with_image_mode(
            Document {
                blocks: vec![image_block()],
                toc: Vec::new(),
                annotations: HashMap::new(),
                chapter_ranges: Vec::new(),
            },
            crate::image::SelectedImageMode::Off,
        );

        assert_eq!(app.image_mode(), crate::image::SelectedImageMode::Off);
    }

    #[test]
    fn app_updates_selected_image_mode_after_detection() {
        let mut app = App::with_image_mode(
            Document {
                blocks: vec![image_block()],
                toc: Vec::new(),
                annotations: HashMap::new(),
                chapter_ranges: Vec::new(),
            },
            crate::image::SelectedImageMode::Halfblock,
        );

        app.set_image_mode(crate::image::SelectedImageMode::Sixel);

        assert_eq!(app.image_mode(), crate::image::SelectedImageMode::Sixel);
    }

    #[test]
    fn cycles_annotations_in_current_sentence() {
        let mut annotations = HashMap::new();
        annotations.insert("note-1".to_string(), "First note.".to_string());
        annotations.insert("note-2".to_string(), "Second note.".to_string());
        let mut app = App::new(Document {
            blocks: vec![Block::Text(TextBlock {
                text: "Text [1] and [2].".to_string(),
                chapter_index: 0,
                annotations: vec![
                    AnnotationRef {
                        id: "note-1".to_string(),
                        offset: "Text ".len(),
                    },
                    AnnotationRef {
                        id: "note-2".to_string(),
                        offset: "Text [1] and ".len(),
                    },
                ],
            })],
            toc: Vec::new(),
            annotations,
            chapter_ranges: Vec::new(),
        });

        app.apply(Action::OpenAnnotationOverlay);
        assert_eq!(app.selected_annotation_index(), 0);

        app.apply(Action::CycleAnnotation);
        assert_eq!(app.selected_annotation_index(), 1);

        app.apply(Action::CycleAnnotation);
        assert_eq!(app.selected_annotation_index(), 0);
    }

    #[test]
    fn immersed_annotation_scrolls_and_resets_with_overlay() {
        let mut app = App::new(Document {
            blocks: vec![annotated_text_block(
                "Text with [1].",
                "note-1",
                "Text with ".len(),
            )],
            toc: Vec::new(),
            annotations: annotation_map(
                "note-1",
                "Top note line\nSecond note line\nThird note line",
            ),
            chapter_ranges: Vec::new(),
        });

        app.apply(Action::OpenAnnotationOverlay);
        app.apply(Action::ImmerseAnnotation);
        app.apply(Action::ScrollAnnotationDown);
        app.apply(Action::ScrollAnnotationDown);
        assert_eq!(app.annotation_scroll(), 2);

        app.apply(Action::ScrollAnnotationDown);
        assert_eq!(app.annotation_scroll(), 2);

        app.apply(Action::ScrollAnnotationUp);
        assert_eq!(app.annotation_scroll(), 1);

        app.apply(Action::ScrollAnnotationUp);
        app.apply(Action::ScrollAnnotationUp);
        assert_eq!(app.annotation_scroll(), 0);

        app.apply(Action::ScrollAnnotationDown);
        app.apply(Action::CloseAnnotationOverlay);
        app.apply(Action::OpenAnnotationOverlay);
        assert_eq!(app.annotation_scroll(), 0);
    }

    fn text_block(text: &str) -> Block {
        Block::Text(TextBlock {
            text: text.to_string(),
            chapter_index: 0,
            annotations: Vec::new(),
        })
    }

    fn annotated_text_block(text: &str, id: &str, offset: usize) -> Block {
        Block::Text(TextBlock {
            text: text.to_string(),
            chapter_index: 0,
            annotations: vec![AnnotationRef {
                id: id.to_string(),
                offset,
            }],
        })
    }

    fn annotation_map(id: &str, note: &str) -> HashMap<String, String> {
        let mut annotations = HashMap::new();
        annotations.insert(id.to_string(), note.to_string());
        annotations
    }

    fn image_block() -> Block {
        Block::Image(ImageBlock {
            alt_text: Some("diagram".to_string()),
            source_path: None,
            data: None,
            chapter_index: 0,
        })
    }
}
