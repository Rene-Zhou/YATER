use std::fs::File;
use std::io::Cursor;
use std::io::Write;
use std::path::{Path, PathBuf};

use tempfile::tempdir;
use yater::document::{Block, Document, ImageBlock};
use yater::epub;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

#[test]
fn generated_epub_corpus_covers_parser_scenarios() {
    let tempdir = tempdir().expect("temp dir");
    let corpus = build_epub_corpus(tempdir.path());

    let names = corpus.iter().map(|entry| entry.name).collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "epub2-ncx-legacy-footnote.epub",
            "epub3-bad-image.epub",
            "epub3-external-footnote.epub",
            "epub3-image-png.epub",
            "epub3-malformed-chapter.epub",
            "epub3-multilingual.epub",
        ]
    );

    assert_epub2_ncx_legacy_footnote(corpus_path(&corpus, "epub2-ncx-legacy-footnote.epub"));
    assert_epub3_bad_image(corpus_path(&corpus, "epub3-bad-image.epub"));
    assert_epub3_external_footnote(corpus_path(&corpus, "epub3-external-footnote.epub"));
    assert_epub3_image_png(corpus_path(&corpus, "epub3-image-png.epub"));
    assert_epub3_malformed_chapter(corpus_path(&corpus, "epub3-malformed-chapter.epub"));
    assert_epub3_multilingual(corpus_path(&corpus, "epub3-multilingual.epub"));
}

struct CorpusEntry {
    name: &'static str,
    path: PathBuf,
}

type CorpusCase = (&'static str, fn(&Path));

fn build_epub_corpus(root: &Path) -> Vec<CorpusEntry> {
    let cases: &[CorpusCase] = &[
        (
            "epub2-ncx-legacy-footnote.epub",
            write_epub2_ncx_legacy_footnote,
        ),
        ("epub3-bad-image.epub", write_epub3_bad_image),
        (
            "epub3-external-footnote.epub",
            write_epub3_external_footnote,
        ),
        ("epub3-image-png.epub", write_epub3_image_png),
        (
            "epub3-malformed-chapter.epub",
            write_epub3_malformed_chapter,
        ),
        ("epub3-multilingual.epub", write_epub3_multilingual),
    ];

    cases
        .iter()
        .map(|(name, write_case)| {
            let path = root.join(name);
            write_case(&path);
            assert!(path.exists(), "fixture was not written: {}", path.display());
            CorpusEntry { name, path }
        })
        .collect()
}

fn corpus_path<'a>(corpus: &'a [CorpusEntry], name: &str) -> &'a Path {
    corpus
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.path.as_path())
        .expect("corpus entry")
}

fn assert_epub2_ncx_legacy_footnote(path: &Path) {
    let (document, issues) = open_with_issues(path);

    assert!(issues.is_empty(), "unexpected issues: {issues:?}");
    assert_eq!(document.toc.len(), 1);
    assert_eq!(document.toc[0].title, "EPUB2 Chapter");
    assert_eq!(document.toc[0].children[0].title, "Legacy Note Anchor");
    assert!(document_text(&document).contains("Legacy source1."));
    assert_eq!(
        document.annotation_text("OEBPS/notes.xhtml#note-1"),
        Some("Legacy footnote text.")
    );
    assert_eq!(
        first_annotated_text_block(&document).annotations[0].id,
        "OEBPS/notes.xhtml#note-1"
    );
}

fn assert_epub3_bad_image(path: &Path) {
    let (document, issues) = open_with_issues(path);
    let image = image_blocks(&document)
        .into_iter()
        .next()
        .expect("image block");

    assert!(issues.iter().any(|issue| issue.contains("bad image")));
    assert_eq!(image.alt_text.as_deref(), Some("corrupted cover"));
    assert_eq!(
        image.source_path.as_deref(),
        Some("OEBPS/images/corrupt.png")
    );
    assert!(image.data.is_none());
}

