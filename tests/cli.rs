use std::process::Command;

use tempfile::tempdir;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

#[test]
fn version_flag_prints_package_version() {
    let yater = std::env::var("CARGO_BIN_EXE_yater").expect("binary path");
    let output = Command::new(yater)
        .arg("--version")
        .output()
        .expect("run yater --version");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout"),
        format!("yater {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn invalid_image_mode_exits_with_error() {
    let yater = std::env::var("CARGO_BIN_EXE_yater").expect("binary path");
    let output = Command::new(yater)
        .args(["book.epub", "--image-mode=bitmap"])
        .output()
        .expect("run yater with invalid image mode");

    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .expect("stderr")
            .contains("invalid value")
    );
}

#[test]
fn missing_epub_path_exits_with_clear_startup_error() {
    let yater = std::env::var("CARGO_BIN_EXE_yater").expect("binary path");
    let output = Command::new(yater)
        .arg("/definitely/not/a/book.epub")
        .output()
        .expect("run yater with missing book");

    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .expect("stderr")
            .contains("file not found")
    );
}

#[test]
fn corrupted_epub_exits_with_clear_startup_error() {
    let tempdir = tempdir().expect("temp dir");
    let epub_path = tempdir.path().join("corrupted.epub");
    std::fs::write(&epub_path, "not an epub").expect("write corrupted epub");
    let yater = std::env::var("CARGO_BIN_EXE_yater").expect("binary path");
    let output = Command::new(yater)
        .arg(epub_path)
        .output()
        .expect("run yater with corrupted book");

    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .expect("stderr")
            .contains("failed to open EPUB")
    );
}

#[test]
fn valid_epub_startup_succeeds() {
    let tempdir = tempdir().expect("temp dir");
    let epub_path = tempdir.path().join("book.epub");
    write_minimal_epub(&epub_path);
    let yater = std::env::var("CARGO_BIN_EXE_yater").expect("binary path");
    let output = Command::new(yater)
        .arg(epub_path)
        .output()
        .expect("run yater with valid book");

    assert!(output.status.success());
}

fn write_minimal_epub(path: &Path) {
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
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="chapter1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="chapter1"/>
  </spine>
</package>"#,
    );
    write_zip_file(
        &mut writer,
        options,
        "OEBPS/nav.xhtml",
        r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc">
      <ol><li><a href="chapter1.xhtml">Chapter One</a></li></ol>
    </nav>
  </body>
</html>"#,
    );
    write_zip_file(
        &mut writer,
        options,
        "OEBPS/chapter1.xhtml",
        r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body><p>First sentence.</p></body>
</html>"#,
    );

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
