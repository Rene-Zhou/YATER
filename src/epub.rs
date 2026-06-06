use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::document::{
    AnnotationRef, AnnotationStore, Block, ChapterRange, Document, ImageBlock, TextBlock, TocNode,
};

#[derive(Debug)]
pub struct EpubError(String);

impl std::fmt::Display for EpubError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for EpubError {}

pub fn open(path: &Path) -> Result<Document, EpubError> {
    open_with_issue_logger(path, |_| {})
}

pub fn open_with_issue_logger(
    path: &Path,
    mut log_issue: impl FnMut(&str),
) -> Result<Document, EpubError> {
    let file = File::open(path).map_err(|error| EpubError(error.to_string()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| EpubError(error.to_string()))?;

    let container_xml = read_zip_text(&mut archive, "META-INF/container.xml")?;
    let opf_path = find_opf_path(&container_xml)?;
    let opf_xml = read_zip_text(&mut archive, &opf_path)?;
    let opf_base = zip_parent(&opf_path);

    let package = parse_package(&opf_xml)?;
    let mut blocks = Vec::new();
    let mut annotations = AnnotationStore::new();
    let mut chapter_ranges = Vec::new();
    let mut target_blocks_by_href = HashMap::new();

    for (chapter_index, idref) in package.spine_idrefs.iter().enumerate() {
        let item = package
            .manifest
            .get(idref)
            .ok_or_else(|| EpubError(format!("spine item not found in manifest: {idref}")))?;
        let chapter_path = join_zip_path(&opf_base, &item.href);
        let start_block = blocks.len();
        let chapter_base = zip_parent(&chapter_path);
        let mut parsed_chapter = match read_zip_text(&mut archive, &chapter_path) {
            Ok(chapter_xml) => match parse_xhtml_chapter(
                &chapter_xml,
                chapter_index,
                &chapter_path,
                &chapter_base,
            ) {
                Ok(chapter) => chapter,
                Err(error) => {
                    log_issue(&format!("malformed HTML: {chapter_path}: {error}"));
                    malformed_chapter_placeholder(&chapter_path, chapter_index)
                }
            },
            Err(error) => {
                log_issue(&format!("malformed HTML: {chapter_path}: {error}"));
                malformed_chapter_placeholder(&chapter_path, chapter_index)
            }
        };
        load_image_data(&mut archive, &mut parsed_chapter.blocks, &mut log_issue);

        target_blocks_by_href.insert(chapter_path.clone(), start_block);
        blocks.extend(parsed_chapter.blocks);
        annotations.extend(parsed_chapter.annotations);

        if start_block < blocks.len() {
            chapter_ranges.push(ChapterRange {
                start_block,
                end_block: blocks.len() - 1,
            });
        }
    }

    let spine_idrefs = package
        .spine_idrefs
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    for (id, item) in &package.manifest {
        if spine_idrefs.contains(id.as_str())
            || item.properties.split_whitespace().any(|value| value == "nav")
            || item.media_type != "application/xhtml+xml"
        {
            continue;
        }

        let item_path = join_zip_path(&opf_base, &item.href);
        match read_zip_text(&mut archive, &item_path) {
            Ok(item_xml) => match parse_xhtml_annotations(&item_xml, &item_path) {
                Ok(item_annotations) => annotations.extend(item_annotations),
                Err(error) => log_issue(&format!("malformed HTML: {item_path}: {error}")),
            },
            Err(error) => log_issue(&format!("malformed HTML: {item_path}: {error}")),
        }
    }

    let toc = if let Some(nav_item) = package.nav_item() {
        let nav_path = join_zip_path(&opf_base, &nav_item.href);
        match read_zip_text(&mut archive, &nav_path) {
            Ok(nav_xml) => match parse_toc(&nav_xml, &opf_base, &target_blocks_by_href) {
                Ok(toc) => toc,
                Err(error) => {
                    log_issue(&format!("malformed HTML: {nav_path}: {error}"));
                    Vec::new()
                }
            },
            Err(error) => {
                log_issue(&format!("malformed HTML: {nav_path}: {error}"));
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    Ok(Document {
        blocks,
        toc,
        annotations,
        chapter_ranges,
    })
}

struct Package {
    manifest: HashMap<String, ManifestItem>,
    spine_idrefs: Vec<String>,
}

impl Package {
    fn nav_item(&self) -> Option<&ManifestItem> {
        self.manifest
            .values()
            .find(|item| item.properties.split_whitespace().any(|value| value == "nav"))
    }
}

struct ManifestItem {
    href: String,
    properties: String,
    media_type: String,
}

struct ParsedChapter {
    blocks: Vec<Block>,
    annotations: AnnotationStore,
}

const EXPLICIT_LINE_BREAK: char = '\u{0}';

fn malformed_chapter_placeholder(chapter_path: &str, chapter_index: usize) -> ParsedChapter {
    ParsedChapter {
        blocks: vec![Block::Text(TextBlock {
            text: format!("[malformed chapter: {chapter_path}]"),
            chapter_index,
            annotations: Vec::new(),
        })],
        annotations: AnnotationStore::new(),
    }
}

fn read_zip_text(archive: &mut zip::ZipArchive<File>, name: &str) -> Result<String, EpubError> {
    let mut file = archive
        .by_name(name)
        .map_err(|error| EpubError(format!("failed to read {name}: {error}")))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|error| EpubError(error.to_string()))?;
    Ok(contents)
}

fn read_zip_bytes(archive: &mut zip::ZipArchive<File>, name: &str) -> Result<Vec<u8>, EpubError> {
    let mut file = archive
        .by_name(name)
        .map_err(|error| EpubError(format!("failed to read {name}: {error}")))?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)
        .map_err(|error| EpubError(error.to_string()))?;
    Ok(contents)
}

fn load_image_data(
    archive: &mut zip::ZipArchive<File>,
    blocks: &mut [Block],
    log_issue: &mut impl FnMut(&str),
) {
    for block in blocks {
        let Block::Image(image) = block else {
            continue;
        };
        let Some(source_path) = image.source_path.clone() else {
            continue;
        };

        match read_zip_bytes(archive, &source_path) {
            Ok(data) => {
                if let Err(error) = ::image::load_from_memory(&data) {
                    log_issue(&format!("bad image: {source_path}: {error}"));
                } else {
                    image.data = Some(data);
                }
            }
            Err(error) => log_issue(&format!("bad image: {source_path}: {error}")),
        }
    }
}

fn find_opf_path(container_xml: &str) -> Result<String, EpubError> {
    let document = roxmltree::Document::parse(container_xml)
        .map_err(|error| EpubError(format!("invalid container.xml: {error}")))?;

    document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "rootfile")
        .and_then(|node| node.attribute("full-path"))
        .map(str::to_string)
        .ok_or_else(|| EpubError("container.xml does not declare an OPF rootfile".to_string()))
}

fn parse_package(opf_xml: &str) -> Result<Package, EpubError> {
    let document = roxmltree::Document::parse(opf_xml)
        .map_err(|error| EpubError(format!("invalid OPF package: {error}")))?;
    let mut manifest = HashMap::new();
    let mut spine_idrefs = Vec::new();

    for node in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "item")
    {
        if let (Some(id), Some(href)) = (node.attribute("id"), node.attribute("href")) {
            manifest.insert(
                id.to_string(),
                ManifestItem {
                    href: href.to_string(),
                    properties: node.attribute("properties").unwrap_or("").to_string(),
                    media_type: node.attribute("media-type").unwrap_or("").to_string(),
                },
            );
        }
    }

    for node in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "itemref")
    {
        if let Some(idref) = node.attribute("idref") {
            spine_idrefs.push(idref.to_string());
        }
    }

    Ok(Package {
        manifest,
        spine_idrefs,
    })
}

