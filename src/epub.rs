use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::document::{
    AnnotationRef, AnnotationStore, Block, ChapterRange, Document, ImageBlock, ListItemMarker,
    ListItemPresentation, TextBlock, TextBlockPresentation, TextBlockRole, TextStyle,
    TextStyleRange, TocNode,
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

pub fn open_with_image_loading(path: &Path, load_images: bool) -> Result<Document, EpubError> {
    open_with_issue_logger_and_image_loading(path, load_images, |_| {})
}

pub fn open_with_issue_logger(
    path: &Path,
    mut log_issue: impl FnMut(&str),
) -> Result<Document, EpubError> {
    open_with_issue_logger_and_image_loading(path, true, &mut log_issue)
}

pub fn open_with_issue_logger_and_image_loading(
    path: &Path,
    load_images: bool,
    mut log_issue: impl FnMut(&str),
) -> Result<Document, EpubError> {
    let file = File::open(path).map_err(|error| EpubError(error.to_string()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| EpubError(error.to_string()))?;

    let container_xml = read_zip_text(&mut archive, "META-INF/container.xml")?;
    let opf_path = find_opf_path(&container_xml)?;
    let opf_xml = read_zip_text(&mut archive, &opf_path)?;
    let opf_base = zip_parent(&opf_path);

    let package = parse_package(&opf_xml)?;
    let legacy_annotation_ids = discover_legacy_annotation_ids(&mut archive, &package, &opf_base);
    let empty_annotation_ids = HashSet::new();
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
                legacy_annotation_ids
                    .get(&chapter_path)
                    .unwrap_or(&empty_annotation_ids),
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
        if load_images {
            load_image_data(&mut archive, &mut parsed_chapter.blocks, &mut log_issue);
        }

        target_blocks_by_href.insert(chapter_path.clone(), start_block);
        for (fragment, relative_block_index) in &parsed_chapter.fragment_targets {
            target_blocks_by_href.insert(
                format!("{chapter_path}#{fragment}"),
                start_block + relative_block_index,
            );
        }
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
            || item
                .properties
                .split_whitespace()
                .any(|value| value == "nav")
            || item.media_type != "application/xhtml+xml"
        {
            continue;
        }

        let item_path = join_zip_path(&opf_base, &item.href);
        match read_zip_text(&mut archive, &item_path) {
            Ok(item_xml) => match parse_xhtml_annotations(
                &item_xml,
                &item_path,
                legacy_annotation_ids
                    .get(&item_path)
                    .unwrap_or(&empty_annotation_ids),
            ) {
                Ok(item_annotations) => annotations.extend(item_annotations),
                Err(error) => log_issue(&format!("malformed HTML: {item_path}: {error}")),
            },
            Err(error) => log_issue(&format!("malformed HTML: {item_path}: {error}")),
        }
    }

    let mut toc = Vec::new();
    for (toc_item, format) in package.toc_items() {
        let toc_path = join_zip_path(&opf_base, &toc_item.href);
        match read_zip_text(&mut archive, &toc_path) {
            Ok(toc_xml) => {
                let parsed_toc = match format {
                    TocFormat::Nav => parse_toc(&toc_xml, &toc_path, &target_blocks_by_href),
                    TocFormat::Ncx => parse_ncx(&toc_xml, &toc_path, &target_blocks_by_href),
                };
                match parsed_toc {
                    Ok(candidate) if !candidate.is_empty() => {
                        toc = candidate;
                        break;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        log_issue(&format!("malformed HTML: {toc_path}: {error}"));
                    }
                }
            }
            Err(error) => {
                log_issue(&format!("malformed HTML: {toc_path}: {error}"));
            }
        }
    }

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
    spine_toc_id: Option<String>,
}

impl Package {
    fn toc_items(&self) -> Vec<(&ManifestItem, TocFormat)> {
        let mut items = Vec::new();
        if let Some(nav_item) = self.manifest.values().find(|item| {
            item.properties
                .split_whitespace()
                .any(|value| value == "nav")
        }) {
            items.push((nav_item, TocFormat::Nav));
        }

        let ncx_item = self
            .spine_toc_id
            .as_ref()
            .and_then(|id| self.manifest.get(id))
            .or_else(|| {
                self.manifest
                    .values()
                    .find(|item| item.media_type == "application/x-dtbncx+xml")
            });
        if let Some(ncx_item) = ncx_item {
            items.push((ncx_item, TocFormat::Ncx));
        }

        items
    }
}

#[derive(Clone, Copy)]
enum TocFormat {
    Nav,
    Ncx,
}

struct ManifestItem {
    href: String,
    properties: String,
    media_type: String,
}

struct ParsedChapter {
    blocks: Vec<Block>,
    annotations: AnnotationStore,
    fragment_targets: HashMap<String, usize>,
}

const EXPLICIT_LINE_BREAK: char = '\u{0}';

fn malformed_chapter_placeholder(chapter_path: &str, chapter_index: usize) -> ParsedChapter {
    ParsedChapter {
        blocks: vec![Block::Text(TextBlock {
            text: format!("[malformed chapter: {chapter_path}]"),
            chapter_index,
            annotations: Vec::new(),
            styles: Vec::new(),
            presentation: TextBlockPresentation::default(),
        })],
        annotations: AnnotationStore::new(),
        fragment_targets: HashMap::new(),
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
    let spine_toc_id = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "spine")
        .and_then(|node| node.attribute("toc"))
        .map(str::to_string);

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
        spine_toc_id,
    })
}

struct LegacyLink {
    target: String,
    is_leading_in_block: bool,
}

