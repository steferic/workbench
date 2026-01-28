use crate::config::KeybindingConfig;
use crate::models::AgentType;
use crate::pty::PtyHandle;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Performance metrics for monitoring frame times, FPS, memory, and PTY batching
#[derive(Debug)]
pub struct PerformanceMetrics {
    /// Rolling buffer of recent frame times (for averaging)
    frame_times: VecDeque<Duration>,
    /// When the last frame started
    pub last_frame_start: Instant,
    /// Maximum samples to keep for rolling average
    max_samples: usize,
    /// Rolling buffer of PTY batch sizes (how many PTY outputs per frame)
    pty_batch_sizes: VecDeque<usize>,
    /// Current frame's PTY batch count (reset each frame)
    current_pty_batch: usize,
}

impl PerformanceMetrics {
    pub fn new() -> Self {
        Self {
            frame_times: VecDeque::with_capacity(60),
            last_frame_start: Instant::now(),
            max_samples: 60, // ~1 second of samples at 60fps
            pty_batch_sizes: VecDeque::with_capacity(60),
            current_pty_batch: 0,
        }
    }

    /// Record frame start time
    pub fn frame_start(&mut self) {
        self.last_frame_start = Instant::now();
        // Reset PTY batch counter for this frame
        self.current_pty_batch = 0;
    }

    /// Record frame end and store duration
    pub fn frame_end(&mut self) {
        let elapsed = self.last_frame_start.elapsed();
        if self.frame_times.len() >= self.max_samples {
            self.frame_times.pop_front();
        }
        self.frame_times.push_back(elapsed);

        // Store PTY batch size for this frame
        if self.pty_batch_sizes.len() >= self.max_samples {
            self.pty_batch_sizes.pop_front();
        }
        self.pty_batch_sizes.push_back(self.current_pty_batch);
    }

    /// Record a PTY output being processed
    pub fn record_pty_output(&mut self) {
        self.current_pty_batch += 1;
    }

    /// Get current FPS based on rolling average
    pub fn fps(&self) -> f64 {
        if self.frame_times.is_empty() {
            return 0.0;
        }
        let total: Duration = self.frame_times.iter().sum();
        let avg_frame_time = total.as_secs_f64() / self.frame_times.len() as f64;
        if avg_frame_time > 0.0 {
            1.0 / avg_frame_time
        } else {
            0.0
        }
    }

    /// Get average frame time in milliseconds
    pub fn frame_time_ms(&self) -> f64 {
        if self.frame_times.is_empty() {
            return 0.0;
        }
        let total: Duration = self.frame_times.iter().sum();
        (total.as_secs_f64() / self.frame_times.len() as f64) * 1000.0
    }

    /// Get average PTY batch size (outputs per frame)
    pub fn avg_pty_batch(&self) -> f64 {
        if self.pty_batch_sizes.is_empty() {
            return 0.0;
        }
        let total: usize = self.pty_batch_sizes.iter().sum();
        total as f64 / self.pty_batch_sizes.len() as f64
    }

    /// Get memory usage in MB (RSS - resident set size)
    pub fn memory_mb(&self) -> f64 {
        #[cfg(target_os = "macos")]
        {
            use std::mem::MaybeUninit;
            unsafe {
                let mut rusage = MaybeUninit::<libc::rusage>::uninit();
                if libc::getrusage(libc::RUSAGE_SELF, rusage.as_mut_ptr()) == 0 {
                    let rusage = rusage.assume_init();
                    // On macOS, ru_maxrss is in bytes
                    return rusage.ru_maxrss as f64 / (1024.0 * 1024.0);
                }
            }
            0.0
        }
        #[cfg(target_os = "linux")]
        {
            use std::mem::MaybeUninit;
            unsafe {
                let mut rusage = MaybeUninit::<libc::rusage>::uninit();
                if libc::getrusage(libc::RUSAGE_SELF, rusage.as_mut_ptr()) == 0 {
                    let rusage = rusage.assume_init();
                    // On Linux, ru_maxrss is in kilobytes
                    return rusage.ru_maxrss as f64 / 1024.0;
                }
            }
            0.0
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            0.0
        }
    }
}

impl Default for PerformanceMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Info needed to start a session (for queued startup)
#[derive(Clone)]
pub struct PendingSessionStart {
    pub session_id: Uuid,
    pub workspace_id: Uuid,
    pub workspace_path: PathBuf,
    pub agent_type: AgentType,
    pub start_command: Option<String>,
    pub dangerously_skip_permissions: bool,
}

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
    /// Queue of sessions waiting to be started (for staggered startup)
    pub startup_queue: VecDeque<PendingSessionStart>,
    /// Keybinding configuration
    pub keybindings: KeybindingConfig,
    /// Config directory to open terminal in (set by config tree, consumed by handler)
    pub pending_config_terminal: Option<PathBuf>,
    /// Performance metrics for FPS monitoring
    pub perf: PerformanceMetrics,
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
            startup_queue: VecDeque::new(),
            keybindings: KeybindingConfig::default(),
            pending_config_terminal: None,
            perf: PerformanceMetrics::new(),
        }
    }
}

impl Default for SystemState {
    fn default() -> Self {
        Self::new()
    }
}
