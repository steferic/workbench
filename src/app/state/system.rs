use crate::app::PARSER_BUFFER_ROWS;
use crate::config::user_config::UserConfig;
use crate::config::KeybindingConfig;
use crate::git::DiffStat;
use crate::models::AgentType;
use crate::pty::PtyHandle;
use crate::tui::utils::convert_vt100_cell_style;
use ratatui::style::Style;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use uuid::Uuid;

const SYNC_OUTPUT_BEGIN: &[u8] = b"\x1b[?2026h";
const SYNC_OUTPUT_END: &[u8] = b"\x1b[?2026l";
const SYNC_OUTPUT_TIMEOUT: Duration = Duration::from_millis(150);
const SYNC_OUTPUT_MAX_BYTES: usize = 2 * 1024 * 1024;

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

    /// How much swap the machine is using, as (used, total) bytes.
    ///
    /// Read straight from the kernel rather than by shelling out: this is for
    /// a heartbeat, and spawning a process once a minute to describe memory
    /// pressure would be a small joke at our own expense.
    #[cfg(target_os = "macos")]
    pub fn system_swap(&self) -> Option<(u64, u64)> {
        #[repr(C)]
        #[derive(Default)]
        struct XswUsage {
            total: u64,
            avail: u64,
            used: u64,
            pagesize: u64,
            encrypted: bool,
        }
        let mut usage = XswUsage::default();
        let mut size = std::mem::size_of::<XswUsage>();
        let name = c"vm.swapusage";
        // SAFETY: sysctlbyname fills `usage` with at most `size` bytes, and
        // the layout matches the kernel's `struct xsw_usage`.
        let ok = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                &mut usage as *mut _ as *mut libc::c_void,
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        } == 0;
        ok.then_some((usage.used, usage.total))
    }

    #[cfg(not(target_os = "macos"))]
    pub fn system_swap(&self) -> Option<(u64, u64)> {
        None
    }

    /// The kernel's own memory-pressure verdict: 1 normal, 2 warning, 4
    /// critical. This is the signal its killer acts on, which makes it the
    /// honest gauge — swap *percent* reads high on a perfectly healthy
    /// machine, because macOS only deletes empty swapfiles and idle pages
    /// stay swapped until their owner touches them. Percent said 91 while
    /// free memory sat at 53; this said 1.
    #[cfg(target_os = "macos")]
    pub fn memory_pressure_level(&self) -> Option<i32> {
        let mut level: i32 = 0;
        let mut size = std::mem::size_of::<i32>();
        let name = c"kern.memorystatus_vm_pressure_level";
        // SAFETY: sysctlbyname writes at most `size` bytes into `level`.
        let ok = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                &mut level as *mut _ as *mut libc::c_void,
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        } == 0;
        ok.then_some(level)
    }

    #[cfg(not(target_os = "macos"))]
    pub fn memory_pressure_level(&self) -> Option<i32> {
        None
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
    /// The agent conversation this session owns, if known (see `Session`).
    pub provider_session_id: Option<String>,
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

struct SynchronizedOutputBuffer {
    bytes: Vec<u8>,
    started_at: Instant,
}

/// Append-style scrollback reconstructed from screen snapshots.
///
/// Redraw-style agents repaint a fixed viewport with clear-screen/cursor-position
/// escape sequences, so replaying raw PTY bytes erases old content instead of
/// producing scrollback. This buffer keeps a conservative styled transcript from
/// visible screen snapshots.
#[derive(Clone, Debug, PartialEq)]
pub struct TranscriptSpan {
    pub text: String,
    pub style: Style,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TranscriptLine {
    text: String,
    spans: Vec<TranscriptSpan>,
}

impl TranscriptLine {
    #[cfg(test)]
    fn raw(text: String) -> Self {
        Self {
            spans: if text.is_empty() {
                Vec::new()
            } else {
                vec![TranscriptSpan {
                    text: text.clone(),
                    style: Style::default(),
                }]
            },
            text,
        }
    }

    /// Build a line from pre-styled spans (used by log-derived history, which
    /// styles by role because the log carries no ANSI of its own).
    pub fn from_styled_spans(spans: Vec<TranscriptSpan>) -> Self {
        Self::from_spans(spans)
    }

    fn from_spans(spans: Vec<TranscriptSpan>) -> Self {
        let text = spans.iter().map(|span| span.text.as_str()).collect();
        Self { text, spans }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn spans(&self) -> &[TranscriptSpan] {
        &self.spans
    }

    fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

}

/// Append-only history reconstructed from a redraw-style agent (Claude, Codex).
///
/// These agents repaint a viewport rather than emitting append-only text, so
/// naive whole-screen snapshot merging re-appends near-full frames whenever a
/// visible line changes in place — the "history repeats over and over" bug.
/// Instead we commit a line to history exactly once, when it scrolls off the top
/// of the viewport, and keep the current frame as a volatile visible tail. How
/// far the content scrolled between frames is inferred by aligning the scrolling
/// content region of successive frames (see [`align_shift`]). The displayed
/// history is `committed` ++ visible frame.
pub struct TranscriptBuffer {
    /// Lines that have scrolled off the top and are final.
    lines: VecDeque<TranscriptLine>,
    /// The current visible frame (trailing blanks trimmed), shown below `lines`.
    visible: Vec<TranscriptLine>,
    /// Full-height previous frame; its top rows become committed on scroll.
    prev_frame: Vec<TranscriptLine>,
    /// Durable history read from the agent's own session log. When present it
    /// *replaces* the frame-diff reconstruction for display: the log is the
    /// deterministic record, the frame differ only a stand-in until it loads.
    log_history: Option<Vec<TranscriptLine>>,
    max_lines: usize,
    pub generation: u64,
}

impl TranscriptBuffer {
    pub fn new(max_lines: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            visible: Vec::new(),
            prev_frame: Vec::new(),
            log_history: None,
            max_lines: max_lines.max(1),
            generation: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.history_len() + self.visible.len()
    }

    /// How much committed history there is: the session log when we have it,
    /// otherwise whatever the frame differ could prove.
    ///
    /// Deliberately a length and an indexed read rather than one `&[_]`. The
    /// fallback lives in a `VecDeque` that is pushed at the back and popped at
    /// the front, so the moment it fills it wraps its ring and its contents
    /// stop being one contiguous slice. Handing back `as_slices().0` compiled
    /// and read like history, but it was only the part before the wrap: with a
    /// full buffer still moving it reported one line where there were eight.
    /// That is scrollback disappearing exactly while an agent is mid-answer,
    /// because a buffer that is full and still rotating is what "still
    /// generating" looks like from in here.
    fn history_len(&self) -> usize {
        match self.log_history.as_deref() {
            Some(log) => log.len(),
            None => self.lines.len(),
        }
    }

    /// One line of committed history. `VecDeque::get` walks the wrap for us.
    fn history_get(&self, index: usize) -> Option<&TranscriptLine> {
        match self.log_history.as_deref() {
            Some(log) => log.get(index),
            None => self.lines.get(index),
        }
    }

    /// Install (or clear) history parsed from the agent's session log.
    pub fn set_log_history(&mut self, lines: Option<Vec<TranscriptLine>>) {
        if self.log_history.as_deref().map(<[_]>::len) != lines.as_deref().map(<[_]>::len) {
            self.generation = self.generation.wrapping_add(1);
        }
        self.log_history = lines;
    }

    pub fn is_empty(&self) -> bool {
        self.history_len() == 0 && self.visible.is_empty()
    }

    /// Index across committed history followed by the current visible frame.
    fn get(&self, index: usize) -> Option<&TranscriptLine> {
        match self.history_get(index) {
            Some(line) => Some(line),
            None => self.visible.get(index - self.history_len()),
        }
    }

    pub fn line(&self, index: usize) -> Option<&str> {
        self.get(index).map(TranscriptLine::text)
    }

    pub fn styled_line(&self, index: usize) -> Option<&TranscriptLine> {
        self.get(index)
    }

    pub fn extract_text(&self, start: (usize, usize), end: (usize, usize)) -> String {
        if self.is_empty() {
            return String::new();
        }

        let (start_row, start_col, end_row, end_col) =
            if start.0 < end.0 || (start.0 == end.0 && start.1 <= end.1) {
                (start.0, start.1, end.0, end.1)
            } else {
                (end.0, end.1, start.0, start.1)
            };

        if start_row >= self.len() {
            return String::new();
        }

        let last_row = end_row.min(self.len() - 1);
        let mut result = String::new();

        for row in start_row..=last_row {
            let Some(line) = self.line(row) else {
                continue;
            };
            let char_count = line.chars().count();
            let row_start = if row == start_row {
                start_col.min(char_count)
            } else {
                0
            };
            let row_end = if row == end_row {
                end_col.min(char_count.saturating_sub(1))
            } else {
                char_count.saturating_sub(1)
            };

            if row_start < char_count && row_start <= row_end {
                result.push_str(&line_slice(line, row_start, row_end + 1));
            }

            if row < last_row {
                result.push('\n');
            }
        }

        result
    }

    /// Ingest the current full-height frame. Redraw-style agents repaint a fixed
    /// viewport (Claude in the alternate screen, Codex inline) with a status /
    /// input region pinned at the bottom, so there is no byte-level scroll signal.
    /// Infer how far the scrolling content region moved up between the previous
    /// and current frame, and commit the lines that left the top.
    fn ingest_aligned_frame(&mut self, frame: Vec<TranscriptLine>) -> bool {
        // Commit exactly the rows that provably left the top, and nothing on a
        // guess: an unresolved frame pair contributes no history (durable
        // history comes from the agent's session log — see `crate::scrollback`).
        let commit_n = align_shift(&self.prev_frame, &frame)
            .map(|shift| shift.max(0) as usize)
            .unwrap_or(0);
        self.commit_top_and_show(commit_n, frame)
    }

    /// Commit the top `commit_n` rows of the previous frame to history (they have
    /// scrolled off), then set the current `frame` as the visible tail (trailing
    /// blanks trimmed, mirroring the live view).
    fn commit_top_and_show(&mut self, commit_n: usize, frame: Vec<TranscriptLine>) -> bool {
        let mut changed = false;

        let take = commit_n.min(self.prev_frame.len());
        if take > 0 {
            for line in self.prev_frame.iter().take(take) {
                self.lines.push_back(line.clone());
            }
            while self.lines.len() > self.max_lines {
                self.lines.pop_front();
            }
            changed = true;
        }

        let mut visible = frame.clone();
        while visible.last().map(TranscriptLine::is_empty).unwrap_or(false) {
            visible.pop();
        }
        if visible != self.visible {
            self.visible = visible;
            changed = true;
        }

        self.prev_frame = frame;
        if changed {
            self.generation = self.generation.wrapping_add(1);
        }
        changed
    }

    /// Full-height frame (one entry per screen row, trailing spaces trimmed per
    /// line). Row N here is screen row N, so the top rows align with what scrolled
    /// off.
    fn frame_from_screen(screen: &vt100::Screen) -> Vec<TranscriptLine> {
        let (rows, cols) = screen.size();
        (0..rows)
            .map(|row| snapshot_line_from_screen(screen, row, cols))
            .collect()
    }
}

fn line_slice(line: &str, start: usize, end: usize) -> String {
    line.chars().skip(start).take(end - start).collect()
}

fn snapshot_line_from_screen(screen: &vt100::Screen, row: u16, cols: u16) -> TranscriptLine {
    let mut spans: Vec<TranscriptSpan> = Vec::new();
    let mut current_text = String::with_capacity(cols as usize);
    let mut current_style = Style::default();

    for col in 0..cols {
        let Some(cell) = screen.cell(row, col) else {
            continue;
        };
        let cell_style = convert_vt100_cell_style(cell);
        if cell_style != current_style && !current_text.is_empty() {
            spans.push(TranscriptSpan {
                text: std::mem::take(&mut current_text),
                style: current_style,
            });
        }
        current_style = cell_style;

        let contents = cell.contents();
        if contents.is_empty() {
            current_text.push(' ');
        } else {
            current_text.push_str(&contents);
        }
    }

    if !current_text.is_empty() {
        spans.push(TranscriptSpan {
            text: current_text,
            style: current_style,
        });
    }

    trim_trailing_span_spaces(&mut spans);
    TranscriptLine::from_spans(spans)
}

fn trim_trailing_span_spaces(spans: &mut Vec<TranscriptSpan>) {
    while let Some(last) = spans.last_mut() {
        let trimmed_len = last.text.trim_end().len();
        last.text.truncate(trimmed_len);
        if last.text.is_empty() {
            spans.pop();
        } else {
            break;
        }
    }
}

/// How far the content moved between the previous and current frame: positive
/// = scrolled up by `n` rows (so `n` rows left the top and are final), negative
/// = pushed down (nothing left), `0` = static.
///
/// Anchors on lines that occur **exactly once in each frame with identical
/// text** — the patience-diff idea. Such a pair is unambiguous, so the offset
/// it implies is a measurement rather than a guess, and a rigid scroll makes
/// every anchor agree on one offset.
///
/// The awkward rows disqualify themselves, which is why this needs no
/// thresholds: a ticking `Cogitated for 2s`, a token counter and a draft being
/// typed never anchor because their text differs between frames, and blank rows
/// or repeated box borders never anchor because they are not unique —
/// correctly so, since a row appearing twice says nothing about position.
///
/// `None` means no anchor survived: the screen was replaced outright and
/// nothing can be *proven* to have scrolled off, so nothing is committed.
/// Durable history for those agents comes from their session log instead (see
/// [`crate::scrollback`]).
fn align_shift(prev: &[TranscriptLine], cur: &[TranscriptLine]) -> Option<isize> {
    use std::collections::BTreeMap;

    /// Text -> its only row index, for texts appearing exactly once.
    fn unique_rows(frame: &[TranscriptLine]) -> HashMap<&str, usize> {
        let mut seen: HashMap<&str, (usize, usize)> = HashMap::new();
        for (idx, line) in frame.iter().enumerate() {
            if line.is_empty() {
                continue;
            }
            let entry = seen.entry(line.text()).or_insert((0, idx));
            entry.0 += 1;
            entry.1 = idx;
        }
        seen.into_iter()
            .filter(|(_, (count, _))| *count == 1)
            .map(|(text, (_, idx))| (text, idx))
            .collect()
    }

    let prev_unique = unique_rows(prev);
    if prev_unique.is_empty() {
        return None;
    }
    let cur_unique = unique_rows(cur);

    // BTreeMap keeps the tally ordered, so ties resolve identically every run.
    let mut offsets: BTreeMap<isize, usize> = BTreeMap::new();
    for (text, prev_idx) in &prev_unique {
        if let Some(cur_idx) = cur_unique.get(text) {
            *offsets
                .entry(*prev_idx as isize - *cur_idx as isize)
                .or_insert(0) += 1;
        }
    }

    // Most-supported offset wins; equal support favours the smaller movement,
    // which commits less rather than more.
    offsets
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.abs().cmp(&a.0.abs())))
        .map(|(shift, _)| shift)
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
    /// Performance metrics for FPS monitoring
    pub perf: PerformanceMetrics,
    /// Raw PTY output bytes for replay-based scrollback
    pub raw_output_buffers: HashMap<Uuid, RawOutputBuffer>,
    /// Text transcript buffers for agents that redraw the screen instead of
    /// emitting append-only terminal output.
    pub transcript_buffers: HashMap<Uuid, TranscriptBuffer>,
    /// Cached replay lines (invalidated on new output or scroll change)
    pub replay_caches: HashMap<Uuid, ReplayCache>,
    /// Buffered terminal synchronized-update blocks (ESC[?2026h ... ESC[?2026l).
    sync_output_buffers: HashMap<Uuid, SynchronizedOutputBuffer>,
    /// Live mirror of each agent's own task list, keyed by session. Parsed
    /// from the agent's session log off-thread (see `crate::agent_tasks`).
    pub agent_tasks: HashMap<Uuid, crate::agent_tasks::TaskTracker>,
    /// Text currently being composed in each agent PTY, reconstructed from
    /// the input bytes Workbench forwards so submitted messages can be logged.
    pub prompt_capture: crate::prompt_log::PromptCapture,
    /// When each session's current process was spawned. Process-local, like
    /// the PTY handles: it anchors which store belongs to this run of the
    /// agent, so a restart cannot keep mirroring the conversation it left.
    pub session_spawned_at: HashMap<Uuid, chrono::DateTime<chrono::Utc>>,
    /// What each agent says it is doing, from its own lifecycle hooks (see
    /// `crate::agent_status`). Absent for providers without a hook contract,
    /// which keep the output-timing inference.
    pub agent_status: HashMap<Uuid, crate::agent_status::AgentStatus>,
    /// Last time the hook reports were re-read.
    pub last_status_refresh: Instant,
    /// Last time the agent task logs were re-read.
    pub last_task_refresh: Instant,
    /// A task-log refresh is running; don't start a second one.
    pub task_refresh_inflight: bool,
    /// Per session, the (log size, pane width, theme) the scrollback was
    /// parsed at, so an unchanged log is never re-read — and a resize or a
    /// dark/light switch does force a re-wrap and re-style.
    pub scrollback_state: HashMap<Uuid, (u64, u16, crate::theme::ThemeMode)>,
    pub scrollback_inflight: bool,
    pub last_scrollback_refresh: Instant,
    /// Git diff stats keyed by working directory path
    pub diff_stats: HashMap<PathBuf, DiffStat>,
    /// Last time diff stats were refreshed
    pub last_diff_refresh: Instant,
    /// User configuration loaded from ~/.config/workbench/user_config.toml
    pub user_config: UserConfig,
    /// Whether to use alternate screen mode (from CLI or config)
    pub use_alternate_screen: bool,
    /// State has unsaved changes; flushed to disk (debounced) by the main loop
    pub state_dirty: bool,
    /// Last time a state flush was started (for debouncing)
    pub last_state_save: Instant,
    /// PTY sizes need syncing to pane sizes. Handled by the main loop AFTER
    /// the next draw, because pane rects (`ui.*_area`) are only computed
    /// during render — resizing immediately would use the previous layout's
    /// dimensions and leave every PTY one resize behind.
    pub pty_resize_pending: bool,
    /// Agent-to-agent comms driver state (see `app::comms_tick`).
    pub comms: crate::app::comms_tick::CommsState,
    /// Snapshot the tailnet page reads, republished each tick, and the
    /// server keeping it alive (see `crate::remote`).
    pub remote_state: crate::remote::Shared,
    pub remote: Option<crate::remote::Remote>,
    /// Loopback-only repository map, started the first time the user opens it.
    pub canvas: Option<crate::canvas::CanvasServer>,
    /// Commands from the phone, applied on the tick by the event loop.
    pub remote_commands: Option<tokio::sync::mpsc::UnboundedReceiver<crate::remote::RemoteCommand>>,
    /// The control socket, and the commands arriving on it. A separate channel
    /// from the phone's on purpose: the phone needs a tailnet and may never
    /// start, and a script on this machine should not depend on that.
    pub control: Option<crate::control::ControlServer>,
    pub control_commands: Option<tokio::sync::mpsc::UnboundedReceiver<crate::remote::RemoteCommand>>,
    pub control_tried: bool,
    /// When the last health line was written (see `handler::health_tick`).
    pub last_health_log: Option<Instant>,
    /// What the last tick published, so this one can say what moved.
    pub control_events: crate::control::EventState,
    /// Set once we have tried to start, so a machine without Tailscale does
    /// not retry every tick.
    pub remote_tried: bool,
    /// The conversation the phone has open, which gets full history.
    pub remote_focus: Option<Uuid>,
    /// Push keypair and the devices listening (see `crate::remote::push`).
    pub push: crate::remote::Push,
    /// Dev servers found listening, refreshed on a slow timer (`crate::ports`).
    pub dev_servers: Vec<crate::ports::DevServer>,
    /// Ports already spliced to the tailnet. Forwarders are never taken down:
    /// a dev server that restarts on the same port is picked up again with no
    /// bookkeeping, and one that is gone refuses the dial exactly as it would
    /// locally.
    pub forwarded: std::collections::HashSet<u16>,
    pub last_port_scan: Option<Instant>,
    pub port_scan_inflight: bool,
    /// What each agent was doing last tick, so the phone is poked on a change
    /// rather than every tick a state persists.
    pub remote_seen: HashMap<String, String>,
    /// When each agent's current spell of work began, so a turn can be told
    /// from a flicker. An idle agent that merely repaints its screen counts as
    /// working for as long as output timing says so, and without this every
    /// repaint reads as a turn beginning and ending.
    pub remote_working_since: HashMap<String, Instant>,
    /// When each agent last finished a turn worth mentioning. Published so the
    /// service worker can say "finished" rather than guessing.
    pub remote_finished: HashMap<String, chrono::DateTime<chrono::Utc>>,
    /// That conversation as last read off disk. Agent journals reach tens of
    /// megabytes; re-parsing one every tick to find nothing new is the kind of
    /// waste that shows up as a warm laptop.
    pub remote_thread: Option<ThreadCache>,
}

/// One session's conversation, and how far into its journal we have read.
#[derive(Debug, Clone)]
pub struct ThreadCache {
    pub session: Uuid,
    /// The journal this was read from — a resumed session may open a new one.
    pub path: std::path::PathBuf,
    pub cursor: crate::remote::Cursor,
    pub messages: Vec<crate::remote::Message>,
    /// Messages ever read, including any since trimmed off the front — the
    /// name the phone uses to say how much of the conversation it holds.
    pub total: usize,
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
            perf: PerformanceMetrics::new(),
            raw_output_buffers: HashMap::new(),
            transcript_buffers: HashMap::new(),
            replay_caches: HashMap::new(),
            sync_output_buffers: HashMap::new(),
            agent_tasks: HashMap::new(),
            prompt_capture: Default::default(),
            session_spawned_at: HashMap::new(),
            agent_status: HashMap::new(),
            last_status_refresh: Instant::now(),
            last_task_refresh: Instant::now(),
            task_refresh_inflight: false,
            scrollback_state: HashMap::new(),
            scrollback_inflight: false,
            last_scrollback_refresh: Instant::now(),
            diff_stats: HashMap::new(),
            last_diff_refresh: Instant::now(),
            user_config: crate::config::user_config::load_user_config(),
            use_alternate_screen: true,
            state_dirty: false,
            last_state_save: Instant::now(),
            pty_resize_pending: false,
            comms: crate::app::comms_tick::CommsState::new(),
            remote_state: Default::default(),
            remote: None,
            canvas: None,
            remote_commands: None,
            control: None,
            control_commands: None,
            control_tried: false,
            last_health_log: None,
            control_events: Default::default(),
            remote_tried: false,
            remote_focus: None,
            remote_thread: None,
            push: Default::default(),
            dev_servers: Vec::new(),
            forwarded: Default::default(),
            last_port_scan: None,
            port_scan_inflight: false,
            remote_seen: Default::default(),
            remote_working_since: Default::default(),
            remote_finished: Default::default(),
        }
    }

    /// Create parser + raw output buffer for a new session.
    pub fn create_session_buffers(
        &mut self,
        session_id: Uuid,
        rows: u16,
        cols: u16,
        agent_type: &AgentType,
    ) {
        let parser_rows = if agent_type.is_redraw_style() {
            rows.max(1)
        } else {
            PARSER_BUFFER_ROWS
        };
        let parser = vt100::Parser::new(parser_rows, cols, self.user_config.live_scrollback_rows);
        self.output_buffers.insert(session_id, parser);
        // Called once per spawn, so this is where a restart starts over: the
        // agent may be writing to a different conversation now (codex forks a
        // new rollout on resume), and the old tracker would keep tailing the
        // one it left behind.
        self.agent_tasks.remove(&session_id);
        self.prompt_capture.reset(session_id);
        self.agent_status.remove(&session_id);
        self.session_spawned_at
            .insert(session_id, chrono::Utc::now());
        self.raw_output_buffers.insert(
            session_id,
            RawOutputBuffer::new(self.user_config.scrollback_buffer_kb * 1024),
        );
        self.transcript_buffers.remove(&session_id);
        self.sync_output_buffers.remove(&session_id);
    }

    /// Remove parser + raw output buffer + replay cache for a session
    pub fn remove_session_buffers(&mut self, session_id: &Uuid) {
        self.agent_tasks.remove(session_id);
        self.prompt_capture.reset(*session_id);
        self.agent_status.remove(session_id);
        self.session_spawned_at.remove(session_id);
        self.output_buffers.remove(session_id);
        self.raw_output_buffers.remove(session_id);
        self.transcript_buffers.remove(session_id);
        self.replay_caches.remove(session_id);
        self.sync_output_buffers.remove(session_id);
    }

    pub fn synchronized_output_chunks(&mut self, session_id: Uuid, data: &[u8]) -> Vec<Vec<u8>> {
        if data.is_empty() {
            return Vec::new();
        }

        let mut chunks = Vec::new();

        if let Some(mut buffer) = self.sync_output_buffers.remove(&session_id) {
            let should_flush = buffer.started_at.elapsed() > SYNC_OUTPUT_TIMEOUT
                || buffer.bytes.len().saturating_add(data.len()) > SYNC_OUTPUT_MAX_BYTES;

            if should_flush {
                if !buffer.bytes.is_empty() {
                    chunks.push(buffer.bytes);
                }
                chunks.extend(self.collect_synchronized_output_chunks(session_id, data));
                return chunks;
            }

            buffer.bytes.extend_from_slice(data);
            if let Some(end_pos) = find_subslice(&buffer.bytes, SYNC_OUTPUT_END) {
                let tail_start = end_pos + SYNC_OUTPUT_END.len();
                let tail = buffer.bytes.split_off(tail_start);
                if !buffer.bytes.is_empty() {
                    chunks.push(buffer.bytes);
                }
                if !tail.is_empty() {
                    chunks.extend(self.synchronized_output_chunks(session_id, &tail));
                }
            } else {
                self.sync_output_buffers.insert(session_id, buffer);
            }

            return chunks;
        }

        self.collect_synchronized_output_chunks(session_id, data)
    }

    fn collect_synchronized_output_chunks(
        &mut self,
        session_id: Uuid,
        data: &[u8],
    ) -> Vec<Vec<u8>> {
        let Some(begin_pos) = find_subslice(data, SYNC_OUTPUT_BEGIN) else {
            return vec![data.to_vec()];
        };

        let mut chunks = Vec::new();
        if begin_pos > 0 {
            chunks.push(data[..begin_pos].to_vec());
        }

        let synchronized = &data[begin_pos..];
        if let Some(end_pos) = find_subslice(synchronized, SYNC_OUTPUT_END) {
            let tail_start = end_pos + SYNC_OUTPUT_END.len();
            chunks.push(synchronized[..tail_start].to_vec());
            if tail_start < synchronized.len() {
                chunks.extend(
                    self.collect_synchronized_output_chunks(
                        session_id,
                        &synchronized[tail_start..],
                    ),
                );
            }
        } else {
            self.sync_output_buffers.insert(
                session_id,
                SynchronizedOutputBuffer {
                    bytes: synchronized.to_vec(),
                    started_at: Instant::now(),
                },
            );
        }

        chunks
    }

    pub fn update_transcript_from_screen(&mut self, session_id: Uuid) -> bool {
        let Some(parser) = self.output_buffers.get(&session_id) else {
            return false;
        };
        let frame = TranscriptBuffer::frame_from_screen(parser.screen());

        let max_lines = self.user_config.transcript_max_lines;
        self.transcript_buffers
            .entry(session_id)
            .or_insert_with(|| TranscriptBuffer::new(max_lines))
            .ingest_aligned_frame(frame)
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }

    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