fn assert_epub3_external_footnote(path: &Path) {
    let (document, issues) = open_with_issues(path);

    assert!(issues.is_empty(), "unexpected issues: {issues:?}");
    assert_eq!(
        document.annotation_text("OEBPS/notes.xhtml#fn-1"),
        Some("External EPUB3 footnote.")
    );
    assert_eq!(
        first_text_block(&document).annotations[0].id,
        "OEBPS/notes.xhtml#fn-1"
    );
    assert!(document_text(&document).contains("A sentence with [1]."));
}

fn assert_epub3_image_png(path: &Path) {
    let (document, issues) = open_with_issues(path);
    let image = image_blocks(&document)
        .into_iter()
        .next()
        .expect("image block");

    assert!(issues.is_empty(), "unexpected issues: {issues:?}");
    assert_eq!(image.alt_text.as_deref(), Some("one pixel cover"));
    assert_eq!(image.source_path.as_deref(), Some("OEBPS/images/pixel.png"));
    assert!(image.data.as_ref().is_some_and(|data| !data.is_empty()));
}

fn assert_epub3_malformed_chapter(path: &Path) {
    let (document, issues) = open_with_issues(path);

    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("malformed HTML: OEBPS/chapter.xhtml")),
        "expected malformed chapter issue, got {issues:?}"
    );
    assert_eq!(
        document_text(&document),
        "[malformed chapter: OEBPS/chapter.xhtml]"
    );
}

fn assert_epub3_multilingual(path: &Path) {
    let (document, issues) = open_with_issues(path);
    let text = document_text(&document);

    assert!(issues.is_empty(), "unexpected issues: {issues:?}");
    assert_eq!(document.toc[0].title, "多语言章节");
    assert!(text.contains("English sentence."));
    assert!(text.contains("中文句子。"));
    assert!(text.contains("日本語の文です。"));
    assert!(text.contains("مرحبا بالعالم."));
    assert!(text.contains("Русский текст."));
}

fn open_with_issues(path: &Path) -> (Document, Vec<String>) {
    let mut issues = Vec::new();
    let document = epub::open_with_issue_logger(path, |issue| issues.push(issue.to_string()))
        .expect("parse EPUB");

    (document, issues)
}

fn document_text(document: &Document) -> String {
    document
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Text(block) => Some(block.text.as_str()),
            Block::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn first_text_block(document: &Document) -> &yater::document::TextBlock {
    document.text_block(0).expect("first text block")
}

fn first_annotated_text_block(document: &Document) -> &yater::document::TextBlock {
    document
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Text(block) if !block.annotations.is_empty() => Some(block),
            Block::Text(_) | Block::Image(_) => None,
        })
        .next()
        .expect("annotated text block")
}

fn image_blocks(document: &Document) -> Vec<&ImageBlock> {
    document
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Image(block) => Some(block),
            Block::Text(_) => None,
        })
        .collect()
}

fn write_epub2_ncx_legacy_footnote(path: &Path) {
    write_epub_package(
        path,
        "2.0",
        &[
            ManifestItem::new("ncx", "toc.ncx", "application/x-dtbncx+xml"),
            ManifestItem::new("chapter", "chapter.xhtml", "application/xhtml+xml"),
            ManifestItem::new("notes", "notes.xhtml", "application/xhtml+xml"),
        ],
        &["chapter"],
        Some("ncx"),
        &[
            EpubFile::text(
                "OEBPS/chapter.xhtml",
                r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <h1 id="chapter">EPUB2 Chapter</h1>
    <h2 id="legacy-anchor">Legacy Note Anchor</h2>
    <p>Legacy source<a id="source-1" href="notes.xhtml#note-1">1</a>.</p>
  </body>
</html>"##,
            ),
            EpubFile::text(
                "OEBPS/notes.xhtml",
                r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p><a id="note-1" href="chapter.xhtml#source-1">[1]</a>Legacy footnote text.</p>
  </body>
</html>"##,
            ),
            EpubFile::text(
                "OEBPS/toc.ncx",
                r#"<?xml version="1.0"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/">
  <navMap>
    <navPoint id="chapter">
      <navLabel><text>EPUB2 Chapter</text></navLabel>
      <content src="chapter.xhtml#chapter"/>
      <navPoint id="legacy-anchor">
        <navLabel><text>Legacy Note Anchor</text></navLabel>
        <content src="chapter.xhtml#legacy-anchor"/>
      </navPoint>
    </navPoint>
  </navMap>
</ncx>"#,
            ),
        ],
    );
}

