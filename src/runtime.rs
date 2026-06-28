use std::collections::VecDeque;
use std::path::Path;
use std::time::Duration;

use crate::app::App;
use crate::document::Document;
use crate::image::SelectedImageMode;
use crate::input::{Action, map_key};
use crate::progress::Progress;
use crate::render;

const PROGRESS_SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

#[derive(Debug)]
pub struct RuntimeError(String);

impl RuntimeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for RuntimeError {}

pub fn build_app(
    file: &Path,
    load_document: impl FnOnce(&Path) -> Result<Document, RuntimeError>,
    load_progress: impl FnOnce(&Path) -> Result<Option<Progress>, RuntimeError>,
) -> Result<App, RuntimeError> {
    build_app_with_image_mode(
        file,
        SelectedImageMode::Halfblock,
        load_document,
        load_progress,
    )
}

pub fn build_app_with_image_mode(
    file: &Path,
    image_mode: SelectedImageMode,
    load_document: impl FnOnce(&Path) -> Result<Document, RuntimeError>,
    load_progress: impl FnOnce(&Path) -> Result<Option<Progress>, RuntimeError>,
) -> Result<App, RuntimeError> {
    let document = load_document(file)?;
    let progress = load_progress(file).unwrap_or(None);

    let mut app = App::with_restored_progress(document, progress);
    app.set_image_mode(image_mode);
    Ok(app)
}

pub fn handle_key(app: &mut App, key: crossterm::event::KeyEvent) -> bool {
    let action = map_key(app.focus(), key);

    if action == Action::Quit {
        return true;
    }

    app.apply(action);
    false
}

pub fn handle_key_with_progress(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    timestamp: impl FnOnce() -> String,
    save_progress: impl FnOnce(Progress) -> Result<(), RuntimeError>,
) -> Result<bool, RuntimeError> {
    let action = map_key(app.focus(), key);

    if action == Action::Quit {
        return Ok(true);
    }

    let position_before = app.position();
    app.apply(action);

    if saves_progress_after(action) && app.position() != position_before {
        save_progress(app.progress(timestamp()))?;
    }

    Ok(false)
}

fn saves_progress_after(action: Action) -> bool {
    matches!(
        action,
        Action::NextSentence
            | Action::PreviousSentence
            | Action::NextParagraph
            | Action::PreviousParagraph
            | Action::FastNextSentence
            | Action::FastPreviousSentence
            | Action::JumpToChapterStart
            | Action::JumpToChapterEnd
            | Action::ExpandOrJumpToc
    )
}

pub fn run_terminal_loop<B: ratatui::backend::Backend>(
    terminal: &mut ratatui::Terminal<B>,
    app: &mut App,
    keys: impl IntoIterator<Item = crossterm::event::KeyEvent>,
) -> Result<(), RuntimeError> {
    sync_terminal_size(terminal, app)?;
    terminal
        .draw(|frame| render::draw(frame, app))
        .map_err(|error| RuntimeError::new(error.to_string()))?;

    for key in keys {
        if handle_key(app, key) {
            break;
        }

        terminal
            .draw(|frame| render::draw(frame, app))
            .map_err(|error| RuntimeError::new(error.to_string()))?;
    }

    Ok(())
}

#[derive(Debug)]
pub enum RuntimeEvent {
    Terminal(crossterm::event::Event),
    ProgressDebounceElapsed,
}

pub trait EventSource {
    fn next_event(&mut self) -> Result<RuntimeEvent, RuntimeError>;
}

#[derive(Default)]
pub struct CrosstermEventSource {
    pending_events: VecDeque<crossterm::event::Event>,
}

impl CrosstermEventSource {
    pub fn new() -> Self {
        Self::default()
    }
}

impl EventSource for CrosstermEventSource {
    fn next_event(&mut self) -> Result<RuntimeEvent, RuntimeError> {
        if let Some(event) = self.pending_events.pop_front() {
            return Ok(RuntimeEvent::Terminal(event));
        }

        let has_event = crossterm::event::poll(PROGRESS_SAVE_DEBOUNCE)
            .map_err(|error| RuntimeError::new(error.to_string()))?;

        if !has_event {
            return Ok(RuntimeEvent::ProgressDebounceElapsed);
        }

        let first_event =
            crossterm::event::read().map_err(|error| RuntimeError::new(error.to_string()))?;
        let mut ready_events = vec![first_event];

        while crossterm::event::poll(Duration::ZERO)
            .map_err(|error| RuntimeError::new(error.to_string()))?
        {
            ready_events.push(
                crossterm::event::read().map_err(|error| RuntimeError::new(error.to_string()))?,
            );
        }

        let mut coalesced_events = coalesce_ready_terminal_events(ready_events).into_iter();
        if let Some(event) = coalesced_events.next() {
            self.pending_events.extend(coalesced_events);
            Ok(RuntimeEvent::Terminal(event))
        } else {
            Ok(RuntimeEvent::ProgressDebounceElapsed)
        }
    }
}

