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
    #[serde(default = "default_scrollback_mb")]
    pub scrollback_mb: usize,

    // Legacy fields — ignored on load, derived from scrollback_mb
    #[serde(skip)]
    pub scrollback_buffer_kb: usize,
    #[serde(skip)]
    pub replay_parser_rows: u16,
    #[serde(skip)]
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
    m.insert("TestToast".into(), "F2".into());
    m.insert("ToggleDebugOverlay".into(), "F11".into());
    m.insert("EnterConfigWindow".into(), "F1".into());
    m
}

fn default_scrollback_mb() -> usize { 2 }

impl UserConfig {
    /// Derive the internal scrollback parameters from the single `scrollback_mb` value.
    /// Call this after loading config from disk.
    pub fn apply_scrollback_derived(&mut self) {
        let mb = self.scrollback_mb.clamp(1, 16);
        self.scrollback_mb = mb;
        // Raw buffer: direct MB to KB conversion
        self.scrollback_buffer_kb = mb * 1024;
        // Replay parser rows: scale proportionally (1 MB = 500 rows)
        self.replay_parser_rows = (mb as u16 * 500).clamp(500, 8000);
        // Live scrollback rows: scale proportionally (1 MB = 200 rows)
        self.live_scrollback_rows = (mb * 200).clamp(200, 4000);
    }
}

impl Default for UserConfig {
    fn default() -> Self {
        let mut config = Self {
            agents: default_agents(),
            global_hotkeys: default_global_hotkeys(),
            scrollback_mb: default_scrollback_mb(),
            scrollback_buffer_kb: 0,
            replay_parser_rows: 0,
            live_scrollback_rows: 0,
        };
        config.apply_scrollback_derived();
        config
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
    let mut config = match user_config_path() {
        Ok(path) if path.exists() => {
            fs::read_to_string(&path)
                .ok()
                .and_then(|s| toml::from_str(&s).ok())
                .unwrap_or_default()
        }
        _ => UserConfig::default(),
    };
    // Merge any new default hotkeys that aren't in the saved config
    for (action, key) in default_global_hotkeys() {
        config.global_hotkeys.entry(action).or_insert(key);
    }
    config.apply_scrollback_derived();
    config
}

pub fn save_user_config(config: &UserConfig) -> anyhow::Result<()> {
    let path = user_config_path()?;
    let contents = toml::to_string_pretty(config)?;
    fs::write(&path, contents)?;
    Ok(())
}