fn discover_legacy_annotation_ids(
    archive: &mut zip::ZipArchive<File>,
    package: &Package,
    opf_base: &str,
) -> HashMap<String, HashSet<String>> {
    let mut links = HashMap::new();

    for item in package
        .manifest
        .values()
        .filter(|item| item.media_type == "application/xhtml+xml")
    {
        let document_path = join_zip_path(opf_base, &item.href);
        let Ok(xhtml) = read_zip_text(archive, &document_path) else {
            continue;
        };
        let Ok(document) = roxmltree::Document::parse(&xhtml) else {
            continue;
        };
        let document_base = zip_parent(&document_path);

        for anchor in document.descendants().filter(|node| {
            node.is_element()
                && node.tag_name().name() == "a"
                && node.attribute("id").is_some()
                && node.attribute("href").is_some()
        }) {
            let source = annotation_key(
                &document_path,
                anchor.attribute("id").expect("filtered anchor id"),
            );
            let Some(target) = annotation_key_from_href(
                anchor.attribute("href").expect("filtered anchor href"),
                &document_path,
                &document_base,
            ) else {
                continue;
            };
            links.insert(
                source,
                LegacyLink {
                    target,
                    is_leading_in_block: is_leading_link_in_text_block(anchor),
                },
            );
        }
    }

    let mut annotation_ids_by_path: HashMap<String, HashSet<String>> = HashMap::new();
    for (source, link) in &links {
        let Some(backlink) = links.get(&link.target) else {
            continue;
        };
        if backlink.target != *source || !backlink.is_leading_in_block || link.is_leading_in_block {
            continue;
        }
        let Some((document_path, id)) = link.target.rsplit_once('#') else {
            continue;
        };
        annotation_ids_by_path
            .entry(document_path.to_string())
            .or_default()
            .insert(id.to_string());
    }

    annotation_ids_by_path
}

fn is_leading_link_in_text_block(anchor: roxmltree::Node<'_, '_>) -> bool {
    let Some(container) = anchor
        .ancestors()
        .skip(1)
        .find(|node| node.is_element() && is_text_block_element(node.tag_name().name()))
    else {
        return false;
    };

    !container
        .descendants()
        .take_while(|node| *node != anchor)
        .any(|node| node.is_text() && node.text().is_some_and(|text| !text.trim().is_empty()))
}

fn parse_xhtml_chapter(
    xhtml: &str,
    chapter_index: usize,
    chapter_path: &str,
    chapter_base: &str,
    legacy_annotation_ids: &HashSet<String>,
) -> Result<ParsedChapter, EpubError> {
    let document = roxmltree::Document::parse(xhtml)
        .map_err(|error| EpubError(format!("invalid XHTML chapter: {error}")))?;
    let mut blocks = Vec::new();
    let annotations = annotations_from_document(&document, chapter_path, legacy_annotation_ids);
    let mut fragment_targets = HashMap::new();

    append_chapter_blocks(
        document.root_element(),
        chapter_index,
        chapter_path,
        chapter_base,
        legacy_annotation_ids,
        &mut blocks,
        &mut fragment_targets,
    );

    Ok(ParsedChapter {
        blocks,
        annotations,
        fragment_targets,
    })
}

fn parse_xhtml_annotations(
    xhtml: &str,
    document_path: &str,
    legacy_annotation_ids: &HashSet<String>,
) -> Result<AnnotationStore, EpubError> {
    let document = roxmltree::Document::parse(xhtml)
        .map_err(|error| EpubError(format!("invalid XHTML annotation document: {error}")))?;

    Ok(annotations_from_document(
        &document,
        document_path,
        legacy_annotation_ids,
    ))
}

fn annotations_from_document(
    document: &roxmltree::Document<'_>,
    document_path: &str,
    legacy_annotation_ids: &HashSet<String>,
) -> AnnotationStore {
    let mut annotations = AnnotationStore::new();

    for node in document
        .descendants()
        .filter(|node| node.is_element() && is_annotation_container(*node, legacy_annotation_ids))
    {
        let annotation_id = node.attribute("id").or_else(|| {
            node.descendants()
                .filter(|descendant| descendant.is_element())
                .filter_map(|descendant| descendant.attribute("id"))
                .find(|id| legacy_annotation_ids.contains(*id))
        });
        if let Some(id) = annotation_id {
            let text = if legacy_annotation_ids.contains(id) {
                node_text_excluding_id(node, id)
            } else {
                node_text(node)
            };

            if !text.is_empty() {
                annotations.insert(annotation_key(document_path, id), text);
            }
        }
    }

    annotations
}

fn parse_toc(
    nav_xml: &str,
    nav_path: &str,
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

    Ok(parse_toc_list(root_list, nav_path, target_blocks_by_href))
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
    nav_path: &str,
    target_blocks_by_href: &HashMap<String, usize>,
) -> Vec<TocNode> {
    list.children()
        .filter(|node| node.is_element() && node.tag_name().name() == "li")
        .filter_map(|node| parse_toc_item(node, nav_path, target_blocks_by_href))
        .collect()
}

fn parse_toc_item(
    item: roxmltree::Node<'_, '_>,
    nav_path: &str,
    target_blocks_by_href: &HashMap<String, usize>,
) -> Option<TocNode> {
    let link = item
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "a")?;
    let href = link.attribute("href")?;
    let target_key = resolve_href(nav_path, href);
    let target_block_index = *target_blocks_by_href.get(&target_key)?;
    let title = normalized_descendant_text(link);

    if title.is_empty() {
        return None;
    }

    let children = item
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "ol")
        .map(|list| parse_toc_list(list, nav_path, target_blocks_by_href))
        .unwrap_or_default();

    Some(TocNode {
        title,
        target_block_index,
        children,
    })
}

fn parse_ncx(
    ncx_xml: &str,
    ncx_path: &str,
    target_blocks_by_href: &HashMap<String, usize>,
) -> Result<Vec<TocNode>, EpubError> {
    let document = roxmltree::Document::parse(ncx_xml)
        .map_err(|error| EpubError(format!("invalid EPUB NCX document: {error}")))?;
    let Some(nav_map) = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "navMap")
    else {
        return Ok(Vec::new());
    };

    Ok(nav_map
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "navPoint")
        .filter_map(|node| parse_ncx_nav_point(node, ncx_path, target_blocks_by_href))
        .collect())
}

