use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize, PtySystem};
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

pub struct PtyHandle {
    pub master: Box<dyn MasterPty + Send>,
    pub child_killer: Box<dyn ChildKiller + Send + Sync>,
    pub process_id: Option<u32>,
    pub writer: Box<dyn Write + Send>,
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

    pub fn kill(&mut self) -> Result<()> {
        self.kill_process_group()
    }

    pub fn interrupt_then_kill(&mut self, grace: Duration) -> Result<()> {
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
                let _ = self.signal_process_group(pgid, libc::SIGKILL);
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

pub struct PtyManager {
    pty_system: Box<dyn PtySystem>,
}

impl PtyManager {
    pub fn new() -> Self {
        Self {
            pty_system: native_pty_system(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_session(
        &self,
        session_id: Uuid,
        agent_type: AgentType,
        working_dir: &Path,
        rows: u16,
        cols: u16,
        pty_tx: mpsc::Sender<Action>,
        dangerously_skip_permissions: bool,
    ) -> Result<PtyHandle> {
        self.spawn_session_with_resume(
            session_id,
            agent_type,
            working_dir,
            rows,
            cols,
            pty_tx,
            false,
            dangerously_skip_permissions,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_session_with_resume(
        &self,
        session_id: Uuid,
        agent_type: AgentType,
        working_dir: &Path,
        rows: u16,
        cols: u16,
        pty_tx: mpsc::Sender<Action>,
        resume: bool,
        dangerously_skip_permissions: bool,
    ) -> Result<PtyHandle> {
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
            // For terminals, use $SHELL or fallback to bash
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
            CommandBuilder::new(shell)
        } else {
            CommandBuilder::new(agent_type.command())
        };
        cmd.cwd(working_dir);

        // Add agent-specific flags (not for terminals)
        match agent_type {
            AgentType::Claude => {
                if dangerously_skip_permissions {
                    cmd.arg("--dangerously-skip-permissions");
                }
                if resume {
                    cmd.arg("--continue");
                }
            }
            AgentType::Gemini => {
                if dangerously_skip_permissions {
                    cmd.arg("--yolo");
                }
                if resume {
                    cmd.arg("--resume");
                }
            }
            AgentType::Codex => {
                // Codex uses a subcommand for resume: `codex resume --last`
                if resume {
                    cmd = CommandBuilder::new("codex");
                    cmd.arg("resume");
                    cmd.arg("--last");
                    if dangerously_skip_permissions {
                        cmd.arg("--dangerously-bypass-approvals-and-sandbox");
                    }
                    cmd.cwd(working_dir);
                } else if dangerously_skip_permissions {
                    cmd.arg("--dangerously-bypass-approvals-and-sandbox");
                }
                // Try inline mode to avoid alternate screen buffer cursor issues
                cmd.arg("--no-alt-screen");
            }
            AgentType::Grok => {
                if dangerously_skip_permissions {
                    cmd.arg("--permission-mode");
                    cmd.arg("full");
                }
                if resume {
                    cmd.arg("--continue");
                }
            }
            AgentType::Terminal(_) => {
                // No special flags for terminals, they're just shells
            }
        }

        // Set TERM for proper terminal emulation
        // Use simpler vt100 for Codex to reduce cursor positioning complexity
        if matches!(agent_type, AgentType::Codex) {
            cmd.env("TERM", "vt100");
        } else {
            cmd.env("TERM", "xterm-256color");
        }

        // Set LINES and COLUMNS environment variables
        // This provides explicit size info that nvim and other apps can use
        // as a fallback, helping with apps that have startup resize issues
        cmd.env("LINES", rows.to_string());
        cmd.env("COLUMNS", cols.to_string());

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
        // Don't skip DSR for any agent - Codex requires it to function
        let is_codex = false;
        std::thread::spawn(move || {
            #[cfg(unix)]
            Self::read_pty_output_with_dsr(
                sid,
                &mut reader,
                pty_tx,
                master_fd,
                pty_rows,
                child,
                is_codex,
            );
            #[cfg(not(unix))]
            Self::read_pty_output(sid, &mut reader, pty_tx, child);
        });

        Ok(PtyHandle {
            master: pair.master,
            child_killer,
            process_id,
            writer,
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
        skip_dsr: bool,
    ) {
        // Terminal query patterns
        const DSR_QUERY: &[u8] = b"\x1b[6n";       // Cursor position query
        const DA_QUERY: &[u8] = b"\x1b[c";          // Primary Device Attributes query
        const DA_QUERY2: &[u8] = b"\x1b[0c";        // Primary DA (alternate)
        const DA2_QUERY: &[u8] = b"\x1b[>c";        // Secondary Device Attributes query
        const DA2_QUERY2: &[u8] = b"\x1b[>0c";      // Secondary DA (alternate)

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
                    let _ = pty_tx.blocking_send(Action::SessionExited(session_id, exit_code));
                    break;
                }
                Ok(n) => {
                    let mut data = buf[..n].to_vec();

                    // Update cursor position by parsing escape sequences in the data
                    Self::track_cursor_position(&data, &mut cursor_row, &mut cursor_col, pty_rows);

                    // Handle terminal queries
                    if let Some(fd) = master_fd {
                        let has_dsr = data.windows(DSR_QUERY.len()).any(|w| w == DSR_QUERY);
                        let has_da = data.windows(DA_QUERY.len()).any(|w| w == DA_QUERY)
                            || data.windows(DA_QUERY2.len()).any(|w| w == DA_QUERY2);
                        let has_da2 = data.windows(DA2_QUERY.len()).any(|w| w == DA2_QUERY)
                            || data.windows(DA2_QUERY2.len()).any(|w| w == DA2_QUERY2);

                        // For Codex (skip_dsr=true), don't respond to DSR - let it timeout
                        // and hopefully use a fallback input mode
                        let should_respond_dsr = has_dsr && !skip_dsr;

                        if should_respond_dsr || has_da || has_da2 {
                            let mut file = std::mem::ManuallyDrop::new(unsafe {
                                std::fs::File::from_raw_fd(fd)
                            });

                            // Respond to DSR with tracked cursor position (if not skipping)
                            if should_respond_dsr {
                                let dsr_response = format!("\x1b[{};{}R", cursor_row, cursor_col);
                                let _ = file.write_all(dsr_response.as_bytes());
                            }

                            // Respond to primary DA
                            if has_da {
                                let _ = file.write_all(DA_RESPONSE);
                            }

                            // Respond to secondary DA
                            if has_da2 {
                                let _ = file.write_all(DA2_RESPONSE);
                            }

                            let _ = file.flush();
                        }

                        // Always strip queries from output (whether we responded or not)
                        if has_dsr || has_da || has_da2 {
                            data = Self::strip_terminal_queries(&data);
                        }
                    }

                    if !data.is_empty()
                        && pty_tx.blocking_send(Action::PtyOutput(session_id, data)).is_err() {
                            break;
                        }
                }
                Err(_e) => {
                    // Don't use eprintln! in TUI - it corrupts the display
                    let _ = pty_tx.blocking_send(Action::SessionExited(session_id, 1));
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

    /// Strip terminal query sequences from data
    #[cfg(unix)]
    fn strip_terminal_queries(data: &[u8]) -> Vec<u8> {
        const DSR_QUERY: &[u8] = b"\x1b[6n";
        const DA_QUERY: &[u8] = b"\x1b[c";
        const DA_QUERY2: &[u8] = b"\x1b[0c";
        const DA2_QUERY: &[u8] = b"\x1b[>c";
        const DA2_QUERY2: &[u8] = b"\x1b[>0c";

        let mut result = Vec::with_capacity(data.len());
        let mut i = 0;
        while i < data.len() {
            // Check for DSR query
            if i + DSR_QUERY.len() <= data.len() && &data[i..i + DSR_QUERY.len()] == DSR_QUERY {
                i += DSR_QUERY.len();
                continue;
            }
            // Check for secondary DA query (longer ones first)
            if i + DA2_QUERY2.len() <= data.len() && &data[i..i + DA2_QUERY2.len()] == DA2_QUERY2 {
                i += DA2_QUERY2.len();
                continue;
            }
            if i + DA2_QUERY.len() <= data.len() && &data[i..i + DA2_QUERY.len()] == DA2_QUERY {
                i += DA2_QUERY.len();
                continue;
            }
            // Check for primary DA query (longer one first)
            if i + DA_QUERY2.len() <= data.len() && &data[i..i + DA_QUERY2.len()] == DA_QUERY2 {
                i += DA_QUERY2.len();
                continue;
            }
            if i + DA_QUERY.len() <= data.len() && &data[i..i + DA_QUERY.len()] == DA_QUERY {
                i += DA_QUERY.len();
                continue;
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
                    let _ = pty_tx.blocking_send(Action::SessionExited(session_id, exit_code));
                    break;
                }
                Ok(n) => {
                    let data = buf[..n].to_vec();
                    if pty_tx.blocking_send(Action::PtyOutput(session_id, data)).is_err() {
                        break;
                    }
                }
                Err(_e) => {
                    // Don't use eprintln! in TUI - it corrupts the display
                    let _ = pty_tx.blocking_send(Action::SessionExited(session_id, 1));
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
    use std::io::{self, Read};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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

        fn take_writer(
            &self,
        ) -> std::result::Result<Box<dyn io::Write + Send>, anyhow::Error> {
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
        PtyManager::read_pty_output(session_id, reader, tx, child);
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
