use crate::app::App;
use crate::document::{Block, TocNode};
use crate::image::SelectedImageMode;
use crate::input::Focus;
use crate::sentence::segment_sentences;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

pub fn draw(frame: &mut ratatui::Frame<'_>, app: &App) {
    let area = frame.area();
    if area.width < 20 || area.height < 3 {
        frame.render_widget(Paragraph::new("Terminal too small").centered(), area);
        return;
    }

    let [top_bar, content] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .areas(area);

    let document = app.document();
    let position = app.position();
    let chapter_title = document
        .chapter_title_for_block(position.block_index)
        .unwrap_or("");
    frame.render_widget(Paragraph::new(chapter_title).centered(), top_bar);

    frame.render_widget(Paragraph::new(current_content_line(app)), content);

    if app.focus() == Focus::Toc {
        draw_toc(frame, content, app);
    }

    if matches!(
        app.focus(),
        Focus::AnnotationOverlay | Focus::AnnotationImmersed
    ) {
        draw_annotation_overlay(frame, content, app);
    }
}

fn current_content_line(app: &App) -> Line<'static> {
    let document = app.document();
    let position = app.position();

    match document.blocks.get(position.block_index) {
        Some(Block::Text(block)) => {
            let current_sentence = segment_sentences(&block.text)
                .into_iter()
                .find(|range| range.0 == position.sentence_offset)
                .map(|range| block.text[range.0..range.1].to_string())
                .unwrap_or_default();

            Line::from(Span::styled(
                current_sentence,
                Style::default().add_modifier(Modifier::REVERSED),
            ))
        }
        Some(Block::Image(image)) => {
            let label = image.alt_text.as_deref().unwrap_or("untitled");
            match app.image_mode() {
                SelectedImageMode::Off => Line::from(format!("[image disabled: {label}]")),
                SelectedImageMode::Sixel | SelectedImageMode::Halfblock => {
                    Line::from(format!("[image: {label}]"))
                }
            }
        }
        None => Line::default(),
    }
}

fn draw_toc(frame: &mut ratatui::Frame<'_>, content: Rect, app: &App) {
    let width = (content.width / 3).max(20).min(content.width);
    let area = Rect {
        x: content.x,
        y: content.y,
        width,
        height: content.height,
    };
    let mut rows = Vec::new();
    let toc = app.document().toc.as_slice();

    for (index, node) in toc.iter().enumerate() {
        append_toc_rows(
            node,
            &[index],
            app,
            index + 1 == toc.len(),
            "",
            false,
            &mut rows,
        );
    }

    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(rows), area);
}

fn append_toc_rows<'a>(
    node: &'a TocNode,
    path: &[usize],
    app: &App,
    is_last: bool,
    prefix: &str,
    show_branch: bool,
    rows: &mut Vec<Line<'a>>,
) {
    let is_collapsed = app.is_toc_path_collapsed(path);
    let marker = if node.children.is_empty() {
        ""
    } else if is_collapsed {
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
    rows.push(Line::from(format!("{prefix}{branch}{marker}{}", node.title)));

    let child_prefix = if prefix.is_empty() {
        String::new()
    } else if is_last {
        format!("{prefix}  ")
    } else {
        format!("{prefix}│ ")
    };

    if is_collapsed {
        return;
    }

    for (index, child) in node.children.iter().enumerate() {
        let mut child_path = path.to_vec();
        child_path.push(index);
        append_toc_rows(
            child,
            &child_path,
            app,
            index + 1 == node.children.len(),
            &child_prefix,
            true,
            rows,
        );
    }
}

fn draw_annotation_overlay(frame: &mut ratatui::Frame<'_>, content: Rect, app: &App) {
    let Some(annotation) = current_annotation(app) else {
        return;
    };
    let is_immersed = app.focus() == Focus::AnnotationImmersed;
    let height = if is_immersed {
        content.height
    } else {
        3.min(content.height)
    };
    let width = content.width.min(50);
    let area = Rect {
        x: content.x,
        y: content.y,
        width,
        height,
    };
    let annotation_text = if is_immersed {
        annotation.scrolled_text(app.annotation_scroll())
    } else {
        annotation.display_text()
    };

    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(annotation_text), area);
}

struct VisibleAnnotation {
    text: String,
    index: usize,
    total: usize,
}

impl VisibleAnnotation {
    fn display_text(&self) -> String {
        if self.total > 1 {
            format!("[{}/{}] {}", self.index + 1, self.total, self.text)
        } else {
            self.text.clone()
        }
    }

