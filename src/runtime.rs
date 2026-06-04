use std::path::Path;

use crate::app::App;
use crate::document::Document;
use crate::image::SelectedImageMode;
use crate::input::{map_key, Action};
use crate::progress::Progress;
use crate::render;

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
    let progress = load_progress(file)?;

    let position = progress
        .filter(|progress| progress.block_index < document.blocks.len())
        .map(|progress| crate::app::ReadingPosition {
            block_index: progress.block_index,
            sentence_offset: progress.sentence_offset,
        })
        .unwrap_or(crate::app::ReadingPosition {
            block_index: 0,
            sentence_offset: 0,
        });

    Ok(App::with_position_and_image_mode(
        document,
        position,
        image_mode,
    ))
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

    let should_save = saves_progress_after(action);
    app.apply(action);

    if should_save {
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
            | Action::PageDown
            | Action::PageUp
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

pub trait EventSource {
    fn next_event(&mut self) -> Result<crossterm::event::Event, RuntimeError>;
}

pub struct CrosstermEventSource;

impl EventSource for CrosstermEventSource {
    fn next_event(&mut self) -> Result<crossterm::event::Event, RuntimeError> {
        crossterm::event::read().map_err(|error| RuntimeError::new(error.to_string()))
    }
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
    run_terminal_event_loop_with_key_handler(terminal, app, events, |app, key| {
        handle_key_with_progress(app, key, &mut timestamp, &mut save_progress)
    })
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
    terminal
        .draw(|frame| render::draw(frame, app))
        .map_err(|error| RuntimeError::new(error.to_string()))?;

    loop {
        match events.next_event()? {
            crossterm::event::Event::Key(key) => {
                if handle_key_event(app, key)? {
                    break;
                }

                terminal
                    .draw(|frame| render::draw(frame, app))
                    .map_err(|error| RuntimeError::new(error.to_string()))?;
            }
            crossterm::event::Event::Resize(_, _) => {
                terminal
                    .draw(|frame| render::draw(frame, app))
                    .map_err(|error| RuntimeError::new(error.to_string()))?;
            }
            _ => {}
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use crate::document::{Block, Document, TextBlock};
    use crate::image::SelectedImageMode;
    use crate::input::Focus;
    use crate::progress::Progress;

    use super::{build_app, EventSource, RuntimeError};

    struct VecEventSource {
        events: Vec<Event>,
    }

    impl EventSource for VecEventSource {
        fn next_event(&mut self) -> Result<Event, RuntimeError> {
            if self.events.is_empty() {
                return Err(RuntimeError::new("no more events"));
            }

            Ok(self.events.remove(0))
        }
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
    fn builds_app_with_selected_image_mode() {
        let app = super::build_app_with_image_mode(
            Path::new("/books/book.epub"),
            SelectedImageMode::Off,
            |_| {
                Ok(Document {
                    blocks: vec![Block::Text(TextBlock {
                        text: "First.".to_string(),
                        chapter_index: 0,
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

        let should_quit = super::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        );

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
    fn terminal_loop_draws_and_stops_on_quit_key() {
        let mut app = build_app(
            Path::new("/books/book.epub"),
            |_| {
                Ok(Document {
                    blocks: vec![Block::Text(TextBlock {
                        text: "First. Second.".to_string(),
                        chapter_index: 0,
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
            [KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
             KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)],
        )
        .expect("run terminal loop");

        assert_eq!(app.position().sentence_offset, "First.".len());
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
                Event::Resize(80, 24),
                Event::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
                Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
            ],
        };

        super::run_terminal_event_loop(&mut terminal, &mut app, &mut events)
            .expect("run terminal event loop");

        assert_eq!(app.position().sentence_offset, "First.".len());
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
                Event::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
                Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
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
}