fn parse_xhtml_chapter(
    xhtml: &str,
    chapter_index: usize,
    chapter_path: &str,
    chapter_base: &str,
) -> Result<ParsedChapter, EpubError> {
    let document = roxmltree::Document::parse(xhtml)
        .map_err(|error| EpubError(format!("invalid XHTML chapter: {error}")))?;
    let mut blocks = Vec::new();
    let annotations = annotations_from_document(&document, chapter_path);

    append_chapter_blocks(
        document.root_element(),
        chapter_index,
        chapter_path,
        chapter_base,
        &mut blocks,
    );

    Ok(ParsedChapter {
        blocks,
        annotations,
    })
}

fn parse_xhtml_annotations(
    xhtml: &str,
    document_path: &str,
) -> Result<AnnotationStore, EpubError> {
    let document = roxmltree::Document::parse(xhtml)
        .map_err(|error| EpubError(format!("invalid XHTML annotation document: {error}")))?;

    Ok(annotations_from_document(&document, document_path))
}

fn annotations_from_document(
    document: &roxmltree::Document<'_>,
    document_path: &str,
) -> AnnotationStore {
    let mut annotations = AnnotationStore::new();

    for node in document
        .descendants()
        .filter(|node| node.is_element() && is_annotation_container(*node))
    {
        if let Some(id) = node.attribute("id") {
            let text = node_text(node);

            if !text.is_empty() {
                annotations.insert(annotation_key(document_path, id), text);
            }
        }
    }

    annotations
}

fn parse_toc(
    nav_xml: &str,
    opf_base: &str,
    target_blocks_by_href: &HashMap<String, usize>,
) -> Result<Vec<TocNode>, EpubError> {
    let document = roxmltree::Document::parse(nav_xml)
        .map_err(|error| EpubError(format!("invalid EPUB nav document: {error}")))?;
    let Some(toc_nav) = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "nav" && is_toc_nav(*node))
    else {
        return Ok(Vec::new());
    };
    let Some(root_list) = toc_nav
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "ol")
    else {
        return Ok(Vec::new());
    };

    Ok(parse_toc_list(root_list, opf_base, target_blocks_by_href))
}

fn is_toc_nav(node: roxmltree::Node<'_, '_>) -> bool {
    node.attributes()
        .find(|attribute| attribute.name() == "type")
        .is_some_and(|attribute| {
            attribute
                .value()
                .split_whitespace()
                .any(|value| value == "toc")
        })
}

fn parse_toc_list(
    list: roxmltree::Node<'_, '_>,
    opf_base: &str,
    target_blocks_by_href: &HashMap<String, usize>,
) -> Vec<TocNode> {
    list.children()
        .filter(|node| node.is_element() && node.tag_name().name() == "li")
        .filter_map(|node| parse_toc_item(node, opf_base, target_blocks_by_href))
        .collect()
}

fn parse_toc_item(
    item: roxmltree::Node<'_, '_>,
    opf_base: &str,
    target_blocks_by_href: &HashMap<String, usize>,
) -> Option<TocNode> {
    let link = item
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "a")?;
    let href = link.attribute("href")?;
    let href_without_fragment = href.split('#').next().unwrap_or(href);
    let target_path = join_zip_path(opf_base, href_without_fragment);
    let target_block_index = *target_blocks_by_href.get(&target_path)?;
    let title = normalized_descendant_text(link);

    if title.is_empty() {
        return None;
    }

    let children = item
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "ol")
        .map(|list| parse_toc_list(list, opf_base, target_blocks_by_href))
        .unwrap_or_default();

    Some(TocNode {
        title,
        target_block_index,
        children,
    })
}

