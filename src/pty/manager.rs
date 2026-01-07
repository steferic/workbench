use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize, PtySystem};
use std::io::{Read, Write};
use std::path::Path;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::app::Action;
use crate::models::AgentType;

pub struct PtyHandle {
    pub session_id: Uuid,
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

    pub fn is_alive(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_none()
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
    ) -> Result<PtyHandle> {
        self.spawn_session_with_resume(session_id, agent_type, working_dir, rows, cols, action_tx, false)
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
                cmd.arg("--dangerously-skip-permissions");
                if resume {
                    cmd.arg("--continue");
                }
            }
            AgentType::Gemini => {
                cmd.arg("--yolo");
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
                    cmd.arg("--dangerously-bypass-approvals-and-sandbox");
                    cmd.cwd(working_dir);
                } else {
                    cmd.arg("--dangerously-bypass-approvals-and-sandbox");
                }
            }
            AgentType::Grok => {
                cmd.arg("--permission-mode");
                cmd.arg("full");
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

        // Spawn async task to read PTY output
        let tx = action_tx.clone();
        let sid = session_id;
        std::thread::spawn(move || {
            Self::read_pty_output(sid, &mut reader, tx);
        });

        Ok(PtyHandle {
            session_id,
            master: pair.master,
            child,
            writer,
        })
    }

    fn read_pty_output(
        session_id: Uuid,
        reader: &mut Box<dyn Read + Send>,
        action_tx: mpsc::UnboundedSender<Action>,
    ) {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    // EOF - process exited
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
