/// Returns byte-offset ranges for sentences in `text`.
pub fn segment_sentences(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    let use_cjk_boundaries = text.chars().any(is_cjk_character);

    while let Some((index, ch)) = chars.next() {
        let end = index + ch.len_utf8();

        match ch {
            '.' | '!' | '?' if !use_cjk_boundaries => {
                ranges.push((start, end));
                start = end;
            }
            '。' | '！' | '？' => {
                match quoted_boundary(text, end) {
                    Some(QuotedBoundary::Delay) => {}
                    Some(QuotedBoundary::SplitAt(boundary_end)) => {
                        ranges.push((start, boundary_end));
                        start = boundary_end;
                    }
                    None => {
                        ranges.push((start, end));
                        start = end;
                    }
                }
            }
            '…' => {
                if let Some(&(next_index, '…')) = chars.peek() {
                    chars.next();
                    let end = next_index + '…'.len_utf8();
                    match quoted_boundary(text, end) {
                        Some(QuotedBoundary::Delay) => {}
                        Some(QuotedBoundary::SplitAt(boundary_end)) => {
                            ranges.push((start, boundary_end));
                            start = boundary_end;
                        }
                        None => {
                            ranges.push((start, end));
                            start = end;
                        }
                    }
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

enum QuotedBoundary {
    Delay,
    SplitAt(usize),
}

fn quoted_boundary(text: &str, boundary_end: usize) -> Option<QuotedBoundary> {
    let quote_end = closing_quote_end(text, boundary_end);
    if quote_end == boundary_end {
        return None;
    }

    let next_index = skip_whitespace(text, quote_end);
    let Some((next, _)) = char_at(text, next_index) else {
        return Some(QuotedBoundary::SplitAt(quote_end));
    };

    if is_cjk_sentence_terminal(next) || next == '…' || is_post_quote_continuation(next) {
        return Some(QuotedBoundary::Delay);
    }

    if is_dialogue_attribution(text, next_index) {
        Some(QuotedBoundary::Delay)
    } else {
        Some(QuotedBoundary::SplitAt(quote_end))
    }
}

fn closing_quote_end(text: &str, start: usize) -> usize {
    let mut index = start;
    while let Some((ch, next_index)) = char_at(text, index) {
        if !is_closing_quote(ch) {
            break;
        }
        index = next_index;
    }
    index
}

fn skip_whitespace(text: &str, start: usize) -> usize {
    let mut index = start;
    while let Some((ch, next_index)) = char_at(text, index) {
        if !ch.is_whitespace() {
            break;
        }
        index = next_index;
    }
    index
}

fn char_at(text: &str, index: usize) -> Option<(char, usize)> {
    let ch = text.get(index..)?.chars().next()?;
    Some((ch, index + ch.len_utf8()))
}

fn is_closing_quote(character: char) -> bool {
    matches!(character, '"' | '\'' | '”' | '’' | '」' | '』')
}

fn is_post_quote_continuation(character: char) -> bool {
    matches!(character, ',' | ';' | ':' | '，' | '；' | '：' | '、')
}

fn is_cjk_sentence_terminal(character: char) -> bool {
    matches!(character, '。' | '！' | '？')
}

fn is_dialogue_attribution(text: &str, start: usize) -> bool {
    let mut phrase = String::new();

    for (_, ch) in text[start..].char_indices() {
        if is_opening_quote(ch) {
            return false;
        }
        if is_cjk_sentence_terminal(ch) || ch == '…' {
            break;
        }
        phrase.push(ch);
        if phrase.chars().count() > 48 {
            return false;
        }
    }

    let phrase = phrase.trim();
    if phrase.is_empty() {
        return false;
    }

    DIALOGUE_ATTRIBUTION_ENDINGS
        .iter()
        .any(|ending| phrase.ends_with(ending))
}

fn is_opening_quote(character: char) -> bool {
    matches!(character, '"' | '\'' | '“' | '‘' | '「' | '『')
}

const DIALOGUE_ATTRIBUTION_ENDINGS: &[&str] = &[
    "嘟囔道",
    "嘀咕道",
    "咕哝道",
    "喃喃道",
    "低声说",
    "轻声说",
    "大声说",
    "高声说",
    "悄声说",
    "尖叫道",
    "回答道",
    "补充道",
    "解释道",
    "命令道",
    "抱怨道",
    "说道",
    "问道",
    "答道",
    "喊道",
    "叫道",
    "吼道",
    "嚷道",
    "骂道",
    "笑道",
    "叹道",
    "念道",
    "嘟囔",
    "嘀咕",
    "咕哝",
    "喃喃",
    "尖叫",
    "回答",
    "补充",
    "解释",
    "命令",
    "抱怨",
    "说",
    "问",
    "答",
    "喊",
    "叫",
    "吼",
    "嚷",
    "骂",
    "笑",
    "叹",
    "念",
];

fn is_cjk_character(character: char) -> bool {
    matches!(
        character,
        '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{2A6DF}'
            | '\u{2A700}'..='\u{2B73F}'
            | '\u{2B740}'..='\u{2B81F}'
            | '\u{2B820}'..='\u{2CEAF}'
            | '\u{2CEB0}'..='\u{2EBEF}'
            | '\u{30000}'..='\u{3134F}'
    )
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
    fn keeps_quoted_dialogue_with_following_attribution() {
        let text = "“好吧！”有个声音嘟囔道。下一句。";

        let ranges = segment_sentences(text);

        assert_eq!(
            ranges
                .into_iter()
                .map(|range| &text[range.0..range.1])
                .collect::<Vec<_>>(),
            vec!["“好吧！”有个声音嘟囔道。", "下一句。"]
        );
    }

    #[test]
    fn keeps_ascii_quoted_dialogue_with_following_attribution() {
        let text = "\"好吧！\"有个声音嘟囔道。下一句。";

        let ranges = segment_sentences(text);

        assert_eq!(
            ranges
                .into_iter()
                .map(|range| &text[range.0..range.1])
                .collect::<Vec<_>>(),
            vec!["\"好吧！\"有个声音嘟囔道。", "下一句。"]
        );
    }

    #[test]
    fn keeps_ordinary_quote_until_outer_sentence_boundary() {
        let text = "他想起“好吧！”。下一句。";

        let ranges = segment_sentences(text);

        assert_eq!(
            ranges
                .into_iter()
                .map(|range| &text[range.0..range.1])
                .collect::<Vec<_>>(),
            vec!["他想起“好吧！”。", "下一句。"]
        );
    }

    #[test]
    fn does_not_split_cjk_text_at_ascii_punctuation() {
        let text = "版本 v1.2 还没结束。下一句。";

        let ranges = segment_sentences(text);

        assert_eq!(
            ranges
                .into_iter()
                .map(|range| &text[range.0..range.1])
                .collect::<Vec<_>>(),
            vec!["版本 v1.2 还没结束。", "下一句。"]
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