fn is_text_block_element(name: &str) -> bool {
    matches!(
        name,
        "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "div" | "figure" | "blockquote"
            | "li"
    )
}

fn append_chapter_blocks(
    node: roxmltree::Node<'_, '_>,
    chapter_index: usize,
    chapter_path: &str,
    chapter_base: &str,
    blocks: &mut Vec<Block>,
) {
    for child in node.children().filter(|child| child.is_element()) {
        if is_annotation_container(child) || has_annotation_ancestor(child) {
            continue;
        }

        if is_text_block_element(child.tag_name().name()) {
            blocks.extend(blocks_from_xhtml_element(
                child,
                chapter_index,
                chapter_path,
                chapter_base,
            ));
        } else {
            append_chapter_blocks(
                child,
                chapter_index,
                chapter_path,
                chapter_base,
                blocks,
            );
        }
    }
}

fn is_annotation_container(node: roxmltree::Node<'_, '_>) -> bool {
    matches!(node.tag_name().name(), "aside" | "note") && node.attribute("id").is_some()
}

fn has_annotation_ancestor(node: roxmltree::Node<'_, '_>) -> bool {
    node.ancestors()
        .skip(1)
        .any(|ancestor| ancestor.is_element() && is_annotation_container(ancestor))
}

fn is_non_visible_element(node: roxmltree::Node<'_, '_>) -> bool {
    matches!(node.tag_name().name(), "script" | "style")
}

fn node_text(node: roxmltree::Node<'_, '_>) -> String {
    let mut blocks = Vec::new();
    append_annotation_text_blocks(node, &mut blocks);

    if !blocks.is_empty() {
        return blocks.join("\n");
    }

    normalized_descendant_text(node)
}

fn append_annotation_text_blocks(node: roxmltree::Node<'_, '_>, blocks: &mut Vec<String>) {
    for child in node.children().filter(|child| child.is_element()) {
        if is_non_visible_element(child) {
            continue;
        }

        if is_text_block_element(child.tag_name().name()) {
            let mut nested_blocks = Vec::new();
            append_annotation_text_blocks(child, &mut nested_blocks);

            if nested_blocks.is_empty() {
                let text = normalized_descendant_text(child);
                if !text.is_empty() {
                    blocks.push(text);
                }
            } else {
                blocks.extend(nested_blocks);
            }
        } else {
            append_annotation_text_blocks(child, blocks);
        }
    }
}

fn normalized_descendant_text(node: roxmltree::Node<'_, '_>) -> String {
    let mut text = String::new();
    append_descendant_text(node, &mut text);

    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn append_descendant_text(node: roxmltree::Node<'_, '_>, text: &mut String) {
    for child in node.children() {
        if child.is_text() {
            if let Some(child_text) = child.text() {
                text.push_str(child_text);
            }
        } else if child.is_element() {
            if is_non_visible_element(child) {
                continue;
            }

            if child.tag_name().name() == "br" {
                text.push('\n');
            } else {
                append_descendant_text(child, text);
            }
        }
    }
}

fn blocks_from_xhtml_element(
    node: roxmltree::Node<'_, '_>,
    chapter_index: usize,
    chapter_path: &str,
    chapter_base: &str,
) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut text = String::new();
    let mut annotation_refs = Vec::new();

    append_visible_blocks(
        node,
        chapter_index,
        chapter_path,
        chapter_base,
        &mut blocks,
        &mut text,
        &mut annotation_refs,
    );
    flush_text_block(&mut blocks, chapter_index, &mut text, &mut annotation_refs);

    blocks
}

fn append_visible_blocks(
    node: roxmltree::Node<'_, '_>,
    chapter_index: usize,
    chapter_path: &str,
    chapter_base: &str,
    blocks: &mut Vec<Block>,
    text: &mut String,
    annotation_refs: &mut Vec<AnnotationRef>,
) {
    for child in node.children() {
        if child.is_text() {
            if let Some(child_text) = child.text() {
                text.push_str(child_text);
            }
        } else if child.is_element() {
            if is_non_visible_element(child) {
                continue;
            }

            if child.tag_name().name() == "br" {
                text.push(EXPLICIT_LINE_BREAK);
                continue;
            }

            if is_text_block_element(child.tag_name().name()) {
                flush_text_block(blocks, chapter_index, text, annotation_refs);
                blocks.extend(blocks_from_xhtml_element(
                    child,
                    chapter_index,
                    chapter_path,
                    chapter_base,
                ));
                continue;
            }

            if child.tag_name().name() == "img" {
                flush_text_block(blocks, chapter_index, text, annotation_refs);
                blocks.push(Block::Image(ImageBlock {
                    alt_text: child.attribute("alt").map(str::to_string),
                    source_path: child
                        .attribute("src")
                        .map(|source_path| join_zip_path(chapter_base, source_path)),
                    data: None,
                    chapter_index,
                }));
                continue;
            }

            if let Some(id) = child
                .attribute("href")
                .and_then(|href| annotation_key_from_href(href, chapter_path, chapter_base))
            {
                annotation_refs.push(AnnotationRef {
                    id,
                    offset: text.len(),
                });
            }

            append_visible_blocks(
                child,
                chapter_index,
                chapter_path,
                chapter_base,
                blocks,
                text,
                annotation_refs,
            );
        }
    }
}

