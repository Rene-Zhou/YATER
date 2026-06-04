use std::panic::AssertUnwindSafe;

#[derive(Debug)]
pub struct TerminalError(String);

impl TerminalError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for TerminalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for TerminalError {}

pub trait TerminalSessionBackend {
    fn enter(&mut self) -> Result<(), TerminalError>;
    fn exit(&mut self) -> Result<(), TerminalError>;
}

pub struct CrosstermTerminalSession;

impl TerminalSessionBackend for CrosstermTerminalSession {
    fn enter(&mut self) -> Result<(), TerminalError> {
        crossterm::terminal::enable_raw_mode()
            .map_err(|error| TerminalError::new(error.to_string()))?;
        crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::EnterAlternateScreen
        )
        .map_err(|error| TerminalError::new(error.to_string()))
    }

    fn exit(&mut self) -> Result<(), TerminalError> {
        let leave_result = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen
        )
        .map_err(|error| TerminalError::new(error.to_string()));
        let raw_mode_result =
            crossterm::terminal::disable_raw_mode()
                .map_err(|error| TerminalError::new(error.to_string()));

        leave_result.and(raw_mode_result)
    }
}

pub fn should_run_interactive(stdin_is_terminal: bool, stdout_is_terminal: bool) -> bool {
    stdin_is_terminal && stdout_is_terminal
}

pub fn with_terminal_session<B, F, T>(
    backend: &mut B,
    run: F,
) -> Result<T, TerminalError>
where
    B: TerminalSessionBackend,
    F: FnOnce() -> Result<T, TerminalError>,
{
    backend.enter()?;
    let result = std::panic::catch_unwind(AssertUnwindSafe(run));
    let exit_result = backend.exit();

    match (result, exit_result) {
        (Ok(Ok(value)), Ok(())) => Ok(value),
        (Ok(Err(error)), _) => Err(error),
        (Ok(Ok(_)), Err(error)) => Err(error),
        (Err(panic), Ok(())) => Err(TerminalError::new(format!(
            "runtime panicked: {}",
            panic_message(panic)
        ))),
        (Err(_), Err(error)) => Err(error),
    }
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        return (*message).to_string();
    }

    if let Some(message) = panic.downcast_ref::<String>() {
        return message.clone();
    }

    "unknown panic".to_string()
}

#[cfg(test)]
mod tests {
    use super::{with_terminal_session, TerminalError, TerminalSessionBackend};

    #[derive(Default)]
    struct RecordingBackend {
        events: Vec<&'static str>,
    }

    impl TerminalSessionBackend for RecordingBackend {
        fn enter(&mut self) -> Result<(), TerminalError> {
            self.events.push("enter");
            Ok(())
        }

        fn exit(&mut self) -> Result<(), TerminalError> {
            self.events.push("exit");
            Ok(())
        }
    }

    #[test]
    fn exits_terminal_session_after_successful_run() {
        let mut backend = RecordingBackend::default();

        let result = with_terminal_session(&mut backend, || Ok(42)).expect("run session");

        assert_eq!(result, 42);
        assert_eq!(backend.events, vec!["enter", "exit"]);
    }

    #[test]
    fn exits_terminal_session_after_failed_run() {
        let mut backend = RecordingBackend::default();

        let error = with_terminal_session(&mut backend, || -> Result<(), TerminalError> {
            Err(TerminalError::new("runtime failed"))
        })
        .expect_err("run should fail");

        assert_eq!(error.to_string(), "runtime failed");
        assert_eq!(backend.events, vec!["enter", "exit"]);
    }

    #[test]
    fn exits_terminal_session_after_panic() {
        let mut backend = RecordingBackend::default();

        let error = with_terminal_session(&mut backend, || -> Result<(), TerminalError> {
            panic!("render panic");
        })
        .expect_err("panic should be converted to terminal error");

        assert_eq!(error.to_string(), "runtime panicked: render panic");
        assert_eq!(backend.events, vec!["enter", "exit"]);
    }

    #[test]
    fn runs_interactively_only_when_stdin_and_stdout_are_terminals() {
        assert!(super::should_run_interactive(true, true));
        assert!(!super::should_run_interactive(false, true));
        assert!(!super::should_run_interactive(true, false));
        assert!(!super::should_run_interactive(false, false));
    }
}
