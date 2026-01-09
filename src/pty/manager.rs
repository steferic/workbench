use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize, PtySystem};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::io::{FromRawFd, RawFd};

use crate::app::Action;
use crate::models::AgentType;

pub struct PtyHandle {
    pub master: Box<dyn MasterPty + Send>,
    pub child: Box<dyn Child + Send + Sync>,
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
        self.child.kill()?;
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

    pub fn spawn_session(
        &self,
        session_id: Uuid,
        agent_type: AgentType,
        working_dir: &Path,
        rows: u16,
        cols: u16,
        action_tx: mpsc::UnboundedSender<Action>,
        dangerously_skip_permissions: bool,
    ) -> Result<PtyHandle> {
        self.spawn_session_with_resume(
            session_id,
            agent_type,
            working_dir,
            rows,
            cols,
            action_tx,
            false,
            dangerously_skip_permissions,
        )
    }

    pub fn spawn_session_with_resume(
        &self,
        session_id: Uuid,
        agent_type: AgentType,
        working_dir: &Path,
        rows: u16,
        cols: u16,
        action_tx: mpsc::UnboundedSender<Action>,
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
                } else {
                    if dangerously_skip_permissions {
                        cmd.arg("--dangerously-bypass-approvals-and-sandbox");
                    }
                }
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
        cmd.env("TERM", "xterm-256color");

        // Spawn the process
        let child = pair
            .slave
            .spawn_command(cmd)
            .context("Failed to spawn agent process")?;

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

        // Flag to ensure we only respond to DSR once per session
        let dsr_responded = Arc::new(AtomicBool::new(false));
        let dsr_responded_clone = dsr_responded.clone();

        // Spawn async task to read PTY output
        let tx = action_tx.clone();
        let sid = session_id;
        let pty_rows = rows;
        std::thread::spawn(move || {
            #[cfg(unix)]
            Self::read_pty_output_with_dsr(sid, &mut reader, tx, master_fd, dsr_responded_clone, pty_rows);
            #[cfg(not(unix))]
            Self::read_pty_output(sid, &mut reader, tx);
        });

        Ok(PtyHandle {
            master: pair.master,
            child,
            writer,
        })
    }

    /// Read PTY output with immediate DSR response (Unix only)
    #[cfg(unix)]
    fn read_pty_output_with_dsr(
        session_id: Uuid,
        reader: &mut Box<dyn Read + Send>,
        action_tx: mpsc::UnboundedSender<Action>,
        master_fd: Option<RawFd>,
        dsr_responded: Arc<AtomicBool>,
        _rows: u16,
    ) {
        // Terminal query patterns
        const DSR_QUERY: &[u8] = b"\x1b[6n";       // Cursor position query
        const DA_QUERY: &[u8] = b"\x1b[c";          // Primary Device Attributes query
        const DA_QUERY2: &[u8] = b"\x1b[0c";        // Primary DA (alternate)
        const DA2_QUERY: &[u8] = b"\x1b[>c";        // Secondary Device Attributes query
        const DA2_QUERY2: &[u8] = b"\x1b[>0c";      // Secondary DA (alternate)

        // Responses - use simple VT102 identification
        // DSR: report cursor at 1,1 (simple, expected position)
        const DSR_RESPONSE: &[u8] = b"\x1b[1;1R";
        // Primary DA: VT102 (simpler than VT100 with AVO)
        const DA_RESPONSE: &[u8] = b"\x1b[?6c";
        // Secondary DA: VT102 version 1.0 (>0;0;0c format: terminal;firmware;keyboard)
        const DA2_RESPONSE: &[u8] = b"\x1b[>0;0;0c";

        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    // EOF - process exited
                    let _ = action_tx.send(Action::SessionExited(session_id, 0));
                    break;
                }
                Ok(n) => {
                    let mut data = buf[..n].to_vec();

                    // Handle terminal queries
                    if let Some(fd) = master_fd {
                        let has_dsr = data.windows(DSR_QUERY.len()).any(|w| w == DSR_QUERY);
                        let has_da = data.windows(DA_QUERY.len()).any(|w| w == DA_QUERY)
                            || data.windows(DA_QUERY2.len()).any(|w| w == DA_QUERY2);
                        let has_da2 = data.windows(DA2_QUERY.len()).any(|w| w == DA2_QUERY)
                            || data.windows(DA2_QUERY2.len()).any(|w| w == DA2_QUERY2);

                        if has_dsr || has_da || has_da2 {
                            let mut file = std::mem::ManuallyDrop::new(unsafe {
                                std::fs::File::from_raw_fd(fd)
                            });

                            // Respond to DSR only once (for startup check)
                            if has_dsr && !dsr_responded.swap(true, Ordering::SeqCst) {
                                let _ = file.write_all(DSR_RESPONSE);
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

                            // Strip queries from output
                            data = Self::strip_terminal_queries(&data);
                        }
                    }

                    if !data.is_empty() {
                        if action_tx.send(Action::PtyOutput(session_id, data)).is_err() {
                            break;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("PTY read error for session {}: {}", session_id, e);
                    let _ = action_tx.send(Action::SessionExited(session_id, 1));
                    break;
                }
            }
        }
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
        action_tx: mpsc::UnboundedSender<Action>,
    ) {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = action_tx.send(Action::SessionExited(session_id, 0));
                    break;
                }
                Ok(n) => {
                    let data = buf[..n].to_vec();
                    if action_tx.send(Action::PtyOutput(session_id, data)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("PTY read error for session {}: {}", session_id, e);
                    let _ = action_tx.send(Action::SessionExited(session_id, 1));
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
