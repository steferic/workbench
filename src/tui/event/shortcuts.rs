use crate::app::Action;
use crate::models::AgentType;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(super) fn check_global_keys(
    key: &KeyEvent,
    user_config: &crate::config::user_config::UserConfig,
) -> Option<Action> {
    use crate::config::keybindings::KeyCombo;

    let pressed = KeyCombo::new(key.code, key.modifiers);
    let pressed_str = pressed.display();

    for action_name in crate::config::user_config::global_hotkey_actions() {
        if let Some(key_str) = user_config.global_hotkeys.get(*action_name) {
            if !key_str.is_empty() && key_str.eq_ignore_ascii_case(&pressed_str) {
                return match *action_name {
                    "CycleNextWorkspace" => Some(Action::CycleNextWorkspace),
                    "CyclePrevWorkspace" => Some(Action::CyclePrevWorkspace),
                    "CycleNextSession" => Some(Action::CycleNextSession),
                    "CyclePrevSession" => Some(Action::CyclePrevSession),
                    "InitiateQuit" => Some(Action::InitiateQuit),
                    "EnterHelpMode" => Some(Action::EnterConfigWindow),
                    "ToggleDebugOverlay" => Some(Action::ToggleDebugOverlay),
                    "EnterConfigWindow" => Some(Action::EnterConfigWindow),
                    "TestToast" => Some(Action::TestToast),
                    _ => None,
                };
            }
        }
    }
    None
}

pub(super) fn agent_shortcut(
    key: &KeyEvent,
    agents: &[crate::config::user_config::AgentConfig],
) -> Option<(AgentType, bool, bool)> {
    if key.modifiers.contains(KeyModifiers::CONTROL)
        || key.modifiers.contains(KeyModifiers::SUPER)
        || key.modifiers.contains(KeyModifiers::META)
    {
        return None;
    }

    let shifted = key.modifiers.contains(KeyModifiers::SHIFT);
    let with_worktree = key.modifiers.contains(KeyModifiers::ALT);

    let key_char = match key.code {
        KeyCode::Char(c) => c.to_string(),
        _ => return None,
    };

    // Map shift+number to the number (e.g. '!' -> "1", '@' -> "2")
    let unshifted = match key_char.as_str() {
        "!" => Some("1"),
        "@" => Some("2"),
        "#" => Some("3"),
        "$" => Some("4"),
        "%" => Some("5"),
        "^" => Some("6"),
        "&" => Some("7"),
        "*" => Some("8"),
        "(" => Some("9"),
        _ => None,
    };

    for agent in agents {
        if !agent.enabled {
            continue;
        }
        let matches =
            agent.hotkey == key_char || unshifted.map(|s| s == agent.hotkey).unwrap_or(false);
        if matches {
            let agent_type = config_to_agent_type(agent);
            let skip_perms = shifted || unshifted.is_some();
            return Some((agent_type, skip_perms, with_worktree));
        }
    }
    None
}

fn config_to_agent_type(agent: &crate::config::user_config::AgentConfig) -> AgentType {
    match agent.command.as_str() {
        "claude" => AgentType::Claude,
        "gemini" => AgentType::Gemini,
        "codex" => AgentType::Codex,
        "grok" => AgentType::Grok,
        _ => AgentType::Custom {
            command: agent.command.clone(),
            display_name: agent.display_name.clone(),
            badge: agent.badge.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::agent_shortcut;
    use crate::config::user_config::AgentConfig;
    use crate::models::AgentType;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn agent(hotkey: &str) -> AgentConfig {
        AgentConfig {
            command: "codex".to_string(),
            display_name: "Codex".to_string(),
            badge: "C".to_string(),
            hotkey: hotkey.to_string(),
            enabled: true,
        }
    }

    #[test]
    fn agent_shortcut_maps_shifted_number_to_hotkey_and_skip_permissions() {
        let key = KeyEvent::new(KeyCode::Char('!'), KeyModifiers::SHIFT);

        let (agent_type, skip_permissions, with_worktree) =
            agent_shortcut(&key, &[agent("1")]).unwrap();

        assert_eq!(agent_type, AgentType::Codex);
        assert!(skip_permissions);
        assert!(!with_worktree);
    }

    #[test]
    fn agent_shortcut_uses_alt_for_worktree() {
        let key = KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT);

        let (_, skip_permissions, with_worktree) = agent_shortcut(&key, &[agent("1")]).unwrap();

        assert!(!skip_permissions);
        assert!(with_worktree);
    }

    #[test]
    fn agent_shortcut_ignores_disabled_agents() {
        let mut disabled = agent("1");
        disabled.enabled = false;
        let key = KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE);

        assert!(agent_shortcut(&key, &[disabled]).is_none());
    }
}
