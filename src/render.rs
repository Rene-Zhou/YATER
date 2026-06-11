use crate::app::App;
use crate::document::{Block, TocNode};
use crate::image::SelectedImageMode;
use crate::input::Focus;
use crate::sentence::segment_sentences;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as WidgetBlock, Borders, Clear, Paragraph, Wrap};
use ratatui_image::{Image as TerminalImage, Resize};
use unicode_width::UnicodeWidthChar;

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
    let (reading_content, annotation_area) = annotation_layout(app, content);

    let document = app.document();
    let position = app.position();
    let chapter_title = document
        .chapter_title_for_block(position.block_index)
        .unwrap_or("");
    frame.render_widget(Paragraph::new(chapter_title).centered(), top_bar);

    if !draw_current_image(frame, reading_content, app) {
        let scroll = content_scroll_offset(app, reading_content);
        frame.render_widget(
            Paragraph::new(current_content_lines(app, reading_content))
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0)),
            reading_content,
        );
    }

    if app.focus() == Focus::Toc {
        draw_toc(frame, content, app);
    }

    if matches!(
        app.focus(),
        Focus::AnnotationOverlay | Focus::AnnotationImmersed
    ) {
        draw_annotation_overlay(frame, annotation_area, app);
    }
}

fn current_content_lines(app: &App, content: Rect) -> Vec<Line<'static>> {
    let document = app.document();
    let position = app.position();
    let chapter_range = document
        .chapter_range_for_block(position.block_index)
        .map(|range| range.start_block..=range.end_block)
        .unwrap_or(position.block_index..=position.block_index);
    let mut lines = Vec::new();

    for block_index in chapter_range {
        match document.blocks.get(block_index) {
            Some(Block::Text(block)) => {
                let mut block_lines = vec![Vec::new()];
                let highlighted_sentence_offset =
                    (block_index == position.block_index).then_some(position.sentence_offset);
                let mut cursor = 0;
                for range in segment_sentences(&block.text) {
                    if cursor < range.0 {
                        push_text_span_lines(
                            &mut block_lines,
                            block.text[cursor..range.0].to_string(),
                            Style::default(),
                        );
                    }

                    let style = if highlighted_sentence_offset == Some(range.0) {
                        Style::default().add_modifier(Modifier::REVERSED)
                    } else {
                        Style::default()
                    };
                    push_sentence_span_lines(
                        &mut block_lines,
                        block,
                        range,
                        style,
                        document,
                    );
                    cursor = range.1;
                }

                if cursor < block.text.len() {
                    push_text_span_lines(
                        &mut block_lines,
                        block.text[cursor..].to_string(),
                        Style::default(),
                    );
                }

                lines.extend(block_lines.into_iter().map(Line::from));
            }
            Some(Block::Image(image)) => {
                lines.extend(image_content_lines(image, app.image_mode(), content));
            }
            None => {}
        }
    }

    if lines.is_empty() {
        lines.push(Line::default());
    }

    lines
}

fn push_sentence_span_lines(
    lines: &mut Vec<Vec<Span<'static>>>,
    block: &crate::document::TextBlock,
    sentence_range: (usize, usize),
    base_style: Style,
    document: &crate::document::Document,
) {
    let mut cursor = sentence_range.0;
    let mut annotation_refs = block
        .annotations
        .iter()
        .filter(|annotation_ref| {
            sentence_range.0 <= annotation_ref.offset
                && annotation_ref.offset < sentence_range.1
                && document.annotation_text(&annotation_ref.id).is_some()
        })
        .collect::<Vec<_>>();
    annotation_refs.sort_by_key(|annotation_ref| annotation_ref.offset);

    for annotation_ref in annotation_refs {
        if annotation_ref.offset < cursor {
            continue;
        }
        if cursor < annotation_ref.offset {
            push_text_span_lines(
                lines,
                block.text[cursor..annotation_ref.offset].to_string(),
                base_style,
            );
        }

        let marker_end =
            annotation_marker_end(&block.text, annotation_ref.offset, sentence_range.1);
        push_text_span_lines(
            lines,
            block.text[annotation_ref.offset..marker_end].to_string(),
            base_style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        );
        cursor = marker_end;
    }

    if cursor < sentence_range.1 {
        push_text_span_lines(
            lines,
            block.text[cursor..sentence_range.1].to_string(),
            base_style,
        );
    }
}