fn annotation_key_from_href(
    href: &str,
    document_path: &str,
    document_base: &str,
) -> Option<String> {
    let (target, id) = href.rsplit_once('#')?;
    if id.is_empty() {
        None
    } else {
        let target_path = if target.is_empty() {
            document_path.to_string()
        } else {
            join_zip_path(document_base, target)
        };
        Some(annotation_key(&target_path, id))
    }
}

fn annotation_key(document_path: &str, id: &str) -> String {
    format!("{document_path}#{id}")
}

fn flush_text_block(
    blocks: &mut Vec<Block>,
    chapter_index: usize,
    text: &mut String,
    annotation_refs: &mut Vec<AnnotationRef>,
) {
    let (normalized, annotations) = normalize_visible_text_and_annotations(text, annotation_refs);

    if !normalized.is_empty() {
        blocks.push(Block::Text(TextBlock {
            text: normalized,
            chapter_index,
            annotations,
        }));
    }

    text.clear();
    annotation_refs.clear();
}

fn normalize_visible_text_and_annotations(
    text: &str,
    annotation_refs: &[AnnotationRef],
) -> (String, Vec<AnnotationRef>) {
    let mut normalized = String::new();
    let mut pending_space = false;
    let mut normalized_annotation_refs = Vec::new();
    let mut refs_by_offset = annotation_refs.iter().collect::<Vec<_>>();
    refs_by_offset.sort_by_key(|annotation_ref| annotation_ref.offset);
    let mut next_ref = 0;

    for (byte_index, character) in text.char_indices() {
        while next_ref < refs_by_offset.len() && refs_by_offset[next_ref].offset <= byte_index {
            let annotation_ref = refs_by_offset[next_ref];
            normalized_annotation_refs.push(AnnotationRef {
                id: annotation_ref.id.clone(),
                offset: normalized_offset(&normalized, pending_space),
            });
            next_ref += 1;
        }

        if character == EXPLICIT_LINE_BREAK {
            if !normalized.is_empty() && !normalized.ends_with('\n') {
                normalized.push('\n');
            }
            pending_space = false;
        } else if character.is_whitespace() {
            pending_space = true;
        } else {
            if pending_space && !normalized.is_empty() && !normalized.ends_with('\n') {
                normalized.push(' ');
            }
            normalized.push(character);
            pending_space = false;
        }
    }

    while next_ref < refs_by_offset.len() {
        let annotation_ref = refs_by_offset[next_ref];
        normalized_annotation_refs.push(AnnotationRef {
            id: annotation_ref.id.clone(),
            offset: normalized_offset(&normalized, pending_space),
        });
        next_ref += 1;
    }

    (normalized, normalized_annotation_refs)
}

fn normalized_offset(normalized: &str, pending_space: bool) -> usize {
    normalized.len()
        + usize::from(pending_space && !normalized.is_empty() && !normalized.ends_with('\n'))
}

fn zip_parent(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_default()
}

fn join_zip_path(base: &str, href: &str) -> String {
    let joined = if base.is_empty() {
        href.to_string()
    } else {
        format!("{base}/{href}")
    };
    normalize_zip_path(&joined)
}

fn normalize_zip_path(path: &str) -> String {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Cursor;
    use std::io::Write;
    use std::path::Path;

    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    use crate::document::{AnnotationRef, Block, ChapterRange};

    use super::open;

    #[test]
    fn parses_spine_ordered_text_blocks_toc_and_chapter_ranges() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <h1>Chapter One</h1>
    <p>First paragraph.</p>
    <p>第二段。</p>
  </body>
</html>"#,
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(
            document
                .blocks
                .iter()
                .filter_map(|block| match block {
                    Block::Text(block) => Some(block.text.as_str()),
                    Block::Image(_) => None,
                })
                .collect::<Vec<_>>(),
            vec!["Chapter One", "First paragraph.", "第二段。"]
        );
        assert_eq!(document.chapter_title_for_block(0), Some("Chapter One"));
        assert_eq!(
            document.chapter_range_for_block(2),
            Some(ChapterRange {
                start_block: 0,
                end_block: 2,
            })
        );
    }

    #[test]
    fn parses_nested_block_elements_without_duplicate_text() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <div>
      <p>First nested paragraph.</p>
      <p>Second nested paragraph.</p>
    </div>
  </body>
</html>"#,
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(
            document
                .blocks
                .iter()
                .filter_map(|block| match block {
                    Block::Text(block) => Some(block.text.as_str()),
                    Block::Image(_) => None,
                })
                .collect::<Vec<_>>(),
            vec!["First nested paragraph.", "Second nested paragraph."]
        );
    }

    #[test]
    fn extracts_list_items_as_text_blocks() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <ul>
      <li>First item.</li>
      <li>Second item.</li>
    </ul>
  </body>
</html>"#,
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(
            document
                .blocks
                .iter()
                .filter_map(|block| match block {
                    Block::Text(block) => Some(block.text.as_str()),
                    Block::Image(_) => None,
                })
                .collect::<Vec<_>>(),
            vec!["First item.", "Second item."]
        );
    }

    #[test]
    fn preserves_inline_line_breaks_in_text_blocks() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>First line.<br/>Second line.</p>
  </body>
</html>"#,
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(
            document.text_block(0).map(|block| block.text.as_str()),
            Some("First line.\nSecond line.")
        );
    }

    #[test]
    fn strips_inline_formatting_and_source_whitespace_from_text_blocks() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>
      First <em>formatted</em>
      text.
    </p>
  </body>
</html>"#,
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(
            document.text_block(0).map(|block| block.text.as_str()),
            Some("First formatted text.")
        );
    }

    #[test]
    fn ignores_non_visible_inline_content_in_text_blocks() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>Visible <script>hidden()</script> text.</p>
  </body>
