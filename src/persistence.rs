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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    #[serde(default = "default_banner_visible")]
    pub banner_visible: bool,
}

fn default_banner_visible() -> bool {
    true
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            banner_visible: true,
        }
    }
}

impl PersistedState {
    pub fn new() -> Self {
        Self {
            workspaces: Vec::new(),
            sessions: HashMap::new(),
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

    Ok(state)
}

pub fn save(workspaces: &[Workspace], sessions: &HashMap<Uuid, Vec<Session>>) -> Result<()> {
    let path = config_path()?;

    let state = PersistedState {
        workspaces: workspaces.to_vec(),
        sessions: sessions.clone(),
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