fn annotation_marker_end(text: &str, start: usize, limit: usize) -> usize {
    let marker = &text[start..limit];
    let mut characters = marker.char_indices();
    let Some((_, first)) = characters.next() else {
        return start;
    };

    if first.is_ascii_digit() {
        return start
            + marker
                .find(|character: char| !character.is_ascii_digit())
                .unwrap_or(marker.len());
    }

    if matches!(first, '[' | '(' | '［' | '（') {
        let closing = match first {
            '[' => ']',
            '(' => ')',
            '［' => '］',
            '（' => '）',
            _ => unreachable!(),
        };
        if let Some((index, character)) = characters.find(|(_, character)| *character == closing) {
            return start + index + character.len_utf8();
        }
    }

    if is_super_or_subscript_digit(first) {
        let relative_end = characters
            .find(|(_, character)| !is_super_or_subscript_digit(*character))
            .map(|(index, _)| index)
            .unwrap_or(marker.len());
        return start + relative_end;
    }

    start + first.len_utf8()
}

fn is_super_or_subscript_digit(character: char) -> bool {
    matches!(
        character,
        '⁰' | '¹'
            | '²'
            | '³'
            | '⁴'
            | '⁵'
            | '⁶'
            | '⁷'
            | '⁸'
            | '⁹'
            | '₀'..='₉'
    )
}

fn image_content_lines(
    image: &crate::document::ImageBlock,
    image_mode: SelectedImageMode,
    content: Rect,
) -> Vec<Line<'static>> {
    let label = image.alt_text.as_deref().unwrap_or("untitled");
    match image_mode {
        SelectedImageMode::Off => vec![Line::from(format!("[image disabled: {label}]"))],
        SelectedImageMode::Kitty | SelectedImageMode::Iterm2 | SelectedImageMode::Sixel
            if image.data.is_none() && image.source_path.is_some() =>
        {
            vec![Line::from(format!("[image unavailable: {label}]"))]
        }
        SelectedImageMode::Kitty | SelectedImageMode::Iterm2 | SelectedImageMode::Sixel
            if image
                .data
                .as_deref()
                .is_some_and(|data| ::image::load_from_memory(data).is_err()) =>
        {
            vec![Line::from(format!("[image unavailable: {label}]"))]
        }
        SelectedImageMode::Kitty | SelectedImageMode::Iterm2 | SelectedImageMode::Sixel => {
            vec![Line::from(format!("[image: {label}]"))]
        }
        SelectedImageMode::Halfblock => match image.data.as_deref() {
            Some(data) => render_halfblock_image(data, content.width, content.height)
                .unwrap_or_else(|| vec![Line::from(format!("[image unavailable: {label}]"))]),
            None if image.source_path.is_some() => {
                vec![Line::from(format!("[image unavailable: {label}]"))]
            }
            None => vec![Line::from(format!("[image: {label}]"))],
        },
    }
}

fn text_block_screen_rows(text: &str, width: u16) -> u16 {
    let width = usize::from(width.max(1));
    let mut rows = 1usize;
    let mut column = 0usize;

    for character in text.chars() {
        if character == '\n' {
            rows += 1;
            column = 0;
            continue;
        }

        let character_width = character.width().unwrap_or(0);
        if character_width == 0 {
            continue;
        }

        if column + character_width > width {
            rows += 1;
            column = 0;
        }

        column += character_width;
        if column == width {
            column = 0;
            rows += 1;
        }
    }

    if column == 0 && rows > 1 && !text.ends_with('\n') {
        rows -= 1;
    }

    rows.min(u16::MAX as usize) as u16
}

fn push_text_span_lines(lines: &mut Vec<Vec<Span<'static>>>, text: String, style: Style) {
    let mut parts = text.split('\n').peekable();

    while let Some(part) = parts.next() {
        if !part.is_empty() {
            if let Some(line) = lines.last_mut() {
                line.push(Span::styled(part.to_string(), style));
            }
        }

        if parts.peek().is_some() {
            lines.push(Vec::new());
        }
    }
}