fn parse_ncx_nav_point(
    nav_point: roxmltree::Node<'_, '_>,
    ncx_path: &str,
    target_blocks_by_href: &HashMap<String, usize>,
) -> Option<TocNode> {
    let title = nav_point
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "navLabel")
        .map(normalized_descendant_text)?;
    if title.is_empty() {
        return None;
    }

    let href = nav_point
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "content")?
        .attribute("src")?;
    let target_key = resolve_href(ncx_path, href);
    let target_block_index = *target_blocks_by_href.get(&target_key)?;
    let children = nav_point
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "navPoint")
        .filter_map(|node| parse_ncx_nav_point(node, ncx_path, target_blocks_by_href))
        .collect();

    Some(TocNode {
        title,
        target_block_index,
        children,
    })
}

fn is_text_block_element(name: &str) -> bool {
    matches!(
        name,
        "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "div" | "figure" | "blockquote" | "li"
    )
}

fn append_chapter_blocks(
    node: roxmltree::Node<'_, '_>,
    chapter_index: usize,
    chapter_path: &str,
    chapter_base: &str,
    legacy_annotation_ids: &HashSet<String>,
    blocks: &mut Vec<Block>,
    fragment_targets: &mut HashMap<String, usize>,
) {
    for child in node.children().filter(|child| child.is_element()) {
        if is_annotation_container(child, legacy_annotation_ids)
            || has_annotation_ancestor(child, legacy_annotation_ids)
        {
            continue;
        }

        if is_text_block_element(child.tag_name().name()) {
            let block_offset = blocks.len();
            blocks.extend(blocks_from_xhtml_element(
                child,
                chapter_index,
                chapter_path,
                chapter_base,
                block_offset,
                fragment_targets,
            ));
        } else {
            append_chapter_blocks(
                child,
                chapter_index,
                chapter_path,
                chapter_base,
                legacy_annotation_ids,
                blocks,
                fragment_targets,
            );
        }
    }
}

fn is_annotation_container(
    node: roxmltree::Node<'_, '_>,
    legacy_annotation_ids: &HashSet<String>,
) -> bool {
    let is_annotation_collection = attribute_contains_any_token(node, "role", &["doc-endnotes"])
        || attribute_contains_any_token(node, "type", &["endnotes", "rearnotes"]);
    let is_semantic_annotation = node.attribute("id").is_some()
        && (matches!(node.tag_name().name(), "aside" | "note")
            || attribute_contains_any_token(node, "type", &["footnote", "endnote"])
            || attribute_contains_any_token(node, "role", &["doc-footnote", "doc-endnote"]));
    let is_legacy_annotation = is_text_block_element(node.tag_name().name())
        && node.descendants().any(|descendant| {
            descendant.is_element()
                && descendant
                    .attribute("id")
                    .is_some_and(|id| legacy_annotation_ids.contains(id))
        });
    let is_endnote_collection_entry = node.attribute("id").is_some()
        && is_text_block_element(node.tag_name().name())
        && node.ancestors().skip(1).any(|ancestor| {
            ancestor.is_element()
                && (attribute_contains_any_token(ancestor, "role", &["doc-endnotes"])
                    || attribute_contains_any_token(ancestor, "type", &["endnotes", "rearnotes"]))
        });

    is_annotation_collection
        || is_semantic_annotation
        || is_legacy_annotation
        || is_endnote_collection_entry
}

fn attribute_contains_any_token(
    node: roxmltree::Node<'_, '_>,
    attribute_name: &str,
    expected: &[&str],
) -> bool {
    node.attributes().any(|attribute| {
        attribute.name() == attribute_name
            && attribute
                .value()
                .split_whitespace()
                .any(|value| expected.contains(&value))
    })
}

fn has_annotation_ancestor(
    node: roxmltree::Node<'_, '_>,
    legacy_annotation_ids: &HashSet<String>,
) -> bool {
    node.ancestors().skip(1).any(|ancestor| {
        ancestor.is_element() && is_annotation_container(ancestor, legacy_annotation_ids)
    })
}

fn is_non_visible_element(node: roxmltree::Node<'_, '_>) -> bool {
    matches!(node.tag_name().name(), "script" | "style")
}

fn is_annotation_backlink(node: roxmltree::Node<'_, '_>) -> bool {
    attribute_contains_any_token(node, "role", &["doc-backlink"])
}

fn node_text(node: roxmltree::Node<'_, '_>) -> String {
    let mut blocks = Vec::new();
    append_annotation_text_blocks(node, &mut blocks);

    if !blocks.is_empty() {
        return blocks.join("\n");
    }

    normalized_descendant_text(node)
}

