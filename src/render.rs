use crate::app::{
    App, ContentRowMetricKey, ContentRowMetrics, HalfblockImageRaster, HalfblockImageRasterKey,
};
use crate::document::{Block, ListItemMarker, TextBlockRole};
use crate::image::SelectedImageMode;
use crate::input::Focus;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as WidgetBlock, Borders, Clear, Paragraph, Wrap};
use ratatui_image::{Image as TerminalImage, Resize};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn draw(frame: &mut ratatui::Frame<'_>, app: &App) {
    let area = frame.area();
    if area.width < 20 || area.height < 4 {
        frame.render_widget(Paragraph::new("Terminal too small").centered(), area);
        return;
    }

    let position = app.position();
    let title = frame_title(
        app.chapter_title_for_block(position.block_index)
            .unwrap_or(""),
        area.width,
    );
    let footer = shortcut_footer(app.focus(), area.width);
    let shell = WidgetBlock::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Line::from(title).left_aligned())
        .title_bottom(footer.left_aligned());
    let content = shell.inner(area);

    frame.render_widget(shell, area);

    let (reading_area, toc_area, divider_area) = if app.focus() == Focus::Toc {
        toc_layout(content)
    } else {
        (content, None, None)
    };
    let (reading_content, annotation_area) = annotation_layout(app, reading_area);

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
        if let Some(area) = toc_area {
            draw_toc(frame, area, app);
        }
        if let Some(area) = divider_area {
            frame.render_widget(toc_divider(area.height), area);
        }
    }

    if matches!(
        app.focus(),
        Focus::AnnotationOverlay | Focus::AnnotationImmersed
    ) {
        draw_annotation_overlay(frame, annotation_area, app);
    }
}

fn frame_title(chapter_title: &str, width: u16) -> Vec<Span<'static>> {
    let available_width = usize::from(width.saturating_sub(6));
    if available_width <= 7 {
        return vec![
            Span::raw("  "),
            Span::styled(
                "YATER",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
    }

    let chapter_width = available_width.saturating_sub("YATER | ".len());
    vec![
        Span::raw("  "),
        Span::styled(
            "YATER",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            truncate_to_width(chapter_title, chapter_width),
            Style::default().fg(Color::White),
        ),
    ]
}

fn shortcut_footer(focus: Focus, width: u16) -> Line<'static> {
    let text = match (focus, width) {
        (Focus::Content, width) if width < 44 => "READ j/k | ; | Tab | q",
        (Focus::Content, width) if width < 54 => "READ j/k | ; note | Tab toc | q",
        (Focus::Content, _) => {
            "READ j/k sentence | h/l paragraph | u/n fast | ; notes | Tab toc | q quit"
        }
        (Focus::Toc, width) if width < 44 => "TOC j/k | Enter | Esc",
        (Focus::Toc, width) if width < 54 => "TOC j/k move | Enter open | Esc",
        (Focus::Toc, _) => "TOC j/k move | l/Enter open | h parent | Tab/Esc close",
        (Focus::AnnotationOverlay, width) if width < 54 => {
            if width < 44 {
                "NOTE ; | Enter | Esc"
            } else {
                "NOTE ; next | Enter full | Esc"
            }
        }
        (Focus::AnnotationOverlay, _) => "NOTE ; next note | Enter expand | Esc/other close",
        (Focus::AnnotationImmersed, width) if width < 44 => "NOTE j/k | Esc",
        (Focus::AnnotationImmersed, width) if width < 54 => "NOTE j/k scroll | Esc",
        (Focus::AnnotationImmersed, _) => "NOTE j/k scroll note | Up/Down scroll | Esc compact",
    };

    shortcut_line(text)
}

fn shortcut_line(text: &str) -> Line<'static> {
    let mut spans = vec![Span::raw("  ")];
    for (index, part) in text.split(' ').enumerate() {
        if index > 0 {
            spans.push(Span::raw(" "));
        }

        let style = if matches!(
            part,
            "READ"
                | "TOC"
                | "NOTE"
                | "j/k"
                | "h/l"
                | "u/n"
                | ";"
                | "Tab"
                | "q"
                | "Enter"
                | "Esc"
                | "l/Enter"
                | "Tab/Esc"
                | "Esc/other"
                | "Up/Down"
        ) {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if part == "|" {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(part.to_string(), style));
    }

    Line::from(spans)
}

fn truncate_to_width(text: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width <= 1 {
        return "…".to_string();
    }

    let mut width = 0;
    let mut output = String::new();
    for character in text.chars() {
        let character_width = character.width().unwrap_or(0);
        if width + character_width > max_width.saturating_sub(1) {
            break;
        }
        output.push(character);
        width += character_width;
    }
    output.push('…');
    output
}

