use crate::models::{Session, SessionStatus, Workspace};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct PersistedState {
    pub workspaces: Vec<Workspace>,
    pub sessions: HashMap<Uuid, Vec<Session>>,
    #[serde(default)]
    pub notepad_content: HashMap<Uuid, String>, // workspace_id -> notepad text
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    #[serde(default = "default_banner_visible")]
    pub banner_visible: bool,

    // Pane ratios (persisted across sessions)
    #[serde(default = "default_left_panel_ratio")]
    pub left_panel_ratio: f32,
    #[serde(default = "default_workspace_ratio")]
    pub workspace_ratio: f32,
    #[serde(default = "default_sessions_ratio")]
    pub sessions_ratio: f32,
    #[serde(default = "default_todos_ratio")]
    pub todos_ratio: f32,
    #[serde(default = "default_output_split_ratio")]
    pub output_split_ratio: f32,
}

fn default_banner_visible() -> bool {
    true
}

fn default_left_panel_ratio() -> f32 {
    0.30
}

fn default_workspace_ratio() -> f32 {
    0.40
}

fn default_sessions_ratio() -> f32 {
    0.40
}

fn default_todos_ratio() -> f32 {
    0.50
}

fn default_output_split_ratio() -> f32 {
    0.50
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            banner_visible: default_banner_visible(),
            left_panel_ratio: default_left_panel_ratio(),
            workspace_ratio: default_workspace_ratio(),
            sessions_ratio: default_sessions_ratio(),
            todos_ratio: default_todos_ratio(),
            output_split_ratio: default_output_split_ratio(),
        }
    }
}

impl PersistedState {
    pub fn new() -> Self {
        Self {
            workspaces: Vec::new(),
            sessions: HashMap::new(),
            notepad_content: HashMap::new(),
        }
    }
}

fn config_path() -> Result<PathBuf> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?
        .join("workbench");

    // Create directory if it doesn't exist
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)?;
    }

    Ok(config_dir.join("state.json"))
}

pub fn load() -> Result<PersistedState> {
    let path = config_path()?;

    if !path.exists() {
        return Ok(PersistedState::new());
    }

    let contents = fs::read_to_string(&path)?;
    let mut state: PersistedState = serde_json::from_str(&contents)?;

    // Mark all sessions as stopped (PTYs don't survive restart)
    for sessions in state.sessions.values_mut() {
        for session in sessions.iter_mut() {
            if session.status == SessionStatus::Running {
                session.status = SessionStatus::Stopped;
            }
        }
    }

    // Clean up orphaned active_worktree_session_id references
    // (can happen if session was deleted without clearing the workspace reference)
    for workspace in state.workspaces.iter_mut() {
        if let Some(worktree_session_id) = workspace.active_worktree_session_id {
            let session_exists = state.sessions
                .get(&workspace.id)
                .map(|sessions| sessions.iter().any(|s| s.id == worktree_session_id))
                .unwrap_or(false);
            if !session_exists {
                workspace.active_worktree_session_id = None;
            }
        }
    }

    Ok(state)
}

pub fn save(
    workspaces: &[Workspace],
    sessions: &HashMap<Uuid, Vec<Session>>,
) -> Result<()> {
    save_with_notepad(workspaces, sessions, &HashMap::new())
}

pub fn save_with_notepad(
    workspaces: &[Workspace],
    sessions: &HashMap<Uuid, Vec<Session>>,
    notepad_content: &HashMap<Uuid, String>,
) -> Result<()> {
    let path = config_path()?;

    let state = PersistedState {
        workspaces: workspaces.to_vec(),
        sessions: sessions.clone(),
        notepad_content: notepad_content.clone(),
    };

    let contents = serde_json::to_string_pretty(&state)?;
    fs::write(&path, contents)?;

    Ok(())
}

fn global_config_path() -> Result<PathBuf> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?
        .join("workbench");

    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)?;
    }

    Ok(config_dir.join("config.json"))
}

pub fn load_config() -> Result<GlobalConfig> {
    let path = global_config_path()?;

    if !path.exists() {
        return Ok(GlobalConfig::default());
    }

    let contents = fs::read_to_string(&path)?;
    let config: GlobalConfig = serde_json::from_str(&contents)?;
    Ok(config)
}

pub fn save_config(config: &GlobalConfig) -> Result<()> {
    let path = global_config_path()?;
    let contents = serde_json::to_string_pretty(config)?;
    fs::write(&path, contents)?;
    Ok(())
}