fn coalesce_ready_terminal_events(
    events: Vec<crossterm::event::Event>,
) -> Vec<crossterm::event::Event> {
    let mut coalesced = Vec::new();
    let mut index = 0;

    while index < events.len() {
        let event = &events[index];
        let crossterm::event::Event::Key(key) = event else {
            coalesced.push(event.clone());
            index += 1;
            continue;
        };

        if !is_coalescible_navigation_key(*key) {
            coalesced.push(event.clone());
            index += 1;
            continue;
        }

        if key.kind == crossterm::event::KeyEventKind::Press {
            coalesced.push(event.clone());
        }

        index += 1;
        while index < events.len() {
            if !is_same_coalescible_key_event(&events[index], *key) {
                break;
            }
            index += 1;
        }
    }

    coalesced
}

fn is_same_coalescible_key_event(
    event: &crossterm::event::Event,
    key: crossterm::event::KeyEvent,
) -> bool {
    let crossterm::event::Event::Key(candidate) = event else {
        return false;
    };

    is_coalescible_navigation_key(*candidate)
        && candidate.code == key.code
        && candidate.modifiers == key.modifiers
}

fn is_coalescible_navigation_key(key: crossterm::event::KeyEvent) -> bool {
    use crossterm::event::{KeyCode, KeyEventKind};

    matches!(
        key.kind,
        KeyEventKind::Press | KeyEventKind::Repeat | KeyEventKind::Release
    ) && matches!(
        key.code,
        KeyCode::Char('j')
            | KeyCode::Char('k')
            | KeyCode::Char('h')
            | KeyCode::Char('l')
            | KeyCode::Char('u')
            | KeyCode::Char('n')
            | KeyCode::Up
            | KeyCode::Down
    )
}

pub fn run_terminal_event_loop<B: ratatui::backend::Backend, E: EventSource>(
    terminal: &mut ratatui::Terminal<B>,
    app: &mut App,
    events: &mut E,
) -> Result<(), RuntimeError> {
    run_terminal_event_loop_with_key_handler(terminal, app, events, |app, key| {
        Ok(handle_key(app, key))
    })
}

pub fn run_terminal_event_loop_with_progress<B, E>(
    terminal: &mut ratatui::Terminal<B>,
    app: &mut App,
    events: &mut E,
    mut timestamp: impl FnMut() -> String,
    mut save_progress: impl FnMut(Progress) -> Result<(), RuntimeError>,
) -> Result<(), RuntimeError>
where
    B: ratatui::backend::Backend,
    E: EventSource,
{
    sync_terminal_size(terminal, app)?;
    terminal
        .draw(|frame| render::draw(frame, app))
        .map_err(|error| RuntimeError::new(error.to_string()))?;

    let mut pending_progress = None;

    loop {
        let event = match events.next_event() {
            Ok(event) => event,
            Err(error) => {
                flush_pending_progress(&mut pending_progress, &mut save_progress)?;
                return Err(error);
            }
        };

        match event {
            RuntimeEvent::Terminal(crossterm::event::Event::Key(key)) => {
                let action = map_key(app.focus(), key);
                if action == Action::Quit {
                    flush_pending_progress(&mut pending_progress, &mut save_progress)?;
                    break;
                }

                let position_before = app.position();
                app.apply(action);

                if saves_progress_after(action) && app.position() != position_before {
                    pending_progress = Some(app.progress(timestamp()));
                }

                if let Err(error) = terminal.draw(|frame| render::draw(frame, app)) {
                    flush_pending_progress(&mut pending_progress, &mut save_progress)?;
                    return Err(RuntimeError::new(error.to_string()));
                }
            }
            RuntimeEvent::Terminal(crossterm::event::Event::Resize(width, height)) => {
                app.set_terminal_size(width, height);
                terminal
                    .draw(|frame| render::draw(frame, app))
                    .map_err(|error| RuntimeError::new(error.to_string()))?;
            }
            RuntimeEvent::ProgressDebounceElapsed => {
                flush_pending_progress(&mut pending_progress, &mut save_progress)?;
            }
            RuntimeEvent::Terminal(_) => {}
        }
    }

    Ok(())
}

