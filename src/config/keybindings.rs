use crossterm::event::{KeyCode, KeyModifiers};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Default keybindings embedded at compile time
const DEFAULT_KEYBINDINGS: &str = include_str!("defaults.toml");

/// A key combination (key code + modifiers)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyCombo {
    pub fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    /// Parse a key string like "Ctrl-c", "Shift-Tab", "Enter", "`"
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let mut modifiers = KeyModifiers::NONE;
        let mut parts: Vec<&str> = s.split('-').collect();

        // Handle special case of single dash key
        if s == "-" {
            return Some(Self::new(KeyCode::Char('-'), KeyModifiers::NONE));
        }

        // Process modifiers (all but the last part)
        while parts.len() > 1 {
            let modifier = parts.remove(0).to_lowercase();
            match modifier.as_str() {
                "ctrl" | "c" => modifiers |= KeyModifiers::CONTROL,
                "alt" | "a" | "opt" | "option" => modifiers |= KeyModifiers::ALT,
                "shift" | "s" => modifiers |= KeyModifiers::SHIFT,
                "super" | "cmd" | "command" | "meta" => modifiers |= KeyModifiers::SUPER,
                _ => return None, // Unknown modifier
            }
        }

        // Parse the key code (last part)
        let key_str = parts[0];
        let code = parse_key_code(key_str)?;

        Some(Self::new(code, modifiers))
    }

    /// Convert to display string for UI
    pub fn display(&self) -> String {
        let mut parts = Vec::new();

        if self.modifiers.contains(KeyModifiers::SUPER) {
            parts.push("Cmd");
        }
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            parts.push("Ctrl");
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            parts.push("Alt");
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            parts.push("Shift");
        }

        let key_name = key_code_display(&self.code);
        parts.push(&key_name);

        parts.join("-")
    }
}

fn parse_key_code(s: &str) -> Option<KeyCode> {
    // Handle single character keys
    if s.len() == 1 {
        let c = s.chars().next()?;
        return Some(KeyCode::Char(c));
    }

    // Handle special keys (case insensitive)
    match s.to_lowercase().as_str() {
        "enter" | "return" => Some(KeyCode::Enter),
        "esc" | "escape" => Some(KeyCode::Esc),
        "tab" => Some(KeyCode::Tab),
        "backtab" => Some(KeyCode::BackTab),
        "backspace" | "bs" => Some(KeyCode::Backspace),
        "delete" | "del" => Some(KeyCode::Delete),
        "insert" | "ins" => Some(KeyCode::Insert),
        "up" => Some(KeyCode::Up),
        "down" => Some(KeyCode::Down),
        "left" => Some(KeyCode::Left),
        "right" => Some(KeyCode::Right),
        "home" => Some(KeyCode::Home),
        "end" => Some(KeyCode::End),
        "pageup" | "pgup" => Some(KeyCode::PageUp),
        "pagedown" | "pgdn" => Some(KeyCode::PageDown),
        "space" => Some(KeyCode::Char(' ')),
        "backtick" => Some(KeyCode::Char('`')),
        "tilde" => Some(KeyCode::Char('~')),
        "f1" => Some(KeyCode::F(1)),
        "f2" => Some(KeyCode::F(2)),
        "f3" => Some(KeyCode::F(3)),
        "f4" => Some(KeyCode::F(4)),
        "f5" => Some(KeyCode::F(5)),
        "f6" => Some(KeyCode::F(6)),
        "f7" => Some(KeyCode::F(7)),
        "f8" => Some(KeyCode::F(8)),
        "f9" => Some(KeyCode::F(9)),
        "f10" => Some(KeyCode::F(10)),
        "f11" => Some(KeyCode::F(11)),
        "f12" => Some(KeyCode::F(12)),
        _ => None,
    }
}

fn key_code_display(code: &KeyCode) -> String {
    match code {
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::BackTab => "BackTab".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::Insert => "Insert".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        KeyCode::F(n) => format!("F{}", n),
        _ => "?".to_string(),
    }
}

