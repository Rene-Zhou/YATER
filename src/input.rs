use crossterm::event::{KeyCode, KeyEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Content,
    Toc,
    AnnotationOverlay,
    AnnotationImmersed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    NextSentence,
    PreviousSentence,
    NextParagraph,
    PreviousParagraph,
    PageUp,
    PageDown,
    JumpToChapterStart,
    JumpToChapterEnd,
    OpenAnnotationOverlay,
    OpenToc,
    Quit,
    NextTocItem,
    PreviousTocItem,
    ExpandOrJumpToc,
    CollapseOrParentToc,
    CloseToc,
    CycleAnnotation,
    ImmerseAnnotation,
    CloseAnnotationOverlay,
    ScrollAnnotationDown,
    ScrollAnnotationUp,
    ExitAnnotationImmersion,
    Unhandled,
}

pub fn map_key(focus: Focus, key: KeyEvent) -> Action {
    match (focus, key.code) {
        (Focus::Content, KeyCode::Char('j') | KeyCode::Down) => Action::NextSentence,
        (Focus::Content, KeyCode::Char('k') | KeyCode::Up) => Action::PreviousSentence,
        (Focus::Content, KeyCode::Char('l')) => Action::NextParagraph,
        (Focus::Content, KeyCode::Char('h')) => Action::PreviousParagraph,
        (Focus::Content, KeyCode::Char('u')) => Action::PageUp,
        (Focus::Content, KeyCode::Char('n')) => Action::PageDown,
        (Focus::Content, KeyCode::Char('i')) => Action::JumpToChapterStart,
        (Focus::Content, KeyCode::Char('m')) => Action::JumpToChapterEnd,
        (Focus::Content, KeyCode::Char(';')) => Action::OpenAnnotationOverlay,
        (Focus::Content, KeyCode::Tab) => Action::OpenToc,
        (Focus::Content, KeyCode::Char('q')) => Action::Quit,
        (Focus::Toc, KeyCode::Char('j') | KeyCode::Down) => Action::NextTocItem,
        (Focus::Toc, KeyCode::Char('k') | KeyCode::Up) => Action::PreviousTocItem,
        (Focus::Toc, KeyCode::Char('l') | KeyCode::Enter) => Action::ExpandOrJumpToc,
        (Focus::Toc, KeyCode::Char('h')) => Action::CollapseOrParentToc,
        (Focus::Toc, KeyCode::Tab | KeyCode::Esc) => Action::CloseToc,
        (Focus::AnnotationOverlay, KeyCode::Char(';')) => Action::CycleAnnotation,
        (Focus::AnnotationOverlay, KeyCode::Enter) => Action::ImmerseAnnotation,
        (Focus::AnnotationOverlay, _) => Action::CloseAnnotationOverlay,
        (Focus::AnnotationImmersed, KeyCode::Char('j') | KeyCode::Down) => {
            Action::ScrollAnnotationDown
        }
        (Focus::AnnotationImmersed, KeyCode::Char('k') | KeyCode::Up) => Action::ScrollAnnotationUp,
        (Focus::AnnotationImmersed, KeyCode::Esc) => Action::ExitAnnotationImmersion,
        _ => Action::Unhandled,
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{map_key, Action, Focus};

    #[test]
    fn content_j_moves_to_next_sentence() {
        let action = map_key(
            Focus::Content,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );

        assert_eq!(action, Action::NextSentence);
    }

    #[test]
    fn content_mode_maps_reader_navigation_keys() {
        let cases = [
            (KeyCode::Down, Action::NextSentence),
            (KeyCode::Char('k'), Action::PreviousSentence),
            (KeyCode::Up, Action::PreviousSentence),
            (KeyCode::Char('l'), Action::NextParagraph),
            (KeyCode::Char('h'), Action::PreviousParagraph),
            (KeyCode::Char('u'), Action::PageUp),
            (KeyCode::Char('n'), Action::PageDown),
            (KeyCode::Char('i'), Action::JumpToChapterStart),
            (KeyCode::Char('m'), Action::JumpToChapterEnd),
            (KeyCode::Char(';'), Action::OpenAnnotationOverlay),
            (KeyCode::Tab, Action::OpenToc),
            (KeyCode::Char('q'), Action::Quit),
        ];

        for (key_code, expected_action) in cases {
            let action = map_key(
                Focus::Content,
                KeyEvent::new(key_code, KeyModifiers::NONE),
            );

            assert_eq!(action, expected_action);
        }
    }

    #[test]
    fn toc_mode_maps_sidebar_navigation_keys() {
        let cases = [
            (KeyCode::Char('j'), Action::NextTocItem),
            (KeyCode::Down, Action::NextTocItem),
            (KeyCode::Char('k'), Action::PreviousTocItem),
            (KeyCode::Up, Action::PreviousTocItem),
            (KeyCode::Char('l'), Action::ExpandOrJumpToc),
            (KeyCode::Enter, Action::ExpandOrJumpToc),
            (KeyCode::Char('h'), Action::CollapseOrParentToc),
            (KeyCode::Tab, Action::CloseToc),
            (KeyCode::Esc, Action::CloseToc),
        ];

        for (key_code, expected_action) in cases {
            let action = map_key(Focus::Toc, KeyEvent::new(key_code, KeyModifiers::NONE));

            assert_eq!(action, expected_action);
        }
    }

    #[test]
    fn annotation_overlay_maps_cycle_immerse_and_close_keys() {
        let cases = [
            (KeyCode::Char(';'), Action::CycleAnnotation),
            (KeyCode::Enter, Action::ImmerseAnnotation),
            (KeyCode::Char('j'), Action::CloseAnnotationOverlay),
            (KeyCode::Esc, Action::CloseAnnotationOverlay),
        ];

        for (key_code, expected_action) in cases {
            let action = map_key(
                Focus::AnnotationOverlay,
                KeyEvent::new(key_code, KeyModifiers::NONE),
            );

            assert_eq!(action, expected_action);
        }
    }

    #[test]
    fn annotation_immersed_maps_scroll_and_exit_keys() {
        let cases = [
            (KeyCode::Char('j'), Action::ScrollAnnotationDown),
            (KeyCode::Down, Action::ScrollAnnotationDown),
            (KeyCode::Char('k'), Action::ScrollAnnotationUp),
            (KeyCode::Up, Action::ScrollAnnotationUp),
            (KeyCode::Esc, Action::ExitAnnotationImmersion),
        ];

        for (key_code, expected_action) in cases {
            let action = map_key(
                Focus::AnnotationImmersed,
                KeyEvent::new(key_code, KeyModifiers::NONE),
            );

            assert_eq!(action, expected_action);
        }
    }
}