fn flush_pending_progress(
    pending_progress: &mut Option<Progress>,
    save_progress: &mut impl FnMut(Progress) -> Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    if let Some(progress) = pending_progress.take() {
        save_progress(progress)?;
    }

    Ok(())
}

fn run_terminal_event_loop_with_key_handler<B, E>(
    terminal: &mut ratatui::Terminal<B>,
    app: &mut App,
    events: &mut E,
    mut handle_key_event: impl FnMut(&mut App, crossterm::event::KeyEvent) -> Result<bool, RuntimeError>,
) -> Result<(), RuntimeError>
where
    B: ratatui::backend::Backend,
    E: EventSource,
{
    sync_terminal_size(terminal, app)?;
    terminal
        .draw(|frame| render::draw(frame, app))
        .map_err(|error| RuntimeError::new(error.to_string()))?;

    loop {
        match events.next_event()? {
            RuntimeEvent::Terminal(crossterm::event::Event::Key(key)) => {
                if handle_key_event(app, key)? {
                    break;
                }

                terminal
                    .draw(|frame| render::draw(frame, app))
                    .map_err(|error| RuntimeError::new(error.to_string()))?;
            }
            RuntimeEvent::Terminal(crossterm::event::Event::Resize(width, height)) => {
                app.set_terminal_size(width, height);
                terminal
                    .draw(|frame| render::draw(frame, app))
                    .map_err(|error| RuntimeError::new(error.to_string()))?;
            }
            RuntimeEvent::ProgressDebounceElapsed | RuntimeEvent::Terminal(_) => {}
        }
    }

    Ok(())
}

