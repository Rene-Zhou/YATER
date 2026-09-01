use std::path::Path;

use crate::document::{Block, ChapterRange, Document, TextBlock};

#[derive(Debug)]
pub struct TxtError(String);

impl std::fmt::Display for TxtError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for TxtError {}

pub fn open(path: &Path) -> Result<Document, TxtError> {
    let bytes = std::fs::read(path).map_err(|error| TxtError(error.to_string()))?;
    let text = decode(&bytes);
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");

    let blocks: Vec<Block> = normalized
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            Block::Text(TextBlock {
                text: line.to_string(),
                chapter_index: 0,
                annotations: Vec::new(),
                styles: Vec::new(),
                presentation: Default::default(),
            })
        })
        .collect();

    if blocks.is_empty() {
        return Err(TxtError(format!("no text content: {}", path.display())));
    }

    let chapter_ranges = vec![ChapterRange {
        start_block: 0,
        end_block: blocks.len() - 1,
    }];

    Ok(Document {
        blocks,
        toc: Vec::new(),
        annotations: Default::default(),
        chapter_ranges,
    })
}

fn decode(bytes: &[u8]) -> String {
    let (bom_encoding, bom_length) = match bytes {
        [0xEF, 0xBB, 0xBF, ..] => (Some(encoding_rs::UTF_8), 3),
        [0xFF, 0xFE, ..] => (Some(encoding_rs::UTF_16LE), 2),
        [0xFE, 0xFF, ..] => (Some(encoding_rs::UTF_16BE), 2),
        _ => (None, 0),
    };

    if let Some(encoding) = bom_encoding {
        let (text, _, _) = encoding.decode(&bytes[bom_length..]);
        return text.into_owned();
    }

    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }

    let (text, _, _) = encoding_rs::GB18030.decode(bytes);
    text.into_owned()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use super::open;
    use crate::document::{Block, ChapterRange};

    fn write_temp_txt(dir: &Path, contents: &[u8]) -> std::path::PathBuf {
        let path = dir.join("book.txt");
        fs::write(&path, contents).expect("write temp txt");
        path
    }

    fn block_texts(document: &crate::document::Document) -> Vec<&str> {
        document
            .blocks
            .iter()
            .map(|block| match block {
                Block::Text(text) => text.text.as_str(),
                Block::Image(_) => panic!("txt documents must not contain image blocks"),
            })
            .collect()
    }

    #[test]
    fn parses_utf8_lines_into_single_chapter_text_blocks() {
        let tempdir = tempdir().expect("tempdir");
        let path = write_temp_txt(
            tempdir.path(),
            "第一段。\n\n  第二段。  \n第三段。\n".as_bytes(),
        );

        let document = open(&path).expect("open utf8 txt");

        assert_eq!(block_texts(&document), ["第一段。", "第二段。", "第三段。"]);
        assert!(document.toc.is_empty());
        assert!(document.annotations.is_empty());
        assert_eq!(
            document.chapter_ranges,
            vec![ChapterRange {
                start_block: 0,
                end_block: 2,
            }]
        );
        for block in &document.blocks {
            match block {
                Block::Text(text) => {
                    assert_eq!(text.chapter_index, 0);
                    assert!(text.styles.is_empty());
                    assert!(text.annotations.is_empty());
                }
                Block::Image(_) => unreachable!(),
            }
        }
    }

    #[test]
    fn strips_utf8_bom() {
        let tempdir = tempdir().expect("tempdir");
        let path = write_temp_txt(tempdir.path(), "\u{feff}hello\n".as_bytes());

        let document = open(&path).expect("open utf8 bom txt");

        assert_eq!(block_texts(&document), ["hello"]);
    }

    #[test]
    fn decodes_utf16le_with_bom() {
        let tempdir = tempdir().expect("tempdir");
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "你好，世界。\n第二行。".encode_utf16() {
            bytes.extend(unit.to_le_bytes());
        }
        let path = write_temp_txt(tempdir.path(), &bytes);

        let document = open(&path).expect("open utf16le txt");

        assert_eq!(block_texts(&document), ["你好，世界。", "第二行。"]);
    }

    #[test]
    fn decodes_utf16be_with_bom() {
        let tempdir = tempdir().expect("tempdir");
        let mut bytes = vec![0xFE, 0xFF];
        for unit in "你好，世界。".encode_utf16() {
            bytes.extend(unit.to_be_bytes());
        }
        let path = write_temp_txt(tempdir.path(), &bytes);

        let document = open(&path).expect("open utf16be txt");

        assert_eq!(block_texts(&document), ["你好，世界。"]);
    }

    #[test]
    fn decodes_gbk_when_bytes_are_not_utf8() {
        let tempdir = tempdir().expect("tempdir");
        // "你好，世界。" in GBK.
        let gbk_bytes = [
            0xC4, 0xE3, 0xBA, 0xC3, 0xA3, 0xAC, 0xCA, 0xC0, 0xBD, 0xE7, 0xA1, 0xA3,
        ];
        let path = write_temp_txt(tempdir.path(), &gbk_bytes);

        let document = open(&path).expect("open gbk txt");

        assert_eq!(block_texts(&document), ["你好，世界。"]);
    }

    #[test]
    fn decodes_gb18030_four_byte_characters() {
        let tempdir = tempdir().expect("tempdir");
        let (encoded, _, _) = encoding_rs::GB18030.encode("𠀀字。");
        let path = write_temp_txt(tempdir.path(), &encoded);

        let document = open(&path).expect("open gb18030 txt");

        assert_eq!(block_texts(&document), ["𠀀字。"]);
    }

    #[test]
    fn normalizes_crlf_and_cr_line_endings() {
        let tempdir = tempdir().expect("tempdir");
        let path = write_temp_txt(tempdir.path(), "one\r\n\r\ntwo\rthree\n".as_bytes());

        let document = open(&path).expect("open crlf txt");

        assert_eq!(block_texts(&document), ["one", "two", "three"]);
    }

    #[test]
    fn rejects_file_with_no_text_content() {
        let tempdir = tempdir().expect("tempdir");
        let path = write_temp_txt(tempdir.path(), " \n\t\n\n".as_bytes());

        let error = open(&path).expect_err("empty txt must fail");

        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn reports_missing_file() {
        let tempdir = tempdir().expect("tempdir");
        let path = tempdir.path().join("missing.txt");

        let error = open(&path).expect_err("missing file must fail");

        assert!(!error.to_string().is_empty());
    }
}
