use std::path::PathBuf;

use yater::document::{Block, ListItemMarker, TextBlockRole};

#[test]
fn synthetic_epub_fixture_exercises_basic_formatting_model() {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/basic-formatting.epub");

    let document = yater::epub::open(&fixture).expect("open basic formatting fixture");
    let text_blocks = document
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Text(block) => Some(block),
            Block::Image(_) => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(text_blocks[0].presentation.role, TextBlockRole::Heading(1));
    assert!(
        text_blocks
            .iter()
            .flat_map(|block| &block.styles)
            .any(|range| range.style.bold)
    );
    assert!(
        text_blocks
            .iter()
            .flat_map(|block| &block.styles)
            .any(|range| range.style.italic)
    );
    assert!(
        text_blocks
            .iter()
            .flat_map(|block| &block.styles)
            .any(|range| range.style.underlined)
    );
    assert!(
        text_blocks
            .iter()
            .flat_map(|block| &block.styles)
            .any(|range| range.style.crossed_out)
    );
    assert!(
        text_blocks
            .iter()
            .any(|block| block.presentation.quote_depth == 1)
    );
    assert!(text_blocks.iter().any(|block| {
        block
            .presentation
            .list_item
            .is_some_and(|item| item.marker == ListItemMarker::Bullet)
    }));
    assert!(text_blocks.iter().any(|block| {
        block
            .presentation
            .list_item
            .is_some_and(|item| item.marker == ListItemMarker::Ordered(3))
    }));
}
