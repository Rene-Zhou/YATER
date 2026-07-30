use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use crate::document::Block;
use crate::document::ChapterRange;
use crate::document::Document;
use crate::document::TocNode;
use crate::image::SelectedImageMode;
use crate::input::{Action, Focus};
use crate::progress::Progress;
use crate::sentence::segment_sentences;
use unicode_width::UnicodeWidthStr;

const FAST_SENTENCE_COUNT: usize = 5;
const DEFAULT_ANNOTATION_TEXT_WIDTH: usize = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadingPosition {
    pub block_index: usize,
    pub sentence_offset: usize,
}

#[derive(Debug)]
pub struct App {
    document: Document,
    sentence_ranges_by_block: Vec<Vec<(usize, usize)>>,
    chapter_ranges_by_block: Vec<Option<ChapterRange>>,
    chapter_titles_by_block: Vec<Option<String>>,
    render_cache: RefCell<RenderCache>,
    position: ReadingPosition,
    focus: Focus,
    selected_toc_row: usize,
    toc_scroll_offset: Cell<u16>,
    collapsed_toc_paths: HashSet<Vec<usize>>,
    visible_toc_rows: Vec<VisibleTocRow>,
    image_mode: SelectedImageMode,
    selected_annotation_index: usize,
    annotation_scroll: usize,
    annotation_text_width: usize,
    annotation_viewport_height: usize,
}

impl App {
    pub fn new(document: Document) -> Self {
        Self::with_position_and_image_mode(
            document,
            ReadingPosition {
                block_index: 0,
                sentence_offset: 0,
            },
            SelectedImageMode::Halfblock,
        )
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
        let sentence_ranges_by_block = build_sentence_ranges_by_block(&document);
        Self::from_parts(document, sentence_ranges_by_block, position, image_mode)
    }

    fn from_parts(
        document: Document,
        sentence_ranges_by_block: Vec<Vec<(usize, usize)>>,
        position: ReadingPosition,
        image_mode: SelectedImageMode,
    ) -> Self {
        let chapter_ranges_by_block = build_chapter_ranges_by_block(&document);
        let chapter_titles_by_block = build_chapter_titles_by_block(&document);
        let collapsed_toc_paths = HashSet::new();
        let visible_toc_rows = build_visible_toc_rows(&document.toc, &collapsed_toc_paths);
        Self {
            document,
            sentence_ranges_by_block,
            chapter_ranges_by_block,
            chapter_titles_by_block,
            render_cache: RefCell::new(RenderCache::default()),
            position,
            focus: Focus::Content,
            selected_toc_row: 0,
            toc_scroll_offset: Cell::new(0),
            collapsed_toc_paths,
            visible_toc_rows,
            image_mode,
            selected_annotation_index: 0,
            annotation_scroll: 0,
            annotation_text_width: DEFAULT_ANNOTATION_TEXT_WIDTH,
            annotation_viewport_height: 1,
        }
    }

