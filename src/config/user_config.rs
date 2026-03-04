use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub command: String,
    pub display_name: String,
    pub badge: String,
    pub hotkey: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    #[serde(default = "default_agents")]
    pub agents: Vec<AgentConfig>,
    #[serde(default = "default_global_hotkeys")]
    pub global_hotkeys: HashMap<String, String>,
    #[serde(default = "default_scrollback_buffer_kb")]
    pub scrollback_buffer_kb: usize,
    #[serde(default = "default_replay_parser_rows")]
    pub replay_parser_rows: u16,
    #[serde(default = "default_live_scrollback_rows")]
    pub live_scrollback_rows: usize,
}

fn default_agents() -> Vec<AgentConfig> {
    vec![
        AgentConfig { command: "claude".into(), display_name: "Claude".into(), badge: "C".into(), hotkey: "1".into(), enabled: true },
        AgentConfig { command: "gemini".into(), display_name: "Gemini".into(), badge: "G".into(), hotkey: "2".into(), enabled: true },
        AgentConfig { command: "codex".into(), display_name: "Codex".into(), badge: "X".into(), hotkey: "3".into(), enabled: true },
        AgentConfig { command: "grok".into(), display_name: "Grok".into(), badge: "K".into(), hotkey: "4".into(), enabled: true },
    ]
}

fn default_global_hotkeys() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("CycleNextWorkspace".into(), "Ctrl-z".into());
    m.insert("CycleNextSession".into(), "Ctrl-x".into());
    m.insert("InitiateQuit".into(), "Ctrl-q".into());
    m.insert("ToggleDebugOverlay".into(), "F11".into());
    m.insert("EnterConfigWindow".into(), "F1".into());
    m
}

fn default_scrollback_buffer_kb() -> usize { 512 }
fn default_replay_parser_rows() -> u16 { 500 }
fn default_live_scrollback_rows() -> usize { 200 }

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            agents: default_agents(),
            global_hotkeys: default_global_hotkeys(),
            scrollback_buffer_kb: default_scrollback_buffer_kb(),
            replay_parser_rows: default_replay_parser_rows(),
            live_scrollback_rows: default_live_scrollback_rows(),
        }
    }
}

fn user_config_path() -> anyhow::Result<PathBuf> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?
        .join("workbench");
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)?;
    }
    Ok(config_dir.join("user_config.toml"))
}

pub fn load_user_config() -> UserConfig {
    match user_config_path() {
        Ok(path) if path.exists() => {
            fs::read_to_string(&path)
                .ok()
                .and_then(|s| toml::from_str(&s).ok())
                .unwrap_or_default()
        }
        _ => UserConfig::default(),
    }
}

pub fn save_user_config(config: &UserConfig) -> anyhow::Result<()> {
    let path = user_config_path()?;
    let contents = toml::to_string_pretty(config)?;
    fs::write(&path, contents)?;
    Ok(())
}