fn draw_current_image(frame: &mut ratatui::Frame<'_>, content: Rect, app: &App) -> bool {
    let document = app.document();
    let position = app.position();
    let Some(Block::Image(image)) = document.blocks.get(position.block_index) else {
        return false;
    };
    let Some(protocol_type) = app.image_mode().protocol_type() else {
        return false;
    };
    let Some(data) = image.data.as_deref() else {
        return false;
    };

    let Ok(decoded_image) = ::image::load_from_memory(data) else {
        return false;
    };
    let mut picker = ratatui_image::picker::Picker::halfblocks();
    picker.set_protocol_type(protocol_type);
    let Ok(protocol) = picker.new_protocol(decoded_image, content.as_size(), Resize::Fit(None))
    else {
        return false;
    };

    frame.render_widget(TerminalImage::new(&protocol).allow_clipping(true), content);
    true
}

fn render_halfblock_image(
    data: &[u8],
    terminal_width: u16,
    terminal_height: u16,
) -> Option<Vec<Line<'static>>> {
    if terminal_width == 0 || terminal_height == 0 {
        return None;
    }

    let max_width = terminal_width as u32;
    let max_height = (terminal_height as u32).saturating_mul(2);
    let decoded = ::image::load_from_memory(data).ok()?;
    let image = if decoded.width() > max_width || decoded.height() > max_height {
        decoded.resize(
            max_width,
            max_height,
            ::image::imageops::FilterType::Nearest,
        )
    } else {
        decoded
    }
    .to_rgba8();
    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return None;
    }

    let mut lines = Vec::new();
    for y in (0..height).step_by(2) {
        let mut spans = Vec::new();
        for x in 0..width {
            let top = image.get_pixel(x, y);
            let bottom = if y + 1 < height {
                image.get_pixel(x, y + 1)
            } else {
                top
            };
            spans.push(Span::styled(
                "▀",
                Style::default()
                    .fg(rgba_to_color(top.0))
                    .bg(rgba_to_color(bottom.0)),
            ));
        }
        lines.push(Line::from(spans));
    }

    Some(lines)
}