fn node_text_excluding_id(node: roxmltree::Node<'_, '_>, excluded_id: &str) -> String {
    let mut text = String::new();
    append_descendant_text_excluding_id(node, excluded_id, &mut text);

    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn append_descendant_text_excluding_id(
    node: roxmltree::Node<'_, '_>,
    excluded_id: &str,
    text: &mut String,
) {
    for child in node.children() {
        if child.is_text() {
            if let Some(child_text) = child.text() {
                text.push_str(child_text);
            }
        } else if child.is_element() {
            if is_non_visible_element(child)
                || is_annotation_backlink(child)
                || child.attribute("id") == Some(excluded_id)
            {
                continue;
            }

            if child.tag_name().name() == "br" {
                text.push('\n');
            } else {
                append_descendant_text_excluding_id(child, excluded_id, text);
            }
        }
    }
}

fn append_annotation_text_blocks(node: roxmltree::Node<'_, '_>, blocks: &mut Vec<String>) {
    for child in node.children().filter(|child| child.is_element()) {
        if is_non_visible_element(child) || is_annotation_backlink(child) {
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
            if is_non_visible_element(child) || is_annotation_backlink(child) {
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
    block_offset: usize,
    fragment_targets: &mut HashMap<String, usize>,
) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut text = String::new();
    let mut annotation_refs = Vec::new();
    let mut style_ranges = Vec::new();
    let mut context = VisibleBlockContext {
        chapter_index,
        chapter_path,
        chapter_base,
        block_offset,
        fragment_targets,
        text_style: TextStyle::default(),
        presentation: presentation_for_node(node),
    };
    let mut output = VisibleBlockOutput {
        blocks: &mut blocks,
        text: &mut text,
        annotation_refs: &mut annotation_refs,
        style_ranges: &mut style_ranges,
    };

    append_visible_blocks(node, &mut context, &mut output);
    flush_text_block(
        &mut blocks,
        chapter_index,
        &mut text,
        &mut annotation_refs,
        &mut style_ranges,
        context.presentation,
    );

    if !blocks.is_empty()
        && let Some(id) = node.attribute("id")
    {
        fragment_targets.insert(id.to_string(), block_offset);
    }

    blocks
}

struct VisibleBlockContext<'a> {
    chapter_index: usize,
    chapter_path: &'a str,
    chapter_base: &'a str,
    block_offset: usize,
    fragment_targets: &'a mut HashMap<String, usize>,
    text_style: TextStyle,
    presentation: TextBlockPresentation,
}

struct VisibleBlockOutput<'a> {
    blocks: &'a mut Vec<Block>,
    text: &'a mut String,
    annotation_refs: &'a mut Vec<AnnotationRef>,
    style_ranges: &'a mut Vec<TextStyleRange>,
}

fn append_visible_blocks(
    node: roxmltree::Node<'_, '_>,
    context: &mut VisibleBlockContext<'_>,
    output: &mut VisibleBlockOutput<'_>,
) {
    for child in node.children() {
        if child.is_text() {
            if let Some(child_text) = child.text() {
                let start = output.text.len();
                output.text.push_str(child_text);
                if context.text_style != TextStyle::default() && start < output.text.len() {
                    output.style_ranges.push(TextStyleRange {
                        start,
                        end: output.text.len(),
                        style: context.text_style,
                    });
                }
            }
        } else if child.is_element() {
            if is_non_visible_element(child) {
                continue;
            }

            if child.tag_name().name() == "br" {
                output.text.push(EXPLICIT_LINE_BREAK);
                continue;
            }

            if is_text_block_element(child.tag_name().name()) {
                flush_text_block(
                    output.blocks,
                    context.chapter_index,
                    output.text,
                    output.annotation_refs,
                    output.style_ranges,
                    context.presentation,
                );
                let child_block_offset = context.block_offset + output.blocks.len();
                output.blocks.extend(blocks_from_xhtml_element(
                    child,
                    context.chapter_index,
                    context.chapter_path,
                    context.chapter_base,
                    child_block_offset,
                    context.fragment_targets,
                ));
                continue;
            }

            if child.tag_name().name() == "img" {
                flush_text_block(
                    output.blocks,
                    context.chapter_index,
                    output.text,
                    output.annotation_refs,
                    output.style_ranges,
                    context.presentation,
                );
                if let Some(id) = child.attribute("id") {
                    context
                        .fragment_targets
                        .insert(id.to_string(), context.block_offset + output.blocks.len());
                }
                output.blocks.push(Block::Image(ImageBlock {
                    alt_text: child.attribute("alt").map(str::to_string),
                    source_path: child
                        .attribute("src")
                        .map(|source_path| join_zip_path(context.chapter_base, source_path)),
                    data: None,
                    chapter_index: context.chapter_index,
                }));
                continue;
            }

            if let Some(id) = child.attribute("id") {
                context
                    .fragment_targets
                    .insert(id.to_string(), context.block_offset + output.blocks.len());
            }

            if let Some(id) = child.attribute("href").and_then(|href| {
                annotation_key_from_href(href, context.chapter_path, context.chapter_base)
            }) {
                output.annotation_refs.push(AnnotationRef {
                    id,
                    offset: output.text.len(),
                });
            }

            let inherited_style = context.text_style;
            context.text_style = style_for_element(inherited_style, child.tag_name().name());
            append_visible_blocks(child, context, output);
            context.text_style = inherited_style;
        }
    }
}

fn style_for_element(mut style: TextStyle, element_name: &str) -> TextStyle {
    if matches!(element_name, "strong" | "b") {
        style.bold = true;
    }
    if matches!(element_name, "em" | "i") {
        style.italic = true;
    }
    if matches!(element_name, "u" | "ins") {
        style.underlined = true;
    }
    if matches!(element_name, "s" | "strike" | "del") {
        style.crossed_out = true;
    }
    style
}

fn presentation_for_node(node: roxmltree::Node<'_, '_>) -> TextBlockPresentation {
    let element_name = node.tag_name().name();
    let role = element_name
        .strip_prefix('h')
        .and_then(|level| level.parse::<u8>().ok())
        .filter(|level| (1..=6).contains(level))
        .map(TextBlockRole::Heading)
        .unwrap_or(TextBlockRole::Paragraph);
    let quote_depth = node
        .ancestors()
        .filter(|ancestor| ancestor.is_element() && ancestor.tag_name().name() == "blockquote")
        .count();
    let list_item = list_item_presentation(node);
    TextBlockPresentation {
        role,
        quote_depth,
        list_item,
    }
}

fn list_item_presentation(node: roxmltree::Node<'_, '_>) -> Option<ListItemPresentation> {
    let list_item = node
        .ancestors()
        .find(|ancestor| ancestor.is_element() && ancestor.tag_name().name() == "li")?;
    let list = list_item.ancestors().skip(1).find(|ancestor| {
        ancestor.is_element() && matches!(ancestor.tag_name().name(), "ul" | "ol")
    })?;
    let list_depth = list_item
        .ancestors()
        .filter(|ancestor| {
            ancestor.is_element() && matches!(ancestor.tag_name().name(), "ul" | "ol")
        })
        .count()
        .saturating_sub(1);
    let item_index = list
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == "li")
        .position(|child| child == list_item)
        .unwrap_or(0) as i64;
    let marker = if list.tag_name().name() == "ol" {
        let start = list
            .attribute("start")
            .and_then(|start| start.parse::<i64>().ok())
            .unwrap_or(1);
        ListItemMarker::Ordered(start.saturating_add(item_index))
    } else {
        ListItemMarker::Bullet
    };
    let first_item_block = list_item.descendants().skip(1).find(|candidate| {
        candidate.is_element()
            && is_text_block_element(candidate.tag_name().name())
            && candidate
                .ancestors()
                .find(|ancestor| ancestor.is_element() && ancestor.tag_name().name() == "li")
                == Some(list_item)
    });
    let continuation = node != list_item && first_item_block.is_some_and(|first| first != node);

    Some(ListItemPresentation {
        depth: list_depth,
        marker,
        continuation,
    })
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

fn resolve_href(source_path: &str, href: &str) -> String {
    let (target, fragment) = href
        .split_once('#')
        .map_or((href, None), |(target, fragment)| (target, Some(fragment)));
    let target_path = if target.is_empty() {
        source_path.to_string()
    } else {
        join_zip_path(&zip_parent(source_path), target)
    };

    fragment
        .filter(|fragment| !fragment.is_empty())
        .map_or(target_path.clone(), |fragment| {
            format!("{target_path}#{fragment}")
        })
}

fn flush_text_block(
    blocks: &mut Vec<Block>,
    chapter_index: usize,
    text: &mut String,
    annotation_refs: &mut Vec<AnnotationRef>,
    style_ranges: &mut Vec<TextStyleRange>,
    presentation: TextBlockPresentation,
) {
    let (normalized, annotations, styles) =
        normalize_visible_content(text, annotation_refs, style_ranges);

    if !normalized.is_empty() {
        blocks.push(Block::Text(TextBlock {
            text: normalized,
            chapter_index,
            annotations,
            styles,
            presentation,
        }));
    }

    text.clear();
    annotation_refs.clear();
    style_ranges.clear();
}

fn normalize_visible_content(
    text: &str,
    annotation_refs: &[AnnotationRef],
    style_ranges: &[TextStyleRange],
) -> (String, Vec<AnnotationRef>, Vec<TextStyleRange>) {
    let mut normalized = String::new();
    let mut pending_space = false;
    let mut normalized_offsets = vec![None; text.len().saturating_add(1)];

    for (byte_index, character) in text.char_indices() {
        normalized_offsets[byte_index] = Some(normalized_offset(&normalized, pending_space));

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
    normalized_offsets[text.len()] = Some(normalized_offset(&normalized, pending_space));

    let normalized_annotation_refs = annotation_refs
        .iter()
        .map(|annotation_ref| AnnotationRef {
            id: annotation_ref.id.clone(),
            offset: mapped_offset(&normalized_offsets, annotation_ref.offset),
        })
        .collect();

    let mut normalized_style_ranges = style_ranges
        .iter()
        .filter_map(|range| {
            let start = mapped_offset(&normalized_offsets, range.start);
            let end = mapped_offset(&normalized_offsets, range.end);
            (start < end).then_some(TextStyleRange {
                start,
                end,
                style: range.style,
            })
        })
        .collect::<Vec<_>>();
    merge_adjacent_style_ranges(&mut normalized_style_ranges);
    normalized_style_ranges = normalized_style_ranges
        .into_iter()
        .filter_map(|range| {
            trim_style_range(&normalized, range.start, range.end).map(|(start, end)| {
                TextStyleRange {
                    start,
                    end,
                    style: range.style,
                }
            })
        })
        .collect();
    merge_adjacent_style_ranges(&mut normalized_style_ranges);

    (
        normalized,
        normalized_annotation_refs,
        normalized_style_ranges,
    )
}

fn mapped_offset(offsets: &[Option<usize>], source_offset: usize) -> usize {
    offsets
        .get(source_offset)
        .and_then(|offset| *offset)
        .unwrap_or_else(|| offsets.iter().rev().find_map(|offset| *offset).unwrap_or(0))
}

fn trim_style_range(text: &str, mut start: usize, mut end: usize) -> Option<(usize, usize)> {
    while start < end {
        let character = text.get(start..end)?.chars().next()?;
        if !character.is_whitespace() {
            break;
        }
        start += character.len_utf8();
    }
    while start < end {
        let character = text.get(start..end)?.chars().next_back()?;
        if !character.is_whitespace() {
            break;
        }
        end -= character.len_utf8();
    }
    (start < end).then_some((start, end))
}

fn merge_adjacent_style_ranges(ranges: &mut Vec<TextStyleRange>) {
    let mut merged: Vec<TextStyleRange> = Vec::with_capacity(ranges.len());
    for range in ranges.drain(..) {
        if let Some(previous) = merged.last_mut()
            && previous.style == range.style
            && previous.end == range.start
        {
            previous.end = range.end;
        } else {
            merged.push(range);
        }
    }
    *ranges = merged;
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
    use zip::{CompressionMethod, ZipWriter};

    use crate::document::{
        AnnotationRef, Block, ChapterRange, ListItemMarker, TextBlockRole, TextStyle,
        TextStyleRange,
    };

    use super::open;

    #[test]
    fn preserves_strong_emphasis_as_bold_style_range_after_whitespace_normalization() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>Before <strong> bold  text </strong> after.</p>
  </body>
</html>"#,
        );

        let document = open(&epub_path).expect("parse EPUB");
        let block = document.text_block(0).expect("first text block");

        assert_eq!(block.text, "Before bold text after.");
        assert_eq!(
            block.styles,
            vec![TextStyleRange {
                start: "Before ".len(),
                end: "Before bold text".len(),
                style: TextStyle {
                    bold: true,
                    ..TextStyle::default()
                },
            }]
        );
    }

    #[test]
    fn keeps_unicode_style_ranges_on_utf8_character_boundaries() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body><p>前 <strong>加粗😊</strong> 后。</p></body>
</html>"#,
        );

        let document = open(&epub_path).expect("parse EPUB");
        let block = document.text_block(0).expect("first text block");
        let range = block.styles.first().expect("bold range");

        assert!(block.text.is_char_boundary(range.start));
        assert!(block.text.is_char_boundary(range.end));
        assert_eq!(&block.text[range.start..range.end], "加粗😊");
    }

    #[test]
    fn flattens_nested_bold_and_italic_tags_into_effective_style_ranges() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p><strong>Bold <em>both</em></strong> <i>italic</i>.</p>
  </body>
