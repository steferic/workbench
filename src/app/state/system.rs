use crate::app::PARSER_BUFFER_ROWS;
use crate::config::user_config::UserConfig;
use crate::config::KeybindingConfig;
use crate::git::DiffStat;
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
            // SAFETY: MaybeUninit provides a valid pointer for getrusage to write into.
            // RUSAGE_SELF is always valid. We only read the struct after confirming success.
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
            // SAFETY: Same as macOS block above — valid pointer, valid resource argument.
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
    /// If the session uses worktree isolation, spawn in this directory instead
    pub worktree_path: Option<PathBuf>,
}

/// Circular buffer storing raw PTY output bytes for replay-based scrollback
pub struct RawOutputBuffer {
    pub bytes: VecDeque<u8>,
    pub capacity: usize,
    pub generation: u64,
}

impl RawOutputBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            bytes: VecDeque::with_capacity(capacity),
            capacity,
            generation: 0,
        }
    }

    pub fn append(&mut self, data: &[u8]) {
        // Trim from front if exceeding capacity
        let total = self.bytes.len() + data.len();
        if total > self.capacity {
            let to_drain = total - self.capacity;
            if to_drain >= self.bytes.len() {
                self.bytes.clear();
                // If data itself exceeds capacity, only keep the tail
                if data.len() > self.capacity {
                    let start = data.len() - self.capacity;
                    self.bytes.extend(&data[start..]);
                } else {
                    self.bytes.extend(data);
                }
            } else {
                self.bytes.drain(..to_drain);
                self.bytes.extend(data);
            }
        } else {
            self.bytes.extend(data);
        }
        self.generation = self.generation.wrapping_add(1);
    }
}

/// Cached replay parser to avoid re-replaying raw bytes every frame.
/// The parser is expensive to create (feeds all raw bytes through vt100),
/// but rendering visible lines from it each frame is cheap.
pub struct ReplayCache {
    pub generation: u64,
    pub cols: u16,
    pub parser: vt100::Parser,
    pub content_length: usize,
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
    /// Raw PTY output bytes for replay-based scrollback
    pub raw_output_buffers: HashMap<Uuid, RawOutputBuffer>,
    /// Cached replay lines (invalidated on new output or scroll change)
    pub replay_caches: HashMap<Uuid, ReplayCache>,
    /// Git diff stats keyed by working directory path
    pub diff_stats: HashMap<PathBuf, DiffStat>,
    /// Last time diff stats were refreshed
    pub last_diff_refresh: Instant,
    /// Play a sound when an agent finishes (goes idle)
    pub agent_done_sound_enabled: bool,
    /// Last time the agent-done sound was played (for debouncing)
    pub last_agent_done_sound: Instant,
    /// User configuration loaded from ~/.config/workbench/user_config.toml
    pub user_config: UserConfig,
    /// Whether to use alternate screen mode (from CLI or config)
    pub use_alternate_screen: bool,
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
            raw_output_buffers: HashMap::new(),
            replay_caches: HashMap::new(),
            diff_stats: HashMap::new(),
            last_diff_refresh: Instant::now(),
            agent_done_sound_enabled: true,
            last_agent_done_sound: Instant::now(),
            user_config: crate::config::user_config::load_user_config(),
            use_alternate_screen: true,
        }
    }

    /// Create parser + raw output buffer for a new session.
    pub fn create_session_buffers(&mut self, session_id: Uuid, cols: u16) {
        let parser = vt100::Parser::new(
            PARSER_BUFFER_ROWS,
            cols,
            self.user_config.live_scrollback_rows,
        );
        self.output_buffers.insert(session_id, parser);
        self.raw_output_buffers.insert(
            session_id,
            RawOutputBuffer::new(self.user_config.scrollback_buffer_kb * 1024),
        );
    }

    /// Remove parser + raw output buffer + replay cache for a session
    pub fn remove_session_buffers(&mut self, session_id: &Uuid) {
        self.output_buffers.remove(session_id);
        self.raw_output_buffers.remove(session_id);
        self.replay_caches.remove(session_id);
    }
}

impl Default for SystemState {
    fn default() -> Self {
        Self::new()
    }
}
