use anyhow::{Context, Result};
use portable_pty::{
    native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize, PtySystem,
};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;
use tokio::sync::mpsc;
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::io::{FromRawFd, RawFd};

use crate::app::Action;
use crate::models::AgentType;

fn report_session_exited(pty_tx: &mpsc::Sender<Action>, session_id: Uuid, exit_code: i32) {
    if let Err(err) = pty_tx.blocking_send(Action::SessionExited(session_id, exit_code)) {
        crate::logger::warn(format!(
            "failed to report session {session_id} exit status: {err}"
        ));
    }
}

pub struct PtyHandle {
    pub master: Box<dyn MasterPty + Send>,
    pub child_killer: Box<dyn ChildKiller + Send + Sync>,
    pub process_id: Option<u32>,
    pub writer: Box<dyn Write + Send>,
    /// Set once the child is known dead (exit reported) or a kill was already
    /// issued; Drop then skips its safety-net kill so a recycled pid can't be
    /// signalled by mistake.
    cleanup_done: bool,
}

/// Safety net: a handle dropped without an explicit kill (session deletion,
/// handle-map churn) still takes its process group down, so agent processes
/// can't outlive their session.
impl Drop for PtyHandle {
    fn drop(&mut self) {
        if self.cleanup_done {
            return;
        }
        if let Err(err) = self.kill_process_group() {
            crate::logger::warn(format!("failed to kill PTY process group on drop: {err}"));
        }
    }
}

impl PtyHandle {
    pub fn send_input(&mut self, data: &[u8]) -> Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    /// Record that the child already exited on its own, so Drop won't send a
    /// kill to a process group that may no longer be ours.
    pub fn mark_exited(&mut self) {
        self.cleanup_done = true;
    }

    pub fn kill(&mut self) -> Result<()> {
        self.cleanup_done = true;
        self.kill_process_group()
    }

    pub fn interrupt_then_kill(&mut self, grace: Duration) -> Result<()> {
        self.cleanup_done = true;
        #[cfg(unix)]
        {
            if let Some(pgid) = self.process_group_id() {
                // Send SIGINT to the process group for a graceful shutdown.
                if self.signal_process_group(pgid, libc::SIGINT).is_err() {
                    self.child_killer.kill()?;
                    return Ok(());
                }

                let start = Instant::now();
                while start.elapsed() < grace {
                    if !self.process_group_alive(pgid) {
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }

                // Escalate to SIGKILL if the group is still alive.
                if let Err(err) = self.signal_process_group(pgid, libc::SIGKILL) {
                    crate::logger::warn(format!("failed to kill PTY process group: {err}"));
                }
                return Ok(());
            }
        }

        self.child_killer.kill()?;
        Ok(())
    }

    #[cfg(unix)]
    fn process_group_id(&self) -> Option<libc::pid_t> {
        self.process_id
            .filter(|pid| *pid > 0)
            .map(|pid| pid as libc::pid_t)
    }

    #[cfg(unix)]
    fn signal_process_group(&self, pgid: libc::pid_t, signal: i32) -> Result<()> {
        // SAFETY: pgid is validated > 0 by process_group_id(). Negating it
        // targets the entire process group. kill() with a valid signal is safe.
        let result = unsafe { libc::kill(-pgid, signal) };
        if result == -1 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ESRCH) {
                return Ok(());
            }
            return Err(err.into());
        }
        Ok(())
    }

    #[cfg(unix)]
    fn process_group_alive(&self, pgid: libc::pid_t) -> bool {
        // SAFETY: signal 0 is a null signal used only to check process existence.
        // pgid is validated > 0 by process_group_id().
        let result = unsafe { libc::kill(-pgid, 0) };
        if result == 0 {
            return true;
        }
        let err = std::io::Error::last_os_error();
        err.raw_os_error() != Some(libc::ESRCH)
    }

    #[cfg(not(unix))]
    fn kill_process_group(&mut self) -> Result<()> {
        self.child_killer.kill()?;
        Ok(())
    }

    #[cfg(unix)]
    fn kill_process_group(&mut self) -> Result<()> {
        if let Some(pgid) = self.process_group_id() {
            // portable-pty uses setsid() on spawn, so pid == pgid for the child.
            if self.signal_process_group(pgid, libc::SIGKILL).is_ok() {
                return Ok(());
            }
        }

        self.child_killer.kill()?;
        Ok(())
    }
}