</html>"#,
        );

        let document = open(&epub_path).expect("parse EPUB");
        let block = document.text_block(0).expect("first text block");

        assert_eq!(block.text, "Bold both italic.");
        assert_eq!(
            block.styles,
            vec![
                TextStyleRange {
                    start: 0,
                    end: "Bold".len(),
                    style: TextStyle {
                        bold: true,
                        ..TextStyle::default()
                    },
                },
                TextStyleRange {
                    start: "Bold ".len(),
                    end: "Bold both".len(),
                    style: TextStyle {
                        bold: true,
                        italic: true,
                        ..TextStyle::default()
                    },
                },
                TextStyleRange {
                    start: "Bold both ".len(),
                    end: "Bold both italic".len(),
                    style: TextStyle {
                        italic: true,
                        ..TextStyle::default()
                    },
                },
            ]
        );
    }

    #[test]
    fn preserves_underline_and_deletion_tag_aliases_as_style_ranges() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p><u>under</u> <ins>inserted</ins> <s>old</s> <strike>struck</strike> <del>deleted</del>.</p>
  </body>
</html>"#,
        );

        let document = open(&epub_path).expect("parse EPUB");
        let block = document.text_block(0).expect("first text block");
        let expected = [
            (
                "under",
                TextStyle {
                    underlined: true,
                    ..TextStyle::default()
                },
            ),
            (
                "inserted",
                TextStyle {
                    underlined: true,
                    ..TextStyle::default()
                },
            ),
            (
                "old",
                TextStyle {
                    crossed_out: true,
                    ..TextStyle::default()
                },
            ),
            (
                "struck",
                TextStyle {
                    crossed_out: true,
                    ..TextStyle::default()
                },
            ),
            (
                "deleted",
                TextStyle {
                    crossed_out: true,
                    ..TextStyle::default()
                },
            ),
        ];

        assert_eq!(block.text, "under inserted old struck deleted.");
        assert_eq!(block.styles.len(), expected.len());
        for (range, (content, style)) in block.styles.iter().zip(expected) {
            assert_eq!(&block.text[range.start..range.end], content);
            assert_eq!(range.style, style);
        }
    }

    #[test]
    fn preserves_heading_levels_as_block_presentation() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <h1>Book title</h1>
    <h3>Section title</h3>
  </body>
