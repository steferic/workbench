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

/// Which theme the UI is rendered with. Persisted in the global config.
///
/// `Dark` and `Light` are the hand-written pair this started as; the rest are
/// generated from a catalog of traditional Japanese colours (see the note above
/// [`Theme::KIMIDORI`]). They are unit variants so the config keeps storing a
/// plain name — a file written before any of them existed still loads, and one
/// naming a theme this binary does not have falls back rather than throwing the
/// rest of the config away (see `persistence::theme_or_default`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
    Kimidori,
    Tsuyukusa,
    Edo,
    Ruri,
    Kusa,
    Fuji,
    Ayame,
    Sumire,
    Aomidori,
    Botan,
    Kinu,
    Budo,
}

impl ThemeMode {
    /// Every theme, in picker order: the two originals, then the generated ones
    /// grouped by harmony.
    pub const ALL: [ThemeMode; 14] = [
        ThemeMode::Dark,
        ThemeMode::Light,
        ThemeMode::Kimidori,
        ThemeMode::Tsuyukusa,
        ThemeMode::Edo,
        ThemeMode::Ruri,
        ThemeMode::Kusa,
        ThemeMode::Fuji,
        ThemeMode::Ayame,
        ThemeMode::Sumire,
        ThemeMode::Aomidori,
        ThemeMode::Botan,
        ThemeMode::Kinu,
        ThemeMode::Budo,
    ];