/// How a spawned agent attaches to conversation history.
///
/// `MostRecent` is the fallback the providers give us (`claude --continue`,
/// `codex resume --last`) and it is scoped to the *directory*, not the
/// session — several agents in one project all land on the same conversation.
/// Prefer `Conversation` whenever the session's own id is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resume {
    /// Start a fresh conversation.
    No,
    /// Resume this exact provider conversation.
    Conversation(String),
    /// Resume whatever this directory used last (id not known yet).
    MostRecent,
}

/// Configuration for spawning a PTY session.
pub struct SessionSpawnConfig<'a> {
    pub session_id: Uuid,
    pub workspace_id: Uuid,
    pub agent_type: AgentType,
    pub working_dir: &'a Path,
    pub rows: u16,
    pub cols: u16,
    pub pty_tx: mpsc::Sender<Action>,
    pub resume: Resume,
    pub dangerously_skip_permissions: bool,
    pub use_alternate_screen: bool,
}

/// The provider-specific CLI arguments for a session.
///
/// Split out from spawning so the resume contract is testable: getting this
/// wrong silently merges two agents' histories rather than failing loudly.
/// `claude_id_free` is whether Claude has no log under our session uuid yet —
/// it refuses `--session-id` for an id it has already written.
fn agent_args(
    agent_type: &AgentType,
    session_id: Uuid,
    resume: &Resume,
    dangerously_skip_permissions: bool,
    claude_id_free: bool,
) -> Vec<String> {
    if agent_type.is_terminal() {
        return Vec::new();
    }
    let mut args: Vec<String> = Vec::new();
    // Dispatch on the command, not the enum, so an agent added through
    // `user_config.toml` behaves exactly like a built-in one.
    match agent_type.command() {
        "claude" => {
            if dangerously_skip_permissions {
                args.push("--dangerously-skip-permissions".into());
            }
            match resume {
                // This session's own conversation.
                Resume::Conversation(id) => {
                    args.push("--resume".into());
                    args.push(id.clone());
                }
                // Directory-scoped — only until we learn this session's id.
                Resume::MostRecent => args.push("--continue".into()),
                Resume::No => {
                    // Pin Claude's session id to ours so its log is at a path
                    // we can predict (`agent_tasks::files`) and the
                    // conversation is addressable on restart. Restarting a
                    // stopped session reuses the uuid, and Claude refuses an id
                    // it has already written a log for — then let it pick its
                    // own and match the log by cwd instead.
                    if claude_id_free {
                        args.push("--session-id".into());
                        args.push(session_id.to_string());
                    }
                }
            }
        }
        "codex" => {
            // Codex resumes via a subcommand, so it has to come first.
            match resume {
                Resume::No => {}
                Resume::Conversation(id) => {
                    args.push("resume".into());
                    args.push(id.clone());
                }
                Resume::MostRecent => {
                    args.push("resume".into());
                    args.push("--last".into());
                }
            }
            if dangerously_skip_permissions {
                args.push("--dangerously-bypass-approvals-and-sandbox".into());
            }
        }
        "hermes" => {
            if dangerously_skip_permissions {
                args.push("--yolo".into());
            }
            match resume {
                Resume::Conversation(id) => {
                    args.push("--resume".into());
                    args.push(id.clone());
                }
                Resume::MostRecent => args.push("--continue".into()),
                Resume::No => {}
            }
        }
        "gemini" => {
            if dangerously_skip_permissions {
                args.push("--yolo".into());
            }
            if resume != &Resume::No {
                args.push("--resume".into());
            }
        }
        "grok" => {
            if dangerously_skip_permissions {
                args.push("--permission-mode".into());
                args.push("full".into());
            }
            if resume != &Resume::No {
                args.push("--continue".into());
            }
        }
        // Anything else (including opencode, whose resume flags are not yet
        // verified) runs bare. Its task list is still mirrored — that reads the
        // agent's own store and needs no cooperation from the command line.
        _ => {}
    }
    args
}

pub struct PtyManager {
    pty_system: Box<dyn PtySystem>,
}

impl PtyManager {
    pub fn new() -> Self {
        Self {
            pty_system: native_pty_system(),
        }
    }

