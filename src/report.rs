//! Progress / log reporting shared by the install and uninstall engines.
//!
//! The engines only talk to a [`Reporter`]; the GUI implements it with shared
//! state polled every frame, silent mode prints to the console.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Error text used when the user cancelled an operation.
pub const CANCELLED: &str = "Cancelled by user";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Ok,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub level: Level,
    pub text: String,
}

pub trait Reporter: Send + Sync {
    fn log(&self, level: Level, text: &str);
    /// Short description of the current step ("Downloading Zenless Torrent…").
    fn status(&self, text: &str);
    /// Overall progress, 0..=1.
    fn progress(&self, fraction: f32);
    fn cancelled(&self) -> bool;

    fn info(&self, text: &str) {
        self.log(Level::Info, text);
    }
    fn ok(&self, text: &str) {
        self.log(Level::Ok, text);
    }
    fn warn(&self, text: &str) {
        self.log(Level::Warn, text);
    }
    fn error(&self, text: &str) {
        self.log(Level::Error, text);
    }
    /// `Err(CANCELLED)` when the user asked to stop.
    fn check_cancel(&self) -> Result<(), String> {
        if self.cancelled() { Err(CANCELLED.to_owned()) } else { Ok(()) }
    }
}

/// Maps the progress of one step into its slice of the overall bar.
#[derive(Clone, Copy)]
pub struct Stage<'a> {
    pub reporter: &'a dyn Reporter,
    pub base: f32,
    pub span: f32,
}

impl<'a> Stage<'a> {
    pub fn new(reporter: &'a dyn Reporter, base: f32, span: f32) -> Self {
        Self { reporter, base, span }
    }

    /// Reports `fraction` (0..=1) of this stage as overall progress.
    pub fn set(&self, fraction: f32) {
        self.reporter
            .progress(self.base + self.span * fraction.clamp(0.0, 1.0));
    }

    pub fn done(&self) {
        self.set(1.0);
    }
}

// ---------------------------------------------------------------------------
// GUI reporter: shared state + repaint callback
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct TaskState {
    pub log: Vec<LogLine>,
    pub status: String,
    pub progress: f32,
}

pub struct SharedReporter {
    pub state: Arc<Mutex<TaskState>>,
    pub cancel: Arc<AtomicBool>,
    repaint: Box<dyn Fn() + Send + Sync>,
}

impl SharedReporter {
    pub fn new(
        state: Arc<Mutex<TaskState>>,
        cancel: Arc<AtomicBool>,
        repaint: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self { state, cancel, repaint: Box::new(repaint) }
    }

    fn with(&self, f: impl FnOnce(&mut TaskState)) {
        if let Ok(mut s) = self.state.lock() {
            f(&mut s);
        }
        (self.repaint)();
    }
}

impl Reporter for SharedReporter {
    fn log(&self, level: Level, text: &str) {
        self.with(|s| s.log.push(LogLine { level, text: text.to_owned() }));
    }
    fn status(&self, text: &str) {
        self.with(|s| s.status = text.to_owned());
    }
    fn progress(&self, fraction: f32) {
        self.with(|s| s.progress = fraction.clamp(0.0, 1.0));
    }
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------------------
// Console reporter (silent mode)
// ---------------------------------------------------------------------------

pub struct ConsoleReporter {
    log_file: Option<Mutex<std::fs::File>>,
}

impl ConsoleReporter {
    pub fn new(log_path: Option<&std::path::Path>) -> Self {
        let log_file = log_path
            .and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok())
            .map(Mutex::new);
        Self { log_file }
    }
}

impl Reporter for ConsoleReporter {
    fn log(&self, level: Level, text: &str) {
        let tag = match level {
            Level::Info => "     ",
            Level::Ok => "[ok] ",
            Level::Warn => "[warn]",
            Level::Error => "[error]",
        };
        let line = format!("{tag} {text}");
        // Printing can fail when there is no console at all; that's fine.
        let _ = writeln!(std::io::stdout(), "{line}");
        if let Some(f) = &self.log_file
            && let Ok(mut f) = f.lock()
        {
            let _ = writeln!(f, "{line}");
        }
    }
    fn status(&self, _text: &str) {}
    fn progress(&self, _fraction: f32) {}
    fn cancelled(&self) -> bool {
        false
    }
}