fn current_content_lines(app: &App, content: Rect) -> Vec<Line<'static>> {
    let document = app.document();
    let position = app.position();
    let chapter_range = app
        .chapter_range_for_block(position.block_index)
        .map(|range| range.start_block..=range.end_block)
        .unwrap_or(position.block_index..=position.block_index);
    let mut lines = Vec::new();
    let (top_padding, bottom_padding) = typewriter_padding(content.height);

    lines.extend((0..top_padding).map(|_| Line::default()));

    for block_index in chapter_range {
        match document.blocks.get(block_index) {
            Some(Block::Text(block)) => {
                if matches!(block.presentation.role, TextBlockRole::Heading(_)) {
                    lines.push(Line::default());
                }
                let mut block_lines = vec![Vec::new()];
                let block_style = block_presentation_style(block);
                let highlighted_sentence_offset = (app.focus() != Focus::Toc
                    && block_index == position.block_index)
                    .then_some(position.sentence_offset);
                let mut cursor = 0;
                for range in app.sentence_ranges_for_block(block_index).iter().copied() {
                    if cursor < range.0 {
                        push_block_text_span_lines(
                            &mut block_lines,
                            block,
                            (cursor, range.0),
                            block_style,
                        );
                    }

                    let style = if highlighted_sentence_offset == Some(range.0) {
                        block_style.patch(focus_highlight_style())
                    } else {
                        block_style
                    };
                    push_sentence_span_lines(&mut block_lines, block, range, style, document);
                    cursor = range.1;
                }

                if cursor < block.text.len() {
                    push_block_text_span_lines(
                        &mut block_lines,
                        block,
                        (cursor, block.text.len()),
                        block_style,
                    );
                }

                lines.extend(decorated_text_block_lines(
                    block,
                    block_lines,
                    content.width,
                ));
                if matches!(block.presentation.role, TextBlockRole::Heading(_)) {
                    lines.push(Line::default());
                }
            }
            Some(Block::Image(image)) => {
                lines.extend(image_content_lines(app, block_index, image, content));
            }
            None => {}
        }
    }

    if lines.is_empty() {
        lines.push(Line::default());
    }

    lines.extend((0..bottom_padding).map(|_| Line::default()));

    lines
}

fn block_presentation_style(block: &crate::document::TextBlock) -> Style {
    match block.presentation.role {
        TextBlockRole::Paragraph => Style::default(),
        TextBlockRole::Heading(1) => {
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        }
        TextBlockRole::Heading(_) => Style::default().add_modifier(Modifier::BOLD),
    }
}

fn decorated_text_block_lines(
    block: &crate::document::TextBlock,
    logical_lines: Vec<Vec<Span<'static>>>,
    width: u16,
) -> Vec<Line<'static>> {
    let prefixes = text_block_prefixes(block, width);
    if prefixes.first.is_empty() {
        return logical_lines.into_iter().map(Line::from).collect();
    }

    let content_width = usize::from(text_block_content_width(block, width));
    let mut first_visual_line = true;
    let mut lines = Vec::new();
    for logical_line in logical_lines {
        for line in wrap_styled_line(logical_line, content_width) {
            let mut spans = if first_visual_line {
                prefixes.first.clone()
            } else {
                prefixes.continuation.clone()
            };
            spans.extend(line);
            lines.push(Line::from(spans));
            first_visual_line = false;
        }
    }
    lines
}

struct TextBlockPrefixes {
    first: Vec<Span<'static>>,
    continuation: Vec<Span<'static>>,
}

fn text_block_prefixes(block: &crate::document::TextBlock, width: u16) -> TextBlockPrefixes {
    let max_prefix_width = usize::from(width.saturating_sub(1));
    let marker = block.presentation.list_item.map(|item| match item.marker {
        ListItemMarker::Bullet => "• ".to_string(),
        ListItemMarker::Ordered(ordinal) => format!("{ordinal}. "),
    });
    let marker = marker.map(|marker| truncate_to_width(&marker, max_prefix_width));
    let marker_width = marker.as_deref().map(UnicodeWidthStr::width).unwrap_or(0);
    let visible_quote_depth = block
        .presentation
        .quote_depth
        .min(max_prefix_width.saturating_sub(marker_width) / 2);
    let quote_width = visible_quote_depth.saturating_mul(2);
    let desired_list_indent = block
        .presentation
        .list_item
        .map(|item| item.depth.saturating_mul(2))
        .unwrap_or(0);
    let list_indent_width = desired_list_indent.min(
        max_prefix_width
            .saturating_sub(quote_width)
            .saturating_sub(marker_width),
    );

    let mut first = (0..visible_quote_depth)
        .map(|_| Span::styled("│ ", Style::default().fg(Color::DarkGray)))
        .collect::<Vec<_>>();
    let mut continuation = first.clone();
    if let (Some(item), Some(marker)) = (block.presentation.list_item, marker) {
        let indentation = " ".repeat(list_indent_width);
        if !indentation.is_empty() {
            first.push(Span::raw(indentation.clone()));
            continuation.push(Span::raw(indentation));
        }
        if item.continuation {
            first.push(Span::raw(" ".repeat(marker_width)));
        } else {
            first.push(Span::raw(marker));
        }
        continuation.push(Span::raw(" ".repeat(marker_width)));
    }

    TextBlockPrefixes {
        first,
        continuation,
    }
}

fn text_block_content_width(block: &crate::document::TextBlock, width: u16) -> u16 {
    let prefix_width = text_block_prefixes(block, width)
        .first
        .iter()
        .map(|span| span.width())
        .sum::<usize>();
    usize::from(width)
        .saturating_sub(prefix_width)
        .max(1)
        .min(u16::MAX as usize) as u16
}

fn wrap_styled_line(spans: Vec<Span<'static>>, width: usize) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);
    let mut lines = vec![Vec::new()];
    let mut column = 0usize;
    let mut line_full = false;

    for span in spans {
        for character in span.content.chars() {
            let character_width = character.width().unwrap_or(0);
            if character_width > 0
                && (line_full || (column > 0 && column + character_width > width))
            {
                lines.push(Vec::new());
                column = 0;
            }

            push_styled_character(
                lines.last_mut().expect("wrapped line exists"),
                character,
                span.style,
            );
            column = column.saturating_add(character_width);
            line_full = character_width > 0 && column >= width;
        }
    }

    lines
}