fn rgba_to_color([red, green, blue, alpha]: [u8; 4]) -> Color {
    if alpha == 0 {
        Color::Reset
    } else {
        Color::Rgb(red, green, blue)
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
    let scroll = toc_scroll_offset(app.selected_toc_row(), area.height);
    frame.render_widget(Paragraph::new(rows).scroll((scroll, 0)), area);
}

fn toc_scroll_offset(selected_row: usize, visible_height: u16) -> u16 {
    let visible_height = visible_height as usize;
    if visible_height == 0 || selected_row < visible_height {
        0
    } else {
        (selected_row + 1 - visible_height) as u16
    }
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
    let mut row = Line::from(format!("{prefix}{branch}{marker}{}", node.title));
    if rows.len() == app.selected_toc_row() {
        row = row.style(Style::default().add_modifier(Modifier::REVERSED));
    }
    rows.push(row);

    let child_prefix = if !show_branch {
        prefix.to_string()
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

fn annotation_layout(app: &App, content: Rect) -> (Rect, Rect) {
    if app.focus() != Focus::AnnotationOverlay {
        return (content, content);
    }

    let overlay_height = 3.min(content.height);
    let sentence_row = current_sentence_screen_row(app, content).unwrap_or(0);
    let visible_sentence_row = sentence_row.saturating_sub(content_scroll_offset(app, content));
    let clearance = overlay_height.saturating_sub(visible_sentence_row);
    let reading_content = Rect {
        y: content.y.saturating_add(clearance),
        height: content.height.saturating_sub(clearance),
        ..content
    };
    let annotation_area = Rect {
        x: content.x,
        y: reading_content
            .y
            .saturating_add(visible_sentence_row)
            .saturating_sub(overlay_height),
        width: content.width.min(50),
        height: overlay_height,
    };

    (reading_content, annotation_area)
}

fn draw_annotation_overlay(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let Some(annotation) = current_annotation(app) else {
        return;
    };
    let is_immersed = app.focus() == Focus::AnnotationImmersed;
    let annotation_text = annotation.display_text();
    let mut paragraph =
        Paragraph::new(annotation_text).block(WidgetBlock::default().borders(Borders::ALL));
    if is_immersed {
        paragraph = paragraph
            .wrap(Wrap { trim: false })
            .scroll((app.annotation_scroll().min(u16::MAX as usize) as u16, 0));
    }

    frame.render_widget(Clear, area);
    frame.render_widget(paragraph, area);
}

fn current_sentence_screen_row(app: &App, content: Rect) -> Option<u16> {
    let position = app.position();
    let block = app.document().text_block(position.block_index)?;
    let sentence_range = segment_sentences(&block.text)
        .into_iter()
        .find(|range| range.0 == position.sentence_offset)?;
    let sentence = block.text.get(sentence_range.0..sentence_range.1)?;
    let visible_sentence = sentence.trim_start_matches(char::is_whitespace);
    let visible_start = sentence_range.1 - visible_sentence.len();
    let prefix = block.text.get(..visible_start)?;
    let chapter_start = app
        .document()
        .chapter_range_for_block(position.block_index)
        .map(|range| range.start_block)
        .unwrap_or(position.block_index);
    let preceding_rows = app.document().blocks[chapter_start..position.block_index]
        .iter()
        .map(|block| match block {
            Block::Text(block) => text_block_screen_rows(&block.text, content.width),
            Block::Image(image) => {
                image_content_lines(image, app.image_mode(), content).len() as u16
            }
        })
        .fold(0u16, u16::saturating_add);

    Some(
        preceding_rows.saturating_add(wrapped_row_for_prefix(prefix, content.width)),
    )
}

fn content_scroll_offset(app: &App, content: Rect) -> u16 {
    let Some(sentence_row) = current_sentence_screen_row(app, content) else {
        return 0;
    };

    sentence_row.saturating_sub(content.height.saturating_sub(1))
}

fn wrapped_row_for_prefix(prefix: &str, width: u16) -> u16 {
    let width = usize::from(width.max(1));
    let mut row = 0;
    let mut column = 0;

    for character in prefix.chars() {
        if character == '\n' {
            row += 1;
            column = 0;
            continue;
        }

        let character_width = character.width().unwrap_or(0);
        if character_width == 0 {
            continue;
        }

        if column + character_width > width {
            row += 1;
            column = 0;
        }

        column += character_width;
        if column >= width {
            row += 1;
            column = 0;
        }
    }

    row
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
        .filter_map(|annotation_ref| {
            document
                .annotation_text(&annotation_ref.id)
                .map(|text| (annotation_ref, text))
        })
        .collect::<Vec<_>>();
    let total = annotation_refs.len();
    let index = app.selected_annotation_index().min(total.saturating_sub(1));
    let (_, text) = annotation_refs.get(index)?;

    Some(VisibleAnnotation {
        text: (*text).to_string(),
        index,
        total,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Cursor;

    use ratatui::backend::TestBackend;
    use ratatui::style::{Color, Modifier};
    use ratatui::Terminal;

    use crate::app::{App, ReadingPosition};
    use crate::document::{
        AnnotationRef, Block, ChapterRange, Document, ImageBlock, TextBlock, TocNode,
    };
    use crate::input::Action;

    use super::{draw, wrapped_row_for_prefix};

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
    fn renders_surrounding_paragraph_text_around_current_sentence() {
        let document = Document {
            blocks: vec![Block::Text(TextBlock {
                text: "First sentence. Second sentence. Third sentence.".to_string(),
                chapter_index: 0,
                annotations: Vec::new(),
            })],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        };
        let app = App::with_position(
            document,
            ReadingPosition {
                block_index: 0,
                sentence_offset: "First sentence.".len(),
            },
        );
        let backend = TestBackend::new(80, 6);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("First sentence."));
        assert!(rendered.contains("Second sentence."));
        assert!(rendered.contains("Third sentence."));
    }

    #[test]
    fn renders_parsed_annotation_markers_underlined() {
        let text = "姬赛斯205乌黑油亮，[12]括号，¹⁴上标，*符号。".to_string();
        let app = App::new(Document {
            blocks: vec![Block::Text(TextBlock {
                annotations: ["205", "[12]", "¹⁴", "*"]
                    .into_iter()
                    .enumerate()
                    .map(|(index, marker)| AnnotationRef {
                        id: format!("note-{index}"),
                        offset: text.find(marker).expect("annotation marker"),
                    })
                    .collect(),
                text,
                chapter_index: 0,
            })],
            toc: Vec::new(),
            annotations: (0..4)
                .map(|index| (format!("note-{index}"), format!("Note {index}.")))
                .collect(),
            chapter_ranges: Vec::new(),
        });
        let backend = TestBackend::new(30, 4);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        for marker in ["205", "[12]", "¹⁴", "*"] {
            assert!(text_has_modifier(
                terminal.backend().buffer(),
                marker,
                Modifier::UNDERLINED,
            ), "marker {marker:?}");
        }
        assert!(text_has_modifier(
            terminal.backend().buffer(),
            "205",
            Modifier::REVERSED,
        ));
    }

    #[test]
    fn scrolls_content_to_keep_current_sentence_visible() {
        let prefix = "One.\nTwo.\nThree.\nFour.\nFive.\nSix.\n";
        let document = Document {
            blocks: vec![Block::Text(TextBlock {
                text: format!("{prefix}Seven."),
                chapter_index: 0,
                annotations: Vec::new(),
            })],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        };
        let app = App::with_position(
            document,
            ReadingPosition {
                block_index: 0,
                sentence_offset: prefix.len() - 1,
            },
        );
        let backend = TestBackend::new(30, 5);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        assert!(row_with_text_has_modifier(
            terminal.backend().buffer(),
            "Seven.",
            Modifier::REVERSED,
        ));
        assert!(!buffer_text(terminal.backend().buffer()).contains("One."));
    }

    #[test]
    fn renders_text_block_line_breaks_on_separate_rows() {
        let app = App::new(Document {
            blocks: vec![Block::Text(TextBlock {
                text: "First line.\nSecond line.".to_string(),
                chapter_index: 0,
                annotations: Vec::new(),
            })],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        });
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rows = buffer_rows(terminal.backend().buffer());
        assert!(rows.iter().any(|row| row.contains("First line.")));
        assert!(rows.iter().any(|row| row.contains("Second line.")));
        assert!(!rows.iter().any(|row| row.contains("First line.Second line.")));
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
    fn renders_nested_toc_indent_guides() {
        let mut app = App::new(branching_toc_document());
        app.apply(Action::OpenToc);
        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("├ ▾ Section One"));
        assert!(rendered.contains("│ └ Subsection One"));
    }

    #[test]
    fn renders_selected_toc_row_highlighted() {
        let mut app = App::new(test_document());
        app.apply(Action::OpenToc);
        app.apply(Action::NextTocItem);
        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        assert!(row_with_text_has_modifier(
            terminal.backend().buffer(),
            "Section One",
            Modifier::REVERSED,
        ));
    }

    #[test]
    fn scrolls_toc_sidebar_to_selected_row() {
        let mut app = App::new(long_toc_document());
        app.apply(Action::OpenToc);
        for _ in 0..6 {
            app.apply(Action::NextTocItem);
        }
        let backend = TestBackend::new(60, 4);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("Section Six"));
        assert!(row_with_text_has_modifier(
            terminal.backend().buffer(),
            "Section Six",
            Modifier::REVERSED,
        ));
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
    fn renders_annotation_overlay_with_border() {
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
            toc: Vec::new(),
            annotations,
            chapter_ranges: Vec::new(),
        };
        let mut app = App::new(document);
        app.apply(Action::OpenAnnotationOverlay);
        let backend = TestBackend::new(50, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("┌"));
        assert!(rendered.contains("┘"));
    }

    #[test]
    fn annotation_overlay_bottom_edge_sits_above_current_sentence() {
        let mut annotations = HashMap::new();
        annotations.insert("note-1".to_string(), "Footnote text.".to_string());
        let prefix = concat!(
            "aaaa bbbb cccc dddd. ",
            "eeee ffff gggg hhhh. ",
            "iiii jjjj kkkk llll. ",
            "mmmm nnnn oooo pppp.",
        );
        let document = Document {
            blocks: vec![Block::Text(TextBlock {
                text: format!("{prefix} Target [1]."),
                chapter_index: 0,
                annotations: vec![AnnotationRef {
                    id: "note-1".to_string(),
                    offset: prefix.len() + " Target ".len(),
                }],
            })],
            toc: Vec::new(),
            annotations,
            chapter_ranges: Vec::new(),
        };
        let mut app = App::with_position(
            document,
            ReadingPosition {
                block_index: 0,
                sentence_offset: prefix.len(),
            },
        );
        app.apply(Action::OpenAnnotationOverlay);
        let backend = TestBackend::new(20, 9);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let buffer = terminal.backend().buffer();
        let highlighted_row = row_index_with_modifier(buffer, Modifier::REVERSED)
            .expect("highlighted sentence row");
        let overlay_bottom_row = row_index_containing_text(buffer, "└")
            .expect("overlay bottom border row");

        assert_eq!(overlay_bottom_row + 1, highlighted_row);
    }

    #[test]
    fn wrapped_row_for_prefix_counts_cjk_display_width() {
        assert_eq!(wrapped_row_for_prefix("你好你好", 4), 2);
        assert_eq!(wrapped_row_for_prefix("a你好", 2), 3);
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
    fn renders_available_annotation_when_earlier_ref_is_missing() {
        let mut annotations = HashMap::new();
        annotations.insert("note-2".to_string(), "Second note.".to_string());
        let document = Document {
            blocks: vec![Block::Text(TextBlock {
                text: "Text [1] and [2].".to_string(),
                chapter_index: 0,
                annotations: vec![
                    AnnotationRef {
                        id: "missing-note".to_string(),
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
        let backend = TestBackend::new(50, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
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
    fn scrolls_wrapped_single_line_annotation_when_immersed() {
        let mut annotations = HashMap::new();
        annotations.insert(
            "note-1".to_string(),
            concat!(
                "Start alpha beta gamma delta epsilon zeta eta theta ",
                "iota kappa lambda mu nu xi omicron pi rho sigma tau end."
            )
            .to_string(),
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
        let backend = TestBackend::new(50, 6);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(!rendered.contains("Start alpha"));
        assert!(rendered.contains("sigma tau end."));
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
    fn renders_unavailable_placeholder_when_image_data_is_missing() {
        let document = Document {
            blocks: vec![Block::Image(ImageBlock {
                alt_text: Some("Map of routes".to_string()),
                source_path: Some("OEBPS/images/missing.png".to_string()),
                data: None,
                chapter_index: 0,
            })],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        };
        let app = App::new(document);
        let backend = TestBackend::new(50, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("[image unavailable: Map of routes]"));
    }

    #[test]
    fn renders_unavailable_placeholder_when_bitmap_image_data_is_invalid() {
        let document = Document {
            blocks: vec![Block::Image(ImageBlock {
                alt_text: Some("Map of routes".to_string()),
                source_path: Some("OEBPS/images/map.png".to_string()),
                data: Some(vec![1, 2, 3]),
                chapter_index: 0,
            })],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        };
        let app = App::with_image_mode(document, crate::image::SelectedImageMode::Sixel);
        let backend = TestBackend::new(50, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("[image unavailable: Map of routes]"));
    }

    #[test]
    fn renders_halfblock_image_when_data_is_loaded() {
        let document = Document {
            blocks: vec![Block::Image(ImageBlock {
                alt_text: Some("Map of routes".to_string()),
                source_path: Some("OEBPS/images/map.png".to_string()),
                data: Some(test_png_bytes()),
                chapter_index: 0,
            })],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        };
        let app = App::with_image_mode(document, crate::image::SelectedImageMode::Halfblock);
        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("▀▀"));
        assert!(!rendered.contains("[image: Map of routes]"));
    }

    #[test]
    fn halfblock_image_scales_to_include_the_full_source_width() {
        let document = Document {
            blocks: vec![Block::Image(ImageBlock {
                alt_text: Some("Wide diagram".to_string()),
                source_path: Some("OEBPS/images/wide.png".to_string()),
                data: Some(wide_test_png_bytes()),
                chapter_index: 0,
            })],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        };
        let app = App::with_image_mode(document, crate::image::SelectedImageMode::Halfblock);
        let backend = TestBackend::new(20, 4);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        assert!(terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .any(|cell| cell.fg == Color::Rgb(0, 0, 255)));
    }

    #[test]
    fn renders_sixel_image_when_data_is_loaded() {
        let document = Document {
            blocks: vec![Block::Image(ImageBlock {
                alt_text: Some("Map of routes".to_string()),
                source_path: Some("OEBPS/images/map.png".to_string()),
                data: Some(test_png_bytes()),
                chapter_index: 0,
            })],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        };
        let app = App::with_image_mode(document, crate::image::SelectedImageMode::Sixel);
        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("\u{1b}P"));
        assert!(!rendered.contains("[image: Map of routes]"));
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

    fn long_toc_document() -> Document {
        Document {
            blocks: vec![Block::Text(TextBlock {
                text: "First sentence.".to_string(),
                chapter_index: 0,
                annotations: Vec::new(),
            })],
            toc: vec![TocNode {
                title: "Chapter One".to_string(),
                target_block_index: 0,
                children: [
                    "Section One",
                    "Section Two",
                    "Section Three",
                    "Section Four",
                    "Section Five",
                    "Section Six",
                    "Section Seven",
                ]
                .into_iter()
                .map(|title| TocNode {
                    title: title.to_string(),
                    target_block_index: 0,
                    children: Vec::new(),
                })
                .collect(),
            }],
            annotations: HashMap::new(),
            chapter_ranges: vec![ChapterRange {
                start_block: 0,
                end_block: 0,
            }],
        }
    }

    fn branching_toc_document() -> Document {
        Document {
            blocks: vec![Block::Text(TextBlock {
                text: "First sentence.".to_string(),
                chapter_index: 0,
                annotations: Vec::new(),
            })],
            toc: vec![TocNode {
                title: "Chapter One".to_string(),
                target_block_index: 0,
                children: vec![
                    TocNode {
                        title: "Section One".to_string(),
                        target_block_index: 0,
                        children: vec![TocNode {
                            title: "Subsection One".to_string(),
                            target_block_index: 0,
                            children: Vec::new(),
                        }],
                    },
                    TocNode {
                        title: "Section Two".to_string(),
                        target_block_index: 0,
                        children: Vec::new(),
                    },
                ],
            }],
            annotations: HashMap::new(),
            chapter_ranges: vec![ChapterRange {
                start_block: 0,
                end_block: 0,
            }],
        }
    }

    fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
        buffer_rows(buffer).join("\n")
    }

    fn buffer_rows(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
        buffer
            .content()
            .chunks(buffer.area.width as usize)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
    }

    fn row_with_text_has_modifier(
        buffer: &ratatui::buffer::Buffer,
        text: &str,
        modifier: Modifier,
    ) -> bool {
        buffer
            .content()
            .chunks(buffer.area.width as usize)
            .any(|row| {
                let row_text = row.iter().map(|cell| cell.symbol()).collect::<String>();
                row_text.contains(text) && row.iter().any(|cell| cell.modifier.contains(modifier))
            })
    }

    fn text_has_modifier(
        buffer: &ratatui::buffer::Buffer,
        text: &str,
        modifier: Modifier,
    ) -> bool {
        buffer
            .content()
            .iter()
            .filter(|cell| cell.modifier.contains(modifier))
            .map(|cell| cell.symbol())
            .collect::<String>()
            .contains(text)
    }

    fn row_index_with_modifier(
        buffer: &ratatui::buffer::Buffer,
        modifier: Modifier,
    ) -> Option<usize> {
        buffer
            .content()
            .chunks(buffer.area.width as usize)
            .position(|row| row.iter().any(|cell| cell.modifier.contains(modifier)))
    }

    fn row_index_containing_text(buffer: &ratatui::buffer::Buffer, text: &str) -> Option<usize> {
        buffer
            .content()
            .chunks(buffer.area.width as usize)
            .position(|row| {
                row.iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>()
                    .contains(text)
            })
    }

    fn test_png_bytes() -> Vec<u8> {
        let image = ::image::RgbaImage::from_fn(2, 2, |x, y| match (x, y) {
            (0, 0) => ::image::Rgba([255, 0, 0, 255]),
            (1, 0) => ::image::Rgba([0, 255, 0, 255]),
            (0, 1) => ::image::Rgba([0, 0, 255, 255]),
            _ => ::image::Rgba([255, 255, 255, 255]),
        });
        let mut bytes = Cursor::new(Vec::new());
        ::image::DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ::image::ImageFormat::Png)
            .expect("encode png");
        bytes.into_inner()
    }

    fn wide_test_png_bytes() -> Vec<u8> {
        let image = ::image::RgbaImage::from_fn(100, 2, |x, _| {
            if x < 50 {
                ::image::Rgba([255, 0, 0, 255])
            } else {
                ::image::Rgba([0, 0, 255, 255])
            }
        });
        let mut bytes = Cursor::new(Vec::new());
        ::image::DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ::image::ImageFormat::Png)
            .expect("encode png");
        bytes.into_inner()
    }
}
