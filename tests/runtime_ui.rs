use std::collections::HashMap;
use std::io::Cursor;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use ratatui::Terminal;
use yater::app::App;
use yater::document::{
    AnnotationRef, Block, ChapterRange, Document, ImageBlock, TextBlock, TocNode,
};
use yater::runtime::{
    run_terminal_event_loop, run_terminal_loop, EventSource, RuntimeError, RuntimeEvent,
};

#[test]
fn initial_runtime_frame_shows_the_reading_context() {
    let mut app = App::new(reading_document());
    let backend = TestBackend::new(32, 7);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_terminal_loop(
        &mut terminal,
        &mut app,
        [KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)],
    )
    .expect("run terminal");

    assert_eq!(
        frame_snapshot(terminal.backend().buffer()),
        concat!(
            "┌  YATER | Chapter One─────────┐\n",
            "│                              │\n",
            "│                              │\n",
            "│Heading.                      │\n",
            "│First paragraph. Second       │\n",
            "│sentence.                     │\n",
            "└  READ j/k | ; | Tab | q──────┘"
        )
    );
    assert_eq!(highlighted_text(terminal.backend().buffer()), "Heading.");
}

#[test]
fn paragraph_navigation_scrolls_the_runtime_frame_with_context() {
    let mut app = App::new(reading_document());
    let backend = TestBackend::new(32, 4);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_terminal_loop(
        &mut terminal,
        &mut app,
        [
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        ],
    )
    .expect("run terminal");

    assert_eq!(
        frame_snapshot(terminal.backend().buffer()),
        concat!(
            "┌  YATER | Section One─────────┐\n",
            "│sentence.                     │\n",
            "│Final paragraph.              │\n",
            "└  READ j/k | ; | Tab | q──────┘"
        )
    );
    assert_eq!(
        highlighted_text(terminal.backend().buffer()),
        "Final paragraph."
    );
}

#[test]
fn toc_key_opens_a_tree_sidebar_in_the_runtime_frame() {
    let mut app = App::new(reading_document());
    let backend = TestBackend::new(32, 7);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_terminal_loop(
        &mut terminal,
        &mut app,
        [
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        ],
    )
    .expect("run terminal");

    assert_eq!(
        region_snapshot(terminal.backend().buffer(), 1, 1, 20, 5),
        concat!(
            "▾ Chapter One\n",
            "└ Section One\n",
            "\n",
            "\n",
            ""
        )
    );
    assert!(frame_snapshot(terminal.backend().buffer()).contains("TOC j/k | Enter"));
    assert_eq!(
        highlighted_text(terminal.backend().buffer()),
        "▾ Chapter One"
    );
}

#[test]
fn navigation_scrolls_past_the_rendered_height_of_an_inline_image() {
    let document = Document {
        blocks: vec![
            text_block("Heading."),
            Block::Image(ImageBlock {
                alt_text: Some("diagram".to_string()),
                source_path: Some("diagram.png".to_string()),
                data: Some(test_png_bytes(2, 6)),
                chapter_index: 0,
            }),
            text_block("After image."),
        ],
        toc: vec![TocNode {
            title: "Chapter One".to_string(),
            target_block_index: 0,
            children: Vec::new(),
        }],
        annotations: HashMap::new(),
        chapter_ranges: vec![ChapterRange {
            start_block: 0,
            end_block: 2,
        }],
    };
    let mut app = App::new(document);
    let backend = TestBackend::new(24, 4);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_terminal_loop(
        &mut terminal,
        &mut app,
        [
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        ],
    )
    .expect("run terminal");

    let snapshot = frame_snapshot(terminal.backend().buffer());
    assert_eq!(
        highlighted_text(terminal.backend().buffer()),
        "After image.",
        "{snapshot:?}"
    );
    assert!(!snapshot.contains("Heading."));
}

