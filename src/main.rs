mod agent_status;
mod agent_tasks;
mod app;
mod audio;
mod canvas;
mod cli;
mod comms;
mod control;
mod config;
mod git;
mod logger;
mod models;
mod persistence;
mod scrollback;
mod ports;
mod prompt_log;
mod pty;
mod remote;
mod theme;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use app::run_tui;
use config::user_config::load_user_config;

#[derive(Parser)]
#[command(name = "workbench")]
#[command(author = "Stefan Lenoach")]
#[command(version = "0.1.0")]
#[command(about = "TUI for managing AI agent workspaces and sessions")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Start with a specific workspace directory
    #[arg(short, long)]
    workspace: Option<PathBuf>,

    /// Disable alternate screen mode (overrides config setting)
    #[arg(long)]
    no_alt_screen: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a workspace directory
    Add {
        /// Path to the workspace directory
        path: PathBuf,
        /// Custom name for the workspace
        #[arg(short, long)]
        name: Option<String>,
    },
    /// List all workspaces
    List,
    /// List agent sessions in this workspace (agent-to-agent comms)
    Agents,
    /// Print a peer agent's recent conversation transcript
    Transcript {
        /// Target agent: short id, alias, or provider name (if unique)
        target: String,
        /// How many trailing lines to print
        #[arg(long, default_value_t = 200)]
        lines: usize,
        /// Print the entire transcript (can be large)
        #[arg(long)]
        all: bool,
    },
    /// Queue a question for a live peer agent; prints a ticket
    Ask {
        /// Target agent: short id, alias, or provider name (if unique)
        target: String,
        /// The question to deliver
        message: String,
        /// Block until the reply arrives (or timeout)
        #[arg(long)]
        wait: bool,
        /// Timeout in seconds for --wait
        #[arg(long, default_value_t = 600)]
        timeout: u64,
    },
    /// Ask a peer for a structured handoff summary of its work
    Handoff {
        /// Target agent: short id, alias, or provider name (if unique)
        target: String,
        /// Block until the handoff arrives (or timeout)
        #[arg(long)]
        wait: bool,
        /// Timeout in seconds for --wait
        #[arg(long, default_value_t = 600)]
        timeout: u64,
    },
    /// Collect the reply for a consult ticket
    Replies {
        ticket: String,
        /// Block until the reply arrives (or timeout)
        #[arg(long)]
        wait: bool,
        /// Timeout in seconds for --wait
        #[arg(long, default_value_t = 600)]
        timeout: u64,
    },
    /// Set this session's alias for agent-to-agent addressing
    Alias { name: String },
    /// Block until an agent stops working (for scripts and other agents)
    Wait {
        /// Target agent: short id, an unambiguous prefix, or a provider name
        target: String,
        /// Which states count, comma-separated: idle, working, blocked,
        /// stopped. Defaults to any of idle, blocked, stopped — "not working" —
        /// because `--state idle` alone waits forever on a permission prompt.
        #[arg(long)]
        state: Option<String>,
        /// Which project to look in, by name. Only needed from a plain
        /// shell: inside a workbench pane the agent's own project is used.
        #[arg(long)]
        project: Option<String>,
        /// Give up after this long. Exits 3 on timeout, so a script can tell
        /// that apart from a failure.
        #[arg(long, default_value_t = 600)]
        timeout: u64,
        /// Print one JSON object instead of a sentence
        #[arg(long)]
        json: bool,
    },
    /// Analyze messages submitted to agents through Workbench
    Prompts {
        /// How many recent messages to include
        #[arg(long, default_value_t = 30)]
        limit: usize,
        /// Print structured records for custom analysis
        #[arg(long)]
        json: bool,
    },
    /// Report an agent lifecycle event (invoked by the agent's own hooks)
    Hook {
        /// The provider's event name, e.g. `Stop` or `Notification`. Omitted
        /// by providers whose hook command cannot carry arguments (Codex),
        /// where the event is read from the payload instead.
        event: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Add { path, name }) => {
            let abs_path = if path.is_absolute() {
                path
            } else {
                std::env::current_dir()?.join(path)
            };
            println!(
                "Added workspace: {} at {:?}",
                name.unwrap_or_else(|| abs_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string()),
                abs_path
            );
        }
        Some(Commands::List) => {
            println!("Workspaces: (in-memory only, no persistence)");
        }
        Some(Commands::Agents) => cli::cmd_agents()?,
        Some(Commands::Transcript { target, lines, all }) => {
            cli::cmd_transcript(target, lines, all)?
        }
        Some(Commands::Ask {
            target,
            message,
            wait,
            timeout,
        }) => cli::cmd_ask(target, message, wait, timeout)?,
        Some(Commands::Handoff {
            target,
            wait,
            timeout,
        }) => cli::cmd_handoff(target, wait, timeout)?,
        Some(Commands::Replies {
            ticket,
            wait,
            timeout,
        }) => cli::cmd_replies(ticket, wait, timeout)?,
        Some(Commands::Hook { event }) => cli::cmd_hook(event.as_deref()),
        Some(Commands::Alias { name }) => cli::cmd_alias(name)?,
        Some(Commands::Wait {
            target,
            state,
            project,
            timeout,
            json,
        }) => cli::cmd_wait(target, state, project, timeout, json)?,
        Some(Commands::Prompts { limit, json }) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&prompt_log::recent(limit)?)?);
            } else {
                println!("{}", prompt_log::analysis_lines(limit)?.join("\n"));
            }
        }
        None => {
            // Load config to get default, CLI flag overrides
            let config = load_user_config();
            let use_alt_screen = if cli.no_alt_screen {
                false
            } else {
                config.use_alternate_screen
            };
            run_tui(cli.workspace, use_alt_screen).await?;
        }
    }

    Ok(())
}
