use std::collections::HashMap;
use std::io::Cursor;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;
use ratatui::Terminal;
use yater::app::App;
use yater::document::{Block, ChapterRange, Document, ImageBlock, TextBlock, TocNode};
use yater::runtime::run_terminal_loop;

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
            "           Chapter One\n",
            "Heading.\n",
            "First paragraph. Second\n",
            "sentence.\n",
            "Final paragraph.\n",
            "\n",
            ""
        )
    );
    assert_eq!(reversed_text(terminal.backend().buffer()), "Heading.");
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
            "           Section One\n",
            "First paragraph. Second\n",
            "sentence.\n",
            "Final paragraph."
        )
    );
    assert_eq!(
        reversed_text(terminal.backend().buffer()),
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
        region_snapshot(terminal.backend().buffer(), 0, 1, 20, 6),
        concat!(
            "▾ Chapter One\n",
            "└ Section One\n",
            "\n",
            "\n",
            "\n",
            ""
        )
    );
    assert_eq!(
        reversed_text(terminal.backend().buffer()),
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
        reversed_text(terminal.backend().buffer()),
        "After image.",
        "{snapshot:?}"
    );
    assert!(!snapshot.contains("Heading."));
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

fn reversed_text(buffer: &ratatui::buffer::Buffer) -> String {
    buffer
        .content()
        .iter()
        .filter(|cell| cell.modifier.contains(Modifier::REVERSED))
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
