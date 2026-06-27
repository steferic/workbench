use crate::models::{Session, SessionStatus, Workspace};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

/// Schema version of `state.json`. Bump this whenever the on-disk format
/// gains a required field or otherwise becomes incompatible with the prior
/// shape, and add a migration arm in `migrate_state`. Files without a
/// `version` field on disk are treated as version 1 (the original shape).
pub const STATE_SCHEMA_VERSION: u32 = 1;

fn default_state_version() -> u32 {
    1
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PersistedState {
    #[serde(default = "default_state_version")]
    pub version: u32,
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

    #[serde(default = "default_agent_done_sound")]
    pub agent_done_sound_enabled: bool,

    #[serde(default)]
    pub theme_mode: crate::theme::ThemeMode,
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

fn default_agent_done_sound() -> bool {
    true
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
            agent_done_sound_enabled: default_agent_done_sound(),
            theme_mode: crate::theme::ThemeMode::default(),
        }
    }
}

impl PersistedState {
    pub fn new() -> Self {
        Self {
            version: STATE_SCHEMA_VERSION,
            workspaces: Vec::new(),
            sessions: HashMap::new(),
            notepad_content: HashMap::new(),
        }
    }
}

/// Migrate a raw JSON value written by an older schema version up to the
/// current shape. Currently a no-op (we only have v1), but future bumps add
/// arms here. Files saved by a *newer* version than we know about are
/// rejected — silently downgrading would lose fields the user's data depends
/// on.
fn migrate_state(mut value: serde_json::Value, from_version: u32) -> Result<serde_json::Value> {
    if from_version > STATE_SCHEMA_VERSION {
        return Err(anyhow!(
            "state.json was written by a newer workbench (schema v{from_version}); \
             refusing to load to avoid data loss"
        ));
    }

    // Future migrations slot in here, e.g.:
    //   if from_version < 2 { value = migrate_v1_to_v2(value)?; }

    // Stamp the current version so the in-memory value is consistent.
    if let serde_json::Value::Object(ref mut map) = value {
        map.insert(
            "version".to_string(),
            serde_json::Value::from(STATE_SCHEMA_VERSION),
        );
    }
    Ok(value)
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

    match load_inner(&path) {
        Ok(state) => Ok(state),
        Err(err) => {
            // Preserve the unreadable file instead of letting the next save
            // overwrite it. This protects users from silent data loss when a
            // schema bump or disk corruption breaks `load`.
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
            let backup = path.with_file_name(format!("state.json.corrupt-{stamp}"));
            if let Err(rename_err) = fs::rename(&path, &backup) {
                crate::logger::warn(format!(
                    "could not back up corrupt state.json to {}: {rename_err}",
                    backup.display()
                ));
            } else {
                crate::logger::warn(format!(
                    "state.json failed to load ({err}); preserved at {}",
                    backup.display()
                ));
            }
            Err(err)
        }
    }
}

fn load_inner(path: &PathBuf) -> Result<PersistedState> {
    let contents = fs::read_to_string(path)?;

    // Peek at the version field before fully deserializing so we can refuse
    // forward-version files explicitly and run migrations for backward ones.
    let raw: serde_json::Value = serde_json::from_str(&contents)?;
    let from_version = raw
        .get("version")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(1);
    let migrated = migrate_state(raw, from_version)?;
    let mut state: PersistedState = serde_json::from_value(migrated)?;

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
            let session_exists = state
                .sessions
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

/// Borrowing view of state for serialization (avoids cloning all data on save)
#[derive(Serialize)]
struct PersistedStateRef<'a> {
    version: u32,
    workspaces: &'a [Workspace],
    sessions: &'a HashMap<Uuid, Vec<Session>>,
    notepad_content: &'a HashMap<Uuid, String>,
}

pub fn save(workspaces: &[Workspace], sessions: &HashMap<Uuid, Vec<Session>>) -> Result<()> {
    static EMPTY: std::sync::LazyLock<HashMap<Uuid, String>> =
        std::sync::LazyLock::new(HashMap::new);
    save_with_notepad(workspaces, sessions, &EMPTY)
}

pub fn save_with_notepad(
    workspaces: &[Workspace],
    sessions: &HashMap<Uuid, Vec<Session>>,
    notepad_content: &HashMap<Uuid, String>,
) -> Result<()> {
    let path = config_path()?;

    let state = PersistedStateRef {
        version: STATE_SCHEMA_VERSION,
        workspaces,
        sessions,
        notepad_content,
    };

    let file = fs::File::create(&path)?;
    let writer = std::io::BufWriter::new(file);
    serde_json::to_writer_pretty(writer, &state)?;

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
