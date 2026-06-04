/// Returns byte-offset ranges for sentences in `text`.
pub fn segment_sentences(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();

    while let Some((index, ch)) = chars.next() {
        let end = index + ch.len_utf8();

        match ch {
            '.' | '!' | '?' | '。' | '！' | '？' => {
                ranges.push((start, end));
                start = end;
            }
            '…' => {
                if let Some(&(next_index, '…')) = chars.peek() {
                    chars.next();
                    let end = next_index + '…'.len_utf8();
                    ranges.push((start, end));
                    start = end;
                }
            }
            _ => {}
        }
    }

    if start < text.len() {
        ranges.push((start, text.len()));
    }

    ranges
}

#[cfg(test)]
mod tests {
    use super::segment_sentences;

    #[test]
    fn segments_chinese_sentences_without_splitting_clauses() {
        let text = "你好，世界；还没结束。第二句？第三句！";

        let ranges = segment_sentences(text);

        assert_eq!(
            ranges
                .into_iter()
                .map(|range| &text[range.0..range.1])
                .collect::<Vec<_>>(),
            vec!["你好，世界；还没结束。", "第二句？", "第三句！"]
        );
    }

    #[test]
    fn treats_chinese_ellipsis_as_sentence_boundary() {
        let text = "等等……然后继续。";

        let ranges = segment_sentences(text);

        assert_eq!(
            ranges
                .into_iter()
                .map(|range| &text[range.0..range.1])
                .collect::<Vec<_>>(),
            vec!["等等……", "然后继续。"]
        );
    }

    #[test]
    fn segments_english_sentence_boundaries() {
        let text = "First sentence. Second? Third!";

        let ranges = segment_sentences(text);

        assert_eq!(
            ranges
                .into_iter()
                .map(|range| &text[range.0..range.1])
                .collect::<Vec<_>>(),
            vec!["First sentence.", " Second?", " Third!"]
        );
    }

    #[test]
    fn returns_trailing_text_without_terminal_punctuation() {
        let text = "One complete. unfinished tail";

        let ranges = segment_sentences(text);

        assert_eq!(
            ranges
                .into_iter()
                .map(|range| &text[range.0..range.1])
                .collect::<Vec<_>>(),
            vec!["One complete.", " unfinished tail"]
        );
    }
}
