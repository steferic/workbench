use anyhow::{Context, Result};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize, PtySystem};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;
use tokio::sync::mpsc;
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::io::FromRawFd;

use super::output::TerminalOutput;
use crate::app::Action;
use crate::models::AgentType;

fn report_session_exited(
    pty_tx: &mpsc::Sender<Action>,
    session_id: Uuid,
    generation: Uuid,
    exit_code: i32,
) {
    if let Err(err) = pty_tx.blocking_send(Action::SessionExited(session_id, generation, exit_code))
    {
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
    pub generation: Uuid,
    output: Arc<Mutex<TerminalOutput>>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    process_tree: Option<std::sync::Arc<std::sync::Mutex<super::process_tree::ProcessTree>>>,
    reaped: Option<std::sync::mpsc::Receiver<()>>,
    /// Set only after termination and reaping succeed; failures remain
    /// eligible for the Drop safety net.
    cleanup_done: bool,
    /// The child's kernel start time, captured at spawn. Before any signal is
    /// sent, the pid is checked against this: a pid whose start time changed
    /// belongs to someone else now, and gets left alone. See `proc_identity`.
    spawned_start: Option<crate::pty::proc_identity::ProcStart>,
    /// Who this handle belongs to, for the kill log — every signal this
    /// process sends is written down, so an abrupt end elsewhere on the
    /// machine can be checked against what workbench itself was doing.
    label: String,
}

/// A handle dropped without explicit cleanup still terminates its owned
/// processes and reaps the direct child.
impl Drop for PtyHandle {
    fn drop(&mut self) {
        if self.cleanup_done {
            return;
        }
        if let Err(err) = self.kill() {
            crate::logger::warn(format!("failed to clean up PTY on drop: {err}"));
        }
    }
}

impl PtyHandle {
    pub fn send_input(&mut self, data: &[u8]) -> Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Validate an approval against the reader's latest screen while holding
    /// its parser lock. It must not wait behind queued UI output or actions.
    pub fn send_input_if_screen_matches(
        &mut self,
        data: &[u8],
        matches: impl FnOnce(&str) -> bool,
    ) -> Result<bool> {
        let output = self.output.lock().unwrap_or_else(|e| e.into_inner());
        if !matches(&output.screen().contents()) {
            return Ok(false);
        }
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(true)
    }

    pub fn screen_contents(&self) -> String {
        self.output
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .screen()
            .contents()
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        let (rows, cols) = (rows.max(1), cols.max(1));
        // Keep query replies in step with the kernel's dimensions. The reader
        // only holds this lock while parsing, never while waiting on IO.
        let mut output = self.output.lock().unwrap_or_else(|e| e.into_inner());
        let current = self.master.get_size()?;
        if (current.rows, current.cols) != (rows, cols) {
            self.master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })?;
        }
        output.resize(rows, cols);
        Ok(())
    }

    pub fn kill(&mut self) -> Result<()> {
        self.terminate(Duration::ZERO)
    }

    pub fn interrupt_then_kill(&mut self, grace: Duration) -> Result<()> {
        self.terminate(grace)
    }

    fn terminate(&mut self, grace: Duration) -> Result<()> {
        if self.cleanup_done {
            return Ok(());
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if let Some(tree) = &self.process_tree {
            tree.lock()
                .unwrap_or_else(|e| e.into_inner())
                .terminate(grace, &self.label)?;
            self.wait_for_reap()?;
            self.cleanup_done = true;
            return Ok(());
        }
        if grace.is_zero() {
            self.kill_process_group()?;
        } else {
            self.interrupt_process_group(grace)?;
        }
        self.wait_for_reap()?;
        self.cleanup_done = true;
        Ok(())
    }

    fn wait_for_reap(&mut self) -> Result<()> {
        if let Some(done) = &self.reaped {
            done.recv_timeout(Duration::from_secs(2))
                .context("PTY child was not reaped after termination")?;
            self.reaped = None;
        }
        Ok(())
    }

    fn interrupt_process_group(&mut self, grace: Duration) -> Result<()> {
        #[cfg(unix)]
        {
            if let Some(pgid) = self.verified_pgid("interrupt") {
                // Send SIGINT to the process group for a graceful shutdown.
                self.log_signal("SIGINT", pgid);
                if self.signal_process_group(pgid, libc::SIGINT).is_err() {
                    return self.kill_via_handle();
                }

                let start = Instant::now();
                while start.elapsed() < grace {
                    if !self.process_group_alive(pgid) {
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }

                // Escalate to SIGKILL if the group is still alive — re-checked
                // first: the group being "alive" is exactly what a recycled
                // pid looks like, and the grace period is a window for the
                // child to exit and the number to move on.
                if self.verified_pgid("escalate").is_some() {
                    self.log_signal("SIGKILL", pgid);
                    if let Err(err) = self.signal_process_group(pgid, libc::SIGKILL) {
                        crate::logger::warn(format!("failed to kill PTY process group: {err}"));
                    }
                }
                return Ok(());
            }
            if self.process_id.is_some() {
                // Named a pid but could not vouch for it; nothing to signal.
                return Ok(());
            }
        }

        self.kill_via_handle()
    }

    /// portable-pty's own killer. It signals by raw pid too, so it gets the
    /// same identity check and the same log line as the group paths.
    fn kill_via_handle(&mut self) -> Result<()> {
        if let (Some(pid), Some(spawned)) = (self.process_id, self.spawned_start) {
            use crate::pty::proc_identity::{owner, PidOwner};
            match owner(pid, spawned) {
                PidOwner::Ours => {}
                PidOwner::Gone => return Ok(()),
                PidOwner::Recycled => {
                    crate::logger::warn(format!(
                        "kill: pid {pid} ({}) was recycled to another process; not signalling",
                        self.label
                    ));
                    return Ok(());
                }
            }
        }
        crate::logger::info(format!(
            "kill: SIGKILL pid {:?} ({}) via child handle",
            self.process_id, self.label
        ));
        self.child_killer.kill()?;
        Ok(())
    }

    /// One line per signal actually sent. `info`, not debug: these are rare,
    /// and the whole point is that they survive in the log.
    #[cfg(unix)]
    fn log_signal(&self, signal: &str, pgid: libc::pid_t) {
        crate::logger::info(format!("kill: {signal} group -{pgid} ({})", self.label));
    }

    /// The process group to signal, but only if the pid still names the child
    /// we spawned.
    ///
    /// `None` for a recycled or vanished pid — with a warning for recycled,
    /// because that is the friendly-fire case this exists to catch. A spawn
    /// whose start time was never readable is treated as ours: the guard must
    /// fail open there or an unreadable /proc would strand every kill.
    #[cfg(unix)]
    fn verified_pgid(&self, doing: &str) -> Option<libc::pid_t> {
        use crate::pty::proc_identity::{owner, PidOwner};
        let pgid = self.process_group_id()?;
        let Some(spawned) = self.spawned_start else {
            return Some(pgid);
        };
        match owner(pgid as u32, spawned) {
            PidOwner::Ours => Some(pgid),
            PidOwner::Gone => None,
            PidOwner::Recycled => {
                crate::logger::warn(format!(
                    "kill: pid {pgid} ({}) was recycled to another process; {doing} skipped",
                    self.label
                ));
                None
            }
        }
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
        if let Some(pgid) = self.verified_pgid("group kill") {
            // portable-pty uses setsid() on spawn, so pid == pgid for the child.
            self.log_signal("SIGKILL", pgid);
            if self.signal_process_group(pgid, libc::SIGKILL).is_ok() {
                return Ok(());
            }
        } else if self.process_id.is_some() {
            // Named a pid but could not vouch for it: recycled or gone, and
            // in neither case is there anything of ours left to signal.
            return Ok(());
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
    pub scrollback_rows: usize,
    /// Extra variables for the child, on top of workbench's own identity
    /// set. A job run puts its id here so the CLI inside the pane can report
    /// against it without being told which run it is.
    pub extra_env: Vec<(String, String)>,
}

/// The provider-specific CLI arguments for a session.
///
/// Split out from spawning so the resume contract is testable: getting this
/// wrong silently merges two agents' histories rather than failing loudly.
/// `claude_id_free` is whether Claude has no log under our session uuid yet —
/// it refuses `--session-id` for an id it has already written. `hook_bin` is
/// the workbench binary agents call back on lifecycle events; `None` disables
/// status reporting for this spawn.
fn agent_args(
    agent_type: &AgentType,
    session_id: Uuid,
    resume: &Resume,
    dangerously_skip_permissions: bool,
    claude_id_free: bool,
    hook_bin: Option<&str>,
) -> Vec<String> {
    if agent_type.is_terminal() {
        return Vec::new();
    }
    let mut args: Vec<String> = Vec::new();
    // Dispatch on the command, not the enum, so an agent added through
    // `user_config.toml` behaves exactly like a built-in one.
    match agent_type.command() {
        "claude" => {
            // Lifecycle hooks, so the agent reports what it is doing instead
            // of us guessing from its output. Passed inline rather than
            // written into ~/.claude/settings.json: Claude *merges* what
            // `--settings` carries with the user's own settings, so their
            // hooks keep running and workbench leaves nothing behind.
            if let Some(bin) = hook_bin {
                args.push("--settings".into());
                args.push(crate::agent_status::claude_hook_settings(bin));
            }
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
            // Codex only runs hooks it has been told to trust, so status
            // reporting rides along with the session's existing consent: a
            // ⚡ session already opted out of approval gates, and an ordinary
            // one keeps the output-timing inference rather than being handed
            // a flag named "dangerously".
            let hook_script = hook_bin.filter(|_| dangerously_skip_permissions);

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
            if let Some(script) = hook_script {
                args.push("--dangerously-bypass-hook-trust".into());
                args.extend(crate::agent_status::codex_hook_args(Path::new(script)));
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
                args.push("bypassPermissions".into());
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
        "pi" => {
            // pi has no approval-bypass flag; `--approve` only trusts
            // project-local files, so `dangerously_skip_permissions` has
            // nothing to map onto here.
            match resume {
                Resume::Conversation(id) => {
                    args.push("--session".into());
                    args.push(id.clone());
                }
                Resume::MostRecent => args.push("--continue".into()),
                // Pin the session id like Claude, so the conversation is
                // addressable on restart rather than merging into whatever
                // ran last in this directory.
                Resume::No => {
                    args.push("--session-id".into());
                    args.push(session_id.to_string());
                }
            }
        }
        "opencode" => match resume {
            Resume::Conversation(id) => {
                args.push("--session".into());
                args.push(id.clone());
            }
            Resume::MostRecent => args.push("--continue".into()),
            Resume::No => {}
        },
        // Anything else runs bare. Its task list is still mirrored — that reads
        // the agent's own store and needs no cooperation from the command line.
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
            scrollback_rows,
            extra_env,
        } = config;
        let rows = rows.max(1);
        let cols = cols.max(1);

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
        // Agents call back on lifecycle events — Claude runs this binary
        // directly, Codex runs a generated wrapper because its hook command
        // cannot carry the event name. Anything we cannot wire up simply
        // reports nothing and keeps the output-timing inference.
        let hook_bin = std::env::current_exe()
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
            .filter(|_| crate::agent_status::supports_status_hooks(agent_type.command()))
            .and_then(|bin| match agent_type.command() {
                // Only written for sessions that will actually use it: Codex
                // hooks need trust this session has already waived.
                "codex" if !dangerously_skip_permissions => None,
                "codex" => crate::agent_status::ensure_codex_hook_script(&bin)
                    .map_err(|err| {
                        crate::logger::warn(format!("codex hook script unavailable: {err}"))
                    })
                    .ok()
                    .map(|path| path.to_string_lossy().into_owned()),
                _ => Some(bin),
            });
        for arg in agent_args(
            &agent_type,
            session_id,
            &resume,
            dangerously_skip_permissions,
            claude_id_free,
            hook_bin.as_deref(),
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
        // Where to reach workbench itself. An agent inside a pane can read its
        // own workspace and drive its peers over the control socket, and it
        // cannot be expected to guess a path that moves with the platform.
        if let Ok(socket) = crate::control::socket_path() {
            cmd.env(crate::control::ENV_SOCKET, socket);
        }
        for (key, value) in &extra_env {
            cmd.env(key, value);
        }

        // Do NOT export LINES/COLUMNS. Exported, they override the live
        // TIOCGWINSZ size in Ink (Claude) and other TUI frameworks, freezing
        // the child's layout at its spawn-time dimensions — the pane then
        // renders clipped or mis-wrapped after any pane/window resize, and
        // no amount of SIGWINCH fixes it. The PTY itself always carries the
        // correct size (set at open, updated by resize()).
        cmd.env_remove("LINES");
        cmd.env_remove("COLUMNS");

        // Acquire fallible resources before starting the child.
        let mut reader = pair
            .master
            .try_clone_reader()
            .context("Failed to clone PTY reader")?;
        let writer = pair
            .master
            .take_writer()
            .context("Failed to take PTY writer")?;
        let generation = Uuid::new_v4();
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        cmd.env(super::process_tree::ENV_OWNER, generation.to_string());
        let mut child = pair
            .slave
            .spawn_command(cmd)
            .context("Failed to spawn agent process")?;
        let child_killer = child.clone_killer();
        let process_id = child.process_id();
        let spawned_start = process_id.and_then(crate::pty::proc_identity::start_time);
        let label = format!("{} {}", &session_id.to_string()[..8], agent_type.command());
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let process_tree = std::sync::Arc::new(std::sync::Mutex::new(
            super::process_tree::ProcessTree::new(process_id, &generation.to_string()),
        ));
        let (reaped_tx, reaped) = std::sync::mpsc::channel();
        let exit_tx = pty_tx.clone();
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let exit_tree = process_tree.clone();
        let exit_label = label.clone();
        // Reap independently of PTY EOF: descendants can hold the slave open
        // long after the direct child dies. Tell cleanup the child was reaped
        // before publishing into the bounded UI queue, which may be full.
        std::thread::spawn(move || {
            let code = child
                .wait()
                .map(|status| status.exit_code() as i32)
                .unwrap_or(1);
            let _ = reaped_tx.send(());
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            if let Err(err) = exit_tree
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .terminate(Duration::ZERO, &exit_label)
            {
                crate::logger::warn(format!("failed to clean up exited {exit_label}: {err}"));
            }
            report_session_exited(&exit_tx, session_id, generation, code);
        });

        #[cfg(unix)]
        let query_writer = pair.master.as_raw_fd().and_then(|fd| {
            let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
            (duplicate >= 0).then(|| unsafe { std::fs::File::from_raw_fd(duplicate) })
        });
        #[cfg(not(unix))]
        let query_writer = None;

        // Spawn async task to read PTY output
        let pty_tx = pty_tx.clone();
        let sid = session_id;
        let strip_alt_screen = agent_type.is_redraw_style() || !use_alternate_screen;
        let output = Arc::new(Mutex::new(
            TerminalOutput::new(rows, cols, strip_alt_screen)
                .with_reflow(!agent_type.is_redraw_style(), scrollback_rows),
        ));
        let reader_output = output.clone();
        std::thread::spawn(move || {
            Self::read_pty_output(sid, &mut reader, pty_tx, query_writer, reader_output);
        });

        Ok(PtyHandle {
            master: pair.master,
            child_killer,
            process_id,
            writer,
            generation,
            output,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            process_tree: Some(process_tree),
            reaped: Some(reaped),
            cleanup_done: false,
            spawned_start,
            label,
        })
    }

    /// Parse queries on the reader thread so a busy UI cannot stall startup.
    fn read_pty_output(
        session_id: Uuid,
        reader: &mut Box<dyn Read + Send>,
        pty_tx: mpsc::Sender<Action>,
        mut query_writer: Option<std::fs::File>,
        output: Arc<Mutex<TerminalOutput>>,
    ) {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let parsed = output
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .process(&buf[..n]);
                    if !parsed.replies.is_empty() {
                        if let Some(writer) = query_writer.as_mut() {
                            if let Err(err) = writer
                                .write_all(&parsed.replies)
                                .and_then(|_| writer.flush())
                            {
                                crate::logger::warn(format!(
                                    "failed to answer terminal query: {err}"
                                ));
                            }
                        }
                    }
                    if !parsed.bytes.is_empty() {
                        if let Err(err) =
                            pty_tx.blocking_send(Action::PtyOutput(session_id, parsed.bytes))
                        {
                            crate::logger::warn(format!(
                                "failed to report PTY output for session {session_id}: {err}"
                            ));
                            break;
                        }
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => {
                    crate::logger::warn(format!("PTY read failed for {session_id}: {err}"));
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
            agent_args(&agent, id, &resume, false, claude_id_free, None)
        }

        #[test]
        fn claude_is_spawned_with_status_hooks_pointing_at_this_binary() {
            let args = agent_args(
                &AgentType::Claude,
                Uuid::new_v4(),
                &Resume::No,
                false,
                false,
                Some("/opt/workbench"),
            );
            let settings_idx = args
                .iter()
                .position(|a| a == "--settings")
                .expect("hooks are installed at spawn");
            let settings: serde_json::Value =
                serde_json::from_str(&args[settings_idx + 1]).expect("valid settings JSON");
            assert_eq!(
                settings["hooks"]["Notification"][0]["hooks"][0]["command"],
                "'/opt/workbench' hook Notification"
            );
        }

        #[test]
        fn codex_gets_hooks_only_when_the_session_already_bypasses_approval() {
            let id = Uuid::new_v4();
            let script = Some("/cfg/hooks/codex-hook.sh");

            // Codex refuses untrusted hooks, so an ordinary session would be
            // handed a "dangerously" flag it never asked for.
            let ordinary = agent_args(&AgentType::Codex, id, &Resume::No, false, false, script);
            assert!(
                !ordinary.iter().any(|a| a.contains("hooks.")),
                "{ordinary:?}"
            );
            assert!(
                !ordinary
                    .iter()
                    .any(|a| a == "--dangerously-bypass-hook-trust"),
                "{ordinary:?}"
            );

            let dangerous = agent_args(&AgentType::Codex, id, &Resume::No, true, false, script);
            assert!(dangerous
                .iter()
                .any(|a| a == "--dangerously-bypass-hook-trust"));
            assert!(dangerous
                .iter()
                .any(|a| a.starts_with("hooks.PermissionRequest=")));
        }

        #[test]
        fn codex_hook_flags_follow_the_resume_subcommand() {
            let args = agent_args(
                &AgentType::Codex,
                Uuid::new_v4(),
                &Resume::Conversation("conv".into()),
                true,
                false,
                Some("/cfg/hooks/codex-hook.sh"),
            );
            // `resume` is a subcommand: anything before it is a parse error.
            assert_eq!(args[0], "resume");
            assert_eq!(args[1], "conv");
            let hooks_at = args.iter().position(|a| a.starts_with("hooks.")).unwrap();
            assert!(hooks_at > 1, "{args:?}");
        }

        #[test]
        fn without_a_binary_path_claude_still_spawns_cleanly() {
            // current_exe() can fail; status reporting is optional, spawning
            // is not.
            let args = args(AgentType::Claude, Resume::No, Uuid::new_v4(), true);
            assert!(!args.iter().any(|a| a == "--settings"), "{args:?}");
            assert!(args.iter().any(|a| a == "--session-id"), "{args:?}");
        }

        /// Hooks are additive, and the user's own settings must survive:
        /// verified against Claude Code 2.1.220, where `--settings` merges
        /// rather than replaces (their global hooks still ran).
        #[test]
        fn hooks_do_not_displace_the_users_own_claude_settings() {
            let args = agent_args(
                &AgentType::Claude,
                Uuid::new_v4(),
                &Resume::No,
                false,
                false,
                Some("/opt/workbench"),
            );
            assert!(
                !args.iter().any(|a| a.contains("settings.json")),
                "workbench must not point Claude at a settings *file* it owns: {args:?}"
            );
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
                None,
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
                args(
                    custom("claude"),
                    Resume::Conversation("conv".into()),
                    id,
                    true
                ),
                vec!["--resume", "conv"]
            );
            assert_eq!(
                args(
                    custom("codex"),
                    Resume::Conversation("conv".into()),
                    id,
                    true
                ),
                vec!["resume", "conv"]
            );
        }

        /// `--permission-mode` is an enum on grok's side: an invalid value is
        /// a hard startup error, not a warning.
        #[test]
        fn grok_bypasses_permissions_with_a_value_grok_actually_accepts() {
            let id = Uuid::new_v4();
            let grok = AgentType::Custom {
                command: "grok".into(),
                display_name: "Grok".into(),
                badge: "K".into(),
            };
            assert_eq!(
                agent_args(&grok, id, &Resume::No, true, true, None),
                vec!["--permission-mode", "bypassPermissions"]
            );
        }

        /// Resuming by id has to name the session; falling back to `--continue`
        /// would silently attach to whatever ran last in this directory.
        #[test]
        fn grok_resumes_its_own_conversation_by_id() {
            let id = Uuid::new_v4();
            let grok = |r| {
                args(
                    AgentType::Custom {
                        command: "grok".into(),
                        display_name: "Grok".into(),
                        badge: "K".into(),
                    },
                    r,
                    id,
                    true,
                )
            };
            assert_eq!(
                grok(Resume::Conversation("conv".into())),
                vec!["--resume", "conv"]
            );
            assert_eq!(grok(Resume::MostRecent), vec!["--continue"]);
            assert!(grok(Resume::No).is_empty());
        }

        #[test]
        fn pi_pins_a_new_session_id_and_resumes_that_session_by_id() {
            let id = Uuid::new_v4();
            let pi = |r| {
                args(
                    AgentType::Custom {
                        command: "pi".into(),
                        display_name: "Pi".into(),
                        badge: "P".into(),
                    },
                    r,
                    id,
                    true,
                )
            };
            assert_eq!(pi(Resume::No), vec!["--session-id", &id.to_string()]);
            assert_eq!(
                pi(Resume::Conversation("conv".into())),
                vec!["--session", "conv"]
            );
            assert_eq!(pi(Resume::MostRecent), vec!["--continue"]);
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
                agent_args(&hermes, id, &Resume::No, true, true, None),
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

    /// A real child in its own process group, the shape `kill()` targets.
    #[cfg(unix)]
    fn group_leader_child() -> std::process::Child {
        use std::os::unix::process::CommandExt;
        let mut cmd = std::process::Command::new("sleep");
        cmd.arg("30");
        // SAFETY: setsid in the forked child before exec; no allocation, no
        // locks — the narrow set of things async-signal-safety allows.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        cmd.spawn().expect("spawn a group-leader child")
    }

    #[cfg(unix)]
    fn alive(pid: u32) -> bool {
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    /// The guard's whole reason to exist: a pid whose start time no longer
    /// matches is somebody else, and a kill aimed at it must not fire. On a
    /// machine cycling its pid space in minutes, "must not" is not academic.
    #[test]
    #[cfg(unix)]
    fn a_recycled_pid_is_not_killed() {
        let mut child = group_leader_child();
        let pid = child.id();
        let real = crate::pty::proc_identity::start_time(pid).unwrap();

        let counter = Arc::new(AtomicUsize::new(0));
        let mut handle = test_handle(counter.clone());
        handle.process_id = Some(pid);
        handle.spawned_start = Some(crate::pty::proc_identity::ProcStart { sec: 1, usec: 1 });
        // Forged identity: to the handle, this pid belongs to someone else.
        assert_ne!(handle.spawned_start, Some(real));

        handle.kill().unwrap();
        assert!(alive(pid), "an innocent process group was signalled");
        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "the fallback killer must not fire on an unvouched pid either"
        );

        let _ = child.kill();
        let _ = child.wait();
    }

    /// And the mirror image: the same path with a truthful identity still
    /// kills — the guard must not turn every kill into a no-op.
    #[test]
    #[cfg(unix)]
    fn our_own_child_is_still_killed() {
        let mut child = group_leader_child();
        let pid = child.id();

        let counter = Arc::new(AtomicUsize::new(0));
        let mut handle = test_handle(counter);
        handle.process_id = Some(pid);
        handle.spawned_start = crate::pty::proc_identity::start_time(pid);

        handle.kill().unwrap();
        // Not `alive(pid)`: a SIGKILLed child is a zombie until reaped, and
        // signal 0 counts zombies as alive. try_wait is the honest question.
        let gone = (0..40).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(25));
            matches!(child.try_wait(), Ok(Some(_)))
        });
        assert!(gone, "a vouched-for child should die");
    }

    fn test_handle(counter: Arc<AtomicUsize>) -> PtyHandle {
        PtyHandle {
            master: Box::new(DummyMaster),
            child_killer: Box::new(TestChildKiller { calls: counter }),
            process_id: None,
            writer: Box::new(io::sink()),
            generation: Uuid::nil(),
            output: Arc::new(Mutex::new(TerminalOutput::new(24, 80, false))),
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            process_tree: None,
            reaped: None,
            cleanup_done: false,
            spawned_start: None,
            label: "test".to_string(),
        }
    }

    #[test]
    fn guarded_input_uses_the_readers_current_screen() {
        let mut handle = test_handle(Arc::new(AtomicUsize::new(0)));
        handle.output.lock().unwrap().process(b"new question");
        assert!(!handle
            .send_input_if_screen_matches(b"1", |screen| screen == "old question")
            .unwrap());
        assert!(handle
            .send_input_if_screen_matches(b"1", |screen| screen == "new question")
            .unwrap());
    }

    #[test]
    fn resizing_a_pty_updates_cursor_replies_as_well_as_kernel_dimensions() {
        let pair = native_pty_system().openpty(PtySize::default()).unwrap();
        let mut handle = test_handle(Arc::new(AtomicUsize::new(0)));
        handle.master = pair.master;
        handle.output.lock().unwrap().process(b"\x1b[24;80H");
        handle.resize(3, 10).unwrap();
        let size = handle.master.get_size().unwrap();
        assert_eq!((size.rows, size.cols), (3, 10));
        assert_eq!(
            handle.output.lock().unwrap().process(b"\x1b[6n").replies,
            b"\x1b[3;10R"
        );
        // An unchanged resize must preserve terminal state as well.
        handle.resize(3, 10).unwrap();
        assert_eq!(
            handle.output.lock().unwrap().process(b"\x1b[6n").replies,
            b"\x1b[3;10R"
        );
    }

    #[test]
    fn reader_writes_split_query_replies_to_the_agent() {
        let (tx, mut rx) = mpsc::channel(10);
        let mut reader: Box<dyn Read + Send> = Box::new(ChunkedReader::new(vec![
            b"\x1b[".to_vec(),
            b"6nabcdefghijkl\x1b[6n".to_vec(),
        ]));
        let mut replies = tempfile::tempfile().unwrap();
        PtyManager::read_pty_output(
            Uuid::new_v4(),
            &mut reader,
            tx,
            Some(replies.try_clone().unwrap()),
            Arc::new(Mutex::new(TerminalOutput::new(24, 10, false))),
        );
        use std::io::Seek;
        replies.rewind().unwrap();
        let mut bytes = Vec::new();
        replies.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"\x1b[1;1R\x1b[2;3R");
        let mut screen = vt100::Parser::new(24, 10, 0);
        while let Ok(Action::PtyOutput(_, data)) = rx.try_recv() {
            screen.process(&data);
        }
        assert_eq!(screen.screen().cursor_position(), (1, 2));
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

    fn run_read(
        session_id: Uuid,
        reader: &mut Box<dyn Read + Send>,
        tx: mpsc::Sender<Action>,
        mut child: Box<dyn Child + Send + Sync>,
    ) {
        PtyManager::read_pty_output(
            session_id,
            reader,
            tx.clone(),
            None,
            Arc::new(Mutex::new(TerminalOutput::new(24, 80, false))),
        );
        report_session_exited(
            &tx,
            session_id,
            Uuid::nil(),
            child.wait().unwrap().exit_code() as i32,
        );
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
            Action::SessionExited(id, _, code) if *id == session_id && *code == 0
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
            Action::SessionExited(id, _, code) if id == session_id && code == 0
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