</html>"#,
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(
            document.text_block(0).map(|block| block.text.as_str()),
            Some("Visible text.")
        );
    }

    #[test]
    fn extracts_inline_annotation_refs_and_plain_text_notes() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>Text with <a href="#note-1">[1]</a>.</p>
    <aside id="note-1"><p>Footnote text.</p></aside>
  </body>
</html>"##,
        );

        let document = open(&epub_path).expect("parse EPUB");
        let block = document.text_block(0).expect("first text block");

        assert_eq!(block.text, "Text with [1].");
        assert_eq!(
            block.annotations,
            vec![AnnotationRef {
                id: "OEBPS/chapter1.xhtml#note-1".to_string(),
                offset: "Text with ".len(),
            }]
        );
        assert_eq!(
            document.annotation_text("OEBPS/chapter1.xhtml#note-1"),
            Some("Footnote text.")
        );
    }

    #[test]
    fn extracts_file_fragment_annotation_refs() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>Text with <a href="chapter1.xhtml#note-1">[1]</a>.</p>
    <aside id="note-1"><p>Footnote text.</p></aside>
  </body>
</html>"##,
        );

        let document = open(&epub_path).expect("parse EPUB");
        let block = document.text_block(0).expect("first text block");

        assert_eq!(block.text, "Text with [1].");
        assert_eq!(
            block.annotations,
            vec![AnnotationRef {
                id: "OEBPS/chapter1.xhtml#note-1".to_string(),
                offset: "Text with ".len(),
            }]
        );
        assert_eq!(
            document.annotation_text("OEBPS/chapter1.xhtml#note-1"),
            Some("Footnote text.")
        );
    }

    #[test]
    fn extracts_annotations_from_non_spine_xhtml_note_files() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_epub_with_extra_files(
            &epub_path,
            &[(
                "chapter1",
                "chapter1.xhtml",
                r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>Text with <a href="notes.xhtml#note-1">[1]</a>.</p>
  </body>
</html>"##,
            )],
            &[(
                "notes",
                "notes.xhtml",
                "application/xhtml+xml",
                r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <aside id="note-1"><p>Separate note text.</p></aside>
  </body>
</html>"##,
            )],
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc">
      <ol><li><a href="chapter1.xhtml">Chapter One</a></li></ol>
    </nav>
  </body>
</html>"#,
            None,
        );

        let document = open(&epub_path).expect("parse EPUB");
        let block = document.text_block(0).expect("first text block");

        assert_eq!(
            block.annotations,
            vec![AnnotationRef {
                id: "OEBPS/notes.xhtml#note-1".to_string(),
                offset: "Text with ".len(),
            }]
        );
        assert_eq!(
            document.annotation_text("OEBPS/notes.xhtml#note-1"),
            Some("Separate note text.")
        );
    }

    #[test]
    fn keeps_same_annotation_id_distinct_across_note_files() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_epub_with_extra_files(
            &epub_path,
            &[(
                "chapter1",
                "chapter1.xhtml",
                r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>
      First <a href="notes-a.xhtml#note-1">[1]</a>.
      Second <a href="notes-b.xhtml#note-1">[2]</a>.
    </p>
  </body>
</html>"##,
            )],
            &[
                (
                    "notes-a",
                    "notes-a.xhtml",
                    "application/xhtml+xml",
                    r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body><aside id="note-1"><p>First note.</p></aside></body>
</html>"##,
                ),
                (
                    "notes-b",
                    "notes-b.xhtml",
                    "application/xhtml+xml",
                    r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body><aside id="note-1"><p>Second note.</p></aside></body>
</html>"##,
                ),
            ],
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc">
      <ol><li><a href="chapter1.xhtml">Chapter One</a></li></ol>
    </nav>
  </body>
</html>"#,
            None,
        );

        let document = open(&epub_path).expect("parse EPUB");
        let block = document.text_block(0).expect("first text block");
        let annotation_texts = block
            .annotations
            .iter()
            .map(|annotation| {
                document
                    .annotation_text(&annotation.id)
                    .expect("annotation text")
            })
            .collect::<Vec<_>>();

        assert_eq!(annotation_texts, vec!["First note.", "Second note."]);
    }

    #[test]
    fn malformed_non_spine_annotation_files_are_logged_non_fatally() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_epub_with_extra_files(
            &epub_path,
            &[(
                "chapter1",
                "chapter1.xhtml",
                r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>Text with <a href="notes.xhtml#note-1">[1]</a>.</p>
  </body>
</html>"##,
            )],
            &[(
                "notes",
                "notes.xhtml",
                "application/xhtml+xml",
                r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body><aside id="note-1"><p>Broken note.
</html>"##,
            )],
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc">
      <ol><li><a href="chapter1.xhtml">Chapter One</a></li></ol>
    </nav>
  </body>
</html>"#,
            None,
        );
        let mut issues = Vec::new();

        let document = super::open_with_issue_logger(&epub_path, |issue| {
            issues.push(issue.to_string());
        })
        .expect("parse EPUB");

        assert_eq!(
            document.text_block(0).map(|block| block.text.as_str()),
            Some("Text with [1].")
        );
        assert_eq!(
            document.annotation_text("OEBPS/notes.xhtml#note-1"),
            None
        );
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("malformed HTML: OEBPS/notes.xhtml"));
    }

    #[test]
    fn missing_non_spine_annotation_files_are_logged_non_fatally() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_epub_with_missing_extra_files(
            &epub_path,
            &[(
                "chapter1",
                "chapter1.xhtml",
                r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>Text with <a href="notes.xhtml#note-1">[1]</a>.</p>
  </body>
</html>"##,
            )],
            &[("notes", "notes.xhtml", "application/xhtml+xml")],
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc">
      <ol><li><a href="chapter1.xhtml">Chapter One</a></li></ol>
    </nav>
  </body>
</html>"#,
        );
        let mut issues = Vec::new();

        let document = super::open_with_issue_logger(&epub_path, |issue| {
            issues.push(issue.to_string());
        })
        .expect("parse EPUB");

        assert_eq!(
            document.text_block(0).map(|block| block.text.as_str()),
            Some("Text with [1].")
        );
        assert_eq!(
            document.annotation_text("OEBPS/notes.xhtml#note-1"),
            None
        );
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("malformed HTML: OEBPS/notes.xhtml"));
    }

    #[test]
    fn preserves_block_breaks_in_annotation_plain_text() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>Text with <a href="#note-1">[1]</a>.</p>
    <aside id="note-1">
      <p>First note paragraph.</p>
      <p>Second note paragraph.</p>
    </aside>
  </body>