</html>"#,
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(
            document.text_block(0).map(|block| block.presentation.role),
            Some(TextBlockRole::Heading(1)),
        );
        assert_eq!(
            document.text_block(1).map(|block| block.presentation.role),
            Some(TextBlockRole::Heading(3)),
        );
    }

    #[test]
    fn preserves_nested_blockquote_depth_on_flat_text_blocks() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <blockquote>Outer quote.<blockquote><p>Nested quote.</p></blockquote></blockquote>
  </body>
</html>"#,
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(
            document.text_block(0).map(|block| block.text.as_str()),
            Some("Outer quote.")
        );
        assert_eq!(
            document
                .text_block(0)
                .map(|block| block.presentation.quote_depth),
            Some(1)
        );
        assert_eq!(
            document.text_block(1).map(|block| block.text.as_str()),
            Some("Nested quote.")
        );
        assert_eq!(
            document
                .text_block(1)
                .map(|block| block.presentation.quote_depth),
            Some(2)
        );
    }

    #[test]
    fn preserves_nested_list_kind_depth_and_ordered_start() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <ul>
      <li>Bullet
        <ol start="3"><li>Third</li><li>Fourth</li></ol>
      </li>
    </ul>
  </body>
</html>"#,
        );

        let document = open(&epub_path).expect("parse EPUB");
        let items = document
            .blocks
            .iter()
            .filter_map(|block| match block {
                Block::Text(block) => Some((block.text.as_str(), block.presentation.list_item)),
                Block::Image(_) => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].0, "Bullet");
        assert_eq!(
            items[0].1.map(|item| (item.depth, item.marker)),
            Some((0, ListItemMarker::Bullet))
        );
        assert_eq!(items[1].0, "Third");
        assert_eq!(
            items[1].1.map(|item| (item.depth, item.marker)),
            Some((1, ListItemMarker::Ordered(3)))
        );
        assert_eq!(items[2].0, "Fourth");
        assert_eq!(
            items[2].1.map(|item| (item.depth, item.marker)),
            Some((1, ListItemMarker::Ordered(4)))
        );
    }

    #[test]
    fn marks_later_paragraphs_in_one_list_item_as_continuations() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <ol start="4"><li><p>First paragraph.</p><p>Continuation paragraph.</p></li></ol>
  </body>