    /// What the picker calls it.
    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::Dark => "Dark",
            ThemeMode::Light => "Light",
            ThemeMode::Kimidori => "Kimidori",
            ThemeMode::Tsuyukusa => "Tsuyukusa",
            ThemeMode::Edo => "Edo",
            ThemeMode::Ruri => "Ruri",
            ThemeMode::Kusa => "Kusa",
            ThemeMode::Fuji => "Fuji",
            ThemeMode::Ayame => "Ayame",
            ThemeMode::Sumire => "Sumire",
            ThemeMode::Aomidori => "Aomidori",
            ThemeMode::Botan => "Botan",
            ThemeMode::Kinu => "Kinu",
            ThemeMode::Budo => "Budo",
        }
    }

    /// How it was made, for the second column of the picker. The originals were
    /// chosen by hand and say so.
    pub fn detail(self) -> &'static str {
        match self {
            ThemeMode::Dark | ThemeMode::Light => "hand-written",
            ThemeMode::Kimidori => "mono · dark",
            ThemeMode::Tsuyukusa => "mono · dark",
            ThemeMode::Edo => "mono · light",
            ThemeMode::Ruri => "mono · light",
            ThemeMode::Kusa => "duo · dark",
            ThemeMode::Fuji => "duo · dark",
            ThemeMode::Ayame => "duo · light",
            ThemeMode::Sumire => "duo · light",
            ThemeMode::Aomidori => "triad · dark",
            ThemeMode::Botan => "triad · dark",
            ThemeMode::Kinu => "triad · light",
            ThemeMode::Budo => "triad · light",
        }
    }

    /// Whether this theme sits on a dark ground. The picker groups on it, and it
    /// is the one thing callers occasionally need that the colours do not say
    /// outright.
    pub fn is_dark(self) -> bool {
        matches!(
            self,
            ThemeMode::Dark
                | ThemeMode::Kimidori
                | ThemeMode::Tsuyukusa
                | ThemeMode::Kusa
                | ThemeMode::Fuji
                | ThemeMode::Aomidori
                | ThemeMode::Botan
        )
    }

    /// The theme a stored name refers to, if this binary has it.
    ///
    /// [`label`](Self::label) is deliberately the variant's own name, which is
    /// also what serde writes, so this reads back exactly what the config
    /// stored. `None` for a name added after this build — see
    /// `persistence::theme_or_default`, which is the only caller that matters.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.label() == name)
    }

    /// The next theme in [`ALL`](Self::ALL), wrapping. What the utility pane's
    /// Enter does — with fourteen themes a list you step through beats a toggle
    /// that can only ever reach two of them.
    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }

    pub fn palette(self) -> Theme {
        match self {
            ThemeMode::Dark => Theme::DARK,
            ThemeMode::Light => Theme::LIGHT,
            ThemeMode::Kimidori => Theme::KIMIDORI,
            ThemeMode::Tsuyukusa => Theme::TSUYUKUSA,
            ThemeMode::Edo => Theme::EDO,
            ThemeMode::Ruri => Theme::RURI,
            ThemeMode::Kusa => Theme::KUSA,
            ThemeMode::Fuji => Theme::FUJI,
            ThemeMode::Ayame => Theme::AYAME,
            ThemeMode::Sumire => Theme::SUMIRE,
            ThemeMode::Aomidori => Theme::AOMIDORI,
            ThemeMode::Botan => Theme::BOTAN,
            ThemeMode::Kinu => Theme::KINU,
            ThemeMode::Budo => Theme::BUDO,
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

    // ---- generated ----------------------------------------------------
    //
    // Everything below is derived from a catalog of 746 traditional Japanese
    // colours, by the generator in `stefanlenoach.com/src/japanese-colors`. It
    // builds ANSI *terminal* palettes, which is not this shape, so the import
    // is a mapping rather than a copy and only half of it comes from the slots:
    //
    //   - The chromatic roles are the catalog's, taken from the ANSI family
    //     that matches — accent from cyan, success from green, error from red.
    //     This is the part worth having; these are real pigments with names.
    //   - The neutral ramp (`fg_dim`, `fg_faint`, `inactive`, `border`) is
    //     interpolated between the ground and the ink instead. Borrowing it
    //     from the slots works in the dark and collapses in the light, because
    //     a terminal's "white" is a *foreground* slot: on a near-white ground
    //     it lands at 1.55:1 and disappears. The ramp gives the same ladder in
    //     both, and `border` stays a divider rather than becoming a rule.
    //   - `command` wants an orange, and no ANSI slot is orange, so it is mixed
    //     from the red and the yellow that bracket it.
    //
    // Seeds were swept and scored, not picked by eye: every chromatic role
    // clears 4.5:1 on its own ground, `border` sits in 1.35–3.4, and the six
    // roles a person reads as *signal* — success, error, warning, info,
    // special, accent — are held apart from each other. A theme where success
    // and error are the same colour is a UI that lies, so that separation is a
    // hard floor rather than a preference. Mono sits just above it by
    // construction; that is what one hue costs, and those themes say so.
    //
    // Regenerating: `node emit.mjs --rust` in the scratchpad's `gen/`.

    /// 鉄黒 Tetsuguro ground under 黄緑 Kimidori.
    ///
    /// mono dark, seed 2811 of the traditional-colour catalog.
    /// Signal roles sit 0.46 apart — close, which is what a
    /// mono palette is. Success and error differ by lightness here, not hue.
    pub const KIMIDORI: Theme = Theme {
        bg: Color::Rgb(23, 20, 18),
        fg: Color::Rgb(208, 216, 212),
        fg_dim: Color::Rgb(153, 157, 154),
        fg_faint: Color::Rgb(108, 110, 107),
        inactive: Color::Rgb(80, 81, 78),
        border: Color::Rgb(75, 75, 72),
        border_focused: Color::Rgb(167, 198, 54),
        accent: Color::Rgb(167, 198, 54),
        active: Color::Rgb(159, 217, 179),
        success: Color::Rgb(226, 192, 68),
        warning: Color::Rgb(159, 217, 179),
        error: Color::Rgb(215, 232, 44),
        info: Color::Rgb(107, 142, 35),
        special: Color::Rgb(56, 180, 139),
        command: Color::Rgb(187, 225, 112),
        danger: Color::Rgb(255, 230, 0),
        on_accent: Color::Rgb(23, 20, 18),
        selection_bg: Color::Rgb(58, 47, 42),
    };

    /// 呂色 Ro-iro ground under 露草色 Tsuyukusa-iro.
    ///
    /// mono dark, seed 3854 of the traditional-colour catalog.
    /// Signal roles sit 0.42 apart — close, which is what a
    /// mono palette is. Success and error differ by lightness here, not hue.
    pub const TSUYUKUSA: Theme = Theme {
        bg: Color::Rgb(13, 13, 13),
        fg: Color::Rgb(214, 214, 214),
        fg_dim: Color::Rgb(154, 154, 154),
        fg_faint: Color::Rgb(105, 105, 105),
        inactive: Color::Rgb(75, 75, 75),
        border: Color::Rgb(69, 69, 69),
        border_focused: Color::Rgb(56, 161, 219),
        accent: Color::Rgb(56, 161, 219),
        active: Color::Rgb(142, 190, 220),
        success: Color::Rgb(89, 185, 198),
        warning: Color::Rgb(142, 190, 220),
        error: Color::Rgb(131, 204, 210),
        info: Color::Rgb(0, 139, 139),
        special: Color::Rgb(0, 163, 163),
        command: Color::Rgb(137, 197, 215),
        danger: Color::Rgb(159, 217, 246),
        on_accent: Color::Rgb(13, 13, 13),
        selection_bg: Color::Rgb(67, 47, 47),
    };

    /// 薄桜鼠 Ususakura-nezumi ground under 江戸紫 Edo-murasaki.
    ///
    /// mono light, seed 41 of the traditional-colour catalog.
    /// Signal roles sit 0.37 apart — close, which is what a
    /// mono palette is. Success and error differ by lightness here, not hue.
    pub const EDO: Theme = Theme {
        bg: Color::Rgb(238, 216, 224),
        fg: Color::Rgb(40, 40, 40),
        fg_dim: Color::Rgb(99, 93, 95),
        fg_faint: Color::Rgb(147, 135, 139),
        inactive: Color::Rgb(177, 161, 167),
        border: Color::Rgb(183, 167, 172),
        border_focused: Color::Rgb(106, 63, 123),
        accent: Color::Rgb(106, 63, 123),
        active: Color::Rgb(100, 1, 37),
        success: Color::Rgb(27, 47, 111),
        warning: Color::Rgb(100, 1, 37),
        error: Color::Rgb(59, 30, 74),
        info: Color::Rgb(118, 53, 104),
        special: Color::Rgb(24, 27, 58),
        command: Color::Rgb(80, 16, 56),
        danger: Color::Rgb(37, 14, 14),
        on_accent: Color::Rgb(238, 216, 224),
        selection_bg: Color::Rgb(200, 196, 188),
    };

    /// 薄香鼠 Usukou-nezumi ground under 瑠璃紺 Ruri-kon.
    ///
    /// mono light, seed 374 of the traditional-colour catalog.
    /// Signal roles sit 0.37 apart — close, which is what a
    /// mono palette is. Success and error differ by lightness here, not hue.
    pub const RURI: Theme = Theme {
        bg: Color::Rgb(226, 221, 230),
        fg: Color::Rgb(40, 40, 38),
        fg_dim: Color::Rgb(96, 94, 96),
        fg_faint: Color::Rgb(140, 138, 142),
        inactive: Color::Rgb(168, 165, 170),
        border: Color::Rgb(174, 170, 176),
        border_focused: Color::Rgb(27, 47, 111),
        accent: Color::Rgb(27, 47, 111),
        active: Color::Rgb(100, 1, 37),
        success: Color::Rgb(59, 30, 74),
        warning: Color::Rgb(100, 1, 37),
        error: Color::Rgb(74, 46, 90),
        info: Color::Rgb(24, 27, 58),
        special: Color::Rgb(118, 53, 104),
        command: Color::Rgb(87, 24, 64),
        danger: Color::Rgb(37, 14, 14),
        on_accent: Color::Rgb(226, 221, 230),
        selection_bg: Color::Rgb(216, 207, 196),
    };

    /// 暗黒色 Ankoku-shoku ground under 草色 Kusa-iro.
    ///
    /// duo dark, seed 2754 of the traditional-colour catalog.
    pub const KUSA: Theme = Theme {
        bg: Color::Rgb(15, 15, 15),
        fg: Color::Rgb(210, 205, 208),
        fg_dim: Color::Rgb(152, 148, 150),
        fg_faint: Color::Rgb(105, 102, 104),
        inactive: Color::Rgb(75, 74, 75),
        border: Color::Rgb(70, 68, 69),
        border_focused: Color::Rgb(107, 142, 35),
        accent: Color::Rgb(107, 142, 35),
        active: Color::Rgb(254, 242, 99),
        success: Color::Rgb(136, 203, 127),
        warning: Color::Rgb(254, 242, 99),
        error: Color::Rgb(246, 177, 195),
        info: Color::Rgb(227, 92, 122),
        special: Color::Rgb(178, 143, 206),
        command: Color::Rgb(250, 210, 147),
        danger: Color::Rgb(246, 177, 195),
        on_accent: Color::Rgb(15, 15, 15),
        selection_bg: Color::Rgb(62, 46, 40),
    };

    /// 鉄黒 Tetsuguro ground under 藤色 Fuji-iro.
    ///
    /// duo dark, seed 1188 of the traditional-colour catalog.
    pub const FUJI: Theme = Theme {
        bg: Color::Rgb(23, 20, 18),
        fg: Color::Rgb(220, 217, 222),
        fg_dim: Color::Rgb(161, 158, 161),
        fg_faint: Color::Rgb(114, 111, 112),
        inactive: Color::Rgb(84, 81, 81),
        border: Color::Rgb(78, 75, 75),
        border_focused: Color::Rgb(178, 143, 206),
        accent: Color::Rgb(178, 143, 206),
        active: Color::Rgb(244, 163, 185),
        success: Color::Rgb(154, 205, 50),
        warning: Color::Rgb(244, 163, 185),
        error: Color::Rgb(254, 242, 99),
        info: Color::Rgb(107, 142, 35),
        special: Color::Rgb(227, 92, 122),
        command: Color::Rgb(249, 203, 142),
        danger: Color::Rgb(254, 242, 99),
        on_accent: Color::Rgb(23, 20, 18),
        selection_bg: Color::Rgb(40, 40, 38),
    };

    /// 胡粉色 Gofun-iro ground under 菖蒲色 Ayame-iro.
    ///
    /// duo light, seed 2577 of the traditional-colour catalog.
    /// Signal roles sit 0.34 apart — close, which is what a
    /// duo palette is. Success and error differ by lightness here, not hue.
    pub const AYAME: Theme = Theme {
        bg: Color::Rgb(255, 255, 251),
        fg: Color::Rgb(43, 43, 43),
        fg_dim: Color::Rgb(107, 107, 105),
        fg_faint: Color::Rgb(157, 157, 155),
        inactive: Color::Rgb(189, 189, 187),
        border: Color::Rgb(196, 196, 193),
        border_focused: Color::Rgb(118, 53, 104),
        accent: Color::Rgb(118, 53, 104),
        active: Color::Rgb(0, 63, 142),
        success: Color::Rgb(59, 30, 74),
        warning: Color::Rgb(0, 63, 142),
        error: Color::Rgb(27, 47, 111),
        info: Color::Rgb(80, 120, 152),
        special: Color::Rgb(139, 95, 191),
        command: Color::Rgb(14, 55, 127),
        danger: Color::Rgb(28, 47, 74),
        on_accent: Color::Rgb(255, 255, 251),
        selection_bg: Color::Rgb(200, 204, 184),
    };

    /// 薄桜 Usu-zakura ground under 菫色 Sumire-iro.
    ///
    /// duo light, seed 684 of the traditional-colour catalog.
    /// Signal roles sit 0.33 apart — close, which is what a
    /// duo palette is. Success and error differ by lightness here, not hue.
    pub const SUMIRE: Theme = Theme {
        bg: Color::Rgb(254, 229, 241),
        fg: Color::Rgb(40, 40, 38),
        fg_dim: Color::Rgb(104, 97, 99),
        fg_faint: Color::Rgb(156, 142, 148),
        inactive: Color::Rgb(188, 170, 178),
        border: Color::Rgb(194, 176, 184),
        border_focused: Color::Rgb(112, 88, 163),
        accent: Color::Rgb(112, 88, 163),
        active: Color::Rgb(59, 30, 74),
        success: Color::Rgb(61, 108, 63),
        warning: Color::Rgb(59, 30, 74),
        error: Color::Rgb(46, 92, 63),
        info: Color::Rgb(107, 110, 35),
        special: Color::Rgb(137, 91, 138),
        command: Color::Rgb(53, 61, 69),
        danger: Color::Rgb(63, 82, 41),
        on_accent: Color::Rgb(254, 229, 241),
        selection_bg: Color::Rgb(205, 211, 217),
    };

    /// 暗黒色 Ankoku-shoku ground under 青緑 Aomidori.
    ///
    /// triad dark, seed 2494 of the traditional-colour catalog.
    pub const AOMIDORI: Theme = Theme {
        bg: Color::Rgb(15, 15, 15),
        fg: Color::Rgb(216, 221, 228),
        fg_dim: Color::Rgb(156, 159, 164),
        fg_faint: Color::Rgb(107, 110, 113),
        inactive: Color::Rgb(77, 79, 81),
        border: Color::Rgb(71, 73, 75),
        border_focused: Color::Rgb(0, 139, 139),
        accent: Color::Rgb(0, 139, 139),
        active: Color::Rgb(89, 185, 198),
        success: Color::Rgb(107, 142, 35),
        warning: Color::Rgb(89, 185, 198),
        error: Color::Rgb(244, 215, 1),
        info: Color::Rgb(178, 143, 206),
        special: Color::Rgb(243, 166, 177),
        command: Color::Rgb(167, 200, 100),
        danger: Color::Rgb(244, 215, 1),
        on_accent: Color::Rgb(15, 15, 15),
        selection_bg: Color::Rgb(47, 37, 30),
    };

    /// 黒 Kuro ground under 牡丹色 Botan-iro.
    ///
    /// triad dark, seed 2571 of the traditional-colour catalog.
    pub const BOTAN: Theme = Theme {
        bg: Color::Rgb(28, 28, 28),
        fg: Color::Rgb(228, 228, 224),
        fg_dim: Color::Rgb(168, 168, 165),
        fg_faint: Color::Rgb(120, 120, 118),
        inactive: Color::Rgb(90, 90, 89),
        border: Color::Rgb(84, 84, 83),
        border_focused: Color::Rgb(244, 163, 185),
        accent: Color::Rgb(244, 163, 185),
        active: Color::Rgb(215, 232, 44),
        success: Color::Rgb(185, 140, 70),
        warning: Color::Rgb(215, 232, 44),
        error: Color::Rgb(89, 185, 198),
        info: Color::Rgb(75, 128, 234),
        special: Color::Rgb(178, 143, 206),
        command: Color::Rgb(152, 209, 121),
        danger: Color::Rgb(143, 197, 214),
        on_accent: Color::Rgb(28, 28, 28),
        selection_bg: Color::Rgb(62, 46, 40),
    };

    /// 絹色 Kinu-iro ground under 瑠璃紺 Ruri-kon.
    ///
    /// triad light, seed 631 of the traditional-colour catalog.
    pub const KINU: Theme = Theme {
        bg: Color::Rgb(234, 224, 216),
        fg: Color::Rgb(40, 48, 48),
        fg_dim: Color::Rgb(98, 101, 98),
        fg_faint: Color::Rgb(145, 143, 139),
        inactive: Color::Rgb(174, 169, 164),
        border: Color::Rgb(180, 175, 169),
        border_focused: Color::Rgb(27, 47, 111),
        accent: Color::Rgb(27, 47, 111),
        active: Color::Rgb(188, 45, 41),
        success: Color::Rgb(59, 30, 74),
        warning: Color::Rgb(188, 45, 41),
        error: Color::Rgb(111, 48, 40),
        info: Color::Rgb(62, 98, 173),
        special: Color::Rgb(106, 63, 123),
        command: Color::Rgb(150, 47, 41),
        danger: Color::Rgb(111, 48, 40),
        on_accent: Color::Rgb(234, 224, 216),
        selection_bg: Color::Rgb(200, 192, 180),
    };

    /// 卯の花色 Unohana-iro ground under 葡萄色 Budō-iro.
    ///
    /// triad light, seed 2481 of the traditional-colour catalog.
    pub const BUDO: Theme = Theme {
        bg: Color::Rgb(247, 252, 248),
        fg: Color::Rgb(40, 40, 38),
        fg_dim: Color::Rgb(102, 104, 101),
        fg_faint: Color::Rgb(152, 154, 151),
        inactive: Color::Rgb(183, 186, 183),
        border: Color::Rgb(189, 193, 189),
        border_focused: Color::Rgb(100, 1, 37),
        accent: Color::Rgb(100, 1, 37),
        active: Color::Rgb(40, 72, 96),
        success: Color::Rgb(220, 48, 35),
        warning: Color::Rgb(40, 72, 96),
        error: Color::Rgb(157, 43, 34),
        info: Color::Rgb(76, 108, 179),
        special: Color::Rgb(139, 95, 191),
        command: Color::Rgb(99, 58, 65),
        danger: Color::Rgb(139, 53, 45),
        on_accent: Color::Rgb(247, 252, 248),
        selection_bg: Color::Rgb(208, 200, 190),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Only a concrete `Rgb` can be measured. `Theme::DARK` deliberately uses
    /// the terminal's own sixteen — `Color::Reset`, `Color::Cyan` — whose real
    /// values belong to whatever the user set Ghostty to, so it is not this
    /// file's contrast to guarantee and the checks below skip it.
    fn rgb(color: Color) -> Option<[f64; 3]> {
        match color {
            Color::Rgb(r, g, b) => Some([r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0]),
            _ => None,
        }
    }

    fn luminance(c: [f64; 3]) -> f64 {
        let f = |v: f64| {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])
    }

    fn contrast(a: Color, b: Color) -> Option<f64> {
        let (a, b) = (luminance(rgb(a)?), luminance(rgb(b)?));
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        Some((hi + 0.05) / (lo + 0.05))
    }

    /// A theme missing from `ALL` is one the picker cannot show and `next` can
    /// never reach — the arms of `palette`/`label`/`detail` are exhaustive
    /// matches, so the compiler already catches a variant with no colours, but
    /// nothing but this catches a variant with no way in.
    #[test]
    fn every_theme_is_reachable_and_names_itself() {
        for theme in ThemeMode::ALL {
            assert_eq!(
                ThemeMode::from_name(theme.label()),
                Some(theme),
                "{} does not round-trip through the name the config stores",
                theme.label(),
            );
        }
        let mut seen: Vec<&str> = ThemeMode::ALL.iter().map(|t| t.label()).collect();
        seen.sort_unstable();
        let count = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), count, "two themes share a name");

        // Stepping through the list has to visit all of it and come home.
        let mut walk = ThemeMode::default();
        for _ in 0..ThemeMode::ALL.len() {
            walk = walk.next();
        }
        assert_eq!(walk, ThemeMode::default(), "next() does not wrap the list");
    }

    /// The floors the seeds were swept against. They are asserted here rather
    /// than trusted from the generator because the colours are checked-in
    /// literals now: the next person to nudge one by hand gets told.
    ///
    /// Only the generated themes are held to them. `Theme::LIGHT` predates this
    /// and does not clear 4.5:1 on five of its roles — `active` and `warning`
    /// at 3.96, `command` at 4.00, `success` at 4.27, `danger` at 4.45 — and
    /// quietly restyling someone's existing theme to satisfy a new test is not
    /// this change's business. Raising them is a small edit whenever it is
    /// wanted; until then the number is written down rather than hidden.
    #[test]
    fn every_generated_theme_keeps_its_contrast_floors() {
        for theme in ThemeMode::ALL {
            if theme.detail() == "hand-written" {
                continue;
            }
            let t = theme.palette();
            let Some(_) = rgb(t.bg) else { continue };
            let name = theme.label();
            let against_bg = |c: Color| contrast(c, t.bg);

            // Body text, then the muted tier below it.
            assert!(against_bg(t.fg).unwrap() >= 7.0, "{name}: fg on bg");
            assert!(against_bg(t.fg_dim).unwrap() >= 3.4, "{name}: fg_dim on bg");

            // Anything carrying meaning has to be readable as text.
            for (role, color) in [
                ("accent", t.accent),
                ("active", t.active),
                ("success", t.success),
                ("warning", t.warning),
                ("error", t.error),
                ("info", t.info),
                ("special", t.special),
                ("command", t.command),
                ("danger", t.danger),
            ] {
                let ratio = against_bg(color).unwrap();
                assert!(ratio >= 4.5, "{name}: {role} on bg is {ratio:.2}");
            }

            // A divider, not a rule: too little and the pane has no edge, too
            // much and the edge is louder than what it contains.
            let border = against_bg(t.border).unwrap();
            assert!(
                (1.35..=3.4).contains(&border),
                "{name}: border on bg is {border:.2}",
            );

            // A label drawn on an accent fill still has to be legible.
            let on_accent = contrast(t.on_accent, t.accent).unwrap();
            assert!(
                on_accent >= 4.5,
                "{name}: on_accent over accent is {on_accent:.2}"
            );
        }
    }

    /// Two roles the user reads as state must not be the same colour. The
    /// generator held these apart perceptually — a floor of 0.30 in OKLCH,
    /// which is what a mono palette can just afford — and this is the blunt
    /// end of that: identical is always wrong, whatever the harmony.
    ///
    /// `warning` and `active` are excluded because they are meant to match;
    /// both are the palette's yellow, in the hand-written themes too.
    #[test]
    fn signal_roles_are_never_the_same_colour() {
        for theme in ThemeMode::ALL {
            let t = theme.palette();
            let signal = [
                ("success", t.success),
                ("error", t.error),
                ("warning", t.warning),
                ("info", t.info),
                ("special", t.special),
                ("accent", t.accent),
            ];
            for (i, (role, color)) in signal.iter().enumerate() {
                for (other, other_color) in &signal[i + 1..] {
                    assert_ne!(
                        color,
                        other_color,
                        "{}: {role} and {other} are the same colour",
                        theme.label(),
                    );
                }
            }
        }
    }
}