</html>"##,
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(
            document.annotation_text("OEBPS/chapter1.xhtml#note-1"),
            Some("First note paragraph.\nSecond note paragraph.")
        );
    }

    #[test]
    fn preserves_inline_line_breaks_in_annotation_plain_text() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>Text with <a href="#note-1">[1]</a>.</p>
    <aside id="note-1">
      <p>First note line.<br/>Second note line.</p>
    </aside>
  </body>
</html>"##,
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(
            document.annotation_text("OEBPS/chapter1.xhtml#note-1"),
            Some("First note line.\nSecond note line.")
        );
    }

    #[test]
    fn ignores_non_visible_inline_content_in_annotation_plain_text() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>Text with <a href="#note-1">[1]</a>.</p>
    <aside id="note-1">
      <p>Visible <script>hidden()</script> note.</p>
    </aside>
  </body>
</html>"##,
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(
            document.annotation_text("OEBPS/chapter1.xhtml#note-1"),
            Some("Visible note.")
        );
    }

    #[test]
    fn splits_inline_images_out_of_text_blocks() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>Before <img src="images/picture.png" alt="Picture"/> after.</p>
  </body>
</html>"#,
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(document.blocks.len(), 3);
        assert!(matches!(
            &document.blocks[0],
            Block::Text(block) if block.text == "Before"
        ));
        assert!(matches!(
            &document.blocks[1],
            Block::Image(block)
                if block.alt_text.as_deref() == Some("Picture")
                    && block.source_path.as_deref() == Some("OEBPS/images/picture.png")
                    && block.data.as_deref() == Some(test_png_bytes().as_slice())
        ));
        assert!(matches!(
            &document.blocks[2],
            Block::Text(block) if block.text == "after."
        ));
    }

    #[test]
    fn normalizes_relative_image_paths_before_loading_data() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_epub_with_files(
            &epub_path,
            &[(
                "chapter1",
                "Text/chapter1.xhtml",
                r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p><img src="../images/picture.png" alt="Picture"/></p>
  </body>
</html>"#,
            )],
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc">
      <ol><li><a href="Text/chapter1.xhtml">Chapter One</a></li></ol>
    </nav>
  </body>
</html>"#,
            Some(("OEBPS/images/picture.png", test_png_bytes().as_slice())),
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert!(matches!(
            &document.blocks[0],
            Block::Image(block)
                if block.source_path.as_deref() == Some("OEBPS/images/picture.png")
                    && block.data.as_deref() == Some(test_png_bytes().as_slice())
        ));
    }

    #[test]
    fn malformed_chapter_is_logged_and_replaced_with_placeholder() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>Broken chapter.
  </body>