#[test]
fn annotation_overlay_keeps_a_top_of_view_sentence_visible() {
    let mut annotations = HashMap::new();
    annotations.insert("note-1".to_string(), "Footnote text.".to_string());
    let mut app = App::new(Document {
        blocks: vec![
            Block::Text(TextBlock {
                text: "Opening [1].".to_string(),
                chapter_index: 0,
                annotations: vec![AnnotationRef {
                    id: "note-1".to_string(),
                    offset: "Opening ".len(),
                }],
            }),
            text_block("Following paragraph."),
        ],
        toc: vec![TocNode {
            title: "Chapter One".to_string(),
            target_block_index: 0,
            children: Vec::new(),
        }],
        annotations,
        chapter_ranges: vec![ChapterRange {
            start_block: 0,
            end_block: 1,
        }],
    });
    let backend = TestBackend::new(32, 7);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_terminal_loop(
        &mut terminal,
        &mut app,
        [KeyEvent::new(KeyCode::Char(';'), KeyModifiers::NONE)],
    )
    .expect("run terminal");

    assert_eq!(
        frame_snapshot(terminal.backend().buffer()),
        concat!(
            "┌  YATER | Chapter One─────────┐\n",
            "│┌────────────────────────────┐│\n",
            "││Footnote text.              ││\n",
            "│└────────────────────────────┘│\n",
            "│Opening [1].                  │\n",
            "│Following paragraph.          │\n",
            "└  NOTE ; | Enter | Esc────────┘"
        )
    );
    assert_eq!(highlighted_text(terminal.backend().buffer()), "Opening [1].");
}

