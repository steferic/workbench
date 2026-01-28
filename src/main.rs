mod app;
mod audio;
mod config;
mod git;
mod models;
mod persistence;
mod pty;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use app::run_tui;

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
        None => {
            run_tui(cli.workspace).await?;
        }
    }

    Ok(())
}