/// The action name as a string (matches Action enum variants)
pub type ActionName = String;

/// Raw TOML structure for keybindings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KeybindingsToml {
    #[serde(default)]
    pub global: HashMap<String, String>,

    #[serde(default)]
    pub mode: ModeBindings,

    #[serde(default)]
    pub panel: PanelBindings,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModeBindings {
    #[serde(default)]
    pub help: HashMap<String, String>,
    #[serde(default)]
    pub workspace_action: HashMap<String, String>,
    #[serde(default)]
    pub workspace_name: HashMap<String, String>,
    #[serde(default)]
    pub create_workspace: HashMap<String, String>,
    #[serde(default)]
    pub create_session: HashMap<String, String>,
    #[serde(default)]
    pub start_command: HashMap<String, String>,
    #[serde(default)]
    pub create_todo: HashMap<String, String>,
    #[serde(default)]
    pub parallel_task: HashMap<String, String>,
    #[serde(default)]
    pub confirm_merge: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PanelBindings {
    #[serde(default)]
    pub workspace_list: HashMap<String, String>,
    #[serde(default)]
    pub session_list: HashMap<String, String>,
    #[serde(default)]
    pub todos_pane: HashMap<String, String>,
    #[serde(default)]
    pub utilities_pane: HashMap<String, String>,
    #[serde(default)]
    pub output_pane: HashMap<String, String>,
    #[serde(default)]
    pub pinned_terminal: HashMap<String, String>,
}

/// Parsed keybinding configuration with KeyCombo lookups
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct KeybindingConfig {
    /// Global bindings (work in any panel/mode)
    pub global: HashMap<KeyCombo, ActionName>,

    /// Mode-specific bindings (for future use when event handler is refactored)
    pub mode_help: HashMap<KeyCombo, ActionName>,
    pub mode_workspace_action: HashMap<KeyCombo, ActionName>,
    pub mode_workspace_name: HashMap<KeyCombo, ActionName>,
    pub mode_create_workspace: HashMap<KeyCombo, ActionName>,
    pub mode_create_session: HashMap<KeyCombo, ActionName>,
    pub mode_start_command: HashMap<KeyCombo, ActionName>,
    pub mode_create_todo: HashMap<KeyCombo, ActionName>,
    pub mode_parallel_task: HashMap<KeyCombo, ActionName>,
    pub mode_confirm_merge: HashMap<KeyCombo, ActionName>,

    /// Panel-specific bindings (when in Normal mode)
    pub panel_workspace_list: HashMap<KeyCombo, ActionName>,
    pub panel_session_list: HashMap<KeyCombo, ActionName>,
    pub panel_todos_pane: HashMap<KeyCombo, ActionName>,
    pub panel_utilities_pane: HashMap<KeyCombo, ActionName>,
    pub panel_output_pane: HashMap<KeyCombo, ActionName>,
    pub panel_pinned_terminal: HashMap<KeyCombo, ActionName>,
}

impl Default for KeybindingConfig {
    fn default() -> Self {
        load_keybindings()
    }
}

impl KeybindingConfig {
    /// Parse a HashMap<String, String> into HashMap<KeyCombo, ActionName>
    fn parse_bindings(raw: &HashMap<String, String>) -> HashMap<KeyCombo, ActionName> {
        raw.iter()
            .filter_map(|(key, action)| {
                KeyCombo::parse(key).map(|combo| (combo, action.clone()))
            })
            .collect()
    }
}

/// Load keybindings from user config, falling back to defaults
pub fn load_keybindings() -> KeybindingConfig {
    // Try to load user config first
    let user_config_path = get_user_config_path();

    let toml_content = if user_config_path.exists() {
        std::fs::read_to_string(&user_config_path).unwrap_or_else(|_| DEFAULT_KEYBINDINGS.to_string())
    } else {
        // Create user config directory and file with defaults
        if let Some(parent) = user_config_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&user_config_path, DEFAULT_KEYBINDINGS);
        DEFAULT_KEYBINDINGS.to_string()
    };

    let raw: KeybindingsToml = toml::from_str(&toml_content).unwrap_or_default();

    KeybindingConfig {
        global: KeybindingConfig::parse_bindings(&raw.global),

        mode_help: KeybindingConfig::parse_bindings(&raw.mode.help),
        mode_workspace_action: KeybindingConfig::parse_bindings(&raw.mode.workspace_action),
        mode_workspace_name: KeybindingConfig::parse_bindings(&raw.mode.workspace_name),
        mode_create_workspace: KeybindingConfig::parse_bindings(&raw.mode.create_workspace),
        mode_create_session: KeybindingConfig::parse_bindings(&raw.mode.create_session),
        mode_start_command: KeybindingConfig::parse_bindings(&raw.mode.start_command),
        mode_create_todo: KeybindingConfig::parse_bindings(&raw.mode.create_todo),
        mode_parallel_task: KeybindingConfig::parse_bindings(&raw.mode.parallel_task),
        mode_confirm_merge: KeybindingConfig::parse_bindings(&raw.mode.confirm_merge),

        panel_workspace_list: KeybindingConfig::parse_bindings(&raw.panel.workspace_list),
        panel_session_list: KeybindingConfig::parse_bindings(&raw.panel.session_list),
        panel_todos_pane: KeybindingConfig::parse_bindings(&raw.panel.todos_pane),
        panel_utilities_pane: KeybindingConfig::parse_bindings(&raw.panel.utilities_pane),
        panel_output_pane: KeybindingConfig::parse_bindings(&raw.panel.output_pane),
        panel_pinned_terminal: KeybindingConfig::parse_bindings(&raw.panel.pinned_terminal),
    }
}