    pub fn spawn_session(&self, config: SessionSpawnConfig) -> Result<PtyHandle> {
        let SessionSpawnConfig {
            session_id,
            workspace_id,
            agent_type,
            working_dir,
            rows,
            cols,
            pty_tx,
            resume,
            dangerously_skip_permissions,
            use_alternate_screen,
        } = config;

        // Create PTY pair
        let pair = self
            .pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to open PTY")?;

        // Build command based on agent type
        let mut cmd = if agent_type.is_terminal() {
            // For terminals, use $SHELL (Unix) or $COMSPEC (Windows) with platform fallbacks
            let shell = if cfg!(windows) {
                std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
            } else {
                std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
            };
            CommandBuilder::new(shell)
        } else {
            CommandBuilder::new(agent_type.command())
        };
        cmd.cwd(working_dir);

        // Add agent-specific flags (not for terminals)
        let claude_id_free = agent_type.command() == "claude"
            && resume == Resume::No
            && crate::agent_tasks::claude_log_for_session(&session_id.to_string()).is_none();
        for arg in agent_args(
            &agent_type,
            session_id,
            &resume,
            dangerously_skip_permissions,
            claude_id_free,
        ) {
            cmd.arg(arg);
        }

        // Set TERM for proper terminal emulation
        // Use simpler vt100 for Codex to reduce cursor positioning complexity
        if agent_type.is_codex_like() {
            cmd.env("TERM", "vt100");
        } else {
            cmd.env("TERM", "xterm-256color");
        }

        // Self-identity for agent-to-agent comms: the `workbench` CLI reads
        // these to know which session is calling and which workspace's
        // roster/inbox to use.
        cmd.env(crate::comms::ENV_SESSION, &session_id.to_string()[..8]);
        cmd.env(crate::comms::ENV_WORKSPACE, workspace_id.to_string());

        // Do NOT export LINES/COLUMNS. Exported, they override the live
        // TIOCGWINSZ size in Ink (Claude) and other TUI frameworks, freezing
        // the child's layout at its spawn-time dimensions — the pane then
        // renders clipped or mis-wrapped after any pane/window resize, and
        // no amount of SIGWINCH fixes it. The PTY itself always carries the
        // correct size (set at open, updated by resize()).

        // Spawn the process
        let child = pair
            .slave
            .spawn_command(cmd)
            .context("Failed to spawn agent process")?;
        let child_killer = child.clone_killer();
        let process_id = child.process_id();

        // Get reader and writer
        let mut reader = pair
            .master
            .try_clone_reader()
            .context("Failed to clone PTY reader")?;
        let writer = pair
            .master
            .take_writer()
            .context("Failed to take PTY writer")?;

        // Get raw fd for immediate DSR response (Unix only)
        #[cfg(unix)]
        let master_fd = pair.master.as_raw_fd();

        // Spawn async task to read PTY output
        let pty_tx = pty_tx.clone();
        let sid = session_id;
        let pty_rows = rows;
        let strip_alt_screen = agent_type.is_redraw_style() || !use_alternate_screen;
        std::thread::spawn(move || {
            #[cfg(unix)]
            Self::read_pty_output_with_dsr(
                sid,
                &mut reader,
                pty_tx,
                master_fd,
                pty_rows,
                child,
                strip_alt_screen,
            );
            #[cfg(not(unix))]
            Self::read_pty_output(sid, &mut reader, pty_tx, child, strip_alt_screen);
        });

        Ok(PtyHandle {
            master: pair.master,
            child_killer,
            process_id,
            writer,
            cleanup_done: false,
        })
    }