    fn scrolled_text(&self, scroll: usize) -> String {
        self.display_text()
            .lines()
            .skip(scroll)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn current_annotation(app: &App) -> Option<VisibleAnnotation> {
    let document = app.document();
    let position = app.position();
    let block = document.text_block(position.block_index)?;
    let sentence_range = segment_sentences(&block.text)
        .into_iter()
        .find(|range| range.0 == position.sentence_offset)?;
    let annotation_refs = block
        .annotations
        .iter()
        .filter(|annotation_ref| {
            sentence_range.0 <= annotation_ref.offset && annotation_ref.offset < sentence_range.1
        })
        .collect::<Vec<_>>();
    let total = annotation_refs.len();
    let index = app.selected_annotation_index().min(total.saturating_sub(1));
    let annotation_ref = annotation_refs.get(index)?;

    document.annotation_text(&annotation_ref.id).map(|text| VisibleAnnotation {
        text: text.to_string(),
        index,
        total,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use crate::app::{App, ReadingPosition};
    use crate::document::{
        AnnotationRef, Block, ChapterRange, Document, ImageBlock, TextBlock, TocNode,
    };
    use crate::input::Action;

    use super::draw;

    #[test]
    fn renders_top_bar_and_current_sentence() {
        let document = test_document();
        let app = App::with_position(
            document,
            ReadingPosition {
                block_index: 0,
                sentence_offset: "First sentence.".len(),
            },
        );
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("Chapter One"));
        assert!(rendered.contains("Second sentence."));
    }

    #[test]
    fn renders_terminal_too_small_message() {
        let app = App::new(test_document());
        let backend = TestBackend::new(24, 2);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("Terminal too small"));
    }

    #[test]
    fn renders_toc_sidebar_when_toc_has_focus() {
        let mut app = App::new(test_document());
        app.apply(Action::OpenToc);
        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("▾ Chapter One"));
        assert!(rendered.contains("└ Section One"));
    }

    #[test]
    fn renders_collapsed_toc_parent_without_children() {
        let mut app = App::new(test_document());
        app.apply(Action::OpenToc);
        app.apply(Action::CollapseOrParentToc);
        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("▸ Chapter One"));
        assert!(!rendered.contains("Section One"));
    }

    #[test]
    fn renders_annotation_overlay_for_current_sentence() {
        let mut annotations = HashMap::new();
        annotations.insert("note-1".to_string(), "Footnote text.".to_string());
        let document = Document {
            blocks: vec![Block::Text(TextBlock {
                text: "Text with [1].".to_string(),
                chapter_index: 0,
                annotations: vec![AnnotationRef {
                    id: "note-1".to_string(),
                    offset: "Text with ".len(),
                }],
            })],
            toc: vec![TocNode {
                title: "Chapter One".to_string(),
                target_block_index: 0,
                children: Vec::new(),
            }],
            annotations,
            chapter_ranges: vec![ChapterRange {
                start_block: 0,
                end_block: 0,
            }],
        };
        let mut app = App::new(document);
        app.apply(Action::OpenAnnotationOverlay);
        let backend = TestBackend::new(50, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("Footnote text."));
    }

    #[test]
    fn renders_cycled_annotation_with_counter() {
        let mut annotations = HashMap::new();
        annotations.insert("note-1".to_string(), "First note.".to_string());
        annotations.insert("note-2".to_string(), "Second note.".to_string());
        let document = Document {
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
        };
        let mut app = App::new(document);
        app.apply(Action::OpenAnnotationOverlay);
        app.apply(Action::CycleAnnotation);
        let backend = TestBackend::new(50, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("[2/2]"));
        assert!(rendered.contains("Second note."));
    }

    #[test]
    fn renders_scrolled_immersed_annotation() {
        let mut annotations = HashMap::new();
        annotations.insert(
            "note-1".to_string(),
            "Top note line\nVisible note line\nAnother visible line".to_string(),
        );
        let document = Document {
            blocks: vec![Block::Text(TextBlock {
                text: "Text with [1].".to_string(),
                chapter_index: 0,
                annotations: vec![AnnotationRef {
                    id: "note-1".to_string(),
                    offset: "Text with ".len(),
                }],
            })],
            toc: Vec::new(),
            annotations,
            chapter_ranges: Vec::new(),
        };
        let mut app = App::new(document);
        app.apply(Action::OpenAnnotationOverlay);
        app.apply(Action::ImmerseAnnotation);
        app.apply(Action::ScrollAnnotationDown);
        let backend = TestBackend::new(50, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(!rendered.contains("Top note line"));
        assert!(rendered.contains("Visible note line"));
        assert!(rendered.contains("Another visible line"));
    }

    #[test]
    fn renders_image_block_placeholder_with_alt_text() {
        let document = Document {
            blocks: vec![Block::Image(ImageBlock {
                alt_text: Some("Map of routes".to_string()),
                source_path: None,
                data: None,
                chapter_index: 0,
            })],
            toc: vec![TocNode {
                title: "Illustrations".to_string(),
                target_block_index: 0,
                children: Vec::new(),
            }],
            annotations: HashMap::new(),
            chapter_ranges: vec![ChapterRange {
                start_block: 0,
                end_block: 0,
            }],
        };
        let app = App::new(document);
        let backend = TestBackend::new(50, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("[image: Map of routes]"));
    }

    #[test]
    fn renders_disabled_image_placeholder_when_image_mode_is_off() {
        let document = Document {
            blocks: vec![Block::Image(ImageBlock {
                alt_text: Some("Map of routes".to_string()),
                source_path: None,
                data: None,
                chapter_index: 0,
            })],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        };
        let app = App::with_image_mode(document, crate::image::SelectedImageMode::Off);
        let backend = TestBackend::new(50, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("[image disabled: Map of routes]"));
    }

    fn test_document() -> Document {
        Document {
            blocks: vec![Block::Text(TextBlock {
                text: "First sentence. Second sentence.".to_string(),
                chapter_index: 0,
                annotations: Vec::new(),
            })],
            toc: vec![TocNode {
                title: "Chapter One".to_string(),
                target_block_index: 0,
                children: vec![TocNode {
                    title: "Section One".to_string(),
                    target_block_index: 0,
                    children: Vec::new(),
                }],
            }],
            annotations: HashMap::new(),
            chapter_ranges: vec![ChapterRange {
                start_block: 0,
                end_block: 0,
            }],
        }
    }

    fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
        buffer
            .content()
            .chunks(buffer.area.width as usize)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
}