fn sync_terminal_size<B: ratatui::backend::Backend>(
    terminal: &ratatui::Terminal<B>,
    app: &mut App,
) -> Result<(), RuntimeError> {
    let size = terminal
        .size()
        .map_err(|error| RuntimeError::new(error.to_string()))?;
    app.set_terminal_size(size.width, size.height);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io;
    use std::path::Path;

    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::{Backend, ClearType, TestBackend, WindowSize};
    use ratatui::buffer::Cell;
    use ratatui::layout::{Position, Size};

    use crate::app::App;
    use crate::document::{AnnotationRef, Block, Document, TextBlock, TocNode};
    use crate::image::SelectedImageMode;
    use crate::input::Focus;
    use crate::progress::Progress;

    use super::{EventSource, RuntimeError, RuntimeEvent, build_app};

    struct VecEventSource {
        events: Vec<RuntimeEvent>,
    }

    struct FailingDrawBackend {
        size: Size,
        draw_calls: usize,
        fail_on_draw: usize,
    }

    impl FailingDrawBackend {
        fn new(width: u16, height: u16, fail_on_draw: usize) -> Self {
            Self {
                size: Size::new(width, height),
                draw_calls: 0,
                fail_on_draw,
            }
        }
    }

    impl Backend for FailingDrawBackend {
        type Error = io::Error;

        fn draw<'a, I>(&mut self, _content: I) -> Result<(), Self::Error>
        where
            I: Iterator<Item = (u16, u16, &'a Cell)>,
        {
            self.draw_calls += 1;
            if self.draw_calls == self.fail_on_draw {
                return Err(io::Error::other("draw failed"));
            }

            Ok(())
        }

        fn hide_cursor(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }

        fn show_cursor(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }

        fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
            Ok(Position::default())
        }

        fn set_cursor_position<P: Into<Position>>(
            &mut self,
            _position: P,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn clear(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }

        fn clear_region(&mut self, _clear_type: ClearType) -> Result<(), Self::Error> {
            Ok(())
        }

        fn size(&self) -> Result<Size, Self::Error> {
            Ok(self.size)
        }

        fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
            Ok(WindowSize {
                columns_rows: self.size,
                pixels: Size::default(),
            })
        }

        fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    impl EventSource for VecEventSource {
        fn next_event(&mut self) -> Result<RuntimeEvent, RuntimeError> {
            if self.events.is_empty() {
                return Err(RuntimeError::new("no more events"));
            }

            Ok(self.events.remove(0))
        }
    }

    fn terminal_event(event: Event) -> RuntimeEvent {
        RuntimeEvent::Terminal(event)
    }

    fn key_event(character: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE))
    }

    #[test]
    fn builds_app_from_document_and_restored_progress() {
        let app = build_app(
            Path::new("/books/book.epub"),
            |_| {
                Ok(Document {
                    blocks: vec![Block::Text(TextBlock {
                        text: "First. Second.".to_string(),
                        chapter_index: 0,
                        presentation: Default::default(),
                        styles: Vec::new(),
                        annotations: Vec::new(),
                    })],
                    toc: Vec::new(),
                    annotations: HashMap::new(),
                    chapter_ranges: Vec::new(),
                })
            },
            |_| {
                Ok(Some(Progress {
                    block_index: 0,
                    sentence_offset: "First.".len(),
                    timestamp: "2026-06-03T12:00:00Z".to_string(),
                }))
            },
        )
        .expect("build app");

        assert_eq!(app.position().block_index, 0);
        assert_eq!(app.position().sentence_offset, "First.".len());
    }

    #[test]
    fn build_app_ignores_stale_restored_sentence_offset() {
        let app = build_app(
            Path::new("/books/book.epub"),
            |_| {
                Ok(Document {
                    blocks: vec![Block::Text(TextBlock {
                        text: "First. Second.".to_string(),
                        chapter_index: 0,
                        presentation: Default::default(),
                        styles: Vec::new(),
                        annotations: Vec::new(),
                    })],
                    toc: Vec::new(),
                    annotations: HashMap::new(),
                    chapter_ranges: Vec::new(),
                })
            },
            |_| {
                Ok(Some(Progress {
                    block_index: 0,
                    sentence_offset: 1,
                    timestamp: "2026-06-03T12:00:00Z".to_string(),
                }))
            },
        )
        .expect("build app");

        assert_eq!(app.position().block_index, 0);
        assert_eq!(app.position().sentence_offset, 0);
    }

    #[test]
    fn build_app_ignores_progress_loader_errors() {
        let app = build_app(
            Path::new("/books/book.epub"),
            |_| {
                Ok(Document {
                    blocks: vec![Block::Text(TextBlock {
                        text: "First. Second.".to_string(),
                        chapter_index: 0,
                        presentation: Default::default(),
                        styles: Vec::new(),
                        annotations: Vec::new(),
                    })],
                    toc: Vec::new(),
                    annotations: HashMap::new(),
                    chapter_ranges: Vec::new(),
                })
            },
            |_| Err(RuntimeError("progress file is corrupt".to_string())),
        )
        .expect("build app without restored progress");

        assert_eq!(app.position().block_index, 0);
        assert_eq!(app.position().sentence_offset, 0);
    }

    #[test]
    fn builds_app_with_selected_image_mode() {
        let app = super::build_app_with_image_mode(
            Path::new("/books/book.epub"),
            SelectedImageMode::Off,
            |_| {
                Ok(Document {
                    blocks: vec![Block::Text(TextBlock {
                        text: "First.".to_string(),
                        chapter_index: 0,
                        presentation: Default::default(),
                        styles: Vec::new(),
                        annotations: Vec::new(),
                    })],
                    toc: Vec::new(),
                    annotations: HashMap::new(),
                    chapter_ranges: Vec::new(),
                })
            },
            |_| Ok(None),
        )
        .expect("build app");

        assert_eq!(app.image_mode(), SelectedImageMode::Off);
    }

    #[test]
    fn returns_document_loader_errors() {
        let error = build_app(
            Path::new("/books/book.epub"),
            |_| Err(RuntimeError("cannot parse EPUB".to_string())),
            |_| Ok(None),
        )
        .expect_err("document loading should fail");

        assert_eq!(error.to_string(), "cannot parse EPUB");
    }

    #[test]
    fn handle_key_dispatches_action_and_reports_quit() {
        let mut app = build_app(
            Path::new("/books/book.epub"),
            |_| {
                Ok(Document {
                    blocks: vec![Block::Text(TextBlock {
                        text: "First. Second.".to_string(),
                        chapter_index: 0,
                        presentation: Default::default(),
                        styles: Vec::new(),
                        annotations: Vec::new(),
                    })],
                    toc: Vec::new(),
                    annotations: HashMap::new(),
                    chapter_ranges: Vec::new(),
                })
            },
            |_| Ok(None),
        )
        .expect("build app");

        let should_quit =
            super::handle_key(&mut app, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

        assert!(!should_quit);
        assert_eq!(app.focus(), Focus::Toc);

        let should_quit = super::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        );
        assert!(!should_quit);

        app.apply(crate::input::Action::CloseToc);
        let should_quit = super::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        );
        assert!(should_quit);
    }

    #[test]
    fn handle_key_with_progress_saves_after_navigation() {
        let mut app = build_app(
            Path::new("/books/book.epub"),
            |_| {
                Ok(Document {
                    blocks: vec![Block::Text(TextBlock {
                        text: "First. Second.".to_string(),
                        chapter_index: 0,
                        presentation: Default::default(),
                        styles: Vec::new(),
                        annotations: Vec::new(),
                    })],
                    toc: Vec::new(),
                    annotations: HashMap::new(),
                    chapter_ranges: Vec::new(),
                })
            },
            |_| Ok(None),
        )
        .expect("build app");
        let mut saved_progress = Vec::new();

        let should_quit = super::handle_key_with_progress(
            &mut app,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
            || "2026-06-04T12:00:00Z".to_string(),
            |progress| {
                saved_progress.push(progress);
                Ok(())
            },
        )
        .expect("handle key");

        assert!(!should_quit);
        assert_eq!(
            saved_progress,
            vec![Progress {
                block_index: 0,
                sentence_offset: "First.".len(),
                timestamp: "2026-06-04T12:00:00Z".to_string(),
            }]
        );
    }

    #[test]
    fn handle_key_with_progress_does_not_save_for_focus_only_action() {
        let mut app = build_app(
            Path::new("/books/book.epub"),
            |_| {
                Ok(Document {
                    blocks: vec![Block::Text(TextBlock {
                        text: "First. Second.".to_string(),
                        chapter_index: 0,
                        presentation: Default::default(),
                        styles: Vec::new(),
                        annotations: Vec::new(),
                    })],
                    toc: Vec::new(),
                    annotations: HashMap::new(),
                    chapter_ranges: Vec::new(),
                })
            },
            |_| Ok(None),
        )
        .expect("build app");
        let mut save_count = 0;

        super::handle_key_with_progress(
            &mut app,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            || "2026-06-04T12:00:00Z".to_string(),
            |_| {
                save_count += 1;
                Ok(())
            },
        )
        .expect("handle key");

        assert_eq!(app.focus(), Focus::Toc);
        assert_eq!(save_count, 0);
    }

    #[test]
    fn handle_key_with_progress_does_not_save_for_toc_expand_only_action() {
        let mut app = build_app(
            Path::new("/books/book.epub"),
            |_| Ok(toc_document()),
            |_| Ok(None),
        )
        .expect("build app");
        app.apply(crate::input::Action::OpenToc);
        app.apply(crate::input::Action::CollapseOrParentToc);
        let mut save_count = 0;

        super::handle_key_with_progress(
            &mut app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            || "2026-06-04T12:00:00Z".to_string(),
            |_| {
                save_count += 1;
                Ok(())
            },
        )
        .expect("handle key");

        assert_eq!(app.focus(), Focus::Toc);
        assert_eq!(app.position().block_index, 0);
        assert_eq!(save_count, 0);
    }

    #[test]
    fn terminal_loop_draws_and_stops_on_quit_key() {
        let mut app = build_app(
            Path::new("/books/book.epub"),
            |_| {
                Ok(Document {
                    blocks: vec![Block::Text(TextBlock {
                        text: "First. Second.".to_string(),
                        chapter_index: 0,
                        presentation: Default::default(),
                        styles: Vec::new(),
                        annotations: Vec::new(),
                    })],
                    toc: Vec::new(),
                    annotations: HashMap::new(),
                    chapter_ranges: Vec::new(),
                })
            },
            |_| Ok(None),
        )
        .expect("build app");
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend).expect("terminal");

        super::run_terminal_loop(
            &mut terminal,
            &mut app,
            [
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            ],
        )
        .expect("run terminal loop");

        assert_eq!(app.position().sentence_offset, "First.".len());
    }

    #[test]
    fn terminal_loop_uses_terminal_width_for_annotation_scrolling() {
        let mut app = App::new(Document {
            blocks: vec![Block::Text(TextBlock {
                text: "Text with [1].".to_string(),
                chapter_index: 0,
                presentation: Default::default(),
                styles: Vec::new(),
                annotations: vec![AnnotationRef {
                    id: "note-1".to_string(),
                    offset: "Text with ".len(),
                }],
            })],
            toc: Vec::new(),
            annotations: HashMap::from([(
                "note-1".to_string(),
                "Alpha beta gamma delta epsilon zeta.".to_string(),
            )]),
            chapter_ranges: Vec::new(),
        });
        let backend = TestBackend::new(20, 4);
        let mut terminal = Terminal::new(backend).expect("terminal");

        super::run_terminal_loop(
            &mut terminal,
            &mut app,
            [
                KeyEvent::new(KeyCode::Char(';'), KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
            ],
        )
        .expect("run terminal loop");

        assert_eq!(app.annotation_scroll(), 1);
    }

    #[test]
    fn terminal_event_loop_dispatches_key_events_and_redraws_on_resize() {
        let mut app = build_app(
            Path::new("/books/book.epub"),
            |_| {
                Ok(Document {
                    blocks: vec![Block::Text(TextBlock {
                        text: "First. Second.".to_string(),
                        chapter_index: 0,
                        presentation: Default::default(),
                        styles: Vec::new(),
                        annotations: Vec::new(),
                    })],
                    toc: Vec::new(),
                    annotations: HashMap::new(),
                    chapter_ranges: Vec::new(),
                })
            },
            |_| Ok(None),
        )
        .expect("build app");
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut events = VecEventSource {
            events: vec![
                terminal_event(Event::Resize(80, 24)),
                terminal_event(Event::Key(KeyEvent::new(
                    KeyCode::Char('j'),
                    KeyModifiers::NONE,
                ))),
                terminal_event(Event::Key(KeyEvent::new(
                    KeyCode::Char('q'),
                    KeyModifiers::NONE,
                ))),
            ],
        };

        super::run_terminal_event_loop(&mut terminal, &mut app, &mut events)
            .expect("run terminal event loop");

        assert_eq!(app.position().sentence_offset, "First.".len());
    }

    #[test]
    fn terminal_event_loop_ignores_auto_repeat_key_events() {
        let mut app = build_app(
            Path::new("/books/book.epub"),
            |_| {
                Ok(Document {
                    blocks: vec![Block::Text(TextBlock {
                        text: "First. Second. Third. Fourth.".to_string(),
                        chapter_index: 0,
                        presentation: Default::default(),
                        styles: Vec::new(),
                        annotations: Vec::new(),
                    })],
                    toc: Vec::new(),
                    annotations: HashMap::new(),
                    chapter_ranges: Vec::new(),
                })
            },
            |_| Ok(None),
        )
        .expect("build app");
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut events = VecEventSource {
            events: vec![
                terminal_event(Event::Key(KeyEvent::new(
                    KeyCode::Char('j'),
                    KeyModifiers::NONE,
                ))),
                terminal_event(Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Char('j'),
                    KeyModifiers::NONE,
                    KeyEventKind::Repeat,
                ))),
                terminal_event(Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Char('j'),
                    KeyModifiers::NONE,
                    KeyEventKind::Repeat,
                ))),
                terminal_event(Event::Key(KeyEvent::new(
                    KeyCode::Char('q'),
                    KeyModifiers::NONE,
                ))),
            ],
        };

        super::run_terminal_event_loop(&mut terminal, &mut app, &mut events)
            .expect("run terminal event loop");

        assert_eq!(app.position().sentence_offset, "First.".len());
    }

    #[test]
    fn coalesces_ready_navigation_key_press_backlog() {
        let events = super::coalesce_ready_terminal_events(vec![
            key_event('j'),
            key_event('j'),
            key_event('j'),
            key_event('q'),
        ]);

        assert_eq!(events, vec![key_event('j'), key_event('q')]);
    }

    #[test]
    fn coalesces_ready_paragraph_navigation_key_backlog() {
        let events = super::coalesce_ready_terminal_events(vec![
            key_event('l'),
            key_event('l'),
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('l'),
                KeyModifiers::NONE,
                KeyEventKind::Repeat,
            )),
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('l'),
                KeyModifiers::NONE,
                KeyEventKind::Release,
            )),
            key_event('q'),
        ]);

        assert_eq!(events, vec![key_event('l'), key_event('q')]);
    }

    #[test]
    fn drops_repeat_and_release_navigation_events_from_ready_backlog() {
        let events = super::coalesce_ready_terminal_events(vec![
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('j'),
                KeyModifiers::NONE,
                KeyEventKind::Repeat,
            )),
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('j'),
                KeyModifiers::NONE,
                KeyEventKind::Release,
            )),
            key_event('q'),
        ]);

        assert_eq!(events, vec![key_event('q')]);
    }

    #[test]
    fn terminal_event_loop_saves_progress_after_navigation_key() {
        let mut app = build_app(
            Path::new("/books/book.epub"),
            |_| {
                Ok(Document {
                    blocks: vec![Block::Text(TextBlock {
                        text: "First. Second.".to_string(),
                        chapter_index: 0,
                        presentation: Default::default(),
                        styles: Vec::new(),
                        annotations: Vec::new(),
                    })],
                    toc: Vec::new(),
                    annotations: HashMap::new(),
                    chapter_ranges: Vec::new(),
                })
            },
            |_| Ok(None),
        )
        .expect("build app");
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut events = VecEventSource {
            events: vec![
                terminal_event(Event::Key(KeyEvent::new(
                    KeyCode::Char('j'),
                    KeyModifiers::NONE,
                ))),
                terminal_event(Event::Key(KeyEvent::new(
                    KeyCode::Char('q'),
                    KeyModifiers::NONE,
                ))),
            ],
        };
        let mut saved_progress = Vec::new();

        super::run_terminal_event_loop_with_progress(
            &mut terminal,
            &mut app,
            &mut events,
            || "2026-06-04T12:00:00Z".to_string(),
            |progress| {
                saved_progress.push(progress);
                Ok(())
            },
        )
        .expect("run terminal event loop");

        assert_eq!(
            saved_progress,
            vec![Progress {
                block_index: 0,
                sentence_offset: "First.".len(),
                timestamp: "2026-06-04T12:00:00Z".to_string(),
            }]
        );
    }

    #[test]
    fn terminal_event_loop_coalesces_consecutive_progress_saves() {
        let mut app = build_app(
            Path::new("/books/book.epub"),
            |_| {
                Ok(Document {
                    blocks: vec![Block::Text(TextBlock {
                        text: "First. Second. Third.".to_string(),
                        chapter_index: 0,
                        presentation: Default::default(),
                        styles: Vec::new(),
                        annotations: Vec::new(),
                    })],
                    toc: Vec::new(),
                    annotations: HashMap::new(),
                    chapter_ranges: Vec::new(),
                })
            },
            |_| Ok(None),
        )
        .expect("build app");
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut events = VecEventSource {
            events: vec![
                terminal_event(Event::Key(KeyEvent::new(
                    KeyCode::Char('j'),
                    KeyModifiers::NONE,
                ))),
                terminal_event(Event::Key(KeyEvent::new(
                    KeyCode::Char('j'),
                    KeyModifiers::NONE,
                ))),
                terminal_event(Event::Key(KeyEvent::new(
                    KeyCode::Char('q'),
                    KeyModifiers::NONE,
                ))),
            ],
        };
        let mut saved_progress = Vec::new();

        super::run_terminal_event_loop_with_progress(
            &mut terminal,
            &mut app,
            &mut events,
            || "2026-06-04T12:00:00Z".to_string(),
            |progress| {
                saved_progress.push(progress);
                Ok(())
            },
        )
        .expect("run terminal event loop");

        assert_eq!(
            saved_progress,
            vec![Progress {
                block_index: 0,
                sentence_offset: "First. Second.".len(),
                timestamp: "2026-06-04T12:00:00Z".to_string(),
            }]
        );
    }

    #[test]
    fn terminal_event_loop_flushes_progress_after_debounce_timeout() {
        let mut app = build_app(
            Path::new("/books/book.epub"),
            |_| {
                Ok(Document {
                    blocks: vec![Block::Text(TextBlock {
                        text: "First. Second.".to_string(),
                        chapter_index: 0,
                        presentation: Default::default(),
                        styles: Vec::new(),
                        annotations: Vec::new(),
                    })],
                    toc: Vec::new(),
                    annotations: HashMap::new(),
                    chapter_ranges: Vec::new(),
                })
            },
            |_| Ok(None),
        )
        .expect("build app");
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut events = VecEventSource {
            events: vec![
                terminal_event(Event::Key(KeyEvent::new(
                    KeyCode::Char('j'),
                    KeyModifiers::NONE,
                ))),
                RuntimeEvent::ProgressDebounceElapsed,
                terminal_event(Event::Key(KeyEvent::new(
                    KeyCode::Char('q'),
                    KeyModifiers::NONE,
                ))),
            ],
        };
        let mut saved_progress = Vec::new();

        super::run_terminal_event_loop_with_progress(
            &mut terminal,
            &mut app,
            &mut events,
            || "2026-06-04T12:00:00Z".to_string(),
            |progress| {
                saved_progress.push(progress);
                Ok(())
            },
        )
        .expect("run terminal event loop");

        assert_eq!(
            saved_progress,
            vec![Progress {
                block_index: 0,
                sentence_offset: "First.".len(),
                timestamp: "2026-06-04T12:00:00Z".to_string(),
            }]
        );
    }

    #[test]
    fn terminal_event_loop_flushes_progress_before_event_source_error() {
        let mut app = build_app(
            Path::new("/books/book.epub"),
            |_| {
                Ok(Document {
                    blocks: vec![Block::Text(TextBlock {
                        text: "First. Second.".to_string(),
                        chapter_index: 0,
                        presentation: Default::default(),
                        styles: Vec::new(),
                        annotations: Vec::new(),
                    })],
                    toc: Vec::new(),
                    annotations: HashMap::new(),
                    chapter_ranges: Vec::new(),
                })
            },
            |_| Ok(None),
        )
        .expect("build app");
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut events = VecEventSource {
            events: vec![terminal_event(Event::Key(KeyEvent::new(
                KeyCode::Char('j'),
                KeyModifiers::NONE,
            )))],
        };
        let mut saved_progress = Vec::new();

        let error = super::run_terminal_event_loop_with_progress(
            &mut terminal,
            &mut app,
            &mut events,
            || "2026-06-04T12:00:00Z".to_string(),
            |progress| {
                saved_progress.push(progress);
                Ok(())
            },
        )
        .expect_err("event source should fail");

        assert_eq!(error.to_string(), "no more events");
        assert_eq!(
            saved_progress,
            vec![Progress {
                block_index: 0,
                sentence_offset: "First.".len(),
                timestamp: "2026-06-04T12:00:00Z".to_string(),
            }]
        );
    }

    #[test]
    fn terminal_event_loop_flushes_progress_before_draw_error() {
        let mut app = build_app(
            Path::new("/books/book.epub"),
            |_| {
                Ok(Document {
                    blocks: vec![Block::Text(TextBlock {
                        text: "First. Second.".to_string(),
                        chapter_index: 0,
                        presentation: Default::default(),
                        styles: Vec::new(),
                        annotations: Vec::new(),
                    })],
                    toc: Vec::new(),
                    annotations: HashMap::new(),
                    chapter_ranges: Vec::new(),
                })
            },
            |_| Ok(None),
        )
        .expect("build app");
        let backend = FailingDrawBackend::new(40, 6, 2);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut events = VecEventSource {
            events: vec![terminal_event(Event::Key(KeyEvent::new(
                KeyCode::Char('j'),
                KeyModifiers::NONE,
            )))],
        };
        let mut saved_progress = Vec::new();

        let error = super::run_terminal_event_loop_with_progress(
            &mut terminal,
            &mut app,
            &mut events,
            || "2026-06-04T12:00:00Z".to_string(),
            |progress| {
                saved_progress.push(progress);
                Ok(())
            },
        )
        .expect_err("draw should fail");

        assert_eq!(error.to_string(), "draw failed");
        assert_eq!(
            saved_progress,
            vec![Progress {
                block_index: 0,
                sentence_offset: "First.".len(),
                timestamp: "2026-06-04T12:00:00Z".to_string(),
            }]
        );
    }

    fn toc_document() -> Document {
        Document {
            blocks: vec![
                Block::Text(TextBlock {
                    text: "Chapter one.".to_string(),
                    chapter_index: 0,
                    presentation: Default::default(),
                    styles: Vec::new(),
                    annotations: Vec::new(),
                }),
                Block::Text(TextBlock {
                    text: "Section one.".to_string(),
                    chapter_index: 0,
                    presentation: Default::default(),
                    styles: Vec::new(),
                    annotations: Vec::new(),
                }),
            ],
            toc: vec![TocNode {
                title: "Chapter One".to_string(),
                target_block_index: 0,
                children: vec![TocNode {
                    title: "Section One".to_string(),
                    target_block_index: 1,
                    children: Vec::new(),
                }],
            }],
            annotations: HashMap::new(),
            chapter_ranges: Vec::new(),
        }
    }
}
