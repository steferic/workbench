use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::Frame;
use std::time::Instant;
use tachyonfx::{fx, Effect, EffectRenderer, Interpolation, Motion};

/// Duration for animations in milliseconds
const EFFECT_DURATION_MS: u32 = 400;

/// Delay between each pane's animation in milliseconds
const STAGGER_DELAY_MS: u32 = 80;

/// Background color for slide animation
const SLIDE_BG: Color = Color::from_u32(0x1D2021);

/// Calculate inner area of a block (excluding 1-pixel border on all sides)
fn inner_area(area: Rect) -> Rect {
    if area.width < 2 || area.height < 2 {
        return area;
    }
    Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

/// Pane identifiers for tracking effects
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneId {
    Workspace,
    Session,
    Todos,
    Utilities,
    Output,
    Pinned(usize),
}

/// Manages visual effects for the TUI
pub struct EffectsManager {
    /// Active effects keyed by pane
    effects: Vec<(PaneId, Effect, Rect)>,
    /// Last frame time for calculating elapsed duration
    last_frame: Instant,
    /// Whether initial startup animation has been triggered
    startup_triggered: bool,
}

impl EffectsManager {
    pub fn new() -> Self {
        Self {
            effects: Vec::new(),
            last_frame: Instant::now(),
            startup_triggered: false,
        }
    }

    /// Create a slide-in effect for a pane with optional delay
    fn create_pane_effect(&self, area: Rect, delay_ms: u32) -> Effect {
        let timer = (EFFECT_DURATION_MS, Interpolation::Linear);
        let slide = fx::slide_in(Motion::UpToDown, 10, 0, SLIDE_BG, timer)
            .with_area(area);

        if delay_ms > 0 {
            fx::sequence(&[
                fx::sleep(delay_ms),
                slide,
            ])
        } else {
            slide
        }
    }

    /// Trigger startup animations for all panes
    pub fn trigger_startup(&mut self, areas: &StartupAreas) {
        if self.startup_triggered {
            return;
        }
        self.startup_triggered = true;

        // Clear any existing effects
        self.effects.clear();

        let mut delay = 0;

        // Banner and status bar are excluded from animation

        // Left panes evolve, staggered top to bottom (use inner area to avoid borders)
        let ws_inner = inner_area(areas.workspace);
        self.effects.push((
            PaneId::Workspace,
            self.create_pane_effect(ws_inner, delay),
            ws_inner,
        ));
        delay += STAGGER_DELAY_MS;

        let session_inner = inner_area(areas.session);
        self.effects.push((
            PaneId::Session,
            self.create_pane_effect(session_inner, delay),
            session_inner,
        ));
        delay += STAGGER_DELAY_MS;

        let todos_inner = inner_area(areas.todos);
        self.effects.push((
            PaneId::Todos,
            self.create_pane_effect(todos_inner, delay),
            todos_inner,
        ));
        delay += STAGGER_DELAY_MS;

        let utils_inner = inner_area(areas.utilities);
        self.effects.push((
            PaneId::Utilities,
            self.create_pane_effect(utils_inner, delay),
            utils_inner,
        ));
        delay += STAGGER_DELAY_MS;

        // Right pane (output) evolves (use inner area)
        let output_inner = inner_area(areas.output);
        self.effects.push((
            PaneId::Output,
            self.create_pane_effect(output_inner, delay),
            output_inner,
        ));
        delay += STAGGER_DELAY_MS;

        // Pinned panes evolve, staggered (use inner area)
        for (idx, area) in areas.pinned.iter().enumerate() {
            let pinned_inner = inner_area(*area);
            self.effects.push((
                PaneId::Pinned(idx),
                self.create_pane_effect(pinned_inner, delay),
                pinned_inner,
            ));
            delay += STAGGER_DELAY_MS;
        }

        // Status bar is excluded from animation (along with banner)
        let _ = delay; // suppress unused variable warning

        self.last_frame = Instant::now();
    }

    /// Process and render all active effects on the frame
    pub fn process(&mut self, frame: &mut Frame) {
        let now = Instant::now();
        let elapsed_std = now.duration_since(self.last_frame);
        self.last_frame = now;

        // Convert std::time::Duration to tachyonfx Duration (milliseconds)
        let elapsed_ms = elapsed_std.as_millis() as u32;
        let elapsed = tachyonfx::Duration::from_millis(elapsed_ms);

        // Process each effect and remove completed ones
        self.effects.retain_mut(|(_, effect, area)| {
            frame.render_effect(effect, *area, elapsed);
            effect.running()
        });
    }

    /// Check if any effects are currently active
    pub fn has_active_effects(&self) -> bool {
        !self.effects.is_empty()
    }

    /// Check if startup animation is complete
    pub fn startup_complete(&self) -> bool {
        self.startup_triggered && self.effects.is_empty()
    }
}

impl Default for EffectsManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Areas for startup animation
#[derive(Debug, Clone, Default)]
pub struct StartupAreas {
    pub workspace: Rect,
    pub session: Rect,
    pub todos: Rect,
    pub utilities: Rect,
    pub output: Rect,
    pub pinned: Vec<Rect>,
}