    /// Read PTY output with immediate DSR response (Unix only)
    #[cfg(unix)]
    fn read_pty_output_with_dsr(
        session_id: Uuid,
        reader: &mut Box<dyn Read + Send>,
        pty_tx: mpsc::Sender<Action>,
        master_fd: Option<RawFd>,
        pty_rows: u16,
        mut child: Box<dyn Child + Send + Sync>,
        strip_alt_screen: bool,
    ) {
        // Responses - use simple VT102 identification
        // Primary DA: VT102 (simpler than VT100 with AVO)
        const DA_RESPONSE: &[u8] = b"\x1b[?6c";
        // Secondary DA: VT102 version 1.0 (>0;0;0c format: terminal;firmware;keyboard)
        const DA2_RESPONSE: &[u8] = b"\x1b[>0;0;0c";

        // Track cursor position by parsing escape sequences
        // Default to bottom of screen where input typically is
        let mut cursor_row: u16 = pty_rows.max(1);
        let mut cursor_col: u16 = 1;

        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    // EOF - process exited; wait for real exit status
                    let exit_code = match child.wait() {
                        Ok(status) => status.exit_code() as i32,
                        Err(_e) => {
                            // Don't use eprintln! in TUI - it corrupts the display
                            1
                        }
                    };
                    report_session_exited(&pty_tx, session_id, exit_code);
                    break;
                }
                Ok(n) => {
                    let mut data = buf[..n].to_vec();

                    // Update cursor position by parsing escape sequences in the data
                    Self::track_cursor_position(&data, &mut cursor_row, &mut cursor_col, pty_rows);

                    // Handle terminal queries (single-pass detection)
                    if let Some(fd) = master_fd {
                        let (has_dsr, has_da, has_da2) = Self::detect_terminal_queries(&data);

                        if has_dsr || has_da || has_da2 {
                            // SAFETY: fd is a valid file descriptor from the PTY master.
                            // Wrapped in ManuallyDrop to avoid closing the fd on drop.
                            let mut file = std::mem::ManuallyDrop::new(unsafe {
                                std::fs::File::from_raw_fd(fd)
                            });

                            if has_dsr {
                                let dsr_response = format!("\x1b[{};{}R", cursor_row, cursor_col);
                                if let Err(err) = file.write_all(dsr_response.as_bytes()) {
                                    crate::logger::warn(format!(
                                        "failed to write cursor-position response: {err}"
                                    ));
                                }
                            }

                            if has_da {
                                if let Err(err) = file.write_all(DA_RESPONSE) {
                                    crate::logger::warn(format!(
                                        "failed to write device-attributes response: {err}"
                                    ));
                                }
                            }

                            if has_da2 {
                                if let Err(err) = file.write_all(DA2_RESPONSE) {
                                    crate::logger::warn(format!(
                                        "failed to write secondary device-attributes response: {err}"
                                    ));
                                }
                            }

                            if let Err(err) = file.flush() {
                                crate::logger::warn(format!(
                                    "failed to flush terminal query response: {err}"
                                ));
                            }
                            data = Self::strip_terminal_queries(&data);
                        }
                    }

                    // Strip alternate screen sequences for inline-mode agents (e.g. Codex)
                    if strip_alt_screen && !data.is_empty() && Self::has_alt_screen_sequences(&data)
                    {
                        data = Self::strip_alt_screen_sequences(&data);
                    }

                    if !data.is_empty() {
                        if let Err(err) = pty_tx.blocking_send(Action::PtyOutput(session_id, data))
                        {
                            crate::logger::warn(format!(
                                "failed to report PTY output for session {session_id}: {err}"
                            ));
                            break;
                        }
                    }
                }
                Err(_e) => {
                    // Don't use eprintln! in TUI - it corrupts the display
                    report_session_exited(&pty_tx, session_id, 1);
                    break;
                }
            }
        }
    }

    /// Track cursor position by parsing escape sequences
    #[cfg(unix)]
    fn track_cursor_position(data: &[u8], row: &mut u16, col: &mut u16, max_rows: u16) {
        let mut i = 0;
        while i < data.len() {
            if data[i] == 0x1b && i + 1 < data.len() && data[i + 1] == b'[' {
                // Found CSI sequence, parse it
                let start = i + 2;
                let mut end = start;

                // Find the end of the sequence (letter character)
                while end < data.len() && (data[end].is_ascii_digit() || data[end] == b';') {
                    end += 1;
                }

                if end < data.len() {
                    let params = &data[start..end];
                    let cmd = data[end];

                    match cmd {
                        // CUP - Cursor Position (ESC[row;colH or ESC[row;colf)
                        b'H' | b'f' => {
                            let (r, c) = Self::parse_two_params(params);
                            *row = r.max(1);
                            *col = c.max(1);
                        }
                        // CUU - Cursor Up (ESC[nA)
                        b'A' => {
                            let n = Self::parse_one_param(params).max(1);
                            *row = row.saturating_sub(n).max(1);
                        }
                        // CUD - Cursor Down (ESC[nB)
                        b'B' => {
                            let n = Self::parse_one_param(params).max(1);
                            *row = (*row + n).min(max_rows);
                        }
                        // CUF - Cursor Forward (ESC[nC)
                        b'C' => {
                            let n = Self::parse_one_param(params).max(1);
                            *col += n;
                        }
                        // CUB - Cursor Backward (ESC[nD)
                        b'D' => {
                            let n = Self::parse_one_param(params).max(1);
                            *col = col.saturating_sub(n).max(1);
                        }
                        // CNL - Cursor Next Line (ESC[nE)
                        b'E' => {
                            let n = Self::parse_one_param(params).max(1);
                            *row = (*row + n).min(max_rows);
                            *col = 1;
                        }
                        // CPL - Cursor Previous Line (ESC[nF)
                        b'F' => {
                            let n = Self::parse_one_param(params).max(1);
                            *row = row.saturating_sub(n).max(1);
                            *col = 1;
                        }
                        // CHA - Cursor Horizontal Absolute (ESC[nG)
                        b'G' => {
                            *col = Self::parse_one_param(params).max(1);
                        }
                        // VPA - Vertical Position Absolute (ESC[nd)
                        b'd' => {
                            *row = Self::parse_one_param(params).max(1);
                        }
                        _ => {}
                    }
                    i = end + 1;
                    continue;
                }
            } else if data[i] == b'\r' {
                // Carriage return
                *col = 1;
            } else if data[i] == b'\n' {
                // Newline
                *row = (*row + 1).min(max_rows);
            } else if data[i] >= 0x20 && data[i] < 0x7f {
                // Printable character advances cursor
                *col += 1;
            }
            i += 1;
        }
    }

    /// Parse a single numeric parameter from CSI sequence
    #[cfg(unix)]
    fn parse_one_param(params: &[u8]) -> u16 {
        if params.is_empty() {
            return 1;
        }
        std::str::from_utf8(params)
            .ok()
            .and_then(|s| s.split(';').next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(1)
    }

    /// Parse two numeric parameters from CSI sequence (row;col format)
    #[cfg(unix)]
    fn parse_two_params(params: &[u8]) -> (u16, u16) {
        if params.is_empty() {
            return (1, 1);
        }
        let s = match std::str::from_utf8(params) {
            Ok(s) => s,
            Err(_) => return (1, 1),
        };
        let mut parts = s.split(';');
        let first = parts.next().and_then(|p| p.parse().ok()).unwrap_or(1);
        let second = parts.next().and_then(|p| p.parse().ok()).unwrap_or(1);
        (first, second)
    }

    /// Detect which terminal queries are present in the data (single pass).
    /// Returns (has_dsr, has_da, has_da2).
    #[cfg(unix)]
    fn detect_terminal_queries(data: &[u8]) -> (bool, bool, bool) {
        let mut has_dsr = false;
        let mut has_da = false;
        let mut has_da2 = false;

        let mut i = 0;
        while i + 2 < data.len() {
            if data[i] == 0x1b && data[i + 1] == b'[' {
                let rest = &data[i + 2..];
                if rest.starts_with(b"6n") {
                    has_dsr = true;
                    i += 4;
                } else if rest.starts_with(b">0c") {
                    has_da2 = true;
                    i += 5;
                } else if rest.starts_with(b">c") {
                    has_da2 = true;
                    i += 4;
                } else if rest.starts_with(b"0c") {
                    has_da = true;
                    i += 4;
                } else if !rest.is_empty() && rest[0] == b'c' {
                    has_da = true;
                    i += 3;
                } else {
                    i += 2;
                }
            } else {
                i += 1;
            }
        }

        (has_dsr, has_da, has_da2)
    }

    /// Check if data contains alternate screen escape sequences
    fn has_alt_screen_sequences(data: &[u8]) -> bool {
        let mut i = 0;
        while i + 4 < data.len() {
            if data[i] == 0x1b && data[i + 1] == b'[' && data[i + 2] == b'?' {
                let rest = &data[i + 3..];
                if rest.starts_with(b"1049h")
                    || rest.starts_with(b"1049l")
                    || rest.starts_with(b"1047h")
                    || rest.starts_with(b"1047l")
                    || rest.starts_with(b"47h")
                    || rest.starts_with(b"47l")
                {
                    return true;
                }
            }
            i += 1;
        }
        false
    }

    /// Strip alternate screen escape sequences from data.
    /// These are stripped at the PTY reader level so the live parser never enters
    /// alternate screen mode (which disables scrollback entirely).
    fn strip_alt_screen_sequences(data: &[u8]) -> Vec<u8> {
        let mut result = Vec::with_capacity(data.len());
        let mut i = 0;
        while i < data.len() {
            if data[i] == 0x1b && i + 4 < data.len() && data[i + 1] == b'[' && data[i + 2] == b'?' {
                let rest = &data[i + 3..];
                if rest.starts_with(b"1049h") {
                    i += 8;
                    continue;
                }
                if rest.starts_with(b"1049l") {
                    i += 8;
                    continue;
                }
                if rest.starts_with(b"1047h") {
                    i += 8;
                    continue;
                }
                if rest.starts_with(b"1047l") {
                    i += 8;
                    continue;
                }
                if rest.starts_with(b"47h") {
                    i += 6;
                    continue;
                }
                if rest.starts_with(b"47l") {
                    i += 6;
                    continue;
                }
            }
            result.push(data[i]);
            i += 1;
        }
        result
    }

    /// Strip terminal query sequences from data (single pass with ESC early-exit)
    #[cfg(unix)]
    fn strip_terminal_queries(data: &[u8]) -> Vec<u8> {
        let mut result = Vec::with_capacity(data.len());
        let mut i = 0;
        while i < data.len() {
            if data[i] == 0x1b && i + 2 < data.len() && data[i + 1] == b'[' {
                let rest = &data[i + 2..];
                if rest.starts_with(b"6n") {
                    i += 4;
                    continue;
                }
                if rest.starts_with(b">0c") {
                    i += 5;
                    continue;
                }
                if rest.starts_with(b">c") {
                    i += 4;
                    continue;
                }
                if rest.starts_with(b"0c") {
                    i += 4;
                    continue;
                }
                if !rest.is_empty() && rest[0] == b'c' {
                    i += 3;
                    continue;
                }
            }
            result.push(data[i]);
            i += 1;
        }
        result
    }

    /// Read PTY output (non-Unix fallback, no DSR handling)
    #[cfg(not(unix))]
    fn read_pty_output(
        session_id: Uuid,
        reader: &mut Box<dyn Read + Send>,
        pty_tx: mpsc::Sender<Action>,
        mut child: Box<dyn Child + Send + Sync>,
        strip_alt_screen: bool,
    ) {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let exit_code = match child.wait() {
                        Ok(status) => status.exit_code() as i32,
                        Err(_e) => {
                            // Don't use eprintln! in TUI - it corrupts the display
                            1
                        }
                    };
                    report_session_exited(&pty_tx, session_id, exit_code);
                    break;
                }
                Ok(n) => {
                    let mut data = buf[..n].to_vec();
                    if strip_alt_screen && Self::has_alt_screen_sequences(&data) {
                        data = Self::strip_alt_screen_sequences(&data);
                    }
                    if !data.is_empty() {
                        if let Err(err) = pty_tx.blocking_send(Action::PtyOutput(session_id, data))
                        {
                            crate::logger::warn(format!(
                                "failed to report PTY output for session {session_id}: {err}"
                            ));
                            break;
                        }
                    }
                }
                Err(_e) => {
                    // Don't use eprintln! in TUI - it corrupts the display
                    report_session_exited(&pty_tx, session_id, 1);
                    break;
                }
            }
        }
    }
}

