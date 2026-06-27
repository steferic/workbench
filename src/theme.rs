//! Centralized UI theme. All chrome colors flow through a [`Theme`] of semantic
//! roles so the whole UI can switch between dark and light. The active theme is
//! kept in a thread-local (the TUI renders on one thread) set once per frame
//! from [`crate::app::AppState`], so deeply-nested render code can read colors
//! via [`current()`] without threading a theme through every signature.
//!
//! NOTE: this themes UI *chrome* only. Colors that come from agent output (the
//! vt100 cell colors) are the program's own colors and are intentionally left
//! untouched.

use std::cell::Cell;

use ratatui::style::Color;
use serde::{Deserialize, Serialize};

/// Which palette the UI is rendered with. Persisted in the global config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

impl ThemeMode {
    pub fn toggled(self) -> Self {
        match self {
            ThemeMode::Dark => ThemeMode::Light,
            ThemeMode::Light => ThemeMode::Dark,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::Dark => "Dark",
            ThemeMode::Light => "Light",
        }
    }

    pub fn palette(self) -> Theme {
        match self {
            ThemeMode::Dark => Theme::DARK,
            ThemeMode::Light => Theme::LIGHT,
        }
    }
}

/// A full set of semantic UI colors. `Copy` and cheap to pass around.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// App-wide background fill.
    pub bg: Color,
    /// Primary text.
    pub fg: Color,
    /// Secondary / muted text.
    pub fg_dim: Color,
    /// Tertiary / disabled text and subtle separators.
    pub fg_faint: Color,
    /// Dimmest foreground: inactive/unfocused hints (below `fg_faint`).
    pub inactive: Color,
    /// Unfocused pane border.
    pub border: Color,
    /// Focused pane border.
    pub border_focused: Color,
    /// Primary accent (links, highlights, focused chrome).
    pub accent: Color,
    /// Active / current selection emphasis.
    pub active: Color,
    /// Success / running / additions.
    pub success: Color,
    /// Warnings.
    pub warning: Color,
    /// Errors / deletions / destructive.
    pub error: Color,
    /// Informational.
    pub info: Color,
    /// Special accent (pinned, badges).
    pub special: Color,
    /// Start-command / shell emphasis (orange family).
    pub command: Color,
    /// Strong danger emphasis (skip-permissions, ⚡).
    pub danger: Color,
    /// Text drawn on top of an accent-colored background.
    pub on_accent: Color,
    /// A neutral selection background.
    pub selection_bg: Color,
}

impl Theme {
    /// Dark palette — preserves the original look (maps onto the literal colors
    /// the UI used before theming).
    pub const DARK: Theme = Theme {
        bg: Color::Reset,
        fg: Color::White,
        fg_dim: Color::Gray,
        fg_faint: Color::DarkGray,
        inactive: Color::Rgb(60, 60, 60),
        border: Color::DarkGray,
        border_focused: Color::Cyan,
        accent: Color::Cyan,
        active: Color::Yellow,
        success: Color::Green,
        warning: Color::Yellow,
        error: Color::Red,
        info: Color::Blue,
        special: Color::Magenta,
        command: Color::Rgb(255, 165, 0),
        danger: Color::Rgb(255, 100, 50),
        on_accent: Color::Black,
        selection_bg: Color::Rgb(40, 44, 52),
    };

    /// Light palette.
    pub const LIGHT: Theme = Theme {
        bg: Color::Rgb(250, 250, 248),
        fg: Color::Rgb(28, 30, 34),
        fg_dim: Color::Rgb(92, 96, 102),
        fg_faint: Color::Rgb(150, 154, 160),
        inactive: Color::Rgb(196, 199, 204),
        border: Color::Rgb(200, 202, 208),
        border_focused: Color::Rgb(0, 118, 148),
        accent: Color::Rgb(0, 118, 148),
        active: Color::Rgb(176, 110, 0),
        success: Color::Rgb(22, 138, 44),
        warning: Color::Rgb(176, 110, 0),
        error: Color::Rgb(196, 36, 36),
        info: Color::Rgb(34, 92, 196),
        special: Color::Rgb(158, 42, 158),
        command: Color::Rgb(184, 104, 0),
        danger: Color::Rgb(206, 70, 30),
        on_accent: Color::Rgb(250, 250, 250),
        selection_bg: Color::Rgb(222, 230, 238),
    };
}

thread_local! {
    static CURRENT: Cell<Theme> = const { Cell::new(Theme::DARK) };
}

/// Set the active theme for the current (render) thread. Call once per frame.
pub fn set_current(mode: ThemeMode) {
    CURRENT.with(|c| c.set(mode.palette()));
}

/// The active theme. Read by render code to resolve semantic colors.
pub fn current() -> Theme {
    CURRENT.with(|c| c.get())
}
