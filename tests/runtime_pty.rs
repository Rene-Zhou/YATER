#![cfg(target_os = "linux")]

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::process::Command;

use tempfile::tempdir;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

#[test]
fn binary_renders_and_restores_a_real_pty_without_graphics_queries_when_images_are_off() {
    let tempdir = tempdir().expect("temp dir");
    let epub_path = tempdir.path().join("book.epub");
    write_runtime_epub(&epub_path);
    let yater = std::env::var("CARGO_BIN_EXE_yater").expect("binary path");
    let output = Command::new("python3")
        .args([
            "tests/support/capture_runtime.py",
            &yater,
            epub_path.to_str().expect("UTF-8 fixture path"),
        ])
        .output()
        .expect("capture reader in PTY");
    let screen = String::from_utf8_lossy(&output.stdout);

    if output.status.code() == Some(77) {
        eprintln!(
            "skipping PTY assertions: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    assert!(
        output.status.success(),
        "reader failed: {}\nPTY output: {screen:?}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(screen.contains("\u{1b}[?1049h"), "did not enter alternate screen");
    assert_eq!(
        AnsiScreen::capture(&output.stdout, 40, 8).snapshot(),
        concat!(
            "               Chapter One\n",
            "Opening heading.\n",
            "First paragraph.\n",
            "Final paragraph.\n",
            "\n",
            "\n",
            "\n",
            ""
        )
    );
    assert!(screen.contains("\u{1b}[?1049l"), "did not leave alternate screen");
    assert!(
        !screen.contains("\u{1b}_G")
            && !screen.contains("\u{1b}[c")
            && !screen.contains("\u{1b}[16t")
            && !screen.contains("\u{1b}[5n"),
        "explicit image-off mode emitted terminal capability queries: {screen:?}"
    );
}

#[test]
fn binary_opens_an_epub_annotation_in_a_real_pty() {
    let tempdir = tempdir().expect("temp dir");
    let epub_path = tempdir.path().join("annotated.epub");
    write_annotated_runtime_epub(&epub_path);
    let yater = std::env::var("CARGO_BIN_EXE_yater").expect("binary path");
    let output = Command::new("python3")
        .args([
            "tests/support/capture_runtime.py",
            &yater,
            epub_path.to_str().expect("UTF-8 fixture path"),
            "Opening [1].",
            ";",
            "PTY footnote text.",
        ])
        .output()
        .expect("capture annotated reader in PTY");

    if output.status.code() == Some(77) {
        eprintln!(
            "skipping PTY assertions: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let screen = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "reader failed: {}\nPTY output: {screen:?}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(screen.contains("\u{1b}[?1049h"));
    assert!(
        screen.contains("PTY") && screen.contains("footnote") && screen.contains("text."),
        "{screen:?}"
    );
    assert!(screen.contains("\u{1b}[?1049l"));
    assert!(!screen.contains("terminal error:"), "{screen:?}");
}

struct AnsiScreen {
    width: usize,
    height: usize,
    cells: Vec<u8>,
    last_alternate: Option<Vec<u8>>,
    in_alternate: bool,
    x: usize,
    y: usize,
}

impl AnsiScreen {
    fn capture(output: &[u8], width: usize, height: usize) -> Self {
        let mut screen = Self {
            width,
            height,
            cells: vec![b' '; width * height],
            last_alternate: None,
            in_alternate: false,
            x: 0,
            y: 0,
        };
        let mut index = 0;

        while index < output.len() {
            match output[index] {
                0x1b if output.get(index + 1) == Some(&b'[') => {
                    index += 2;
                    let parameter_start = index;
                    while index < output.len() && !(0x40..=0x7e).contains(&output[index]) {
                        index += 1;
                    }
                    if index == output.len() {
                        break;
                    }
                    let parameters = &output[parameter_start..index];
                    let command = output[index];
                    screen.apply_csi(parameters, command);
                    index += 1;
                }
                0x1b if output.get(index + 1) == Some(&b'_') => {
                    index += 2;
                    while index + 1 < output.len()
                        && !(output[index] == 0x1b && output[index + 1] == b'\\')
                    {
                        index += 1;
                    }
                    index = (index + 2).min(output.len());
                }
                0x1b => {
                    index = (index + 2).min(output.len());
                }
                b'\r' => {
                    screen.x = 0;
                    index += 1;
                }
                b'\n' => {
                    screen.y = (screen.y + 1).min(screen.height.saturating_sub(1));
                    index += 1;
                }
                byte if byte.is_ascii_graphic() || byte == b' ' => {
                    if screen.x < screen.width && screen.y < screen.height {
                        screen.cells[screen.y * screen.width + screen.x] = byte;
                    }
                    screen.x += 1;
                    index += 1;
                }
                _ => {
                    index += 1;
                }
            }
        }

        screen
    }

    fn apply_csi(&mut self, parameters: &[u8], command: u8) {
        let parameters = String::from_utf8_lossy(parameters);
        if parameters == "?1049" {
            match command {
                b'h' => {
                    self.cells.fill(b' ');
                    self.x = 0;
                    self.y = 0;
                    self.in_alternate = true;
                }
                b'l' if self.in_alternate => {
                    self.last_alternate = Some(self.cells.clone());
                    self.in_alternate = false;
                }
                _ => {}
            }
            return;
        }
        let numeric = parameters.trim_start_matches('?');
        let values = numeric
            .split(';')
            .map(|value| value.parse::<usize>().unwrap_or(0))
            .collect::<Vec<_>>();

        match command {
            b'H' | b'f' => {
                self.y = values.first().copied().unwrap_or(1).saturating_sub(1);
                self.x = values.get(1).copied().unwrap_or(1).saturating_sub(1);
            }
            b'J' if values.first().copied().unwrap_or(0) == 2 => {
                self.cells.fill(b' ');
                self.x = 0;
                self.y = 0;
            }
            b'A' => {
                self.y = self
                    .y
                    .saturating_sub(values.first().copied().unwrap_or(1).max(1));
            }
            b'B' => {
                self.y = (self.y + values.first().copied().unwrap_or(1).max(1))
                    .min(self.height.saturating_sub(1));
            }
            b'C' => {
                self.x = (self.x + values.first().copied().unwrap_or(1).max(1))
                    .min(self.width.saturating_sub(1));
            }
            b'D' => {
                self.x = self
                    .x
                    .saturating_sub(values.first().copied().unwrap_or(1).max(1));
            }
            b'G' => {
                self.x = values.first().copied().unwrap_or(1).saturating_sub(1);
            }
            _ => {}
        }
    }

    fn snapshot(&self) -> String {
        self.last_alternate
            .as_deref()
            .unwrap_or(&self.cells)
            .chunks(self.width)
            .map(|row| String::from_utf8_lossy(row).trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn write_runtime_epub(path: &Path) {
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
  <body>
    <h1>Opening heading.</h1>
    <p>First paragraph.</p>
    <p>Final paragraph.</p>
  </body>
</html>"#,
    );

    writer.finish().expect("finish EPUB");
}

fn write_annotated_runtime_epub(path: &Path) {
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
        r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="chapter"/></spine>
</package>"#,
    );
    write_zip_file(
        &mut writer,
        options,
        "OEBPS/nav.xhtml",
        r#"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body><nav epub:type="toc"><ol><li><a href="chapter.xhtml">Chapter One</a></li></ol></nav></body>
</html>"#,
    );
    write_zip_file(
        &mut writer,
        options,
        "OEBPS/chapter.xhtml",
        r##"<?xml version="1.0"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <p>Opening <a epub:type="noteref" href="#note-1">[1]</a>.</p>
    <aside id="note-1" epub:type="footnote"><p>PTY footnote text.</p></aside>
  </body>
</html>"##,
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