</html>"#,
        );

        let document = open(&epub_path).expect("parse EPUB");
        let first = document
            .text_block(0)
            .and_then(|block| block.presentation.list_item)
            .expect("first list paragraph");
        let continuation = document
            .text_block(1)
            .and_then(|block| block.presentation.list_item)
            .expect("continued list paragraph");

        assert_eq!(first.marker, ListItemMarker::Ordered(4));
        assert!(!first.continuation);
        assert_eq!(continuation.marker, ListItemMarker::Ordered(4));
        assert!(continuation.continuation);
    }

    #[test]
    fn keeps_style_and_annotation_offsets_on_the_same_normalized_text() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p><em>Text   <a href="#note-1">[1]</a></em>.</p>
    <aside id="note-1"><p>Footnote.</p></aside>
  </body>
</html>"##,
        );

        let document = open(&epub_path).expect("parse EPUB");
        let block = document.text_block(0).expect("first text block");
        let marker_start = block.text.find("[1]").expect("marker");

        assert_eq!(block.annotations[0].offset, marker_start);
        assert_eq!(
            &block.text[block.styles[0].start..block.styles[0].end],
            "Text [1]"
        );
        assert!(block.styles[0].style.italic);
    }

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
    fn parses_deflate_compressed_epub() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_epub_with_options(
            &epub_path,
            &[(
                "chapter1",
                "chapter1.xhtml",
                r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body><p>Compressed chapter.</p></body>
</html>"#,
            )],
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc">
      <ol><li><a href="chapter1.xhtml">Chapter One</a></li></ol>
    </nav>
  </body>
</html>"#,
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        );

        let document = open(&epub_path).expect("parse compressed EPUB");

        assert_eq!(
            document.text_block(0).map(|block| block.text.as_str()),
            Some("Compressed chapter.")
        );
        assert_eq!(document.toc.len(), 1);
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
    fn normalizes_source_whitespace_without_exposing_xhtml_markup() {
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
    fn extracts_epub_typed_footnotes_from_list_items() {
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
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <ol>
      <li id="note-1" epub:type="footnote"><p>List footnote text.</p></li>
    </ol>
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
        let annotation = &document
            .text_block(0)
            .expect("first text block")
            .annotations[0];

        assert_eq!(
            document.annotation_text(&annotation.id),
            Some("List footnote text.")
        );
    }

    #[test]
    fn extracts_dpub_aria_footnotes_and_endnotes() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>
      Footnote<a id="source-1" href="#note-1" role="doc-noteref">1</a>.
      Endnote<a href="#note-2" role="doc-noteref">2</a>.
    </p>
    <div id="note-1" role="doc-footnote">
      <a href="#source-1" role="doc-backlink">[1]</a>
      ARIA footnote text.
    </div>
    <li id="note-2" role="doc-endnote">ARIA endnote text.</li>
  </body>
</html>"##,
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(
            document.annotation_text("OEBPS/chapter1.xhtml#note-1"),
            Some("ARIA footnote text.")
        );
        assert_eq!(
            document.annotation_text("OEBPS/chapter1.xhtml#note-2"),
            Some("ARIA endnote text.")
        );
    }

    #[test]
    fn extracts_entries_from_dpub_aria_endnotes_collections() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>Source<a href="#note-1" role="doc-noteref">1</a>.</p>
    <section role="doc-endnotes">
      <h2>Notes</h2>
      <ol>
        <li id="note-1">Collection endnote text.</li>
      </ol>
    </section>
  </body>
