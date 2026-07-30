use std::collections::HashMap;
use std::io::Cursor;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use yater::app::App;
use yater::document::{
    AnnotationRef, Block, ChapterRange, Document, ImageBlock, TextBlock, TocNode,
};
use yater::runtime::{
    EventSource, RuntimeError, RuntimeEvent, run_terminal_event_loop, run_terminal_loop,
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
fn toc_key_opens_toc_sidebar_in_the_runtime_frame() {
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
        frame_snapshot(terminal.backend().buffer()),
        concat!(
            "┌  YATER | Chapter One─────────┐\n",
            "│▾ Chapter One       │         │\n",
            "│└ Section One       │         │\n",
            "│                    │Heading. │\n",
            "│                    │First    │\n",
            "│                    │paragraph│\n",
            "└  TOC j/k | Enter | Esc───────┘"
        )
    );
    assert_eq!(
        highlighted_text(terminal.backend().buffer()),
        "▾ Chapter One"
    );
}

#[test]
fn toc_key_focuses_the_current_reading_chapter() {
    let mut app = App::new(reading_document());
    let backend = TestBackend::new(32, 7);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_terminal_loop(
        &mut terminal,
        &mut app,
        [
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        ],
    )
    .expect("run terminal");

    assert_eq!(
        highlighted_text(terminal.backend().buffer()),
        "└ Section One"
    );
}

#[test]
fn toc_selection_stays_visible_when_long_titles_wrap() {
    let mut app = App::new(long_titled_toc_document());
    let backend = TestBackend::new(40, 6);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_terminal_loop(
        &mut terminal,
        &mut app,
        [
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        ],
    )
    .expect("run terminal");

    assert!(
        highlighted_text(terminal.backend().buffer()).contains("Chapter Three"),
        "{}",
        frame_snapshot(terminal.backend().buffer())
    );
}

#[test]
fn toc_selection_moves_up_before_the_scrolled_viewport_moves() {
    let mut app = App::new(long_toc_document());
    let backend = TestBackend::new(40, 6);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut keys = vec![KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)];
    keys.extend((0..7).map(|_| KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)));
    keys.push(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));

    run_terminal_loop(&mut terminal, &mut app, keys).expect("run terminal");

    assert_eq!(
        highlighted_row(terminal.backend().buffer()),
        Some(3),
        "{}",
        frame_snapshot(terminal.backend().buffer())
    );
}

#[test]
fn toc_viewport_scrolls_up_only_after_selection_reaches_the_top() {
    let mut app = App::new(long_toc_document());
    let backend = TestBackend::new(40, 6);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut keys = vec![KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)];
    keys.extend((0..7).map(|_| KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)));
    keys.extend((0..3).map(|_| KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE)));

    run_terminal_loop(&mut terminal, &mut app, keys).expect("run terminal");

    assert_eq!(highlighted_row(terminal.backend().buffer()), Some(1));
    assert!(
        frame_snapshot(terminal.backend().buffer()).contains("│Chapter Five"),
        "{}",
        frame_snapshot(terminal.backend().buffer())
    );

    run_terminal_loop(
        &mut terminal,
        &mut app,
        [KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE)],
    )
    .expect("run terminal");

    assert_eq!(highlighted_row(terminal.backend().buffer()), Some(1));
    assert!(
        frame_snapshot(terminal.backend().buffer()).contains("│Chapter Four"),
        "{}",
        frame_snapshot(terminal.backend().buffer())
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
                presentation: Default::default(),
                styles: Vec::new(),
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
    assert_eq!(
        highlighted_text(terminal.backend().buffer()),
        "Opening [1]."
    );
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
            presentation: Default::default(),
            styles: Vec::new(),
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
    keys.extend((0..10).map(|_| KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)));

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
            presentation: Default::default(),
            styles: Vec::new(),
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
            presentation: Default::default(),
            styles: Vec::new(),
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
            presentation: Default::default(),
            styles: Vec::new(),
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
            presentation: Default::default(),
            styles: Vec::new(),
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

fn long_titled_toc_document() -> Document {
    Document {
        blocks: vec![text_block("Reading context.")],
        toc: [
            "Chapter One with a long title",
            "Chapter Two with a long title",
            "Chapter Three with a long title",
            "Chapter Four with a long title",
        ]
        .into_iter()
        .map(|title| TocNode {
            title: title.to_string(),
            target_block_index: 0,
            children: Vec::new(),
        })
        .collect(),
        annotations: HashMap::new(),
        chapter_ranges: vec![ChapterRange {
            start_block: 0,
            end_block: 0,
        }],
    }
}

fn long_toc_document() -> Document {
    Document {
        blocks: vec![text_block("Reading context.")],
        toc: [
            "Chapter One",
            "Chapter Two",
            "Chapter Three",
            "Chapter Four",
            "Chapter Five",
            "Chapter Six",
            "Chapter Seven",
            "Chapter Eight",
        ]
        .into_iter()
        .map(|title| TocNode {
            title: title.to_string(),
            target_block_index: 0,
            children: Vec::new(),
        })
        .collect(),
        annotations: HashMap::new(),
        chapter_ranges: vec![ChapterRange {
            start_block: 0,
            end_block: 0,
        }],
    }
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
        .filter(|cell| cell.fg == Color::Rgb(169, 125, 244) && cell.bg == Color::Reset)
        .map(|cell| cell.symbol())
        .collect()
}

fn highlighted_row(buffer: &ratatui::buffer::Buffer) -> Option<usize> {
    buffer
        .content()
        .chunks(buffer.area.width as usize)
        .position(|row| {
            row.iter()
                .any(|cell| cell.fg == Color::Rgb(169, 125, 244) && cell.bg == Color::Reset)
        })
}

fn test_png_bytes(width: u32, height: u32) -> Vec<u8> {
    let image = ::image::RgbaImage::from_pixel(width, height, ::image::Rgba([255, 255, 255, 255]));
    let mut bytes = Cursor::new(Vec::new());
    ::image::DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, ::image::ImageFormat::Png)
        .expect("encode PNG");
    bytes.into_inner()
}
