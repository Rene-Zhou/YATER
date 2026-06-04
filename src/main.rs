use std::io::IsTerminal;

use clap::error::ErrorKind;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use yater::cli::parse_from;
use yater::epub;
use yater::image::{select_image_mode, ImageModeSupport};
use yater::issue_log::IssueLog;
use yater::progress::ProgressStore;
use yater::runtime::{
    build_app_with_image_mode, run_terminal_event_loop_with_progress, CrosstermEventSource,
    RuntimeError,
};
use yater::terminal::{
    should_run_interactive, with_terminal_session, CrosstermTerminalSession, TerminalError,
};

fn main() {
    let cli = match parse_from(std::env::args_os()) {
        Ok(cli) => cli,
        Err(error) => {
            let exit_code = match error.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => 0,
                _ => 1,
            };

            let _ = error.print();
            std::process::exit(exit_code);
        }
    };

    if !cli.file.exists() {
        eprintln!("file not found: {}", cli.file.display());
        std::process::exit(1);
    }

    let image_mode = select_image_mode(cli.image_mode, ImageModeSupport::terminal_default());
    let issue_log = IssueLog::from_env();
    let mut app = match build_app_with_image_mode(
        &cli.file,
        image_mode,
        |path| {
            epub::open_with_issue_logger(path, |issue| {
                if let Some(log) = &issue_log {
                    let _ = log.append(current_timestamp(), issue);
                }
            })
            .map_err(|error| RuntimeError::new(error.to_string()))
        },
        |path| {
            ProgressStore::from_env()
                .map(|store| {
                    store
                        .load(path)
                        .map_err(|error| RuntimeError::new(error.to_string()))
                })
                .unwrap_or(Ok(None))
        },
    ) {
        Ok(app) => app,
        Err(error) => {
            eprintln!("failed to open EPUB: {error}");
            std::process::exit(1);
        }
    };

    if !should_run_interactive(std::io::stdin().is_terminal(), std::io::stdout().is_terminal()) {
        return;
    }

    let mut session = CrosstermTerminalSession;
    if let Err(error) = with_terminal_session(&mut session, || {
        let backend = CrosstermBackend::new(std::io::stdout());
        let mut terminal =
            Terminal::new(backend).map_err(|error| TerminalError::new(error.to_string()))?;
        let mut events = CrosstermEventSource;
        let progress_store = ProgressStore::from_env();
        let book_path = cli.file.clone();

        run_terminal_event_loop_with_progress(
            &mut terminal,
            &mut app,
            &mut events,
            current_timestamp,
            |progress| {
                if let Some(store) = &progress_store {
                    store
                        .save(&book_path, progress)
                        .map_err(|error| RuntimeError::new(error.to_string()))?;
                }

                Ok(())
            },
        )
            .map_err(|error| TerminalError::new(error.to_string()))
    }) {
        eprintln!("terminal error: {error}");
        std::process::exit(1);
    };
}

fn current_timestamp() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}
