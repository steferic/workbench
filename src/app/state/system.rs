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

    fn same_text(&self, other: &TranscriptLine) -> bool {
        self.text == other.text
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
    max_lines: usize,
    pub generation: u64,
}

impl TranscriptBuffer {
    pub fn new(max_lines: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            visible: Vec::new(),
            prev_frame: Vec::new(),
            max_lines: max_lines.max(1),
            generation: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.lines.len() + self.visible.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.visible.is_empty()
    }

    /// Index across committed history followed by the current visible frame.
    fn get(&self, index: usize) -> Option<&TranscriptLine> {
        let committed = self.lines.len();
        if index < committed {
            self.lines.get(index)
        } else {
            self.visible.get(index - committed)
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
        // The pinned bottom (input box / footer / trailing blanks) is the longest
        // common suffix; the content region is everything above it.
        let pinned = pinned_bottom(&self.prev_frame, &frame);
        let limit = self.prev_frame.len().min(frame.len()).saturating_sub(pinned);

        let commit_n = match align_shift(&self.prev_frame, &frame, limit) {
            // Frames overlap: commit the `s` rows that scrolled off the top.
            Some(s) => s,
            // No overlap: the content jumped by more than a viewport (a fast
            // burst), so the previous content region scrolled off entirely.
            // Commit it (trailing blanks trimmed) rather than losing it — but
            // not during the sparse "thinking…" spinner phase (1-3 volatile
            // lines), whose frames never align and would spam history. Real
            // content with paragraph spacing can sit well under 50% fill, so
            // gate on an absolute minimum of content lines rather than a
            // fill ratio.
            None => {
                let mut content_end = limit;
                while content_end > 0 && self.prev_frame[content_end - 1].is_empty() {
                    content_end -= 1;
                }
                let filled = (0..content_end)
                    .filter(|&i| !self.prev_frame[i].is_empty())
                    .count();
                if content_end > 0 && (filled >= 4 || filled * 2 >= limit) {
                    content_end
                } else {
                    0
                }
            }
        };
        self.commit_top_and_show(commit_n, frame)
    }

    /// Commit the top `commit_n` rows of the previous frame to history (they have
    /// scrolled off), then set the current `frame` as the visible tail (trailing
    /// blanks trimmed, mirroring the live view).
    fn commit_top_and_show(&mut self, commit_n: usize, frame: Vec<TranscriptLine>) -> bool {
        let mut changed = false;

        if commit_n > 0 {
            let take = commit_n.min(self.prev_frame.len());
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

/// Number of rows pinned at the bottom of the viewport (codex's input box,
/// footer, and trailing blanks): the longest common suffix, compared by text, of
/// the previous and current frame.
fn pinned_bottom(prev: &[TranscriptLine], cur: &[TranscriptLine]) -> usize {
    let n = prev.len().min(cur.len());
    let mut b = 0;
    while b < n && prev[prev.len() - 1 - b].same_text(&cur[cur.len() - 1 - b]) {
        b += 1;
    }
    b
}

/// How far the scrolling content region `[0, limit)` moved up between the
/// previous and current frame.
///
/// `Some(s)` means the frames overlap: anchoring on the first run of real
/// content in `cur`, that anchor is found `s` rows lower in `prev` (so `prev`
/// scrolled up by `s`). Searching for the anchor *anywhere* in `prev` (rather
/// than requiring a top-aligned match) makes this robust to large jumps and to
/// the volatile status/spinner region below the content. `None` means no
/// overlap at all — `cur`'s content does not appear in `prev`, i.e. the content
/// advanced by more than a viewport (a fast burst) and everything in `prev`
/// scrolled off.
fn align_shift(prev: &[TranscriptLine], cur: &[TranscriptLine], limit: usize) -> Option<usize> {
    const ANCHOR: usize = 3;
    // First non-blank line of cur within the content region — the anchor start.
    let a = (0..limit).find(|&i| !cur[i].is_empty())?;
    let alen = (limit - a).min(ANCHOR);
    // Need at least one real line to anchor on.
    if (a..a + alen).all(|i| cur[i].is_empty()) {
        return None;
    }
    // Find the anchor block `cur[a..a+alen]` at row `p >= a` in prev; the
    // smallest such p gives the least (most recent) shift `s = p - a`.
    for p in a..=prev.len().saturating_sub(alen) {
        if (0..alen).all(|j| prev[p + j].same_text(&cur[a + j])) {
            return Some(p - a);
        }
    }
    None
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
    /// Last time the agent task logs were re-read.
    pub last_task_refresh: Instant,
    /// A task-log refresh is running; don't start a second one.
    pub task_refresh_inflight: bool,
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
            last_task_refresh: Instant::now(),
            task_refresh_inflight: false,
            diff_stats: HashMap::new(),
            last_diff_refresh: Instant::now(),
            user_config: crate::config::user_config::load_user_config(),
            use_alternate_screen: true,
            state_dirty: false,
            last_state_save: Instant::now(),
            pty_resize_pending: false,
            comms: crate::app::comms_tick::CommsState::new(),
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
    fn frame_align_captures_content_when_frames_do_not_overlap() {
        // Fast burst: content jumps by more than a viewport between frames, so
        // there is no overlap. The previous content region must still be
        // committed (not dropped) so history captures it instead of going blank.
        let mut transcript = TranscriptBuffer::new(50);
        transcript.ingest_aligned_frame(snapshot(&["1", "2", "3", "4", "5", "6", "", "> "]));
        // Jump to 20-25 — no overlap with 1-6.
        transcript.ingest_aligned_frame(snapshot(&["20", "21", "22", "23", "24", "25", "", "> "]));

        assert_eq!(transcript.line(0), Some("1"));
        assert!((0..transcript.len()).any(|i| transcript.line(i) == Some("6")));
        assert!((0..transcript.len()).any(|i| transcript.line(i) == Some("20")));
        let ones = (0..transcript.len())
            .filter(|&i| transcript.line(i) == Some("1"))
            .count();
        assert_eq!(ones, 1, "no duplication across the jump");
    }

    #[test]
    fn frame_align_commits_sparse_content_on_burst_jump() {
        // Real Claude output has blank lines between paragraphs, so a content
        // frame can sit well under 50% fill. A fast burst (no overlap) must
        // still commit it — only the 1-3-line spinner phase may be dropped.
        let mut transcript = TranscriptBuffer::new(50);
        // 5 non-blank lines of 14 content rows (~36% fill).
        transcript.ingest_aligned_frame(snapshot(&[
            "para one", "", "para two", "", "para three", "", "para four", "", "para five", "",
            "", "", "", "", "> ",
        ]));
        // Jump with no overlap.
        transcript.ingest_aligned_frame(snapshot(&[
            "x1", "x2", "x3", "x4", "x5", "x6", "x7", "x8", "x9", "x10", "x11", "x12", "x13",
            "x14", "> ",
        ]));

        assert!((0..transcript.len()).any(|i| transcript.line(i) == Some("para one")));
        assert!((0..transcript.len()).any(|i| transcript.line(i) == Some("para five")));
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
    fn frame_align_commits_content_scrolled_above_pinned_bottom() {
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