impl Default for SystemState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{SystemState, TranscriptBuffer, TranscriptLine};
    use crate::app::PARSER_BUFFER_ROWS;
    use crate::models::AgentType;
    use ratatui::style::Style;
    use uuid::Uuid;

    fn snapshot(lines: &[&str]) -> Vec<TranscriptLine> {
        lines
            .iter()
            .map(|line| TranscriptLine::raw((*line).to_string()))
            .collect()
    }

    #[test]
    fn redraw_style_session_buffers_match_pane_rows() {
        let mut system = SystemState::new();
        let session_id = Uuid::new_v4();

        system.create_session_buffers(session_id, 24, 80, &AgentType::Claude);

        let size = system
            .output_buffers
            .get(&session_id)
            .unwrap()
            .screen()
            .size();
        assert_eq!(size, (24, 80));
    }

    #[test]
    fn append_style_session_buffers_preserve_live_scrollback_rows() {
        let mut system = SystemState::new();
        let session_id = Uuid::new_v4();

        system.create_session_buffers(
            session_id,
            24,
            80,
            &AgentType::Terminal("shell".to_string()),
        );

        let size = system
            .output_buffers
            .get(&session_id)
            .unwrap()
            .screen()
            .size();
        assert_eq!(size, (PARSER_BUFFER_ROWS, 80));
    }

    #[test]
    fn synchronized_output_chunks_buffer_until_end_marker() {
        let mut system = SystemState::new();
        let session_id = Uuid::new_v4();

        let first =
            system.synchronized_output_chunks(session_id, b"before\x1b[?2026hpartial frame");
        assert_eq!(first, vec![b"before".to_vec()]);

        let second = system.synchronized_output_chunks(session_id, b" complete\x1b[?2026lafter");
        assert_eq!(
            second,
            vec![
                b"\x1b[?2026hpartial frame complete\x1b[?2026l".to_vec(),
                b"after".to_vec(),
            ]
        );
    }

    #[test]
    fn synchronized_output_chunks_split_complete_frame_in_one_chunk() {
        let mut system = SystemState::new();
        let session_id = Uuid::new_v4();

        let chunks = system
            .synchronized_output_chunks(session_id, b"before\x1b[?2026hframe\x1b[?2026lafter");

        assert_eq!(
            chunks,
            vec![
                b"before".to_vec(),
                b"\x1b[?2026hframe\x1b[?2026l".to_vec(),
                b"after".to_vec(),
            ]
        );
    }

    #[test]
    fn frame_align_does_not_repeat_when_only_the_status_line_changes() {
        // Claude's alternate-screen layout: scrolling content, then a volatile
        // status/spinner line that changes every frame, then a pinned input box.
        // The content does NOT scroll here — only the spinner ticks — so nothing
        // must be committed or duplicated (the regression that produced no/blank
        // scrollback).
        let mut transcript = TranscriptBuffer::new(50);
        let frame = |spinner: &str| {
            snapshot(&["line a", "line b", "line c", "", spinner, "", "> ", "footer"])
        };
        transcript.ingest_aligned_frame(frame("thinking 1s"));
        let len1 = transcript.len();
        transcript.ingest_aligned_frame(frame("thinking 2s"));
        transcript.ingest_aligned_frame(frame("thinking 3s"));

        assert_eq!(transcript.len(), len1, "status ticks must not grow history");
        let a_count = (0..transcript.len())
            .filter(|&i| transcript.line(i) == Some("line a"))
            .count();
        assert_eq!(a_count, 1, "content must not be duplicated by status ticks");
    }

    #[test]
    fn frame_align_commits_content_above_a_changing_status_line() {
        // Content scrolls up by one while the spinner below it keeps changing.
        // The shift must still be detected (top-anchored), committing the line
        // that left the top exactly once.
        let mut transcript = TranscriptBuffer::new(50);
        transcript.ingest_aligned_frame(snapshot(&[
            "1", "2", "3", "4", "5", "", "thinking 1s", "> ",
        ]));
        transcript.ingest_aligned_frame(snapshot(&[
            "2", "3", "4", "5", "6", "", "thinking 2s", "> ",
        ]));

        assert_eq!(transcript.line(0), Some("1"));
        let ones = (0..transcript.len())
            .filter(|&i| transcript.line(i) == Some("1"))
            .count();
        assert_eq!(ones, 1);
    }

    #[test]
    fn frame_align_commits_nothing_when_frames_do_not_overlap() {
        // A screen replaced outright shares no anchor with its predecessor, so
        // nothing can be *proven* to have scrolled off. Guessing here is what
        // used to sweep a whole viewport into history on every frame; durable
        // history for these agents comes from their session log instead.
        let mut transcript = TranscriptBuffer::new(50);
        transcript.ingest_aligned_frame(snapshot(&["1", "2", "3", "4", "5", "6", "", "> "]));
        transcript.ingest_aligned_frame(snapshot(&["20", "21", "22", "23", "24", "25", "", "> "]));

        assert_eq!(transcript.lines.len(), 0, "an unprovable jump commits nothing");
        // The replacement screen is still shown live.
        assert!((0..transcript.len()).any(|i| transcript.line(i) == Some("20")));
    }

    /// The reported bug: a settled Claude screen still repaints every frame,
    /// with an elapsed-time line at the top of the content and an input box
    /// that redraws (cursor, token counter). Nothing has scrolled, so nothing
    /// may be committed — the old exact-anchor alignment found no match, called
    /// it a burst, and appended the whole viewport (input box included) on
    /// every single frame.
    #[test]
    fn settled_screen_with_volatile_status_and_input_box_never_repeats() {
        let mut transcript = TranscriptBuffer::new(5000);

        let frame = |elapsed: u32, tokens: u32, draft: &str| {
            let mut rows = vec![format!("✻ Cogitated for {elapsed}s")];
            for i in 1..=18 {
                rows.push(format!("conversation line {i} — settled content"));
            }
            rows.push(String::new());
            rows.push("╭──────────────────────────────╮".to_string());
            rows.push(format!("│ > {draft}"));
            rows.push("╰──────────────────────────────╯".to_string());
            rows.push(format!("  {tokens} tokens · esc to interrupt"));
            snapshot(&rows.iter().map(String::as_str).collect::<Vec<_>>())
        };

        transcript.ingest_aligned_frame(frame(1, 100, "_"));
        for i in 2..12 {
            transcript.ingest_aligned_frame(frame(i, 100 + i * 37, "hello_"));
        }

        // Nothing scrolled off, so nothing is final. (The live frame still
        // shows the input box — that part is the screen, not history.)
        assert_eq!(
            transcript.lines.len(),
            0,
            "a screen that only redraws must not commit history"
        );
        let repeats = (0..transcript.len())
            .filter(|&i| transcript.line(i) == Some("conversation line 7 — settled content"))
            .count();
        assert_eq!(repeats, 1, "content must appear exactly once");
        assert!(
            !transcript
                .lines
                .iter()
                .any(|l| l.text().contains("esc to interrupt") || l.text().starts_with("│ >")),
            "the input box must never be committed to history"
        );
    }

    /// Once the session log is parsed it *replaces* the frame-diff history:
    /// the log is the deterministic record, the differ only a stand-in.
    /// Committed history is held in a `VecDeque` that is pushed at the back and
    /// popped at the front, which is exactly what makes a ring buffer wrap. Once
    /// it has, its contents are two slices rather than one, and reading only the
    /// first silently drops everything after the wrap — the newest history, and
    /// only once the buffer is full and still moving, which is to say while the
    /// agent is mid-answer and you have scrolled back to read it.
    #[test]
    fn history_survives_the_deque_wrapping_round() {
        let cap = 8;
        let mut transcript = TranscriptBuffer::new(cap);

        // Scroll one line off the top per frame, well past `cap`, so the deque
        // fills and then keeps rotating.
        for n in 0..40 {
            transcript.ingest_aligned_frame(snapshot(&[
                &format!("line {n}"),
                &format!("line {}", n + 1),
                &format!("line {}", n + 2),
                "",
                "> ",
            ]));
        }

        assert_eq!(
            transcript.history_len(),
            cap,
            "history lost lines to the wrap",
        );
        // And what it holds has to be the newest `cap`, reachable by index.
        let held: Vec<String> = (0..transcript.history_len())
            .map(|i| transcript.line(i).unwrap_or_default().trim().to_string())
            .collect();
        // The newest `cap` committed lines, in order, reachable by index —
        // the wrap is a storage detail and must not show through as a gap.
        let expected: Vec<String> = (31..=38).map(|n| format!("line {n}")).collect();
        assert_eq!(held, expected, "history is not the newest lines in order");
    }

    #[test]
    fn log_history_replaces_the_frame_diff_reconstruction() {
        let mut transcript = TranscriptBuffer::new(50);
        transcript.ingest_aligned_frame(snapshot(&["a", "b", "c", "> "]));
        transcript.ingest_aligned_frame(snapshot(&["b", "c", "d", "> "]));
        assert_eq!(transcript.lines.len(), 1, "differ proved one row scrolled off");

        transcript.set_log_history(Some(vec![
            TranscriptLine::raw("logged 1".into()),
            TranscriptLine::raw("logged 2".into()),
        ]));
        assert_eq!(transcript.line(0), Some("logged 1"));
        assert_eq!(transcript.line(1), Some("logged 2"));
        // Visible frame still follows the history.
        assert_eq!(transcript.line(2), Some("b"));
        assert_eq!(transcript.len(), 2 + transcript.visible.len());

        // Clearing falls back to the differ's history.
        transcript.set_log_history(None);
        assert_eq!(transcript.line(0), Some("a"));
    }

    /// Replays the frame sequence a real redraw agent produces across a screen
    /// replacement: a settled screen that only repaints, then a burst of
    /// streaming output. Every content line must land in history exactly once,
    /// and none of the bottom chrome may come with it.
    #[test]
    fn settled_then_streaming_commits_content_once_and_no_chrome() {
        const ROWS: usize = 24;
        let chrome = |draft: String, tokens: u32| {
            vec![
                String::new(),
                ".------------.".to_string(),
                format!("| > draft{draft}"),
                String::new(),
                format!("  {tokens} tokens . esc to interrupt"),
            ]
        };
        let settled = |tick: u32| {
            let mut rows = vec![format!("* Cogitated for {tick}s")];
            for i in 2..=(ROWS - 6) {
                rows.push(format!("STATIC {i:03} settled content row"));
            }
            rows.extend(chrome(".".repeat((tick % 4) as usize), 1000 + tick * 37));
            rows
        };
        let streaming = |n: u32| {
            let mut rows: Vec<String> = (0..(ROWS - 6) as u32)
                .map(|i| format!("STREAM {:03} generated output line", n + i))
                .collect();
            rows.extend(chrome(String::new(), 2000 + n));
            rows
        };

        let mut transcript = TranscriptBuffer::new(9000);
        let mut ingest = |rows: Vec<String>| {
            transcript.ingest_aligned_frame(snapshot(
                &rows.iter().map(String::as_str).collect::<Vec<_>>(),
            ));
        };
        for tick in 1..=15 {
            ingest(settled(tick));
        }
        for n in 1..=60 {
            ingest(streaming(n));
        }

        let committed: Vec<&str> = transcript.lines.iter().map(|l| l.text()).collect();
        assert!(
            !committed
                .iter()
                .any(|l| l.contains("esc to interrupt") || l.starts_with("| > draft")),
            "bottom chrome leaked into history: {:?}",
            committed
                .iter()
                .filter(|l| l.contains("esc to interrupt") || l.starts_with("| > draft"))
                .collect::<Vec<_>>()
        );

        let mut counts: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        for line in committed
            .iter()
            .filter(|l| l.starts_with("STREAM") || l.starts_with("STATIC"))
        {
            *counts.entry(line).or_default() += 1;
        }
        let dupes: Vec<_> = counts.iter().filter(|(_, &n)| n > 1).collect();
        assert!(dupes.is_empty(), "content committed more than once: {dupes:?}");
    }

    /// Content pushed *down* (an input box growing as the user types a
    /// multi-line draft) means nothing scrolled off. An unsigned alignment
    /// cannot express that and would treat it as a burst.
    #[test]
    fn content_pushed_down_commits_nothing() {
        let mut transcript = TranscriptBuffer::new(5000);
        let body: Vec<String> = (1..=12).map(|i| format!("reply line {i}")).collect();

        let frame = |offset: usize, draft_rows: usize| {
            let mut rows: Vec<String> = std::iter::repeat_n(String::new(), offset).collect();
            rows.extend(body.iter().cloned());
            rows.push(String::new());
            for d in 0..draft_rows {
                rows.push(format!("│ > draft line {d}"));
            }
            snapshot(&rows.iter().map(String::as_str).collect::<Vec<_>>())
        };

        transcript.ingest_aligned_frame(frame(0, 1));
        // Draft grows: content slides down the screen.
        transcript.ingest_aligned_frame(frame(2, 3));
        transcript.ingest_aligned_frame(frame(4, 5));

        assert_eq!(
            transcript.lines.len(),
            0,
            "downward movement commits nothing"
        );
        let repeats = (0..transcript.len())
            .filter(|&i| transcript.line(i) == Some("reply line 5"))
            .count();
        assert_eq!(repeats, 1);
    }

    /// Streaming output still scrolls normally: each new line at the bottom
    /// pushes exactly one line into history, once.
    #[test]
    fn streaming_output_commits_each_line_exactly_once() {
        let mut transcript = TranscriptBuffer::new(5000);
        const VIEW: usize = 20;

        for last in VIEW..VIEW + 40 {
            let rows: Vec<String> = ((last + 1 - VIEW)..=last)
                .map(|i| format!("stream line {i}"))
                .collect();
            transcript.ingest_aligned_frame(snapshot(
                &rows.iter().map(String::as_str).collect::<Vec<_>>(),
            ));
        }

        for i in 1..=VIEW + 39 {
            let needle = format!("stream line {i}");
            let seen = (0..transcript.len())
                .filter(|&idx| transcript.line(idx) == Some(needle.as_str()))
                .count();
            assert_eq!(seen, 1, "{needle} should appear exactly once");
        }
    }

    #[test]
    fn frame_align_commits_nothing_on_a_sparse_burst_jump() {
        // Same contract for sparse content: no shared anchor, no commit. There
        // is deliberately no fill-ratio judgement call any more.
        let mut transcript = TranscriptBuffer::new(50);
        transcript.ingest_aligned_frame(snapshot(&[
            "para one", "", "para two", "", "para three", "", "para four", "", "para five", "",
            "", "", "", "", "> ",
        ]));
        transcript.ingest_aligned_frame(snapshot(&[
            "x1", "x2", "x3", "x4", "x5", "x6", "x7", "x8", "x9", "x10", "x11", "x12", "x13",
            "x14", "> ",
        ]));

        assert_eq!(transcript.lines.len(), 0);
    }

    #[test]
    fn frame_align_still_drops_spinner_phase_on_burst_jump() {
        let mut transcript = TranscriptBuffer::new(50);
        transcript.ingest_aligned_frame(snapshot(&[
            "· thinking…", "", "", "", "", "", "", "", "", "", "", "", "", "", "> ",
        ]));
        transcript.ingest_aligned_frame(snapshot(&[
            "y1", "y2", "y3", "y4", "y5", "y6", "y7", "y8", "y9", "y10", "y11", "y12", "y13",
            "y14", "> ",
        ]));

        assert!(
            !(0..transcript.len()).any(|i| transcript.line(i) == Some("· thinking…")),
            "spinner frames must not be committed"
        );
    }

    #[test]
    fn frame_align_commits_content_scrolled_above_a_static_footer() {
        let mut transcript = TranscriptBuffer::new(50);

        // Codex-style frames: content lines at the top, then a pinned bottom
        // ("", input box, footer) that never moves. Between frames the content
        // shifts up by one and a new line appears at the bottom of the content.
        let frame_a = snapshot(&[
            "1", "2", "3", "4", "5", "6", "7", "", "> prompt", "footer",
        ]);
        let frame_b = snapshot(&[
            "2", "3", "4", "5", "6", "7", "8", "", "> prompt", "footer",
        ]);

        transcript.ingest_aligned_frame(frame_a);
        transcript.ingest_aligned_frame(frame_b);

        // "1" scrolled off the top and is committed exactly once; the rest is the
        // current visible frame. The pinned bottom is NOT committed as history.
        assert_eq!(transcript.line(0), Some("1"));
        let ones = (0..transcript.len())
            .filter(|&i| transcript.line(i) == Some("1"))
            .count();
        assert_eq!(ones, 1, "scrolled line must not be duplicated");
    }

    #[test]
    fn frame_align_ignores_static_frames() {
        let mut transcript = TranscriptBuffer::new(50);

        // A repainted-but-unchanged frame (e.g. spinner tick with identical
        // content) must not commit or duplicate anything.
        let frame = snapshot(&["alpha", "beta", "gamma", "", "> prompt", "footer"]);
        transcript.ingest_aligned_frame(frame.clone());
        let len_after_first = transcript.len();
        transcript.ingest_aligned_frame(frame);

        assert_eq!(transcript.len(), len_after_first);
        let alphas = (0..transcript.len())
            .filter(|&i| transcript.line(i) == Some("alpha"))
            .count();
        assert_eq!(alphas, 1);
    }

    #[test]
    fn transcript_frame_preserves_cell_styles() {
        let mut parser = vt100::Parser::new(4, 20, 0);
        parser.process(b"\x1b[31mred\x1b[0m plain");

        let frame = TranscriptBuffer::frame_from_screen(parser.screen());
        // Full-height frame: one entry per screen row.
        assert_eq!(frame.len(), 4);
        assert_eq!(frame[0].text(), "red plain");
        assert_ne!(frame[0].spans()[0].style, Style::default());
        assert_eq!(frame[0].spans()[0].text, "red");
    }
}