#[test]
fn immersed_annotation_stops_scrolling_at_the_last_full_viewport() {
    let mut annotations = HashMap::new();
    annotations.insert(
        "note-1".to_string(),
        "Line one\nLine two\nLine three\nLine four\nLine five\nLine six".to_string(),
    );
    let mut app = App::new(Document {
        blocks: vec![Block::Text(TextBlock {
            text: "Opening [1].".to_string(),
            chapter_index: 0,
            annotations: vec![AnnotationRef {
                id: "note-1".to_string(),
                offset: "Opening ".len(),
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
    });
    let backend = TestBackend::new(30, 7);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut keys = vec![
        KeyEvent::new(KeyCode::Char(';'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    ];
    keys.extend(
        (0..10).map(|_| KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
    );

    run_terminal_loop(&mut terminal, &mut app, keys).expect("run terminal");

    assert_eq!(
        frame_snapshot(terminal.backend().buffer()),
        concat!(
            "┌  YATER | Chapter One───────┐\n",
            "│Line three                  │\n",
            "│Line four                   │\n",
            "│Line five                   │\n",
            "│Line six                    │\n",
            "│                            │\n",
            "└  NOTE j/k | Esc────────────┘"
        )
    );
}

#[test]
fn enter_keeps_a_short_annotation_in_the_compact_overlay() {
    let mut annotations = HashMap::new();
    annotations.insert("note-1".to_string(), "Short note.".to_string());
    let mut app = App::new(Document {
        blocks: vec![Block::Text(TextBlock {
            text: "Opening [1].".to_string(),
            chapter_index: 0,
            annotations: vec![AnnotationRef {
                id: "note-1".to_string(),
                offset: "Opening ".len(),
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
    });
    let backend = TestBackend::new(30, 7);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_terminal_loop(
        &mut terminal,
        &mut app,
        [
            KeyEvent::new(KeyCode::Char(';'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        ],
    )
    .expect("run terminal");

    let snapshot = frame_snapshot(terminal.backend().buffer());
    assert!(snapshot.contains("Short note."));
    assert!(snapshot.contains("Opening [1]."));
    assert_eq!(snapshot.matches('┌').count(), 2);
    assert_eq!(snapshot.matches('└').count(), 2);
    assert!(snapshot.contains("NOTE ; | Enter | Esc"));
}

#[test]
fn enlarging_the_terminal_clamps_immersed_annotation_scroll() {
    let mut annotations = HashMap::new();
    annotations.insert(
        "note-1".to_string(),
        "Line one\nLine two\nLine three\nLine four\nLine five\nLine six".to_string(),
    );
    let mut app = App::new(Document {
        blocks: vec![Block::Text(TextBlock {
            text: "Opening [1].".to_string(),
            chapter_index: 0,
            annotations: vec![AnnotationRef {
                id: "note-1".to_string(),
                offset: "Opening ".len(),
            }],
        })],
        toc: Vec::new(),
        annotations,
        chapter_ranges: vec![ChapterRange {
            start_block: 0,
            end_block: 0,
        }],
    });
    let backend = TestBackend::new(30, 5);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut events = VecEventSource {
        events: vec![
            key_event(';'),
            RuntimeEvent::Terminal(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            ))),
            key_event('j'),
            key_event('j'),
            key_event('j'),
            key_event('j'),
            RuntimeEvent::Terminal(Event::Resize(30, 10)),
        ],
    };

    let error = run_terminal_event_loop(&mut terminal, &mut app, &mut events)
        .expect_err("test event source should finish");

    assert_eq!(error.to_string(), "no more events");
    assert_eq!(app.annotation_scroll(), 0);
}

#[test]
fn escape_steps_back_from_immersion_to_the_compact_overlay() {
    let mut annotations = HashMap::new();
    annotations.insert(
        "note-1".to_string(),
        "Long annotation line one\nLong annotation line two".to_string(),
    );
    let mut app = App::new(Document {
        blocks: vec![Block::Text(TextBlock {
            text: "Opening [1].".to_string(),
            chapter_index: 0,
            annotations: vec![AnnotationRef {
                id: "note-1".to_string(),
                offset: "Opening ".len(),
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
    });
    let backend = TestBackend::new(40, 7);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_terminal_loop(
        &mut terminal,
        &mut app,
        [
            KeyEvent::new(KeyCode::Char(';'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        ],
    )
    .expect("run terminal");

    let frame = frame_snapshot(terminal.backend().buffer());
    assert!(frame.contains("Long annotation line one"), "{frame}");
    assert!(frame.contains("Opening [1]."), "{frame}");
    assert_eq!(frame.matches('┌').count(), 2);
    assert_eq!(frame.matches('└').count(), 2);
    assert!(frame.contains("NOTE ; | Enter | Esc"));
}

#[test]
fn semicolon_cycles_multiple_annotations_in_the_runtime_overlay() {
    let annotations = HashMap::from([
        ("note-1".to_string(), "First note.".to_string()),
        ("note-2".to_string(), "Second note.".to_string()),
    ]);
    let mut app = App::new(Document {
        blocks: vec![Block::Text(TextBlock {
            text: "Opening [1] and [2].".to_string(),
            chapter_index: 0,
            annotations: vec![
                AnnotationRef {
                    id: "note-1".to_string(),
                    offset: "Opening ".len(),
                },
                AnnotationRef {
                    id: "note-2".to_string(),
                    offset: "Opening [1] and ".len(),
                },
            ],
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
    });
    let backend = TestBackend::new(40, 7);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_terminal_loop(
        &mut terminal,
        &mut app,
        [
            KeyEvent::new(KeyCode::Char(';'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char(';'), KeyModifiers::NONE),
        ],
    )
    .expect("run terminal");

    let frame = frame_snapshot(terminal.backend().buffer());
    assert!(frame.contains("[2/2] Second note."), "{frame}");
    assert!(frame.contains("Opening [1] and [2]."), "{frame}");
}

struct VecEventSource {
    events: Vec<RuntimeEvent>,
}

impl EventSource for VecEventSource {
    fn next_event(&mut self) -> Result<RuntimeEvent, RuntimeError> {
        if self.events.is_empty() {
            return Err(RuntimeError::new("no more events"));
        }
        Ok(self.events.remove(0))
    }
}

fn key_event(character: char) -> RuntimeEvent {
    RuntimeEvent::Terminal(Event::Key(KeyEvent::new(
        KeyCode::Char(character),
        KeyModifiers::NONE,
    )))
}

fn reading_document() -> Document {
    Document {
        blocks: vec![
            text_block("Heading."),
            text_block("First paragraph. Second sentence."),
            text_block("Final paragraph."),
        ],
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
        chapter_ranges: vec![ChapterRange {
            start_block: 0,
            end_block: 2,
        }],
    }
}

fn text_block(text: &str) -> Block {
    Block::Text(TextBlock {
        text: text.to_string(),
        chapter_index: 0,
        annotations: Vec::new(),
    })
}

fn frame_snapshot(buffer: &ratatui::buffer::Buffer) -> String {
    buffer
        .content()
        .chunks(buffer.area.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn highlighted_text(buffer: &ratatui::buffer::Buffer) -> String {
    buffer
        .content()
        .iter()
        .filter(|cell| {
            cell.fg == Color::Magenta && cell.bg == Color::Rgb(36, 24, 44)
        })
        .map(|cell| cell.symbol())
        .collect()
}

fn region_snapshot(
    buffer: &ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
) -> String {
    (y..y + height)
        .map(|row| {
            (x..x + width)
                .map(|column| buffer[(column, row)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn test_png_bytes(width: u32, height: u32) -> Vec<u8> {
    let image =
        ::image::RgbaImage::from_pixel(width, height, ::image::Rgba([255, 255, 255, 255]));
    let mut bytes = Cursor::new(Vec::new());
    ::image::DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, ::image::ImageFormat::Png)
        .expect("encode PNG");
    bytes.into_inner()
}