fn push_styled_character(line: &mut Vec<Span<'static>>, character: char, style: Style) {
    if let Some(last) = line.last_mut()
        && last.style == style
    {
        last.content.to_mut().push(character);
    } else {
        line.push(Span::styled(character.to_string(), style));
    }
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
            push_block_text_span_lines(lines, block, (cursor, annotation_ref.offset), base_style);
        }

        let marker_end =
            annotation_marker_end(&block.text, annotation_ref.offset, sentence_range.1);
        push_block_text_span_lines(
            lines,
            block,
            (annotation_ref.offset, marker_end),
            base_style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        );
        cursor = marker_end;
    }

    if cursor < sentence_range.1 {
        push_block_text_span_lines(lines, block, (cursor, sentence_range.1), base_style);
    }
}

fn push_block_text_span_lines(
    lines: &mut Vec<Vec<Span<'static>>>,
    block: &crate::document::TextBlock,
    range: (usize, usize),
    base_style: Style,
) {
    let mut cursor = range.0;
    for style_range in &block.styles {
        let start = style_range.start.max(range.0);
        let end = style_range.end.min(range.1);
        if start >= end {
            continue;
        }
        if cursor < start {
            push_text_span_lines(lines, block.text[cursor..start].to_string(), base_style);
        }
        push_text_span_lines(
            lines,
            block.text[start..end].to_string(),
            apply_epub_text_style(base_style, style_range.style),
        );
        cursor = end;
    }
    if cursor < range.1 {
        push_text_span_lines(lines, block.text[cursor..range.1].to_string(), base_style);
    }
}

fn apply_epub_text_style(mut style: Style, text_style: crate::document::TextStyle) -> Style {
    if text_style.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if text_style.italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if text_style.underlined {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if text_style.crossed_out {
        style = style.add_modifier(Modifier::CROSSED_OUT);
    }
    style
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
        '⁰' | '¹' | '²' | '³' | '⁴' | '⁵' | '⁶' | '⁷' | '⁸' | '⁹' | '₀'..='₉'
    )
}

fn image_content_lines(
    app: &App,
    block_index: usize,
    image: &crate::document::ImageBlock,
    content: Rect,
) -> Vec<Line<'static>> {
    let label = image.alt_text.as_deref().unwrap_or("untitled");
    match app.image_mode() {
        SelectedImageMode::Off => vec![Line::from(format!("[image disabled: {label}]"))],
        SelectedImageMode::Kitty | SelectedImageMode::Iterm2 | SelectedImageMode::Sixel
            if image.data.is_none() && image.source_path.is_some() =>
        {
            vec![Line::from(format!("[image unavailable: {label}]"))]
        }
        SelectedImageMode::Kitty | SelectedImageMode::Iterm2 | SelectedImageMode::Sixel
            if image.data.as_deref().is_some_and(|data| {
                !app.bitmap_image_is_valid(block_index, || ::image::load_from_memory(data).is_ok())
            }) =>
        {
            vec![Line::from(format!("[image unavailable: {label}]"))]
        }
        SelectedImageMode::Kitty | SelectedImageMode::Iterm2 | SelectedImageMode::Sixel => {
            vec![Line::from(format!("[image: {label}]"))]
        }
        SelectedImageMode::Halfblock => match image.data.as_deref() {
            Some(data) => {
                render_halfblock_image(app, block_index, data, content.width, content.height)
                    .unwrap_or_else(|| vec![Line::from(format!("[image unavailable: {label}]"))])
            }
            None if image.source_path.is_some() => {
                vec![Line::from(format!("[image unavailable: {label}]"))]
            }
            None => vec![Line::from(format!("[image: {label}]"))],
        },
    }
}

fn text_block_screen_rows(block: &crate::document::TextBlock, width: u16) -> u16 {
    let width = usize::from(text_block_content_width(block, width));
    let mut rows = 1usize;
    let mut column = 0usize;

    for character in block.text.chars() {
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

    if column == 0 && rows > 1 && !block.text.ends_with('\n') {
        rows -= 1;
    }

    if matches!(block.presentation.role, TextBlockRole::Heading(_)) {
        rows += 2;
    }

    rows.min(u16::MAX as usize) as u16
}

fn push_text_span_lines(lines: &mut Vec<Vec<Span<'static>>>, text: String, style: Style) {
    let mut parts = text.split('\n').peekable();

    while let Some(part) = parts.next() {
        if !part.is_empty()
            && let Some(line) = lines.last_mut()
        {
            line.push(Span::styled(part.to_string(), style));
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
    app: &App,
    block_index: usize,
    data: &[u8],
    terminal_width: u16,
    terminal_height: u16,
) -> Option<Vec<Line<'static>>> {
    if terminal_width == 0 || terminal_height == 0 {
        return None;
    }

    let key = HalfblockImageRasterKey {
        block_index,
        width: terminal_width,
        height: terminal_height,
    };
    let raster = app.halfblock_image_raster(key, || {
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

        Some(HalfblockImageRaster {
            width: image.width(),
            height: image.height(),
            rgba: image.into_raw(),
        })
    })?;
    let width = raster.width;
    let height = raster.height;
    if width == 0 || height == 0 {
        return None;
    }

    let mut lines = Vec::new();
    for y in (0..height).step_by(2) {
        let mut spans = Vec::new();
        for x in 0..width {
            let top = raster_pixel(&raster, x, y)?;
            let bottom = if y + 1 < height {
                raster_pixel(&raster, x, y + 1)?
            } else {
                top
            };
            spans.push(Span::styled(
                "▀",
                Style::default()
                    .fg(rgba_to_color(top))
                    .bg(rgba_to_color(bottom)),
            ));
        }
        lines.push(Line::from(spans));
    }

    Some(lines)
}

fn raster_pixel(raster: &HalfblockImageRaster, x: u32, y: u32) -> Option<[u8; 4]> {
    let offset = ((y * raster.width + x) * 4) as usize;
    Some([
        *raster.rgba.get(offset)?,
        *raster.rgba.get(offset + 1)?,
        *raster.rgba.get(offset + 2)?,
        *raster.rgba.get(offset + 3)?,
    ])
}

fn rgba_to_color([red, green, blue, alpha]: [u8; 4]) -> Color {
    if alpha == 0 {
        Color::Reset
    } else {
        Color::Rgb(red, green, blue)
    }
}

fn draw_toc(frame: &mut ratatui::Frame<'_>, content: Rect, app: &App) {
    let rows = app
        .visible_toc_rows()
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let line = Line::from(row.label.clone());
            if index == app.selected_toc_row() {
                line.style(focus_highlight_style())
            } else {
                line
            }
        })
        .collect::<Vec<_>>();

    frame.render_widget(Clear, content);
    let scroll = toc_scroll_offset(app.selected_toc_row(), content.height);
    frame.render_widget(
        Paragraph::new(rows)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        content,
    );
}