impl Default for PtyManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_pty::{Child, ChildKiller, ExitStatus};

    mod resume_args {
        use super::super::{agent_args, Resume};
        use crate::models::AgentType;
        use uuid::Uuid;

        fn args(agent: AgentType, resume: Resume, id: Uuid, claude_id_free: bool) -> Vec<String> {
            agent_args(&agent, id, &resume, false, claude_id_free)
        }

        /// The bug this guards: `--continue` / `resume --last` are scoped to
        /// the directory, so several agents in one project all restore the
        /// same conversation. A known id must produce a targeted resume.
        #[test]
        fn a_known_conversation_is_resumed_by_id_not_by_directory() {
            let id = Uuid::new_v4();
            let claude = args(
                AgentType::Claude,
                Resume::Conversation("conv-a".into()),
                id,
                true,
            );
            assert_eq!(claude, vec!["--resume", "conv-a"]);
            assert!(!claude.iter().any(|a| a == "--continue"));

            let codex = args(
                AgentType::Codex,
                Resume::Conversation("conv-b".into()),
                id,
                true,
            );
            assert_eq!(codex, vec!["resume", "conv-b"]);
            assert!(!codex.iter().any(|a| a == "--last"));
        }

        #[test]
        fn an_unknown_conversation_falls_back_to_the_directorys_most_recent() {
            let id = Uuid::new_v4();
            assert_eq!(
                args(AgentType::Claude, Resume::MostRecent, id, true),
                vec!["--continue"]
            );
            assert_eq!(
                args(AgentType::Codex, Resume::MostRecent, id, true),
                vec!["resume", "--last"]
            );
        }

        #[test]
        fn a_fresh_claude_session_pins_our_id_so_it_can_be_resumed_later() {
            let id = Uuid::new_v4();
            assert_eq!(
                args(AgentType::Claude, Resume::No, id, true),
                vec!["--session-id".to_string(), id.to_string()]
            );
        }

        /// Restarting a stopped session reuses its uuid; Claude aborts with
        /// "Session ID is already in use" if we pin one it has written before.
        #[test]
        fn a_taken_claude_id_is_not_pinned_again() {
            let id = Uuid::new_v4();
            assert!(args(AgentType::Claude, Resume::No, id, false).is_empty());
        }

        #[test]
        fn codex_puts_the_resume_subcommand_before_its_flags() {
            let id = Uuid::new_v4();
            let with_perms = agent_args(
                &AgentType::Codex,
                id,
                &Resume::Conversation("conv".into()),
                true,
                true,
            );
            assert_eq!(
                with_perms,
                vec![
                    "resume",
                    "conv",
                    "--dangerously-bypass-approvals-and-sandbox"
                ]
            );
        }

        #[test]
        fn terminals_and_unknown_commands_take_no_resume_flags() {
            let id = Uuid::new_v4();
            assert!(args(
                AgentType::Terminal("shell".into()),
                Resume::MostRecent,
                id,
                true
            )
            .is_empty());
            assert!(args(
                AgentType::Custom {
                    command: "some-other-agent".into(),
                    display_name: "Other".into(),
                    badge: "O".into(),
                },
                Resume::MostRecent,
                id,
                true,
            )
            .is_empty());
        }

        /// Agents added through `user_config.toml` are `Custom`, but they are
        /// the same programs — they must get the same flags as a built-in.
        #[test]
        fn a_custom_agent_is_driven_by_its_command_not_its_enum_variant() {
            let id = Uuid::new_v4();
            let custom = |command: &str| AgentType::Custom {
                command: command.into(),
                display_name: "X".into(),
                badge: "X".into(),
            };

            assert_eq!(
                args(custom("claude"), Resume::Conversation("conv".into()), id, true),
                vec!["--resume", "conv"]
            );
            assert_eq!(
                args(custom("codex"), Resume::Conversation("conv".into()), id, true),
                vec!["resume", "conv"]
            );
        }

        #[test]
        fn hermes_resumes_a_named_session_and_falls_back_to_its_last() {
            let id = Uuid::new_v4();
            let hermes = AgentType::Custom {
                command: "hermes".into(),
                display_name: "Hermes".into(),
                badge: "H".into(),
            };
            assert_eq!(
                args(
                    hermes.clone(),
                    Resume::Conversation("20260719_114141_b85183".into()),
                    id,
                    true
                ),
                vec!["--resume", "20260719_114141_b85183"]
            );
            assert_eq!(
                args(hermes.clone(), Resume::MostRecent, id, true),
                vec!["--continue"]
            );
            assert!(args(hermes.clone(), Resume::No, id, true).is_empty());
            // Dangerous mode is the agent's own flag.
            assert_eq!(
                agent_args(&hermes, id, &Resume::No, true, true),
                vec!["--yolo"]
            );
        }
    }

    use std::io::{self, Read};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[derive(Debug)]
    struct DummyMaster;

    impl MasterPty for DummyMaster {
        fn resize(&self, _size: PtySize) -> std::result::Result<(), anyhow::Error> {
            Err(anyhow::anyhow!("unused"))
        }

        fn get_size(&self) -> std::result::Result<PtySize, anyhow::Error> {
            Err(anyhow::anyhow!("unused"))
        }

        fn try_clone_reader(&self) -> std::result::Result<Box<dyn Read + Send>, anyhow::Error> {
            Err(anyhow::anyhow!("unused"))
        }

        fn take_writer(&self) -> std::result::Result<Box<dyn io::Write + Send>, anyhow::Error> {
            Err(anyhow::anyhow!("unused"))
        }

        #[cfg(unix)]
        fn process_group_leader(&self) -> Option<libc::pid_t> {
            None
        }

        #[cfg(unix)]
        fn as_raw_fd(&self) -> Option<std::os::unix::io::RawFd> {
            None
        }
    }

    #[derive(Debug)]
    struct TestChild {
        exit_status: ExitStatus,
    }

    #[derive(Debug)]
    struct TestChildKiller {
        calls: Arc<AtomicUsize>,
    }

    impl ChildKiller for TestChildKiller {
        fn kill(&mut self) -> io::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
            Box::new(TestChildKiller {
                calls: self.calls.clone(),
            })
        }
    }

    impl ChildKiller for TestChild {
        fn kill(&mut self) -> io::Result<()> {
            Ok(())
        }

        fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
            Box::new(TestChildKiller {
                calls: Arc::new(AtomicUsize::new(0)),
            })
        }
    }

    impl Child for TestChild {
        fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            Ok(Some(self.exit_status.clone()))
        }

        fn wait(&mut self) -> io::Result<ExitStatus> {
            Ok(self.exit_status.clone())
        }

        fn process_id(&self) -> Option<u32> {
            None
        }

        #[cfg(windows)]
        fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
            None
        }
    }

    fn test_child(exit_code: u32) -> Box<dyn Child + Send + Sync> {
        Box::new(TestChild {
            exit_status: ExitStatus::with_exit_code(exit_code),
        })
    }

    fn test_handle(counter: Arc<AtomicUsize>) -> PtyHandle {
        PtyHandle {
            master: Box::new(DummyMaster),
            child_killer: Box::new(TestChildKiller { calls: counter }),
            process_id: None,
            writer: Box::new(io::sink()),
            cleanup_done: false,
        }
    }

    struct ChunkedReader {
        chunks: Vec<Vec<u8>>,
        index: usize,
    }

    impl ChunkedReader {
        fn new(chunks: Vec<Vec<u8>>) -> Self {
            Self { chunks, index: 0 }
        }
    }

    impl Read for ChunkedReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.index >= self.chunks.len() {
                return Ok(0);
            }
            let chunk = &self.chunks[self.index];
            let len = chunk.len().min(buf.len());
            buf[..len].copy_from_slice(&chunk[..len]);
            self.index += 1;
            Ok(len)
        }
    }

    #[cfg(unix)]
    fn run_read(
        session_id: Uuid,
        reader: &mut Box<dyn Read + Send>,
        tx: mpsc::Sender<Action>,
        child: Box<dyn Child + Send + Sync>,
    ) {
        PtyManager::read_pty_output_with_dsr(session_id, reader, tx, None, 0, child, false);
    }

    #[cfg(not(unix))]
    fn run_read(
        session_id: Uuid,
        reader: &mut Box<dyn Read + Send>,
        tx: mpsc::Sender<Action>,
        child: Box<dyn Child + Send + Sync>,
    ) {
        PtyManager::read_pty_output(session_id, reader, tx, child, false);
    }

    #[test]
    fn pty_reader_emits_output_and_exit() {
        let (tx, mut rx) = mpsc::channel(10);
        let session_id = Uuid::new_v4();
        let reader = ChunkedReader::new(vec![b"hello".to_vec(), b"world".to_vec()]);
        let mut reader: Box<dyn Read + Send> = Box::new(reader);

        run_read(session_id, &mut reader, tx, test_child(0));

        let mut actions = Vec::new();
        while let Ok(action) = rx.try_recv() {
            actions.push(action);
        }

        assert_eq!(actions.len(), 3);
        assert!(matches!(
            &actions[0],
            Action::PtyOutput(id, data) if *id == session_id && data == b"hello"
        ));
        assert!(matches!(
            &actions[1],
            Action::PtyOutput(id, data) if *id == session_id && data == b"world"
        ));
        assert!(matches!(
            &actions[2],
            Action::SessionExited(id, code) if *id == session_id && *code == 0
        ));
    }

    fn recv_with_timeout(rx: &mut mpsc::Receiver<Action>, timeout: Duration) -> Action {
        let start = Instant::now();
        loop {
            match rx.try_recv() {
                Ok(action) => return action,
                Err(mpsc::error::TryRecvError::Empty) => {
                    if start.elapsed() >= timeout {
                        panic!("timed out waiting for action");
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    panic!("channel closed while waiting for action");
                }
            }
        }
    }

    #[test]
    fn pty_reader_blocks_when_queue_full() {
        let (tx, mut rx) = mpsc::channel(1);
        let session_id = Uuid::new_v4();
        let reader = ChunkedReader::new(vec![b"first".to_vec(), b"second".to_vec()]);
        let mut reader: Box<dyn Read + Send> = Box::new(reader);

        let handle = std::thread::spawn(move || {
            run_read(session_id, &mut reader, tx, test_child(0));
        });

        std::thread::sleep(Duration::from_millis(50));
        assert!(!handle.is_finished(), "reader should block on full queue");

        let first = recv_with_timeout(&mut rx, Duration::from_millis(100));
        assert!(matches!(
            first,
            Action::PtyOutput(id, data) if id == session_id && data == b"first"
        ));

        let second = recv_with_timeout(&mut rx, Duration::from_millis(100));
        assert!(matches!(
            second,
            Action::PtyOutput(id, data) if id == session_id && data == b"second"
        ));

        let third = recv_with_timeout(&mut rx, Duration::from_millis(100));
        assert!(matches!(
            third,
            Action::SessionExited(id, code) if id == session_id && code == 0
        ));

        handle.join().unwrap();
    }

    #[test]
    fn pty_handle_kill_uses_child_killer_when_no_pid() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut handle = test_handle(calls.clone());

        handle.kill().unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn pty_handle_interrupt_then_kill_uses_child_killer_when_no_pid() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut handle = test_handle(calls.clone());

        handle
            .interrupt_then_kill(Duration::from_millis(0))
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
