pub mod components;
pub mod effects;
pub mod event;
pub mod replay;
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
    let mut cmd = stdout();

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
