use std::fs::File;
use std::io::Write;
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tempfile::tempdir;
use yater::app::App;
use yater::epub;
use yater::runtime::run_terminal_loop;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

#[test]
fn epub_annotation_marker_opens_its_note_in_the_runtime_overlay() {
    let tempdir = tempdir().expect("temp dir");
    let epub_path = tempdir.path().join("annotated.epub");
    write_annotated_epub(&epub_path);
    let document = epub::open(&epub_path).expect("open annotated EPUB");
    let mut app = App::new(document);
    let backend = TestBackend::new(40, 7);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_terminal_loop(
        &mut terminal,
        &mut app,
        [KeyEvent::new(KeyCode::Char(';'), KeyModifiers::NONE)],
    )
    .expect("run terminal");

    let frame = frame_snapshot(terminal.backend().buffer());
    assert!(frame.contains("Source sentence [1]."), "{frame}");
    assert!(frame.contains("Parsed footnote text."), "{frame}");
    assert!(frame.contains("Following paragraph."), "{frame}");
}

#[test]
fn epub2_backlinked_annotation_opens_without_its_return_marker() {
    let tempdir = tempdir().expect("temp dir");
    let epub_path = tempdir.path().join("backlinked.epub");
    write_backlinked_epub(&epub_path);
    let document = epub::open(&epub_path).expect("open EPUB2 annotation");
    let mut app = App::new(document);
    let backend = TestBackend::new(40, 7);
    let mut terminal = Terminal::new(backend).expect("terminal");

    run_terminal_loop(
        &mut terminal,
        &mut app,
        [KeyEvent::new(KeyCode::Char(';'), KeyModifiers::NONE)],
    )
    .expect("run terminal");

    let frame = frame_snapshot(terminal.backend().buffer());
    assert!(frame.contains("Source sentence1."), "{frame}");
    assert!(frame.contains("Legacy note text."), "{frame}");
    assert!(!frame.contains("[1]Legacy note text."), "{frame}");
}

fn write_annotated_epub(path: &Path) {
    write_epub(
        path,
        "3.0",
        r#"<itemref idref="chapter"/>"#,
        r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <p>Source sentence <a epub:type="noteref" href="notes.xhtml#note-1">[1]</a>.</p>
    <p>Following paragraph.</p>
  </body>
</html>"#,
        r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <aside id="note-1" epub:type="footnote"><p>Parsed footnote text.</p></aside>
  </body>
</html>"#,
    );
}

fn write_backlinked_epub(path: &Path) {
    write_epub(
        path,
        "2.0",
        r#"<itemref idref="chapter"/><itemref idref="notes"/>"#,
        r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p>Source sentence<a id="source-1" href="notes.xhtml#note-1">1</a>.</p>
  </body>
</html>"##,
        r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <p class="publisher-note"><a id="note-1" href="chapter.xhtml#source-1">[1]</a>Legacy note text.</p>
  </body>
</html>"##,
    );
}

fn write_epub(
    path: &Path,
    package_version: &str,
    spine: &str,
    chapter: &str,
    notes: &str,
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
        &format!(
            r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="{package_version}">
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="notes" href="notes.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    {spine}
  </spine>
</package>"#
        ),
    );
    write_zip_file(
        &mut writer,
        options,
        "OEBPS/nav.xhtml",
        r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc">
      <ol><li><a href="chapter.xhtml">Chapter One</a></li></ol>
    </nav>
  </body>
</html>"#,
    );
    write_zip_file(
        &mut writer,
        options,
        "OEBPS/chapter.xhtml",
        chapter,
    );
    write_zip_file(
        &mut writer,
        options,
        "OEBPS/notes.xhtml",
        notes,
    );

    writer.finish().expect("finish EPUB");
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