fn write_epub3_bad_image(path: &Path) {
    write_epub3_with_nav(
        path,
        &[
            ManifestItem::new("chapter", "chapter.xhtml", "application/xhtml+xml"),
            ManifestItem::new("image", "images/corrupt.png", "image/png"),
        ],
        &["chapter"],
        r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body><nav epub:type="toc"><ol><li><a href="chapter.xhtml">Bad Image</a></li></ol></nav></body>
</html>"#,
        &[
            EpubFile::text(
                "OEBPS/chapter.xhtml",
                r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>Image should be present but invalid.</p>
    <figure><img src="images/corrupt.png" alt="corrupted cover"/></figure>
  </body>
</html>"#,
            ),
            EpubFile::bytes("OEBPS/images/corrupt.png", b"not a png"),
        ],
    );
}

fn write_epub3_external_footnote(path: &Path) {
    write_epub3_with_nav(
        path,
        &[
            ManifestItem::new("chapter", "chapter.xhtml", "application/xhtml+xml"),
            ManifestItem::new("notes", "notes.xhtml", "application/xhtml+xml"),
        ],
        &["chapter"],
        r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body><nav epub:type="toc"><ol><li><a href="chapter.xhtml">Footnotes</a></li></ol></nav></body>
</html>"#,
        &[
            EpubFile::text(
                "OEBPS/chapter.xhtml",
                r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <p>A sentence with <a epub:type="noteref" href="notes.xhtml#fn-1">[1]</a>.</p>
  </body>
</html>"##,
            ),
            EpubFile::text(
                "OEBPS/notes.xhtml",
                r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <aside id="fn-1" epub:type="footnote"><p>External EPUB3 footnote.</p></aside>
  </body>
</html>"#,
            ),
        ],
    );
}

fn write_epub3_image_png(path: &Path) {
    let png = test_png_bytes();
    write_epub3_with_nav(
        path,
        &[
            ManifestItem::new("chapter", "chapter.xhtml", "application/xhtml+xml"),
            ManifestItem::new("image", "images/pixel.png", "image/png"),
        ],
        &["chapter"],
        r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body><nav epub:type="toc"><ol><li><a href="chapter.xhtml">Images</a></li></ol></nav></body>
</html>"#,
        &[
            EpubFile::text(
                "OEBPS/chapter.xhtml",
                r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <h1>Image Chapter</h1>
    <figure><img src="images/pixel.png" alt="one pixel cover"/></figure>
  </body>
</html>"#,
            ),
            EpubFile::bytes("OEBPS/images/pixel.png", png.as_slice()),
        ],
    );
}

fn write_epub3_malformed_chapter(path: &Path) {
    write_epub3_with_nav(
        path,
        &[ManifestItem::new(
            "chapter",
            "chapter.xhtml",
            "application/xhtml+xml",
        )],
        &["chapter"],
        r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body><nav epub:type="toc"><ol><li><a href="chapter.xhtml">Broken</a></li></ol></nav></body>
</html>"#,
        &[EpubFile::text(
            "OEBPS/chapter.xhtml",
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body><p>This chapter never closes"#,
        )],
    );
}

fn write_epub3_multilingual(path: &Path) {
    write_epub3_with_nav(
        path,
        &[ManifestItem::new(
            "chapter",
            "chapter.xhtml",
            "application/xhtml+xml",
        )],
        &["chapter"],
        r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body><nav epub:type="toc"><ol><li><a href="chapter.xhtml#start">多语言章节</a></li></ol></nav></body>
</html>"#,
        &[EpubFile::text(
            "OEBPS/chapter.xhtml",
            r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xml:lang="zh">
  <body>
    <h1 id="start">多语言章节</h1>
    <p>English sentence. 中文句子。日本語の文です。مرحبا بالعالم. Русский текст.</p>
  </body>
</html>"#,
        )],
    );
}