fn toc_layout(content: Rect) -> (Rect, Option<Rect>, Option<Rect>) {
    if content.width <= 12 {
        return (content, Some(content), None);
    }

    let sidebar_width = toc_sidebar_width(content.width);
    let divider_width = if sidebar_width < content.width { 1 } else { 0 };
    let reading_width = content
        .width
        .saturating_sub(sidebar_width)
        .saturating_sub(divider_width);
    let toc_area = Rect {
        x: content.x,
        y: content.y,
        width: sidebar_width,
        height: content.height,
    };
    let divider_area = (divider_width > 0).then_some(Rect {
        x: content.x.saturating_add(sidebar_width),
        y: content.y,
        width: divider_width,
        height: content.height,
    });
    let reading_area = Rect {
        x: content
            .x
            .saturating_add(sidebar_width)
            .saturating_add(divider_width),
        y: content.y,
        width: reading_width,
        height: content.height,
    };

    (reading_area, Some(toc_area), divider_area)
}

fn toc_sidebar_width(content_width: u16) -> u16 {
    let preferred_width = (content_width / 3).max(20);
    if content_width <= 12 {
        content_width
    } else {
        preferred_width.min(content_width.saturating_sub(8))
    }
}

fn toc_divider(height: u16) -> Paragraph<'static> {
    Paragraph::new(
        (0..height)
            .map(|_| Line::from(Span::styled("│", Style::default().fg(Color::DarkGray))))
            .collect::<Vec<_>>(),
    )
}

fn toc_scroll_offset(selected_row: usize, visible_height: u16) -> u16 {
    let visible_height = visible_height as usize;
    if visible_height == 0 || selected_row < visible_height {
        0
    } else {
        (selected_row + 1 - visible_height) as u16
    }
}

fn focus_highlight_style() -> Style {
    Style::default().fg(Color::Rgb(169, 125, 244))
}