</html>"#,
        );
        let mut issues = Vec::new();

        let document = super::open_with_issue_logger(&epub_path, |issue| {
            issues.push(issue.to_string());
        })
        .expect("parse EPUB with placeholder");

        assert!(matches!(
            &document.blocks[0],
            Block::Text(block) if block.text == "[malformed chapter: OEBPS/chapter1.xhtml]"
        ));
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("malformed HTML: OEBPS/chapter1.xhtml"));
    }

    #[test]
    fn missing_chapter_is_logged_and_replaced_with_placeholder() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_epub_with_missing_chapter_file(
            &epub_path,
            "chapter1",
            "chapter1.xhtml",
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc">
      <ol><li><a href="chapter1.xhtml">Chapter One</a></li></ol>
    </nav>
  </body>
</html>"#,
        );
        let mut issues = Vec::new();

        let document = super::open_with_issue_logger(&epub_path, |issue| {
            issues.push(issue.to_string());
        })
        .expect("parse EPUB with placeholder");

        assert!(matches!(
            &document.blocks[0],
            Block::Text(block) if block.text == "[malformed chapter: OEBPS/chapter1.xhtml]"
        ));
        assert_eq!(document.chapter_range_for_block(0), Some(ChapterRange {
            start_block: 0,
            end_block: 0,
        }));
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("malformed HTML: OEBPS/chapter1.xhtml"));
    }

    #[test]
    fn malformed_nav_is_logged_and_ignored_non_fatally() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_epub_with_files(
            &epub_path,
            &[(
                "chapter1",
                "chapter1.xhtml",
                r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body><p>Readable chapter.</p></body>
</html>"#,
            )],
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body><nav><ol><li>Broken
</html>"#,
            None,
        );
        let mut issues = Vec::new();

        let document = super::open_with_issue_logger(&epub_path, |issue| {
            issues.push(issue.to_string());
        })
        .expect("parse EPUB without TOC");

        assert!(matches!(
            &document.blocks[0],
            Block::Text(block) if block.text == "Readable chapter."
        ));
        assert!(document.toc.is_empty());
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("malformed HTML: OEBPS/nav.xhtml"));
    }

    #[test]
    fn missing_nav_file_is_logged_and_ignored_non_fatally() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_epub_without_nav_file(
            &epub_path,
            &[(
                "chapter1",
                "chapter1.xhtml",
                r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body><p>Readable chapter.</p></body>
</html>"#,
            )],
        );
        let mut issues = Vec::new();

        let document = super::open_with_issue_logger(&epub_path, |issue| {
            issues.push(issue.to_string());
        })
        .expect("parse EPUB without nav file");

        assert!(matches!(
            &document.blocks[0],
            Block::Text(block) if block.text == "Readable chapter."
        ));
        assert!(document.toc.is_empty());
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("malformed HTML: OEBPS/nav.xhtml"));
    }

    #[test]
    fn missing_image_data_is_logged_non_fatally() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub_without_image_file(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p><img src="images/missing.png" alt="Missing"/></p>
  </body>
</html>"#,
        );
        let mut issues = Vec::new();

        let document = super::open_with_issue_logger(&epub_path, |issue| {
            issues.push(issue.to_string());
        })
        .expect("parse EPUB");

        assert!(matches!(
            &document.blocks[0],
            Block::Image(block)
                if block.alt_text.as_deref() == Some("Missing")
                    && block.source_path.as_deref() == Some("OEBPS/images/missing.png")
                    && block.data.is_none()
        ));
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("bad image: OEBPS/images/missing.png"));
    }

    #[test]
    fn corrupt_image_data_is_logged_non_fatally() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_epub(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p><img src="images/corrupt.png" alt="Corrupt"/></p>
  </body>
</html>"#,
            Some(("OEBPS/images/corrupt.png", &[1, 2, 3])),
        );
        let mut issues = Vec::new();

        let document = super::open_with_issue_logger(&epub_path, |issue| {
            issues.push(issue.to_string());
        })
        .expect("parse EPUB");

        assert!(matches!(
            &document.blocks[0],
            Block::Image(block)
                if block.alt_text.as_deref() == Some("Corrupt")
                    && block.source_path.as_deref() == Some("OEBPS/images/corrupt.png")
                    && block.data.is_none()
        ));
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("bad image: OEBPS/images/corrupt.png"));
    }

    #[test]
    fn parses_nested_nav_toc_tree() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_epub_with_files(
            &epub_path,
            &[
                (
                    "chapter1",
                    "chapter1.xhtml",
                    r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body><h1>Chapter One</h1></body>
</html>"#,
                ),
                (
                    "chapter2",
                    "chapter2.xhtml",
                    r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body><h2>Section One</h2></body>
</html>"#,
                ),
            ],
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc">
      <ol>
        <li>
          <a href="chapter1.xhtml">Chapter One</a>
          <ol>
            <li><a href="chapter2.xhtml">Section One</a></li>
          </ol>
        </li>
      </ol>
    </nav>
  </body>
</html>"#,
            None,
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(document.toc.len(), 1);
        assert_eq!(document.toc[0].title, "Chapter One");
        assert_eq!(document.toc[0].target_block_index, 0);
        assert_eq!(document.toc[0].children.len(), 1);
        assert_eq!(document.toc[0].children[0].title, "Section One");
        assert_eq!(document.toc[0].children[0].target_block_index, 1);
    }

    #[test]
    fn parses_toc_nav_when_landmarks_precede_it() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_epub_with_files(
            &epub_path,
            &[(
                "chapter1",
                "chapter1.xhtml",
                r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body><h1>Chapter One</h1></body>
</html>"#,
            )],
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="landmarks">
      <ol><li><a href="cover.xhtml">Cover</a></li></ol>
    </nav>
    <nav epub:type="toc">
      <ol><li><a href="chapter1.xhtml">Chapter One</a></li></ol>
    </nav>
  </body>
</html>"#,
            None,
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(document.toc.len(), 1);
        assert_eq!(document.toc[0].title, "Chapter One");
        assert_eq!(document.toc[0].target_block_index, 0);
    }

    #[test]
    fn parses_toc_titles_with_inline_markup() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_epub_with_files(
            &epub_path,
            &[(
                "chapter1",
                "chapter1.xhtml",
                r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body><h1>Chapter One</h1></body>
</html>"#,
            )],
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc">
      <ol><li><a href="chapter1.xhtml"><span>Chapter</span> One</a></li></ol>
    </nav>
  </body>
</html>"#,
            None,
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(document.toc.len(), 1);
        assert_eq!(document.toc[0].title, "Chapter One");
    }

    fn write_minimal_epub(path: &Path, chapter_xhtml: &str) {
        let image = test_png_bytes();
        write_epub(
            path,
            chapter_xhtml,
            Some(("OEBPS/images/picture.png", image.as_slice())),
        );
    }

    fn write_minimal_epub_without_image_file(path: &Path, chapter_xhtml: &str) {
        write_epub(path, chapter_xhtml, None);
    }

    fn write_epub(path: &Path, chapter_xhtml: &str, image_file: Option<(&str, &[u8])>) {
        write_epub_with_files(
            path,
            &[("chapter1", "chapter1.xhtml", chapter_xhtml)],
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc">
      <ol><li><a href="chapter1.xhtml">Chapter One</a></li></ol>
    </nav>
  </body>
</html>"#,
            image_file,
        );
    }

    fn write_epub_with_files(
        path: &Path,
        chapters: &[(&str, &str, &str)],
        nav_xhtml: &str,
        image_file: Option<(&str, &[u8])>,
    ) {
        write_epub_with_extra_files(path, chapters, &[], nav_xhtml, image_file);
    }

    fn write_epub_with_extra_files(
        path: &Path,
        chapters: &[(&str, &str, &str)],
        extra_files: &[(&str, &str, &str, &str)],
        nav_xhtml: &str,
        image_file: Option<(&str, &[u8])>,
    ) {
        write_epub_with_extra_files_and_missing_extra_files(
            path,
            chapters,
            extra_files,
            &[],
            nav_xhtml,
            image_file,
        );
    }

    fn write_epub_with_missing_extra_files(
        path: &Path,
        chapters: &[(&str, &str, &str)],
        missing_extra_files: &[(&str, &str, &str)],
        nav_xhtml: &str,
    ) {
        write_epub_with_extra_files_and_missing_extra_files(
            path,
            chapters,
            &[],
            missing_extra_files,
            nav_xhtml,
            None,
        );
    }

    fn write_epub_with_extra_files_and_missing_extra_files(
        path: &Path,
        chapters: &[(&str, &str, &str)],
        extra_files: &[(&str, &str, &str, &str)],
        missing_extra_files: &[(&str, &str, &str)],
        nav_xhtml: &str,
        image_file: Option<(&str, &[u8])>,
    ) {
        write_epub_with_optional_nav_file(
            path,
            chapters,
            extra_files,
            missing_extra_files,
            Some(nav_xhtml),
            image_file,
        );
    }

    fn write_epub_without_nav_file(path: &Path, chapters: &[(&str, &str, &str)]) {
        write_epub_with_optional_nav_file(path, chapters, &[], &[], None, None);
    }

    fn write_epub_with_missing_chapter_file(
        path: &Path,
        chapter_id: &str,
        chapter_href: &str,
        nav_xhtml: &str,
    ) {
        let file = File::create(path).expect("create epub");
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default();

        write_zip_file(&mut writer, options, "mimetype", "application/epub+zip");
        write_zip_file(
            &mut writer,
            options,
            "META-INF/container.xml",
            r#"<?xml version="1.0"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
        );
        write_zip_file(
            &mut writer,
            options,
            "OEBPS/content.opf",
            &format!(
                r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="{chapter_id}" href="{chapter_href}" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="{chapter_id}"/>
  </spine>
</package>"#
            ),
        );
        write_zip_file(&mut writer, options, "OEBPS/nav.xhtml", nav_xhtml);

        writer.finish().expect("finish epub");
    }

    fn write_epub_with_optional_nav_file(
        path: &Path,
        chapters: &[(&str, &str, &str)],
        extra_files: &[(&str, &str, &str, &str)],
        missing_extra_files: &[(&str, &str, &str)],
        nav_xhtml: Option<&str>,
        image_file: Option<(&str, &[u8])>,
    ) {
        let file = File::create(path).expect("create epub");
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default();

        write_zip_file(
            &mut writer,
            options,
            "mimetype",
            "application/epub+zip",
        );
        write_zip_file(
            &mut writer,
            options,
            "META-INF/container.xml",
            r#"<?xml version="1.0"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
        );
        write_zip_file(
            &mut writer,
            options,
            "OEBPS/content.opf",
            &format!(
                r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
{}
  </manifest>
  <spine>
{}
  </spine>
"</package>"#,
                chapters
                    .iter()
                    .map(|(id, href, _)| format!(
                        r#"    <item id="{id}" href="{href}" media-type="application/xhtml+xml"/>"#
                    ))
                    .chain(extra_files.iter().map(|(id, href, media_type, _)| format!(
                        r#"    <item id="{id}" href="{href}" media-type="{media_type}"/>"#
                    )))
                    .chain(missing_extra_files.iter().map(|(id, href, media_type)| format!(
                        r#"    <item id="{id}" href="{href}" media-type="{media_type}"/>"#
                    )))
                    .collect::<Vec<_>>()
                    .join("\n"),
                chapters
                    .iter()
                    .map(|(id, _, _)| format!(r#"    <itemref idref="{id}"/>"#))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        );
        if let Some(nav_xhtml) = nav_xhtml {
            write_zip_file(&mut writer, options, "OEBPS/nav.xhtml", nav_xhtml);
        }
        for (_, href, chapter_xhtml) in chapters {
            write_zip_file(
                &mut writer,
                options,
                &format!("OEBPS/{href}"),
                chapter_xhtml,
            );
        }
        for (_, href, _, contents) in extra_files {
            write_zip_file(
                &mut writer,
                options,
                &format!("OEBPS/{href}"),
                contents,
            );
        }
        if let Some((name, contents)) = image_file {
            write_zip_bytes(&mut writer, options, name, contents);
        }

        writer.finish().expect("finish epub");
    }

    fn write_zip_file(
        writer: &mut ZipWriter<File>,
        options: SimpleFileOptions,
        name: &str,
        contents: &str,
    ) {
        writer.start_file(name, options).expect("start zip file");
        writer
            .write_all(contents.as_bytes())
            .expect("write zip file");
    }

    fn write_zip_bytes(
        writer: &mut ZipWriter<File>,
        options: SimpleFileOptions,
        name: &str,
        contents: &[u8],
    ) {
        writer.start_file(name, options).expect("start zip file");
        writer.write_all(contents).expect("write zip file");
    }

    fn test_png_bytes() -> Vec<u8> {
        let image = ::image::RgbaImage::from_fn(1, 1, |_, _| ::image::Rgba([0, 0, 0, 255]));
        let mut bytes = Cursor::new(Vec::new());
        ::image::DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ::image::ImageFormat::Png)
            .expect("encode png");
        bytes.into_inner()
    }
}
