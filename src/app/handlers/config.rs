use crate::app::{Action, AppState, ConfigTab};
use crate::config::user_config::{AgentConfig, save_user_config, UserConfig};
use crossterm::event::KeyEvent;

pub fn handle_config_action(state: &mut AppState, action: Action) {
    match action {
        Action::ConfigSwitchTab(tab) => {
            state.ui.config_tab = tab;
            state.ui.config_selected_row = 0;
            state.ui.config_selected_col = 0;
            state.ui.config_editing = false;
            state.ui.config_rebinding = false;
            state.ui.config_scroll_offset = 0;
        }
        Action::ConfigMoveDown => {
            if state.ui.config_tab == ConfigTab::QuickRef {
                state.ui.config_scroll_offset += 1;
            } else {
                let max = max_rows(state);
                if max > 0 && state.ui.config_selected_row + 1 < max {
                    state.ui.config_selected_row += 1;
                }
            }
        }
        Action::ConfigMoveUp => {
            if state.ui.config_tab == ConfigTab::QuickRef {
                if state.ui.config_scroll_offset > 0 {
                    state.ui.config_scroll_offset -= 1;
                }
            } else if state.ui.config_selected_row > 0 {
                state.ui.config_selected_row -= 1;
            }
        }
        Action::ConfigMoveRight => {
            if state.ui.config_tab == ConfigTab::Agents
                && state.ui.config_selected_col < 3 {
                    state.ui.config_selected_col += 1;
                }
        }
        Action::ConfigMoveLeft => {
            if state.ui.config_selected_col > 0 {
                state.ui.config_selected_col -= 1;
            }
        }
        Action::ConfigStartEdit => {
            match state.ui.config_tab {
                ConfigTab::QuickRef => {}
                ConfigTab::Agents => {
                    if let Some(agent) = state.system.user_config.agents.get(state.ui.config_selected_row) {
                        state.ui.config_edit_buffer = match state.ui.config_selected_col {
                            0 => agent.hotkey.clone(),
                            1 => agent.display_name.clone(),
                            2 => agent.command.clone(),
                            3 => agent.badge.clone(),
                            _ => String::new(),
                        };
                        state.ui.config_editing = true;
                    }
                }
                ConfigTab::Hotkeys => {
                    // Start listening for key rebind
                    state.ui.config_rebinding = true;
                }
                ConfigTab::Scrollback => {
                    state.ui.config_edit_buffer = state.system.user_config.scrollback_mb.to_string();
                    state.ui.config_editing = true;
                }
            }
        }
        Action::ConfigFinishEdit => {
            let buf = state.ui.config_edit_buffer.clone();
            match state.ui.config_tab {
                ConfigTab::Agents => {
                    if let Some(agent) = state.system.user_config.agents.get_mut(state.ui.config_selected_row) {
                        match state.ui.config_selected_col {
                            0 => agent.hotkey = buf,
                            1 => agent.display_name = buf,
                            2 => agent.command = buf,
                            3 => {
                                if !buf.is_empty() {
                                    agent.badge = buf.chars().next().unwrap().to_string();
                                }
                            }
                            _ => {}
                        }
                    }
                }
                ConfigTab::Scrollback => {
                    if let Ok(val) = buf.parse::<usize>() {
                        state.system.user_config.scrollback_mb = val.clamp(1, 16);
                        state.system.user_config.apply_scrollback_derived();
                    }
                }
                _ => {}
            }
            state.ui.config_editing = false;
            state.ui.config_edit_buffer.clear();
            let _ = save_user_config(&state.system.user_config);
        }
        Action::ConfigCancelEdit => {
            state.ui.config_editing = false;
            state.ui.config_rebinding = false;
            state.ui.config_edit_buffer.clear();
        }
        Action::ConfigInputChar(c) => {
            state.ui.config_edit_buffer.push(c);
        }
        Action::ConfigInputBackspace => {
            state.ui.config_edit_buffer.pop();
        }
        Action::ConfigAddAgent => {
            if state.ui.config_tab == ConfigTab::Agents {
                let next_key = (state.system.user_config.agents.len() + 1).to_string();
                state.system.user_config.agents.push(AgentConfig {
                    command: "agent".into(),
                    display_name: "New Agent".into(),
                    badge: "N".into(),
                    hotkey: next_key,
                    enabled: true,
                });
                state.ui.config_selected_row = state.system.user_config.agents.len() - 1;
                let _ = save_user_config(&state.system.user_config);
            }
        }
        Action::ConfigDeleteAgent => {
            if state.ui.config_tab == ConfigTab::Agents && !state.system.user_config.agents.is_empty() {
                let row = state.ui.config_selected_row.min(state.system.user_config.agents.len() - 1);
                state.system.user_config.agents.remove(row);
                if state.ui.config_selected_row >= state.system.user_config.agents.len() && state.ui.config_selected_row > 0 {
                    state.ui.config_selected_row -= 1;
                }
                let _ = save_user_config(&state.system.user_config);
            }
        }
        Action::ConfigReorderUp => {
            if state.ui.config_tab == ConfigTab::Agents && state.ui.config_selected_row > 0 {
                let row = state.ui.config_selected_row;
                state.system.user_config.agents.swap(row, row - 1);
                state.ui.config_selected_row -= 1;
                let _ = save_user_config(&state.system.user_config);
            }
        }
        Action::ConfigReorderDown => {
            if state.ui.config_tab == ConfigTab::Agents {
                let row = state.ui.config_selected_row;
                if row + 1 < state.system.user_config.agents.len() {
                    state.system.user_config.agents.swap(row, row + 1);
                    state.ui.config_selected_row += 1;
                    let _ = save_user_config(&state.system.user_config);
                }
            }
        }
        Action::ConfigRebindKey(key_event) => {
            handle_rebind(state, key_event);
        }
        Action::ConfigResetDefault => {
            let defaults = UserConfig::default();
            match state.ui.config_tab {
                ConfigTab::QuickRef => {}
                ConfigTab::Agents => {
                    state.system.user_config.agents = defaults.agents;
                    state.ui.config_selected_row = 0;
                }
                ConfigTab::Hotkeys => {
                    state.system.user_config.global_hotkeys = defaults.global_hotkeys;
                }
                ConfigTab::Scrollback => {
                    state.system.user_config.scrollback_mb = defaults.scrollback_mb;
                    state.system.user_config.apply_scrollback_derived();
                }
            }
            let _ = save_user_config(&state.system.user_config);
        }
        _ => {}
    }
}

fn max_rows(state: &AppState) -> usize {
    match state.ui.config_tab {
        ConfigTab::QuickRef => 0,
        ConfigTab::Agents => state.system.user_config.agents.len(),
        ConfigTab::Hotkeys => state.system.user_config.global_hotkeys.len(),
        ConfigTab::Scrollback => 1,
    }
}

fn handle_rebind(state: &mut AppState, key: KeyEvent) {
    use crate::config::keybindings::KeyCombo;
    let combo = KeyCombo::new(key.code, key.modifiers);
    let key_str = combo.display();

    // Get sorted hotkey list to find which action is selected
    let mut actions: Vec<String> = state.system.user_config.global_hotkeys.keys().cloned().collect();
    actions.sort();

    if let Some(action) = actions.get(state.ui.config_selected_row) {
        state.system.user_config.global_hotkeys.insert(action.clone(), key_str);
        let _ = save_user_config(&state.system.user_config);
    }

    state.ui.config_rebinding = false;
}
