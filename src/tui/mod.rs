pub mod components;
pub mod event;
pub mod ui;
pub mod utils;

use anyhow::Result;
use crossterm::{
    cursor,
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use ratatui::prelude::*;
use std::io::{self, stdout};
use std::sync::atomic::{AtomicBool, Ordering};

pub type Terminal = ratatui::Terminal<CrosstermBackend<io::Stdout>>;

/// Tracks whether the alt-screen is currently active so the panic hook can
/// undo the right state without needing a captured value.
static ALT_SCREEN_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn init(use_alternate_screen: bool) -> Result<Terminal> {
    enable_raw_mode()?;
    redirect_stderr();

    let mut cmd = stdout();
    if use_alternate_screen {
        execute!(cmd, EnterAlternateScreen)?;
        ALT_SCREEN_ACTIVE.store(true, Ordering::SeqCst);
    }
    execute!(cmd, EnableMouseCapture, EnableBracketedPaste)?;

    install_panic_hook();

    let backend = CrosstermBackend::new(stdout());
    let terminal = ratatui::Terminal::new(backend)?;
    Ok(terminal)
}

pub fn restore(use_alternate_screen: bool) -> Result<()> {
    // First, so a failed escape sequence below cannot keep stderr in the log.
    restore_stderr();
    let mut cmd = stdout();
    crate::links::reset_pointer();

    // Send terminal-state escape sequences BEFORE disabling raw mode so the
    // terminal driver doesn't echo them. Order matters: clear the alt screen,
    // disable mouse capture and bracketed paste, leave the alt screen, then
    // re-show the cursor (ratatui hides it during draws).
    if use_alternate_screen && ALT_SCREEN_ACTIVE.load(Ordering::SeqCst) {
        execute!(
            cmd,
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen,
            cursor::Show,
        )?;
        ALT_SCREEN_ACTIVE.store(false, Ordering::SeqCst);
    } else {
        // Without the alt screen, the UI rendered directly to the primary
        // buffer. Wipe it so the user's shell prompt isn't surrounded by
        // workbench residue.
        execute!(
            cmd,
            DisableMouseCapture,
            DisableBracketedPaste,
            Clear(ClearType::All),
            cursor::MoveTo(0, 0),
            cursor::Show,
        )?;
    }

    disable_raw_mode()?;
    Ok(())
}

/// The terminal's own stderr, kept while fd 2 points at the log.
#[cfg(unix)]
static SAVED_STDERR: std::sync::Mutex<Option<libc::c_int>> = std::sync::Mutex::new(None);

/// Point fd 2 at the log for as long as the TUI owns the screen.
///
/// Anything written to stderr lands on the screen between draws and stays
/// there, across pane borders, until the next full redraw. Children that
/// workbench starts outside a pane share that stderr, and so does the system:
/// after a memory-pressure episode macOS had every forked child print
/// "MallocStackLogging: can't turn off malloc stack logging" over the panes.
/// Sent to the log instead, the same text is still there for anyone looking.
///
/// Only when stderr is the terminal: a caller who redirected it chose where
/// it goes. Undone by `restore_stderr`, which `restore` runs on a normal exit
/// and in the panic hook, so a panic message still reaches the terminal.
#[cfg(unix)]
fn redirect_stderr() {
    let mut saved = SAVED_STDERR.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: isatty only reads the descriptor's type.
    if saved.is_some() || unsafe { libc::isatty(libc::STDERR_FILENO) } != 1 {
        return;
    }
    let Some(log) = crate::logger::open_for_stderr() else {
        return;
    };
    use std::os::unix::io::AsRawFd;
    // SAFETY: dup and dup2 on descriptors this process owns. `log` is closed
    // when it drops; fd 2 keeps its own reference to the file.
    unsafe {
        let original = libc::dup(libc::STDERR_FILENO);
        if original < 0 {
            return;
        }
        if libc::dup2(log.as_raw_fd(), libc::STDERR_FILENO) < 0 {
            libc::close(original);
            return;
        }
        *saved = Some(original);
    }
}

#[cfg(unix)]
fn restore_stderr() {
    let mut saved = SAVED_STDERR.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(original) = saved.take() {
        // SAFETY: `original` is the descriptor `redirect_stderr` duplicated.
        unsafe {
            libc::dup2(original, libc::STDERR_FILENO);
            libc::close(original);
        }
    }
}

#[cfg(not(unix))]
fn redirect_stderr() {}

#[cfg(not(unix))]
fn restore_stderr() {}

/// Install a process-wide panic hook that restores the terminal before
/// running the previous hook. Without this, a panic inside the main loop
/// leaves the terminal in raw mode + alt screen + mouse capture, which
/// looks like leftover UI in the user's shell.
fn install_panic_hook() {
    use std::sync::Once;
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let original = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // Write it down before touching the terminal. The default hook
            // prints to stderr, which is the one place it cannot survive: the
            // alt screen is torn down a line later and takes the message with
            // it, so a panic looked exactly like workbench quietly stopping.
            // The log outlives the process and is the only record anyone can
            // go back to.
            let payload = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panicked".to_string());
            let where_ = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                .unwrap_or_else(|| "unknown location".to_string());
            crate::logger::warn(format!("PANIC at {where_}: {payload}"));
            crate::logger::warn(format!(
                "backtrace:\n{}",
                std::backtrace::Backtrace::force_capture()
            ));

            // Best-effort restore — we don't know the original
            // `use_alternate_screen` arg here, but `ALT_SCREEN_ACTIVE`
            // captures the actual runtime state.
            let was_alt = ALT_SCREEN_ACTIVE.load(Ordering::SeqCst);
            if let Err(err) = restore(was_alt) {
                crate::logger::warn(format!("failed to restore terminal during panic: {err}"));
            }
            original(info);
        }));
    });
}