fn annotation_layout(app: &App, content: Rect) -> (Rect, Rect) {
    if app.focus() != Focus::AnnotationOverlay {
        return (content, content);
    }

    let overlay_height = compact_annotation_height(app, content);
    let mut clearance = 0;
    let mut reading_content;
    let mut visible_sentence_row;

    loop {
        reading_content = Rect {
            y: content.y.saturating_add(clearance),
            height: content.height.saturating_sub(clearance),
            ..content
        };
        visible_sentence_row = current_sentence_screen_row(app, reading_content)
            .map(|sentence_row| {
                sentence_row.saturating_sub(content_scroll_offset(app, reading_content))
            })
            .unwrap_or(0);
        let sentence_row_from_content_top = clearance.saturating_add(visible_sentence_row);
        let required_clearance = overlay_height.saturating_sub(sentence_row_from_content_top);
        if required_clearance == 0 || clearance >= content.height {
            break;
        }

        let next_clearance = clearance
            .saturating_add(required_clearance)
            .min(content.height);
        if next_clearance == clearance {
            break;
        }
        clearance = next_clearance;
    }

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

fn compact_annotation_height(app: &App, content: Rect) -> u16 {
    let max_height = if content.height <= 3 {
        content.height
    } else {
        content.height.saturating_sub(1).clamp(3, 8)
    };
    let Some(annotation) = current_annotation(app) else {
        return 3.min(content.height);
    };
    let inner_width = usize::from(content.width.min(50).saturating_sub(2).max(1));
    let text_height = annotation_text_display_height(&annotation.display_text(), inner_width);

    (text_height as u16)
        .saturating_add(2)
        .max(3)
        .min(max_height)
}

fn annotation_text_display_height(text: &str, width: usize) -> usize {
    text.lines()
        .map(|line| UnicodeWidthStr::width(line).max(1).div_ceil(width.max(1)))
        .sum::<usize>()
        .max(1)
}

fn draw_annotation_overlay(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let Some(annotation) = current_annotation(app) else {
        return;
    };
    let is_immersed = app.focus() == Focus::AnnotationImmersed;
    let annotation_text = annotation.display_text();
    let mut paragraph = Paragraph::new(annotation_text);
    if !is_immersed {
        paragraph = paragraph.block(WidgetBlock::default().borders(Borders::ALL));
    }
    paragraph = paragraph.wrap(Wrap { trim: false });
    if is_immersed {
        paragraph = paragraph.scroll((app.annotation_scroll().min(u16::MAX as usize) as u16, 0));
    }

    frame.render_widget(Clear, area);
    frame.render_widget(paragraph, area);
}

fn current_sentence_screen_row(app: &App, content: Rect) -> Option<u16> {
    let position = app.position();
    let block = app.document().text_block(position.block_index)?;
    let sentence_range = app
        .sentence_ranges_for_block(position.block_index)
        .iter()
        .copied()
        .find(|range| range.0 == position.sentence_offset)?;
    let sentence = block.text.get(sentence_range.0..sentence_range.1)?;
    let visible_sentence = sentence.trim_start_matches(char::is_whitespace);
    let visible_start = sentence_range.1 - visible_sentence.len();
    let prefix = block.text.get(..visible_start)?;
    let (chapter_start, metrics) = content_row_metrics(app, content);
    let preceding_rows = position
        .block_index
        .checked_sub(chapter_start)
        .map(|relative_index| {
            metrics
                .block_rows
                .iter()
                .take(relative_index)
                .copied()
                .fold(0u16, u16::saturating_add)
        })
        .unwrap_or(0);

    let (top_padding, _) = typewriter_padding(content.height);
    Some(
        top_padding
            .saturating_add(preceding_rows)
            .saturating_add(text_block_leading_rows(block))
            .saturating_add(wrapped_row_for_prefix(
                prefix,
                text_block_content_width(block, content.width),
            )),
    )
}

fn text_block_leading_rows(block: &crate::document::TextBlock) -> u16 {
    u16::from(matches!(block.presentation.role, TextBlockRole::Heading(_)))
}

fn content_scroll_offset(app: &App, content: Rect) -> u16 {
    let Some(sentence_row) = current_sentence_screen_row(app, content) else {
        return 0;
    };

    let center_row = typewriter_center_row(content.height);
    let desired_scroll = sentence_row.saturating_sub(center_row);
    let max_scroll = content_screen_rows(app, content).saturating_sub(content.height);

    desired_scroll.min(max_scroll)
}

fn content_screen_rows(app: &App, content: Rect) -> u16 {
    let (top_padding, bottom_padding) = typewriter_padding(content.height);
    let (_, metrics) = content_row_metrics(app, content);
    let rows = metrics.total_rows.max(1);

    top_padding
        .saturating_add(rows)
        .saturating_add(bottom_padding)
}

fn content_row_metrics(app: &App, content: Rect) -> (usize, ContentRowMetrics) {
    let document = app.document();
    let position = app.position();
    let range = app.chapter_range_for_block(position.block_index).unwrap_or(
        crate::document::ChapterRange {
            start_block: position.block_index,
            end_block: position.block_index,
        },
    );
    let key = ContentRowMetricKey {
        chapter_start: range.start_block,
        chapter_end: range.end_block,
        width: content.width,
        height: content.height,
        image_mode: app.image_mode(),
    };
    let metrics = app.content_row_metrics(key, || {
        let block_rows = (range.start_block..=range.end_block)
            .filter_map(|block_index| {
                document.blocks.get(block_index).map(|block| match block {
                    Block::Text(block) => text_block_screen_rows(block, content.width),
                    Block::Image(image) => {
                        image_content_lines(app, block_index, image, content).len() as u16
                    }
                })
            })
            .collect::<Vec<_>>();
        let total_rows = block_rows.iter().copied().fold(0u16, u16::saturating_add);

        ContentRowMetrics {
            block_rows,
            total_rows,
        }
    });

    (range.start_block, metrics)
}

fn typewriter_center_row(height: u16) -> u16 {
    height / 2
}

fn typewriter_padding(height: u16) -> (u16, u16) {
    let top = typewriter_center_row(height);
    let bottom = height.saturating_sub(top.saturating_add(1));

    (top, bottom)
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
    let sentence_range = app
        .sentence_ranges_for_block(position.block_index)
        .iter()
        .copied()
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

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::{Color, Modifier};

    use crate::app::{App, ReadingPosition};
    use crate::document::{
        AnnotationRef, Block, ChapterRange, Document, ImageBlock, ListItemMarker,
        ListItemPresentation, TextBlock, TextBlockPresentation, TextBlockRole, TextStyle,
        TextStyleRange, TocNode,
    };
    use crate::input::Action;

    use super::{draw, focus_highlight_style, wrapped_row_for_prefix};

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
    fn caches_content_row_metrics_across_repeated_draws() {
        let document = test_document();
        let app = App::with_position(
            document,
            ReadingPosition {
                block_index: 0,
                sentence_offset: 0,
            },
        );
        let backend = TestBackend::new(80, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");
        assert_eq!(app.cached_content_row_metric_count(), 1);

        terminal
            .draw(|frame| draw(frame, &app))
            .expect("draw again");
        assert_eq!(app.cached_content_row_metric_count(), 1);
    }

    #[test]
    fn renders_surrounding_paragraph_text_around_current_sentence() {
        let document = Document {
            blocks: vec![Block::Text(TextBlock {
                text: "First sentence. Second sentence. Third sentence.".to_string(),
                chapter_index: 0,
                presentation: Default::default(),
                styles: Vec::new(),
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
                presentation: Default::default(),
                styles: Vec::new(),
            })],
            toc: Vec::new(),
            annotations: (0..4)
                .map(|index| (format!("note-{index}"), format!("Note {index}.")))
                .collect(),
            chapter_ranges: Vec::new(),
        });
        let backend = TestBackend::new(30, 6);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        for marker in ["205", "[12]", "¹⁴", "*"] {
            assert!(
                text_has_modifier(terminal.backend().buffer(), marker, Modifier::UNDERLINED,),
                "marker {marker:?}"
            );
        }
        assert!(text_has_modifier(
            terminal.backend().buffer(),
            "205",
            Modifier::UNDERLINED,
        ));
        assert!(text_has_highlight(terminal.backend().buffer(), "205"));
    }

    #[test]
    fn composes_epub_style_focus_and_annotation_marker_styles() {
        let text = "Text [1].".to_string();
        let marker_start = text.find("[1]").expect("marker");
        let app = App::new(Document {
            blocks: vec![Block::Text(TextBlock {
                styles: vec![TextStyleRange {
                    start: marker_start,
                    end: marker_start + "[1]".len(),
                    style: TextStyle {
                        italic: true,
                        ..TextStyle::default()
                    },
                }],
                annotations: vec![AnnotationRef {
                    id: "note-1".to_string(),
                    offset: marker_start,
                }],
                text,
                chapter_index: 0,
                presentation: Default::default(),
            })],
            toc: Vec::new(),
            annotations: [("note-1".to_string(), "Footnote.".to_string())]
                .into_iter()
                .collect(),
            chapter_ranges: Vec::new(),
        });
        let backend = TestBackend::new(30, 6);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let buffer = terminal.backend().buffer();
        for modifier in [Modifier::ITALIC, Modifier::BOLD, Modifier::UNDERLINED] {
            assert!(text_has_modifier(buffer, "[1]", modifier));
        }
        assert!(text_has_highlight(buffer, "[1]"));
    }

    #[test]
    fn renders_bold_epub_text_without_styling_surrounding_text() {
        let text = "Plain bold end.".to_string();
        let app = App::new(Document {
            blocks: vec![Block::Text(TextBlock {
                text,
                chapter_index: 0,
                presentation: Default::default(),
                styles: vec![TextStyleRange {
                    start: "Plain ".len(),
                    end: "Plain bold".len(),
                    style: TextStyle {
                        bold: true,
                        ..TextStyle::default()
                    },
                }],
                annotations: Vec::new(),
            })],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        });
        let backend = TestBackend::new(30, 6);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        assert!(text_has_modifier(
            terminal.backend().buffer(),
            "bold",
            Modifier::BOLD,
        ));
        assert!(!text_has_modifier(
            terminal.backend().buffer(),
            "Plain",
            Modifier::BOLD,
        ));
    }

    #[test]
    fn toc_focus_removes_sentence_highlight_but_keeps_epub_formatting() {
        let text = "Bold sentence.".to_string();
        let mut app = App::new(Document {
            blocks: vec![Block::Text(TextBlock {
                styles: vec![TextStyleRange {
                    start: 0,
                    end: text.len(),
                    style: TextStyle {
                        bold: true,
                        ..TextStyle::default()
                    },
                }],
                text,
                chapter_index: 0,
                presentation: Default::default(),
                annotations: Vec::new(),
            })],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        });
        app.apply(Action::OpenToc);
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let buffer = terminal.backend().buffer();
        assert!(text_has_modifier(buffer, "Bold sentence.", Modifier::BOLD));
        assert!(!text_has_highlight(buffer, "Bold sentence."));
    }

    #[test]
    fn preserves_italic_epub_formatting_in_the_highlighted_sentence() {
        let text = "Styled sentence.".to_string();
        let app = App::new(Document {
            blocks: vec![Block::Text(TextBlock {
                styles: vec![TextStyleRange {
                    start: 0,
                    end: text.len(),
                    style: TextStyle {
                        italic: true,
                        ..TextStyle::default()
                    },
                }],
                text,
                chapter_index: 0,
                presentation: Default::default(),
                annotations: Vec::new(),
            })],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        });
        let backend = TestBackend::new(30, 6);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        assert!(text_has_modifier(
            terminal.backend().buffer(),
            "Styled sentence.",
            Modifier::ITALIC,
        ));
        assert!(text_has_highlight(
            terminal.backend().buffer(),
            "Styled sentence.",
        ));
    }

    #[test]
    fn renders_underlined_and_crossed_out_epub_text() {
        let text = "under old.".to_string();
        let app = App::new(Document {
            blocks: vec![Block::Text(TextBlock {
                styles: vec![
                    TextStyleRange {
                        start: 0,
                        end: "under".len(),
                        style: TextStyle {
                            underlined: true,
                            ..TextStyle::default()
                        },
                    },
                    TextStyleRange {
                        start: "under ".len(),
                        end: "under old".len(),
                        style: TextStyle {
                            crossed_out: true,
                            ..TextStyle::default()
                        },
                    },
                ],
                text,
                chapter_index: 0,
                presentation: Default::default(),
                annotations: Vec::new(),
            })],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        });
        let backend = TestBackend::new(30, 6);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        assert!(text_has_modifier(
            terminal.backend().buffer(),
            "under",
            Modifier::UNDERLINED,
        ));
        assert!(text_has_modifier(
            terminal.backend().buffer(),
            "old",
            Modifier::CROSSED_OUT,
        ));
    }

    #[test]
    fn renders_heading_levels_with_terminal_modifiers_and_focus_highlight() {
        let app = App::new(Document {
            blocks: vec![Block::Text(TextBlock {
                text: "Primary heading".to_string(),
                chapter_index: 0,
                presentation: TextBlockPresentation {
                    role: TextBlockRole::Heading(1),
                    ..TextBlockPresentation::default()
                },
                styles: Vec::new(),
                annotations: Vec::new(),
            })],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        });
        let backend = TestBackend::new(30, 6);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let buffer = terminal.backend().buffer();
        assert!(text_has_modifier(buffer, "Primary heading", Modifier::BOLD,));
        assert!(text_has_modifier(
            buffer,
            "Primary heading",
            Modifier::UNDERLINED,
        ));
        assert!(text_has_highlight(buffer, "Primary heading"));
    }

    #[test]
    fn renders_one_blank_row_before_and_after_a_heading() {
        let plain = |text: &str| {
            Block::Text(TextBlock {
                text: text.to_string(),
                chapter_index: 0,
                presentation: TextBlockPresentation::default(),
                styles: Vec::new(),
                annotations: Vec::new(),
            })
        };
        let app = App::new(Document {
            blocks: vec![
                plain("Before."),
                Block::Text(TextBlock {
                    text: "Heading".to_string(),
                    chapter_index: 0,
                    presentation: TextBlockPresentation {
                        role: TextBlockRole::Heading(2),
                        ..TextBlockPresentation::default()
                    },
                    styles: Vec::new(),
                    annotations: Vec::new(),
                }),
                plain("After."),
            ],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: vec![ChapterRange {
                start_block: 0,
                end_block: 2,
            }],
        });
        let backend = TestBackend::new(40, 14);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let buffer = terminal.backend().buffer();
        let before = row_index_containing_text(buffer, "Before.").expect("before row");
        let heading = row_index_containing_text(buffer, "Heading").expect("heading row");
        let after = row_index_containing_text(buffer, "After.").expect("after row");
        assert_eq!(heading - before, 2);
        assert_eq!(after - heading, 2);
    }

    #[test]
    fn repeats_a_dark_quote_gutter_on_every_wrapped_line() {
        let app = App::new(Document {
            blocks: vec![Block::Text(TextBlock {
                text: "abcdefghijklmnop.".to_string(),
                chapter_index: 0,
                presentation: TextBlockPresentation {
                    quote_depth: 1,
                    ..TextBlockPresentation::default()
                },
                styles: Vec::new(),
                annotations: Vec::new(),
            })],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        });
        let backend = TestBackend::new(20, 10);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let quote_rows = terminal
            .backend()
            .buffer()
            .content()
            .chunks(20)
            .filter(|row| row.get(1).is_some_and(|cell| cell.symbol() == "│"))
            .collect::<Vec<_>>();
        assert!(quote_rows.len() >= 2);
        assert!(quote_rows.iter().all(|row| {
            row[1].fg == Color::DarkGray && row.get(2).is_some_and(|cell| cell.symbol() == " ")
        }));
    }

    #[test]
    fn renders_nested_ordered_marker_with_hanging_indent_on_wrapped_lines() {
        let app = App::new(Document {
            blocks: vec![Block::Text(TextBlock {
                text: "ABCDEFGHIJKLMN".to_string(),
                chapter_index: 0,
                presentation: TextBlockPresentation {
                    list_item: Some(ListItemPresentation {
                        depth: 1,
                        marker: ListItemMarker::Ordered(3),
                        continuation: false,
                    }),
                    ..TextBlockPresentation::default()
                },
                styles: Vec::new(),
                annotations: Vec::new(),
            })],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        });
        let backend = TestBackend::new(20, 10);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rows = buffer_rows(terminal.backend().buffer());
        let first = rows
            .iter()
            .find(|row| row.contains("ABC"))
            .expect("first list row");
        let wrapped = rows
            .iter()
            .find(|row| row.contains('N'))
            .expect("wrapped list row");
        let first_content = first.chars().skip(1).collect::<String>();
        let wrapped_content = wrapped.chars().skip(1).collect::<String>();
        assert!(
            first_content.starts_with("  3. A"),
            "unexpected first row: {first_content:?}"
        );
        assert!(
            wrapped_content.starts_with("     N"),
            "unexpected wrapped row: {wrapped_content:?}"
        );
    }

    #[test]
    fn renders_an_unordered_list_item_with_a_bullet_marker() {
        let app = App::new(Document {
            blocks: vec![Block::Text(TextBlock {
                text: "Bullet item.".to_string(),
                chapter_index: 0,
                presentation: TextBlockPresentation {
                    list_item: Some(ListItemPresentation {
                        depth: 0,
                        marker: ListItemMarker::Bullet,
                        continuation: false,
                    }),
                    ..TextBlockPresentation::default()
                },
                styles: Vec::new(),
                annotations: Vec::new(),
            })],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        });
        let backend = TestBackend::new(24, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        assert!(buffer_text(terminal.backend().buffer()).contains("• Bullet item."));
    }

    #[test]
    fn indents_a_list_item_continuation_without_repeating_its_marker() {
        let app = App::new(Document {
            blocks: vec![Block::Text(TextBlock {
                text: "Continuation.".to_string(),
                chapter_index: 0,
                presentation: TextBlockPresentation {
                    list_item: Some(ListItemPresentation {
                        depth: 0,
                        marker: ListItemMarker::Ordered(4),
                        continuation: true,
                    }),
                    ..TextBlockPresentation::default()
                },
                styles: Vec::new(),
                annotations: Vec::new(),
            })],
            toc: Vec::new(),
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        });
        let backend = TestBackend::new(24, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let row = buffer_rows(terminal.backend().buffer())
            .into_iter()
            .find(|row| row.contains("Continuation."))
            .expect("continuation row");
        let content = row.chars().skip(1).collect::<String>();
        assert!(content.starts_with("   Continuation."));
        assert!(!content.contains("4."));
    }

    #[test]
    fn scrolls_content_to_keep_current_sentence_visible() {
        let prefix = "One.\nTwo.\nThree.\nFour.\nFive.\nSix.\n";
        let document = Document {
            blocks: vec![Block::Text(TextBlock {
                text: format!("{prefix}Seven."),
                chapter_index: 0,
                presentation: Default::default(),
                styles: Vec::new(),
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

        assert!(row_with_text_has_highlight(
            terminal.backend().buffer(),
            "Seven.",
        ));
        assert!(!buffer_text(terminal.backend().buffer()).contains("One."));
    }

    #[test]
    fn focus_highlight_uses_color_without_bold() {
        let style = focus_highlight_style();

        assert_eq!(style.fg, Some(Color::Rgb(169, 125, 244)));
        assert_eq!(style.bg, None);
        assert!(!style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn keeps_highlighted_sentence_on_typewriter_center_row() {
        let prefix = "One.\nTwo.\nThree.\n";
        let document = Document {
            blocks: vec![Block::Text(TextBlock {
                text: format!("{prefix}Four.\nFive.\nSix."),
                chapter_index: 0,
                presentation: Default::default(),
                styles: Vec::new(),
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
        let backend = TestBackend::new(30, 7);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        assert_eq!(
            row_index_with_highlight(terminal.backend().buffer()),
            Some(3)
        );
    }

    #[test]
    fn renders_text_block_line_breaks_on_separate_rows() {
        let app = App::new(Document {
            blocks: vec![Block::Text(TextBlock {
                text: "First line.\nSecond line.".to_string(),
                chapter_index: 0,
                presentation: Default::default(),
                styles: Vec::new(),
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
        assert!(
            !rows
                .iter()
                .any(|row| row.contains("First line.Second line."))
        );
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
        assert!(rendered.contains("First sentence."));
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

        assert!(row_with_text_has_highlight(
            terminal.backend().buffer(),
            "Section One",
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
        assert!(row_with_text_has_highlight(
            terminal.backend().buffer(),
            "Section Six",
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
                presentation: Default::default(),
                styles: Vec::new(),
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
                presentation: Default::default(),
                styles: Vec::new(),
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
    fn compact_annotation_overlay_expands_to_show_multiple_lines() {
        let mut annotations = HashMap::new();
        annotations.insert(
            "note-1".to_string(),
            "Line one\nLine two\nLine three\nLine four".to_string(),
        );
        let document = Document {
            blocks: vec![Block::Text(TextBlock {
                text: "Text with [1].".to_string(),
                chapter_index: 0,
                presentation: Default::default(),
                styles: Vec::new(),
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
        let backend = TestBackend::new(50, 10);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("Line one"));
        assert!(rendered.contains("Line two"));
        assert!(rendered.contains("Line three"));
    }

    #[test]
    fn compact_annotation_overlay_expands_for_wrapped_single_line_note() {
        let mut annotations = HashMap::new();
        annotations.insert(
            "note-1".to_string(),
            concat!(
                "first segment stays visible second segment should wrap ",
                "into the compact overlay third segment should be visible too"
            )
            .to_string(),
        );
        let document = Document {
            blocks: vec![Block::Text(TextBlock {
                text: "Text with [1].".to_string(),
                chapter_index: 0,
                presentation: Default::default(),
                styles: Vec::new(),
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
        let backend = TestBackend::new(50, 10);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal.draw(|frame| draw(frame, &app)).expect("draw");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("compact overlay"));
        assert!(rendered.contains("visible too"));
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
                presentation: Default::default(),
                styles: Vec::new(),
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
        let highlighted_row = row_index_with_highlight(buffer).expect("highlighted sentence row");
        let overlay_bottom_row =
            row_index_containing_text(buffer, "└").expect("overlay bottom border row");

        assert!(overlay_bottom_row < highlighted_row);
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
                presentation: Default::default(),
                styles: Vec::new(),
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
                presentation: Default::default(),
                styles: Vec::new(),
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
                presentation: Default::default(),
                styles: Vec::new(),
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
    fn caches_halfblock_image_raster_across_repeated_draws() {
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
        assert_eq!(app.cached_halfblock_image_raster_count(), 1);

        terminal
            .draw(|frame| draw(frame, &app))
            .expect("draw again");
        assert_eq!(app.cached_halfblock_image_raster_count(), 1);
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

        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .any(|cell| cell.fg == Color::Rgb(0, 0, 255))
        );
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
                presentation: Default::default(),
                styles: Vec::new(),
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
                presentation: Default::default(),
                styles: Vec::new(),
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
                presentation: Default::default(),
                styles: Vec::new(),
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

    fn row_with_text_has_highlight(buffer: &ratatui::buffer::Buffer, text: &str) -> bool {
        buffer
            .content()
            .chunks(buffer.area.width as usize)
            .any(|row| {
                let row_text = row.iter().map(|cell| cell.symbol()).collect::<String>();
                row_text.contains(text) && row.iter().any(is_highlight_cell)
            })
    }

    fn text_has_modifier(buffer: &ratatui::buffer::Buffer, text: &str, modifier: Modifier) -> bool {
        buffer
            .content()
            .iter()
            .filter(|cell| cell.modifier.contains(modifier))
            .map(|cell| cell.symbol())
            .collect::<String>()
            .contains(text)
    }

    fn text_has_highlight(buffer: &ratatui::buffer::Buffer, text: &str) -> bool {
        buffer
            .content()
            .iter()
            .filter(|cell| is_highlight_cell(cell))
            .map(|cell| cell.symbol())
            .collect::<String>()
            .contains(text)
    }

    fn row_index_with_highlight(buffer: &ratatui::buffer::Buffer) -> Option<usize> {
        buffer
            .content()
            .chunks(buffer.area.width as usize)
            .position(|row| row.iter().any(is_highlight_cell))
    }

    fn is_highlight_cell(cell: &ratatui::buffer::Cell) -> bool {
        cell.fg == Color::Rgb(169, 125, 244) && cell.bg == Color::Reset
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