fn write_epub3_with_nav(
    path: &Path,
    manifest_items: &[ManifestItem<'_>],
    spine_ids: &[&str],
    nav_xhtml: &str,
    files: &[EpubFile<'_>],
) {
    let nav = ManifestItem::new("nav", "nav.xhtml", "application/xhtml+xml").properties("nav");
    let mut manifest = Vec::with_capacity(manifest_items.len() + 1);
    manifest.push(nav);
    manifest.extend_from_slice(manifest_items);

    let mut epub_files = Vec::with_capacity(files.len() + 1);
    epub_files.push(EpubFile::text("OEBPS/nav.xhtml", nav_xhtml));
    epub_files.extend_from_slice(files);

    write_epub_package(path, "3.0", &manifest, spine_ids, None, &epub_files);
}

#[derive(Clone, Copy)]
struct ManifestItem<'a> {
    id: &'a str,
    href: &'a str,
    media_type: &'a str,
    properties: &'a str,
}

impl<'a> ManifestItem<'a> {
    fn new(id: &'a str, href: &'a str, media_type: &'a str) -> Self {
        Self {
            id,
            href,
            media_type,
            properties: "",
        }
    }

    fn properties(self, properties: &'a str) -> Self {
        Self { properties, ..self }
    }
}

#[derive(Clone, Copy)]
enum EpubFile<'a> {
    Text { path: &'a str, contents: &'a str },
    Bytes { path: &'a str, contents: &'a [u8] },
}

impl<'a> EpubFile<'a> {
    fn text(path: &'a str, contents: &'a str) -> Self {
        Self::Text { path, contents }
    }

    fn bytes(path: &'a str, contents: &'a [u8]) -> Self {
        Self::Bytes { path, contents }
    }
}

fn write_epub_package(
    path: &Path,
    package_version: &str,
    manifest_items: &[ManifestItem<'_>],
    spine_ids: &[&str],
    spine_toc: Option<&str>,
    files: &[EpubFile<'_>],
) {
    let file = File::create(path).expect("create EPUB");
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
        &content_opf(package_version, manifest_items, spine_ids, spine_toc),
    );

    for file in files {
        match file {
            EpubFile::Text { path, contents } => {
                write_zip_file(&mut writer, options, path, contents)
            }
            EpubFile::Bytes { path, contents } => {
                write_zip_bytes(&mut writer, options, path, contents);
            }
        }
    }

    writer.finish().expect("finish EPUB");
}

fn content_opf(
    package_version: &str,
    manifest_items: &[ManifestItem<'_>],
    spine_ids: &[&str],
    spine_toc: Option<&str>,
) -> String {
    let manifest = manifest_items
        .iter()
        .map(|item| {
            let properties = if item.properties.is_empty() {
                String::new()
            } else {
                format!(r#" properties="{}""#, item.properties)
            };
            format!(
                r#"    <item id="{}" href="{}" media-type="{}"{} />"#,
                item.id, item.href, item.media_type, properties
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let spine = spine_ids
        .iter()
        .map(|id| format!(r#"    <itemref idref="{id}"/>"#))
        .collect::<Vec<_>>()
        .join("\n");
    let toc = spine_toc
        .map(|id| format!(r#" toc="{id}""#))
        .unwrap_or_default();

    format!(
        r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="{package_version}">
  <manifest>
{manifest}
  </manifest>
  <spine{toc}>
{spine}
  </spine>
</package>"#
    )
}

fn write_zip_file(
    writer: &mut ZipWriter<File>,
    options: SimpleFileOptions,
    name: &str,
    contents: &str,
) {
    writer.start_file(name, options).expect("start ZIP file");
    writer
        .write_all(contents.as_bytes())
        .expect("write ZIP file");
}

fn write_zip_bytes(
    writer: &mut ZipWriter<File>,
    options: SimpleFileOptions,
    name: &str,
    contents: &[u8],
) {
    writer.start_file(name, options).expect("start ZIP file");
    writer.write_all(contents).expect("write ZIP file");
}

fn test_png_bytes() -> Vec<u8> {
    let image = ::image::RgbaImage::from_fn(1, 1, |_, _| ::image::Rgba([0, 0, 0, 255]));
    let mut bytes = Cursor::new(Vec::new());
    ::image::DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, ::image::ImageFormat::Png)
        .expect("encode PNG");
    bytes.into_inner()
}
