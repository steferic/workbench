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
    use super::{agent_shortcut, check_global_keys};
    use crate::app::Action;
    use crate::config::user_config::{AgentConfig, UserConfig};
    use crate::models::AgentType;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// Workspace cycling lives on Alt+Shift+Up/Down (clean CSI encodings in
    /// every terminal); Alt+b / Alt+f — what macOS terminals send for
    /// Option+Left/Right — must pass through untouched so agents keep their
    /// word-jump.
    #[test]
    fn vertical_axis_cycles_and_option_left_right_passes_through() {
        let config = UserConfig::default();

        let alt_shift_up = KeyEvent::new(KeyCode::Up, KeyModifiers::ALT | KeyModifiers::SHIFT);
        let alt_shift_down = KeyEvent::new(KeyCode::Down, KeyModifiers::ALT | KeyModifiers::SHIFT);
        assert!(matches!(
            check_global_keys(&alt_shift_up, &config),
            Some(Action::CyclePrevWorkspace)
        ));
        assert!(matches!(
            check_global_keys(&alt_shift_down, &config),
            Some(Action::CycleNextWorkspace)
        ));

        let alt_up = KeyEvent::new(KeyCode::Up, KeyModifiers::ALT);
        assert!(matches!(
            check_global_keys(&alt_up, &config),
            Some(Action::CyclePrevSession)
        ));

        // Option+Left/Right (as terminals actually send them) go to the agent.
        let alt_b = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT);
        let alt_f = KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT);
        assert!(check_global_keys(&alt_b, &config).is_none());
        assert!(check_global_keys(&alt_f, &config).is_none());

        // Plain Shift+Up (output scroll) must not cycle workspaces.
        let shift_up = KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT);
        assert!(check_global_keys(&shift_up, &config).is_none());
    }

    /// Saved configs with the old Alt-Left/Right workspace bindings migrate
    /// to the new vertical pair on load.
    #[test]
    fn alt_left_right_configs_migrate_to_alt_shift_vertical() {
        let mut hotkeys = std::collections::HashMap::new();
        hotkeys.insert("CyclePrevWorkspace".to_string(), "Alt-Left".to_string());
        hotkeys.insert("CycleNextWorkspace".to_string(), "Alt-Right".to_string());
        crate::config::user_config::normalize_global_hotkeys(&mut hotkeys);
        assert_eq!(
            hotkeys.get("CyclePrevWorkspace").map(String::as_str),
            Some("Alt-Shift-Up")
        );
        assert_eq!(
            hotkeys.get("CycleNextWorkspace").map(String::as_str),
            Some("Alt-Shift-Down")
        );
    }

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