/// Get the path to user's keybindings config file
pub fn get_user_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("workbench")
        .join("keybindings.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_key() {
        let combo = KeyCombo::parse("j").unwrap();
        assert_eq!(combo.code, KeyCode::Char('j'));
        assert_eq!(combo.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn test_parse_special_keys() {
        assert_eq!(KeyCombo::parse("Enter").unwrap().code, KeyCode::Enter);
        assert_eq!(KeyCombo::parse("Esc").unwrap().code, KeyCode::Esc);
        assert_eq!(KeyCombo::parse("Tab").unwrap().code, KeyCode::Tab);
        assert_eq!(KeyCombo::parse("Space").unwrap().code, KeyCode::Char(' '));
    }

    #[test]
    fn test_parse_with_modifiers() {
        let combo = KeyCombo::parse("Ctrl-c").unwrap();
        assert_eq!(combo.code, KeyCode::Char('c'));
        assert!(combo.modifiers.contains(KeyModifiers::CONTROL));

        let combo = KeyCombo::parse("Shift-Tab").unwrap();
        assert_eq!(combo.code, KeyCode::Tab);
        assert!(combo.modifiers.contains(KeyModifiers::SHIFT));
    }

    #[test]
    fn test_parse_multiple_modifiers() {
        let combo = KeyCombo::parse("Ctrl-Shift-a").unwrap();
        assert_eq!(combo.code, KeyCode::Char('a'));
        assert!(combo.modifiers.contains(KeyModifiers::CONTROL));
        assert!(combo.modifiers.contains(KeyModifiers::SHIFT));
    }

    #[test]
    fn test_parse_backtick() {
        let combo = KeyCombo::parse("`").unwrap();
        assert_eq!(combo.code, KeyCode::Char('`'));

        let combo = KeyCombo::parse("backtick").unwrap();
        assert_eq!(combo.code, KeyCode::Char('`'));
    }

    #[test]
    fn test_parse_function_keys() {
        assert_eq!(KeyCombo::parse("F1").unwrap().code, KeyCode::F(1));
        assert_eq!(KeyCombo::parse("F12").unwrap().code, KeyCode::F(12));
    }

    #[test]
    fn test_display() {
        let combo = KeyCombo::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(combo.display(), "Ctrl-c");

        let combo = KeyCombo::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(combo.display(), "Enter");
    }
}
