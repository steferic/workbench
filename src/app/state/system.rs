use crate::pty::PtyHandle;
use std::collections::HashMap;
use uuid::Uuid;

pub struct SystemState {
    /// PTY handles (not serializable)
    pub pty_handles: HashMap<Uuid, PtyHandle>,
    /// Output buffers (virtual terminal state)
    pub output_buffers: HashMap<Uuid, vt100::Parser>,
    /// Terminal size
    pub terminal_size: (u16, u16),
    /// Animation frame counter (for spinners)
    pub animation_frame: usize,
    /// Should quit flag
    pub should_quit: bool,
    /// Brown noise player state
    pub brown_noise_playing: bool,
    /// Classical radio (WRTI) player state
    pub classical_radio_playing: bool,
    /// Ocean waves sound state
    pub ocean_waves_playing: bool,
    /// Wind chimes sound state
    pub wind_chimes_playing: bool,
    /// Rainforest rain sound state
    pub rainforest_rain_playing: bool,
}

impl SystemState {
    pub fn new() -> Self {
        Self {
            pty_handles: HashMap::new(),
            output_buffers: HashMap::new(),
            terminal_size: (80, 24),
            animation_frame: 0,
            should_quit: false,
            brown_noise_playing: false,
            classical_radio_playing: false,
            ocean_waves_playing: false,
            wind_chimes_playing: false,
            rainforest_rain_playing: false,
        }
    }
}

impl Default for SystemState {
    fn default() -> Self {
        Self::new()
    }
}