    pub fn with_restored_progress(document: Document, progress: Option<Progress>) -> Self {
        let sentence_ranges_by_block = build_sentence_ranges_by_block(&document);
        let position = progress
            .filter(|progress| {
                Self::is_valid_reading_position(
                    &document,
                    &sentence_ranges_by_block,
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

        Self::from_parts(
            document,
            sentence_ranges_by_block,
            position,
            SelectedImageMode::Halfblock,
        )
    }

    fn is_valid_reading_position(
        document: &Document,
        sentence_ranges_by_block: &[Vec<(usize, usize)>],
        block_index: usize,
        sentence_offset: usize,
    ) -> bool {
        match document.blocks.get(block_index) {
            Some(Block::Text(_)) => sentence_ranges_by_block
                .get(block_index)
                .is_some_and(|ranges| ranges.iter().any(|range| range.0 == sentence_offset)),
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

    pub(crate) fn sentence_ranges_for_block(&self, block_index: usize) -> &[(usize, usize)] {
        self.sentence_ranges_by_block
            .get(block_index)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(crate) fn chapter_range_for_block(&self, block_index: usize) -> Option<ChapterRange> {
        self.chapter_ranges_by_block
            .get(block_index)
            .copied()
            .flatten()
    }

    pub(crate) fn chapter_title_for_block(&self, block_index: usize) -> Option<&str> {
        self.chapter_titles_by_block
            .get(block_index)
            .and_then(Option::as_deref)
    }

    pub(crate) fn content_row_metrics(
        &self,
        key: ContentRowMetricKey,
        build: impl FnOnce() -> ContentRowMetrics,
    ) -> ContentRowMetrics {
        if let Some(metrics) = self.render_cache.borrow().content_row_metrics.get(&key) {
            return metrics.clone();
        }

        let metrics = build();
        self.render_cache
            .borrow_mut()
            .content_row_metrics
            .insert(key, metrics.clone());
        metrics
    }

    pub(crate) fn halfblock_image_raster(
        &self,
        key: HalfblockImageRasterKey,
        build: impl FnOnce() -> Option<HalfblockImageRaster>,
    ) -> Option<HalfblockImageRaster> {
        if let Some(raster) = self.render_cache.borrow().halfblock_image_rasters.get(&key) {
            return raster.clone();
        }

        let raster = build();
        self.render_cache
            .borrow_mut()
            .halfblock_image_rasters
            .insert(key, raster.clone());
        raster
    }

    pub(crate) fn bitmap_image_is_valid(
        &self,
        block_index: usize,
        validate: impl FnOnce() -> bool,
    ) -> bool {
        if let Some(is_valid) = self
            .render_cache
            .borrow()
            .bitmap_image_validity
            .get(&block_index)
        {
            return *is_valid;
        }

        let is_valid = validate();
        self.render_cache
            .borrow_mut()
            .bitmap_image_validity
            .insert(block_index, is_valid);
        is_valid
    }

    #[cfg(test)]
    pub(crate) fn cached_content_row_metric_count(&self) -> usize {
        self.render_cache.borrow().content_row_metrics.len()
    }

    #[cfg(test)]
    pub(crate) fn cached_halfblock_image_raster_count(&self) -> usize {
        self.render_cache.borrow().halfblock_image_rasters.len()
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    pub fn selected_toc_row(&self) -> usize {
        self.selected_toc_row
    }

    pub(crate) fn toc_scroll_offset(&self) -> u16 {
        self.toc_scroll_offset.get()
    }

    pub(crate) fn set_toc_scroll_offset(&self, offset: u16) {
        self.toc_scroll_offset.set(offset);
    }

    pub(crate) fn visible_toc_rows(&self) -> &[VisibleTocRow] {
        &self.visible_toc_rows
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

    pub fn set_terminal_width(&mut self, terminal_width: u16) {
        self.annotation_text_width = usize::from(terminal_width.min(50).saturating_sub(2).max(1));
    }

    pub fn set_terminal_size(&mut self, terminal_width: u16, terminal_height: u16) {
        self.set_terminal_width(terminal_width);
        self.annotation_viewport_height = usize::from(terminal_height.saturating_sub(3).max(1));
        let max_scroll = self
            .current_annotation_line_count()
            .saturating_sub(self.annotation_viewport_height);
        self.annotation_scroll = self.annotation_scroll.min(max_scroll);
    }

    pub fn is_toc_path_collapsed(&self, path: &[usize]) -> bool {
        self.collapsed_toc_paths
            .iter()
            .any(|collapsed_path| collapsed_path == path)
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
            Action::FastNextSentence => self.fast_next_sentence(),
            Action::FastPreviousSentence => self.fast_previous_sentence(),
            Action::JumpToChapterStart => self.jump_to_chapter_start(),
            Action::JumpToChapterEnd => self.jump_to_chapter_end(),
            Action::OpenToc => {
                self.focus = Focus::Toc;
                self.selected_toc_row = self.toc_row_for_current_position().unwrap_or(0);
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
                    && self.current_annotation_line_count() > 1
                {
                    self.focus = Focus::AnnotationImmersed;
                    self.annotation_scroll = 0;
                }
            }
            Action::ExitAnnotationImmersion => {
                self.focus = Focus::AnnotationOverlay;
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
        if self
            .document
            .text_block(self.position.block_index)
            .is_none()
        {
            self.advance_to_next_reading_block();
            return;
        }

        let ranges = self.sentence_ranges_for_block(self.position.block_index);
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
        if self
            .document
            .text_block(self.position.block_index)
            .is_none()
        {
            self.retreat_to_previous_reading_block();
            return;
        }

        let ranges = self.sentence_ranges_for_block(self.position.block_index);
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

    fn fast_next_sentence(&mut self) {
        for _ in 0..FAST_SENTENCE_COUNT {
            self.next_sentence();
        }
    }

    fn fast_previous_sentence(&mut self) {
        for _ in 0..FAST_SENTENCE_COUNT {
            self.previous_sentence();
        }
    }

    fn retreat_to_previous_reading_block(&mut self) {
        let Some(block_index) = self.position.block_index.checked_sub(1) else {
            return;
        };
        let sentence_offset = match &self.document.blocks[block_index] {
            Block::Text(_) => self
                .sentence_ranges_for_block(block_index)
                .iter()
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
        let Some(range) = self.chapter_range_for_block(self.position.block_index) else {
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
        let Some(range) = self.chapter_range_for_block(self.position.block_index) else {
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
        (range.start_block < self.document.blocks.len() && range.start_block <= range.end_block)
            .then_some(range.start_block)
    }

    fn last_reading_position_in_range(&self, range: ChapterRange) -> Option<(usize, usize)> {
        if range.start_block > range.end_block || range.end_block >= self.document.blocks.len() {
            return None;
        }

        let sentence_offset = match &self.document.blocks[range.end_block] {
            Block::Text(_) => self
                .sentence_ranges_for_block(range.end_block)
                .iter()
                .last()
                .map(|sentence| sentence.0)
                .unwrap_or(0),
            Block::Image(_) => 0,
        };

        Some((range.end_block, sentence_offset))
    }

    fn next_toc_item(&mut self) {
        let last_row = self.visible_toc_rows.len().saturating_sub(1);
        self.selected_toc_row = (self.selected_toc_row + 1).min(last_row);
    }

    fn previous_toc_item(&mut self) {
        self.selected_toc_row = self.selected_toc_row.saturating_sub(1);
    }

    fn activate_selected_toc_item(&mut self) {
        let Some(row) = self.visible_toc_rows.get(self.selected_toc_row).cloned() else {
            return;
        };

        if row.has_children && self.collapsed_toc_paths.remove(&row.path) {
            self.refresh_visible_toc_rows();
            return;
        }

        self.position = ReadingPosition {
            block_index: row.target_block_index,
            sentence_offset: 0,
        };
        self.focus = Focus::Content;
    }

    fn collapse_or_select_parent_toc_item(&mut self) {
        let Some(row) = self.visible_toc_rows.get(self.selected_toc_row).cloned() else {
            return;
        };

        if row.has_children && !self.collapsed_toc_paths.contains(&row.path) {
            self.collapsed_toc_paths.insert(row.path);
            self.refresh_visible_toc_rows();
            self.selected_toc_row = self
                .selected_toc_row
                .min(self.visible_toc_rows.len().saturating_sub(1));
            return;
        }

        let Some(parent_path) = row.path.get(..row.path.len().saturating_sub(1)) else {
            return;
        };
        if parent_path.is_empty() {
            return;
        }

        if let Some(parent_row) = self
            .visible_toc_rows
            .iter()
            .position(|visible_row| visible_row.path == parent_path)
        {
            self.selected_toc_row = parent_row;
        }
    }

    fn refresh_visible_toc_rows(&mut self) {
        self.visible_toc_rows =
            build_visible_toc_rows(&self.document.toc, &self.collapsed_toc_paths);
    }

    fn toc_row_for_current_position(&self) -> Option<usize> {
        let rows = self.visible_toc_rows();
        if rows.is_empty() {
            return None;
        }

        if let Some(index) = rows
            .iter()
            .position(|row| row.target_block_index == self.position.block_index)
        {
            return Some(index);
        }

        rows.iter()
            .enumerate()
            .filter(|(_, row)| row.target_block_index <= self.position.block_index)
            .max_by_key(|(index, row)| (row.target_block_index, *index))
            .map(|(index, _)| index)
            .or(Some(0))
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
        let max_scroll = self
            .current_annotation_line_count()
            .saturating_sub(self.annotation_viewport_height);
        self.annotation_scroll = (self.annotation_scroll + 1).min(max_scroll);
    }

    fn current_annotation_line_count(&self) -> usize {
        let annotation_count = self.current_sentence_annotation_count();
        let prefix_width = if annotation_count > 1 {
            UnicodeWidthStr::width(
                format!(
                    "[{}/{}] ",
                    self.selected_annotation_index + 1,
                    annotation_count
                )
                .as_str(),
            )
        } else {
            0
        };

        self.current_annotation_text()
            .map(|text| {
                annotation_display_line_count(text, self.annotation_text_width, prefix_width)
            })
            .unwrap_or(0)
    }

    fn current_annotation_text(&self) -> Option<&str> {
        let block = self.document.text_block(self.position.block_index)?;
        let sentence_range = self
            .sentence_ranges_for_block(self.position.block_index)
            .iter()
            .copied()
            .find(|range| range.0 == self.position.sentence_offset)?;
        let annotation_text = block
            .annotations
            .iter()
            .filter(|annotation_ref| {
                sentence_range.0 <= annotation_ref.offset
                    && annotation_ref.offset < sentence_range.1
            })
            .filter_map(|annotation_ref| self.document.annotation_text(&annotation_ref.id))
            .nth(self.selected_annotation_index)?;

        Some(annotation_text)
    }

    fn current_sentence_annotation_count(&self) -> usize {
        let Some(block) = self.document.text_block(self.position.block_index) else {
            return 0;
        };
        let Some(sentence_range) = self
            .sentence_ranges_for_block(self.position.block_index)
            .iter()
            .copied()
            .find(|range| range.0 == self.position.sentence_offset)
        else {
            return 0;
        };

        block
            .annotations
            .iter()
            .filter(|annotation_ref| {
                sentence_range.0 <= annotation_ref.offset
                    && annotation_ref.offset < sentence_range.1
            })
            .filter(|annotation_ref| self.document.annotation_text(&annotation_ref.id).is_some())
            .count()
    }
}

#[derive(Debug, Default)]
struct RenderCache {
    content_row_metrics: HashMap<ContentRowMetricKey, ContentRowMetrics>,
    halfblock_image_rasters: HashMap<HalfblockImageRasterKey, Option<HalfblockImageRaster>>,
    bitmap_image_validity: HashMap<usize, bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ContentRowMetricKey {
    pub chapter_start: usize,
    pub chapter_end: usize,
    pub width: u16,
    pub height: u16,
    pub image_mode: SelectedImageMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContentRowMetrics {
    pub block_rows: Vec<u16>,
    pub total_rows: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct HalfblockImageRasterKey {
    pub block_index: usize,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HalfblockImageRaster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

fn build_sentence_ranges_by_block(document: &Document) -> Vec<Vec<(usize, usize)>> {
    document
        .blocks
        .iter()
        .map(|block| match block {
            Block::Text(block) => segment_sentences(&block.text),
            Block::Image(_) => Vec::new(),
        })
        .collect()
}

fn build_chapter_ranges_by_block(document: &Document) -> Vec<Option<ChapterRange>> {
    let mut ranges_by_block = vec![None; document.blocks.len()];

    for range in &document.chapter_ranges {
        let end_block = range.end_block.min(document.blocks.len().saturating_sub(1));
        if range.start_block > end_block {
            continue;
        }

        for slot in &mut ranges_by_block[range.start_block..=end_block] {
            *slot = Some(*range);
        }
    }

    ranges_by_block
}

fn build_chapter_titles_by_block(document: &Document) -> Vec<Option<String>> {
    (0..document.blocks.len())
        .map(|block_index| {
            document
                .chapter_title_for_block(block_index)
                .map(str::to_string)
        })
        .collect()
}

fn annotation_display_line_count(
    text: &str,
    width: usize,
    first_line_prefix_width: usize,
) -> usize {
    text.lines()
        .enumerate()
        .map(|(index, line)| {
            (UnicodeWidthStr::width(line)
                + if index == 0 {
                    first_line_prefix_width
                } else {
                    0
                })
            .max(1)
            .div_ceil(width.max(1))
        })
        .sum::<usize>()
        .max(1)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VisibleTocRow {
    pub path: Vec<usize>,
    pub target_block_index: usize,
    pub has_children: bool,
    pub label: String,
}

fn build_visible_toc_rows(
    toc: &[TocNode],
    collapsed_paths: &HashSet<Vec<usize>>,
) -> Vec<VisibleTocRow> {
    let mut rows = Vec::new();

    for (index, node) in toc.iter().enumerate() {
        append_toc_rows(
            node,
            vec![index],
            collapsed_paths,
            index + 1 == toc.len(),
            "",
            false,
            &mut rows,
        );
    }

    rows
}

fn append_toc_rows(
    node: &TocNode,
    path: Vec<usize>,
    collapsed_paths: &HashSet<Vec<usize>>,
    is_last: bool,
    prefix: &str,
    show_branch: bool,
    rows: &mut Vec<VisibleTocRow>,
) {
    let marker = if node.children.is_empty() {
        ""
    } else if collapsed_paths.contains(&path) {
        "▸ "
    } else {
        "▾ "
    };
    let branch = if !show_branch {
        String::new()
    } else if is_last {
        "└ ".to_string()
    } else {
        "├ ".to_string()
    };
    rows.push(VisibleTocRow {
        path: path.clone(),
        target_block_index: node.target_block_index,
        has_children: !node.children.is_empty(),
        label: format!("{prefix}{branch}{marker}{}", node.title),
    });

    if collapsed_paths.contains(&path) {
        return;
    }

    let child_prefix = if !show_branch {
        prefix.to_string()
    } else if is_last {
        format!("{prefix}  ")
    } else {
        format!("{prefix}│ ")
    };

    for (index, child) in node.children.iter().enumerate() {
        let mut child_path = path.clone();
        child_path.push(index);
        append_toc_rows(
            child,
            child_path,
            collapsed_paths,
            index + 1 == node.children.len(),
            &child_prefix,
            true,
            rows,
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::document::{
        AnnotationRef, Block, ChapterRange, Document, ImageBlock, TextBlock, TocNode,
    };
    use crate::input::{Action, Focus};
    use crate::progress::Progress;
    use std::collections::HashMap;

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
    fn precomputes_sentence_ranges_for_text_blocks() {
        let app = App::new(Document {
            blocks: vec![
                text_block("First. Second?"),
                image_block(),
                text_block("第三句。"),
            ],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        });

        assert_eq!(
            app.sentence_ranges_for_block(0),
            &[
                (0, "First.".len()),
                ("First.".len(), "First. Second?".len())
            ]
        );
        assert_eq!(app.sentence_ranges_for_block(1), &[]);
        assert_eq!(app.sentence_ranges_for_block(2), &[(0, "第三句。".len())]);
    }

    #[test]
    fn precomputes_chapter_lookup_indexes() {
        let app = App::new(Document {
            blocks: vec![
                text_block("Chapter."),
                text_block("Section."),
                text_block("Next chapter."),
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
        });

        assert_eq!(
            app.chapter_range_for_block(1),
            Some(ChapterRange {
                start_block: 0,
                end_block: 1
            })
        );
        assert_eq!(app.chapter_title_for_block(1), Some("Section One"));
        assert_eq!(app.chapter_title_for_block(2), Some("Chapter Two"));
    }

    #[test]
    fn caches_visible_toc_rows_and_refreshes_after_collapse() {
        let mut app = App::new(Document {
            blocks: vec![text_block("Chapter."), text_block("Section.")],
            toc: vec![TocNode {
                title: "Chapter One".to_string(),
                target_block_index: 0,
                children: vec![TocNode {
                    title: "Section One".to_string(),
                    target_block_index: 1,
                    children: Vec::new(),
                }],
            }],
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        });

        assert_eq!(app.visible_toc_rows().len(), 2);
        assert_eq!(app.visible_toc_rows()[1].label, "└ Section One");

        app.apply(Action::OpenToc);
        app.apply(Action::CollapseOrParentToc);

        assert_eq!(app.visible_toc_rows().len(), 1);
        assert_eq!(app.visible_toc_rows()[0].label, "▸ Chapter One");
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
    fn fast_navigation_moves_by_five_sentences() {
        let mut app = App::new(Document {
            blocks: vec![text_block(
                "One. Two. Three. Four. Five. Six. Seven. Eight. Nine. Ten. Eleven. Twelve.",
            )],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        });

        app.apply(Action::FastNextSentence);
        assert_eq!(
            app.position(),
            ReadingPosition {
                block_index: 0,
                sentence_offset: "One. Two. Three. Four. Five.".len(),
            }
        );

        app.apply(Action::FastPreviousSentence);
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
            annotations: annotation_map("note-1", "First note line\nSecond note line"),
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
        assert_eq!(app.focus(), Focus::AnnotationOverlay);

        app.apply(Action::CloseAnnotationOverlay);
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
    fn annotation_overlay_ignores_refs_without_note_text() {
        let mut app = App::new(Document {
            blocks: vec![annotated_text_block(
                "Text with [1].",
                "missing-note",
                "Text with ".len(),
            )],
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
    fn opening_toc_selects_current_reading_chapter() {
        let mut app = App::with_position(
            Document {
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
            },
            ReadingPosition {
                block_index: 1,
                sentence_offset: 0,
            },
        );

        app.apply(Action::OpenToc);

        assert_eq!(app.selected_toc_row(), 1);
    }

    #[test]
    fn opening_toc_selects_visible_parent_when_current_section_is_collapsed() {
        let mut app = App::with_position(
            Document {
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
            },
            ReadingPosition {
                block_index: 1,
                sentence_offset: 0,
            },
        );

        app.apply(Action::OpenToc);
        app.apply(Action::CollapseOrParentToc);
        app.apply(Action::CollapseOrParentToc);
        app.apply(Action::CloseToc);
        app.apply(Action::OpenToc);

        assert_eq!(app.selected_toc_row(), 0);
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
                presentation: Default::default(),
                styles: Vec::new(),
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
            presentation: Default::default(),
            styles: Vec::new(),
            annotations: Vec::new(),
        })
    }

    fn annotated_text_block(text: &str, id: &str, offset: usize) -> Block {
        Block::Text(TextBlock {
            text: text.to_string(),
            chapter_index: 0,
            presentation: Default::default(),
            styles: Vec::new(),
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
