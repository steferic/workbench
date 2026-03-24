use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const GLOBAL_HOTKEY_DEFAULTS: [(&str, &str); 8] = [
    ("CyclePrevWorkspace", "F6"),
    ("CycleNextWorkspace", "F7"),
    ("CyclePrevSession", "F8"),
    ("CycleNextSession", "F9"),
    ("InitiateQuit", "Ctrl-q"),
    ("TestToast", "F2"),
    ("ToggleDebugOverlay", "F11"),
    ("EnterConfigWindow", "F1"),
];

const LEGACY_GLOBAL_HOTKEY_MIGRATIONS: [(&str, &str, &str); 5] = [
    ("CycleNextWorkspace", "Ctrl-z", "F7"),
    ("CyclePrevWorkspace", "Ctrl-Shift-z", "F6"),
    ("CycleNextSession", "Ctrl-x", "F9"),
    ("CycleNextSession", "Ctrl-s", "F9"),
    ("CyclePrevSession", "Ctrl-Shift-s", "F8"),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub command: String,
    pub display_name: String,
    pub badge: String,
    pub hotkey: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

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
        AgentConfig {
            command: "claude".into(),
            display_name: "Claude".into(),
            badge: "C".into(),
            hotkey: "1".into(),
            enabled: true,
        },
        AgentConfig {
            command: "gemini".into(),
            display_name: "Gemini".into(),
            badge: "G".into(),
            hotkey: "2".into(),
            enabled: true,
        },
        AgentConfig {
            command: "codex".into(),
            display_name: "Codex".into(),
            badge: "X".into(),
            hotkey: "3".into(),
            enabled: true,
        },
        AgentConfig {
            command: "grok".into(),
            display_name: "Grok".into(),
            badge: "K".into(),
            hotkey: "4".into(),
            enabled: true,
        },
    ]
}

pub fn global_hotkey_actions() -> &'static [&'static str] {
    &[
        "CyclePrevWorkspace",
        "CycleNextWorkspace",
        "CyclePrevSession",
        "CycleNextSession",
        "InitiateQuit",
        "TestToast",
        "ToggleDebugOverlay",
        "EnterConfigWindow",
    ]
}

pub fn ordered_global_hotkey_actions(hotkeys: &HashMap<String, String>) -> Vec<String> {
    let mut actions: Vec<String> = global_hotkey_actions()
        .iter()
        .filter(|action| hotkeys.contains_key(**action))
        .map(|action| (*action).to_string())
        .collect();

    let mut extras: Vec<String> = hotkeys
        .keys()
        .filter(|action| !global_hotkey_actions().contains(&action.as_str()))
        .cloned()
        .collect();
    extras.sort();
    actions.extend(extras);
    actions
}

fn default_global_hotkeys() -> HashMap<String, String> {
    GLOBAL_HOTKEY_DEFAULTS
        .iter()
        .map(|(action, key)| ((*action).to_string(), (*key).to_string()))
        .collect()
}

fn canonicalize_hotkey(key: &str) -> Option<String> {
    use crate::config::keybindings::KeyCombo;

    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Some(String::new());
    }

    KeyCombo::parse(trimmed).map(|combo| combo.display())
}

fn migrate_legacy_global_hotkeys(hotkeys: &mut HashMap<String, String>) {
    for (action, old_key, new_key) in LEGACY_GLOBAL_HOTKEY_MIGRATIONS {
        if hotkeys
            .get(action)
            .map(|key| key.eq_ignore_ascii_case(old_key))
            .unwrap_or(false)
        {
            hotkeys.insert(action.to_string(), new_key.to_string());
        }
    }
}

pub fn normalize_global_hotkeys(hotkeys: &mut HashMap<String, String>) {
    migrate_legacy_global_hotkeys(hotkeys);

    for (action, default_key) in GLOBAL_HOTKEY_DEFAULTS {
        hotkeys
            .entry(action.to_string())
            .or_insert_with(|| default_key.to_string());
    }

    let mut seen_keys: HashMap<String, String> = HashMap::new();

    for (action, default_key) in GLOBAL_HOTKEY_DEFAULTS {
        let configured_key = hotkeys.get(action).cloned().unwrap_or_default();
        let canonical_key =
            canonicalize_hotkey(&configured_key).unwrap_or_else(|| default_key.to_string());

        if canonical_key.is_empty() {
            hotkeys.insert(action.to_string(), canonical_key);
            continue;
        }

        if seen_keys.contains_key(&canonical_key) {
            hotkeys.insert(action.to_string(), String::new());
            continue;
        }

        seen_keys.insert(canonical_key.clone(), action.to_string());
        hotkeys.insert(action.to_string(), canonical_key);
    }
}

fn default_scrollback_mb() -> usize {
    2
}

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

pub fn get_user_config_path() -> anyhow::Result<PathBuf> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?
        .join("workbench");
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)?;
    }
    Ok(config_dir.join("user_config.toml"))
}

pub fn load_user_config() -> UserConfig {
    let mut config = match get_user_config_path() {
        Ok(path) if path.exists() => fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default(),
        _ => UserConfig::default(),
    };
    normalize_global_hotkeys(&mut config.global_hotkeys);
    config.apply_scrollback_derived();
    config
}

pub fn save_user_config(config: &UserConfig) -> anyhow::Result<()> {
    let path = get_user_config_path()?;
    let mut config_to_save = config.clone();
    normalize_global_hotkeys(&mut config_to_save.global_hotkeys);
    let contents = toml::to_string_pretty(&config_to_save)?;
    fs::write(&path, contents)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_global_hotkeys_migrates_legacy_defaults() {
        let mut hotkeys = HashMap::from([
            ("CycleNextWorkspace".to_string(), "Ctrl-z".to_string()),
            ("CyclePrevWorkspace".to_string(), "Ctrl-Shift-z".to_string()),
            ("CycleNextSession".to_string(), "Ctrl-s".to_string()),
            ("CyclePrevSession".to_string(), "Ctrl-Shift-s".to_string()),
            ("InitiateQuit".to_string(), "Ctrl-q".to_string()),
            ("TestToast".to_string(), "F2".to_string()),
            ("ToggleDebugOverlay".to_string(), "F11".to_string()),
            ("EnterConfigWindow".to_string(), "F1".to_string()),
        ]);

        normalize_global_hotkeys(&mut hotkeys);

        assert_eq!(hotkeys.get("CyclePrevWorkspace"), Some(&"F6".to_string()));
        assert_eq!(hotkeys.get("CycleNextWorkspace"), Some(&"F7".to_string()));
        assert_eq!(hotkeys.get("CyclePrevSession"), Some(&"F8".to_string()));
        assert_eq!(hotkeys.get("CycleNextSession"), Some(&"F9".to_string()));
    }

    #[test]
    fn normalize_global_hotkeys_clears_duplicates_deterministically() {
        let mut hotkeys = default_global_hotkeys();
        hotkeys.insert("CyclePrevSession".to_string(), "F7".to_string());

        normalize_global_hotkeys(&mut hotkeys);

        assert_eq!(hotkeys.get("CycleNextWorkspace"), Some(&"F7".to_string()));
        assert_eq!(hotkeys.get("CyclePrevSession"), Some(&String::new()));
    }
}