</html>"##,
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(
            document.annotation_text("OEBPS/chapter1.xhtml#note-1"),
            Some("Collection endnote text.")
        );
        assert_eq!(document.blocks.len(), 1);
    }

    #[test]
    fn extracts_epub2_notes_linked_back_to_their_source_anchor() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_epub_with_extra_files(
            &epub_path,
            &[
                (
                    "chapter1",
                    "chapter1.xhtml",
                    r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>Source sentence<a id="source-1" href="notes.xhtml#note-1">1</a>.</p>
  </body>
</html>"##,
                ),
                (
                    "notes",
                    "notes.xhtml",
                    r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p class="publisher-note"><a id="note-1" href="chapter1.xhtml#source-1">[1]</a>Legacy note text.</p>
  </body>
</html>"##,
                ),
            ],
            &[],
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

        assert_eq!(block.text, "Source sentence1.");
        assert_eq!(
            block.annotations,
            vec![AnnotationRef {
                id: "OEBPS/notes.xhtml#note-1".to_string(),
                offset: "Source sentence".len(),
            }]
        );
        assert_eq!(
            document.annotation_text("OEBPS/notes.xhtml#note-1"),
            Some("Legacy note text.")
        );
        assert_eq!(document.blocks.len(), 1);
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
        assert_eq!(document.annotation_text("OEBPS/notes.xhtml#note-1"), None);
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
        assert_eq!(document.annotation_text("OEBPS/notes.xhtml#note-1"), None);
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
    fn keeps_style_ranges_valid_across_line_breaks_and_image_splits() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p><strong>Before<br/>After<img src="images/picture.png" alt="Picture"/>Tail</strong></p>
  </body>
</html>"#,
        );

        let document = open(&epub_path).expect("parse EPUB");
        let first = document.text_block(0).expect("text before image");
        let tail = document.text_block(2).expect("text after image");
        let styled_text = |block: &crate::document::TextBlock| {
            block
                .styles
                .iter()
                .map(|range| block.text[range.start..range.end].to_string())
                .collect::<Vec<_>>()
        };

        assert_eq!(first.text, "Before\nAfter");
        assert_eq!(
            styled_text(first),
            vec!["Before".to_string(), "After".to_string()]
        );
        assert_eq!(tail.text, "Tail");
        assert_eq!(styled_text(tail), vec!["Tail".to_string()]);
        assert!(
            first
                .styles
                .iter()
                .chain(&tail.styles)
                .all(|range| range.style.bold)
        );
    }

    #[test]
    fn can_open_without_loading_image_bytes() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_minimal_epub(
            &epub_path,
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p><img src="images/picture.png" alt="Picture"/></p>
  </body>
</html>"#,
        );

        let document =
            super::open_with_image_loading(&epub_path, false).expect("parse EPUB without images");

        assert!(matches!(
            &document.blocks[0],
            Block::Image(block)
                if block.source_path.as_deref() == Some("OEBPS/images/picture.png")
                    && block.data.is_none()
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
        assert_eq!(
            document.chapter_range_for_block(0),
            Some(ChapterRange {
                start_block: 0,
                end_block: 0,
            })
        );
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
    fn malformed_nav_falls_back_to_valid_ncx() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_epub_with_extra_files(
            &epub_path,
            &[(
                "chapter1",
                "chapter1.xhtml",
                r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body><p>Readable chapter.</p></body>
</html>"#,
            )],
            &[(
                "ncx",
                "toc.ncx",
                "application/x-dtbncx+xml",
                r#"<?xml version="1.0"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/">
  <navMap>
    <navPoint id="chapter-one">
      <navLabel><text>Chapter One</text></navLabel>
      <content src="chapter1.xhtml"/>
    </navPoint>
  </navMap>
</ncx>"#,
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
        .expect("parse EPUB with NCX fallback");

        assert_eq!(document.toc.len(), 1);
        assert_eq!(document.toc[0].title, "Chapter One");
        assert_eq!(document.toc[0].target_block_index, 0);
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
    fn parses_epub2_ncx_toc_tree() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_epub2_with_ncx(&epub_path);

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(document.toc.len(), 1);
        assert_eq!(document.toc[0].title, "Chapter One");
        assert_eq!(document.toc[0].target_block_index, 0);
        assert_eq!(document.toc[0].children.len(), 1);
        assert_eq!(document.toc[0].children[0].title, "Section Two");
        assert_eq!(document.toc[0].children[0].target_block_index, 2);
    }

    #[test]
    fn resolves_toc_fragments_to_blocks_within_a_chapter() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_epub_with_files(
            &epub_path,
            &[(
                "chapter1",
                "chapter1.xhtml",
                r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <h1 id="chapter-one">Chapter One</h1>
    <p>Opening paragraph.</p>
    <h2 id="section-two">Section Two</h2>
  </body>
</html>"#,
            )],
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc">
      <ol>
        <li><a href="chapter1.xhtml#chapter-one">Chapter One</a></li>
        <li><a href="chapter1.xhtml#section-two">Section Two</a></li>
      </ol>
    </nav>
  </body>
</html>"#,
            None,
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(document.toc[0].target_block_index, 0);
        assert_eq!(document.toc[1].target_block_index, 2);
    }

    #[test]
    fn resolves_inline_fragment_targets_to_their_containing_block() {
        let tempdir = tempdir().expect("temp dir");
        let epub_path = tempdir.path().join("book.epub");
        write_epub_with_files(
            &epub_path,
            &[(
                "chapter1",
                "chapter1.xhtml",
                r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>Opening paragraph.</p>
    <p><span id="section-two">Section Two.</span></p>
  </body>
</html>"#,
            )],
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc">
      <ol><li><a href="chapter1.xhtml#section-two">Section Two</a></li></ol>
    </nav>
  </body>
</html>"#,
            None,
        );

        let document = open(&epub_path).expect("parse EPUB");

        assert_eq!(document.toc.len(), 1);
        assert_eq!(document.toc[0].target_block_index, 1);
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

    fn write_epub2_with_ncx(path: &Path) {
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
            r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="chapter1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx">
    <itemref idref="chapter1"/>
  </spine>
</package>"#,
        );
        write_zip_file(
            &mut writer,
            options,
            "OEBPS/chapter1.xhtml",
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <h1 id="chapter-one">Chapter One</h1>
    <p>Opening paragraph.</p>
    <h2 id="section-two">Section Two</h2>
  </body>
</html>"#,
        );
        write_zip_file(
            &mut writer,
            options,
            "OEBPS/toc.ncx",
            r#"<?xml version="1.0"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/">
  <navMap>
    <navPoint id="chapter-one">
      <navLabel><text>Chapter One</text></navLabel>
      <content src="chapter1.xhtml#chapter-one"/>
      <navPoint id="section-two">
        <navLabel><text>Section Two</text></navLabel>
        <content src="chapter1.xhtml#section-two"/>
      </navPoint>
    </navPoint>
  </navMap>
</ncx>"#,
        );

        writer.finish().expect("finish epub");
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
        write_epub_with_optional_nav_file_and_options(
            path,
            chapters,
            extra_files,
            missing_extra_files,
            nav_xhtml,
            image_file,
            SimpleFileOptions::default(),
        );
    }

    fn write_epub_with_options(
        path: &Path,
        chapters: &[(&str, &str, &str)],
        nav_xhtml: &str,
        options: SimpleFileOptions,
    ) {
        write_epub_with_optional_nav_file_and_options(
            path,
            chapters,
            &[],
            &[],
            Some(nav_xhtml),
            None,
            options,
        );
    }

    fn write_epub_with_optional_nav_file_and_options(
        path: &Path,
        chapters: &[(&str, &str, &str)],
        extra_files: &[(&str, &str, &str, &str)],
        missing_extra_files: &[(&str, &str, &str)],
        nav_xhtml: Option<&str>,
        image_file: Option<(&str, &[u8])>,
        options: SimpleFileOptions,
    ) {
        let file = File::create(path).expect("create epub");
        let mut writer = ZipWriter::new(file);

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
                    .chain(
                        missing_extra_files
                            .iter()
                            .map(|(id, href, media_type)| format!(
                                r#"    <item id="{id}" href="{href}" media-type="{media_type}"/>"#
                            ))
                    )
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
            write_zip_file(&mut writer, options, &format!("OEBPS/{href}"), contents);
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
