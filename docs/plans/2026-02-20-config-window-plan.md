# Config Window Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a configuration window (F11) with three tabs—Agents, Hotkeys, Scrollback—that allows live editing and auto-persists to `~/.config/workbench/user_config.toml`.

**Architecture:** New `UserConfig` struct (TOML-serialized) loaded at startup into `SystemState`. A new `InputMode::ConfigWindow` triggers a modal overlay rendered by a new `config_window.rs` component. Key handling dispatches config-specific actions. Changes are applied immediately and auto-saved.

**Tech Stack:** Rust, ratatui, toml (serde), crossterm

---

### Task 1: Add `UserConfig` and `AgentConfig` data model

**Files:**
- Create: `src/config/user_config.rs`
- Modify: `src/config/mod.rs`

**Step 1: Create `src/config/user_config.rs`**

```rust
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
    m.insert("EnterHelpMode".into(), "F1".into());
    m.insert("ToggleDebugOverlay".into(), "F12".into());
    m.insert("EnterConfigWindow".into(), "F11".into());
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
```

**Step 2: Add `toml` to Cargo.toml dependencies**

Run: `cargo add toml` (if not already present; check first)

**Step 3: Register module in `src/config/mod.rs`**

Add: `pub mod user_config;`

**Step 4: Build to verify**

Run: `cargo build --release 2>&1 | head -20`
Expected: Compiles successfully

**Step 5: Commit**

```bash
git add src/config/user_config.rs src/config/mod.rs Cargo.toml Cargo.lock
git commit -m "feat: add UserConfig data model with TOML persistence"
```

---

### Task 2: Add `AgentType::Custom` variant and wire into existing code

**Files:**
- Modify: `src/models/agent.rs:1-62`
- Modify: `src/app/session_start.rs:22` (inline mode check)

**Step 1: Add Custom variant to AgentType**

In `src/models/agent.rs`, add a new variant to the enum and update all match arms:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentType {
    Claude,
    Gemini,
    Codex,
    Grok,
    Custom { command: String, display_name: String, badge: String },
    Terminal(String),
}
```

Update `command()`:
```rust
AgentType::Custom { command, .. } => command.as_str(),
```

Update `display_name()`:
```rust
AgentType::Custom { display_name, .. } => display_name.clone(),
```

Update `badge()` — needs to return owned String now or use a different approach. Simplest: change return type to `String` (or keep `&str` for known variants and handle Custom specially). Since badge is used in few places, change to return `String`:
```rust
pub fn badge(&self) -> String {
    match self {
        AgentType::Claude => "C".to_string(),
        AgentType::Gemini => "G".to_string(),
        AgentType::Codex => "X".to_string(),
        AgentType::Grok => "K".to_string(),
        AgentType::Custom { badge, .. } => badge.clone(),
        AgentType::Terminal(_) => "T".to_string(),
    }
}
```

**Step 2: Fix all callers of `badge()` that expect `&str`**

Search for `.badge()` usage across codebase and fix any type mismatches (likely just `Span::raw(session.agent_type.badge())` calls — these accept `String` fine via `Into<Cow<str>>`).

**Step 3: Fix inline mode check in `src/app/session_start.rs:22`**

The Codex check `matches!(agent_type, AgentType::Codex)` should also match Custom agents whose command is "codex":
```rust
let inline_mode = matches!(agent_type, AgentType::Codex)
    || matches!(agent_type, AgentType::Custom { command, .. } if command == "codex");
```

**Step 4: Build and verify**

Run: `cargo build --release 2>&1 | head -20`

**Step 5: Commit**

```bash
git add src/models/agent.rs src/app/session_start.rs
git commit -m "feat: add AgentType::Custom variant for user-defined agents"
```

---

### Task 3: Load UserConfig at startup and store in SystemState

**Files:**
- Modify: `src/app/state/system.rs:203-252` (add user_config field)
- Modify: `src/app/mod.rs:14-20` (make constants read from config)
- Modify: wherever `SystemState::new()` is called

**Step 1: Add `user_config` field to SystemState**

In `src/app/state/system.rs`, add to the struct:
```rust
pub user_config: crate::config::user_config::UserConfig,
```

In `SystemState::new()`, add:
```rust
user_config: crate::config::user_config::load_user_config(),
```

**Step 2: Add helper methods to SystemState for scrollback values**

```rust
pub fn raw_output_buffer_capacity(&self) -> usize {
    self.user_config.scrollback_buffer_kb * 1024
}

pub fn replay_parser_rows(&self) -> u16 {
    self.user_config.replay_parser_rows
}

pub fn live_scrollback_rows(&self) -> usize {
    self.user_config.live_scrollback_rows
}
```

**Step 3: Update `create_session_buffers` to use config values**

In `create_session_buffers`, change:
```rust
// Before:
let parser = vt100::Parser::new(PARSER_BUFFER_ROWS, cols, TERMINAL_SCROLLBACK_LIMIT);
self.raw_output_buffers.insert(session_id, RawOutputBuffer::new(RAW_OUTPUT_BUFFER_CAPACITY));

// After:
let parser = vt100::Parser::new(PARSER_BUFFER_ROWS, cols, self.user_config.live_scrollback_rows);
self.raw_output_buffers.insert(session_id, RawOutputBuffer::new(self.raw_output_buffer_capacity()));
```

**Step 4: Update replay parser creation to use config**

Find where `REPLAY_PARSER_ROWS` is used in `src/tui/replay.rs` or `output_pane.rs` and pass `state.system.replay_parser_rows()` instead of the constant.

**Step 5: Build and verify**

Run: `cargo build --release 2>&1 | head -20`

**Step 6: Commit**

```bash
git add src/app/state/system.rs src/app/mod.rs
git commit -m "feat: load UserConfig at startup, use for scrollback settings"
```

---

### Task 4: Add ConfigWindow InputMode, Actions, and UI state

**Files:**
- Modify: `src/app/state/types.rs:12-24` (add ConfigWindow to InputMode)
- Modify: `src/app/action.rs:38-217` (add config actions)
- Modify: `src/app/state/ui.rs:10-108` (add config window state fields)

**Step 1: Add `ConfigWindow` variant to InputMode**

In `src/app/state/types.rs:23`, after `Help,` add:
```rust
ConfigWindow,  // F11 configuration window
```

**Step 2: Add `ConfigTab` enum to types.rs**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfigTab {
    #[default]
    Agents,
    Hotkeys,
    Scrollback,
}
```

**Step 3: Add config actions to Action enum**

In `src/app/action.rs`, after `ToggleDebugOverlay` (line 216), add:
```rust
// Config window
EnterConfigWindow,
ExitConfigWindow,
ConfigSwitchTab(ConfigTab),
ConfigMoveUp,
ConfigMoveDown,
ConfigMoveLeft,
ConfigMoveRight,
ConfigStartEdit,       // Enter on a field
ConfigFinishEdit,      // Enter to confirm edit
ConfigCancelEdit,      // Esc while editing a field
ConfigAddAgent,        // 'a' in agents tab
ConfigDeleteAgent,     // 'd' in agents tab
ConfigReorderUp,       // Shift+k in agents tab
ConfigReorderDown,     // Shift+j in agents tab
ConfigResetDefault,    // 'r' to reset a value
ConfigInputChar(char),
ConfigInputBackspace,
ConfigStartRebind,     // Enter on hotkey field to start listening
ConfigRebindKey(KeyEvent), // Captured key for rebind
```

Import `ConfigTab` at top of action.rs.

**Step 4: Add config state fields to UIState**

In `src/app/state/ui.rs`, add fields:
```rust
// Config window state
pub config_tab: ConfigTab,
pub config_selected_row: usize,
pub config_selected_col: usize,   // For agents tab field selection
pub config_editing: bool,         // Whether we're editing a field value
pub config_edit_buffer: String,   // Buffer for inline editing
pub config_rebinding: bool,       // Whether we're waiting for a key rebind
```

Set defaults in `UIState::new()`:
```rust
config_tab: ConfigTab::default(),
config_selected_row: 0,
config_selected_col: 0,
config_editing: false,
config_edit_buffer: String::new(),
config_rebinding: false,
```

**Step 5: Build and verify**

Run: `cargo build --release 2>&1 | head -20`

**Step 6: Commit**

```bash
git add src/app/state/types.rs src/app/action.rs src/app/state/ui.rs
git commit -m "feat: add ConfigWindow input mode, actions, and UI state"
```

---

### Task 5: Wire F11 key to open config window

**Files:**
- Modify: `src/tui/event/handlers.rs:9-31` (add F11 to global keys)
- Modify: `src/tui/event/handlers.rs:33-176` (add ConfigWindow input mode handler)
- Modify: `src/app/handlers/input.rs` (handle EnterConfigWindow/ExitConfigWindow)

**Step 1: Add F11 to `check_global_keys`**

In `src/tui/event/handlers.rs`, after the F12 check (line 28), add:
```rust
// F11 - Config window
if key.code == KeyCode::F(11) {
    return Some(Action::EnterConfigWindow);
}
```

**Step 2: Add ConfigWindow match arm to `handle_key_event`**

In `src/tui/event/handlers.rs:35`, inside the `match state.ui.input_mode` block, before `InputMode::Normal => {}`, add:

```rust
InputMode::ConfigWindow => {
    // If rebinding a hotkey, capture any key as the new binding
    if state.ui.config_rebinding {
        return Action::ConfigRebindKey(key);
    }
    // If editing a field, handle text input
    if state.ui.config_editing {
        return match key.code {
            KeyCode::Esc => Action::ConfigCancelEdit,
            KeyCode::Enter => Action::ConfigFinishEdit,
            KeyCode::Backspace => Action::ConfigInputBackspace,
            KeyCode::Char(c) => Action::ConfigInputChar(c),
            _ => Action::Tick,
        };
    }
    // Normal config navigation
    return match key.code {
        KeyCode::Esc => Action::ExitConfigWindow,
        KeyCode::Char('1') => Action::ConfigSwitchTab(ConfigTab::Agents),
        KeyCode::Char('2') => Action::ConfigSwitchTab(ConfigTab::Hotkeys),
        KeyCode::Char('3') => Action::ConfigSwitchTab(ConfigTab::Scrollback),
        KeyCode::Tab => {
            // Cycle tabs
            let next = match state.ui.config_tab {
                ConfigTab::Agents => ConfigTab::Hotkeys,
                ConfigTab::Hotkeys => ConfigTab::Scrollback,
                ConfigTab::Scrollback => ConfigTab::Agents,
            };
            Action::ConfigSwitchTab(next)
        }
        KeyCode::Char('j') | KeyCode::Down => Action::ConfigMoveDown,
        KeyCode::Char('k') | KeyCode::Up => Action::ConfigMoveUp,
        KeyCode::Char('h') | KeyCode::Left => Action::ConfigMoveLeft,
        KeyCode::Char('l') | KeyCode::Right => Action::ConfigMoveRight,
        KeyCode::Enter => Action::ConfigStartEdit,
        KeyCode::Char('a') => Action::ConfigAddAgent,
        KeyCode::Char('d') => Action::ConfigDeleteAgent,
        KeyCode::Char('J') => Action::ConfigReorderDown,
        KeyCode::Char('K') => Action::ConfigReorderUp,
        KeyCode::Char('r') => Action::ConfigResetDefault,
        _ => Action::Tick,
    };
}
```

Add the necessary import for `ConfigTab` at the top of the file.

**Step 3: Handle EnterConfigWindow and ExitConfigWindow in input.rs**

In `src/app/handlers/input.rs`, add handling:
```rust
Action::EnterConfigWindow => {
    state.ui.input_mode = InputMode::ConfigWindow;
    state.ui.config_tab = ConfigTab::Agents;
    state.ui.config_selected_row = 0;
    state.ui.config_selected_col = 0;
    state.ui.config_editing = false;
    state.ui.config_rebinding = false;
}
Action::ExitConfigWindow => {
    state.ui.input_mode = InputMode::Normal;
    state.ui.config_editing = false;
    state.ui.config_rebinding = false;
}
```

**Step 4: Build and verify**

Run: `cargo build --release 2>&1 | head -20`

**Step 5: Commit**

```bash
git add src/tui/event/handlers.rs src/app/handlers/input.rs
git commit -m "feat: wire F11 to open config window, handle navigation keys"
```

---

### Task 6: Create config window renderer

**Files:**
- Create: `src/tui/components/config_window.rs`
- Modify: `src/tui/components/mod.rs` (add module)
- Modify: `src/tui/ui.rs:137-170` (render modal)

**Step 1: Create `src/tui/components/config_window.rs`**

This is the largest file. It renders the modal with three tabs. Key structure:

```rust
use crate::app::{AppState, ConfigTab};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Row, Table},
    Frame,
};

pub fn render(frame: &mut Frame, state: &AppState) {
    let area = centered_rect(70, 80, frame.area());
    frame.render_widget(Clear, area);

    // Split: tab bar (3 lines) + content
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);

    render_tab_bar(frame, chunks[0], state);

    match state.ui.config_tab {
        ConfigTab::Agents => render_agents_tab(frame, chunks[1], state),
        ConfigTab::Hotkeys => render_hotkeys_tab(frame, chunks[1], state),
        ConfigTab::Scrollback => render_scrollback_tab(frame, chunks[1], state),
    }
}

fn render_tab_bar(frame: &mut Frame, area: Rect, state: &AppState) {
    // Render "[1: Agents]  2: Hotkeys  3: Scrollback" with active tab highlighted
    let tabs = vec![
        ("1: Agents", ConfigTab::Agents),
        ("2: Hotkeys", ConfigTab::Hotkeys),
        ("3: Scrollback", ConfigTab::Scrollback),
    ];
    let spans: Vec<Span> = tabs.iter().map(|(label, tab)| {
        if *tab == state.ui.config_tab {
            Span::styled(format!(" [{}] ", label), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        } else {
            Span::styled(format!("  {}  ", label), Style::default().fg(Color::DarkGray))
        }
    }).collect();

    let block = Block::default()
        .title(" Settings (F11) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));

    let paragraph = Paragraph::new(Line::from(spans)).block(block);
    frame.render_widget(paragraph, area);
}

fn render_agents_tab(frame: &mut Frame, area: Rect, state: &AppState) {
    // Table of agents from state.system.user_config.agents
    // Highlight selected row, show edit cursor if editing
    // Footer: [j/k] Navigate  [Enter] Edit  [a] Add  [d] Delete  [J/K] Reorder
}

fn render_hotkeys_tab(frame: &mut Frame, area: Rect, state: &AppState) {
    // Table: Action | Key
    // If rebinding, show "Press a key..." on selected row
    // Footer: [j/k] Navigate  [Enter] Rebind  [r] Reset
}

fn render_scrollback_tab(frame: &mut Frame, area: Rect, state: &AppState) {
    // Table: Setting | Value
    // If editing, show input buffer
    // Footer: [j/k] Navigate  [Enter] Edit  [r] Reset
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    // Same as other modals
}
```

Fill in each tab render function with actual table rendering using `Table` widget and the data from `state.system.user_config`.

**Step 2: Register module in `src/tui/components/mod.rs`**

Add: `pub mod config_window;`

**Step 3: Add modal rendering in `src/tui/ui.rs`**

In `src/tui/ui.rs`, add import for `config_window` and add match arm at line ~169:
```rust
InputMode::ConfigWindow => {
    config_window::render(frame, state);
}
```

**Step 4: Build and verify**

Run: `cargo build --release 2>&1 | head -20`

**Step 5: Commit**

```bash
git add src/tui/components/config_window.rs src/tui/components/mod.rs src/tui/ui.rs
git commit -m "feat: add config window renderer with three tabs"
```

---

### Task 7: Implement config action handlers

**Files:**
- Create: `src/app/handlers/config.rs`
- Modify: `src/app/handlers/mod.rs` (add module)
- Modify: `src/app/handler.rs` (dispatch config actions)

**Step 1: Create `src/app/handlers/config.rs`**

```rust
use crate::app::{Action, AppState, ConfigTab};
use crate::config::user_config::{AgentConfig, save_user_config};
use crossterm::event::KeyEvent;

pub fn handle_config_action(state: &mut AppState, action: Action) {
    match action {
        Action::ConfigSwitchTab(tab) => {
            state.ui.config_tab = tab;
            state.ui.config_selected_row = 0;
            state.ui.config_selected_col = 0;
            state.ui.config_editing = false;
            state.ui.config_rebinding = false;
        }
        Action::ConfigMoveDown => {
            let max = max_rows(state);
            if state.ui.config_selected_row + 1 < max {
                state.ui.config_selected_row += 1;
            }
        }
        Action::ConfigMoveUp => {
            if state.ui.config_selected_row > 0 {
                state.ui.config_selected_row -= 1;
            }
        }
        Action::ConfigMoveRight => {
            if state.ui.config_tab == ConfigTab::Agents {
                let max_cols = 4; // hotkey, name, command, badge
                if state.ui.config_selected_col + 1 < max_cols {
                    state.ui.config_selected_col += 1;
                }
            }
        }
        Action::ConfigMoveLeft => {
            if state.ui.config_selected_col > 0 {
                state.ui.config_selected_col -= 1;
            }
        }
        Action::ConfigStartEdit => {
            match state.ui.config_tab {
                ConfigTab::Agents => {
                    // Load current field value into edit buffer
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
                    state.ui.config_rebinding = true;
                }
                ConfigTab::Scrollback => {
                    let val = match state.ui.config_selected_row {
                        0 => state.system.user_config.scrollback_buffer_kb.to_string(),
                        1 => state.system.user_config.replay_parser_rows.to_string(),
                        2 => state.system.user_config.live_scrollback_rows.to_string(),
                        _ => String::new(),
                    };
                    state.ui.config_edit_buffer = val;
                    state.ui.config_editing = true;
                }
            }
        }
        Action::ConfigFinishEdit => {
            match state.ui.config_tab {
                ConfigTab::Agents => {
                    let row = state.ui.config_selected_row;
                    let val = state.ui.config_edit_buffer.clone();
                    if let Some(agent) = state.system.user_config.agents.get_mut(row) {
                        match state.ui.config_selected_col {
                            0 => agent.hotkey = val,
                            1 => agent.display_name = val,
                            2 => agent.command = val,
                            3 => { if !val.is_empty() { agent.badge = val.chars().next().unwrap().to_string(); } }
                            _ => {}
                        }
                    }
                }
                ConfigTab::Scrollback => {
                    if let Ok(val) = state.ui.config_edit_buffer.parse::<usize>() {
                        match state.ui.config_selected_row {
                            0 => state.system.user_config.scrollback_buffer_kb = val.max(64).min(4096),
                            1 => state.system.user_config.replay_parser_rows = (val.max(100).min(2000)) as u16,
                            2 => state.system.user_config.live_scrollback_rows = val.max(50).min(1000),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            state.ui.config_editing = false;
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
            if state.ui.config_tab == ConfigTab::Agents
                && !state.system.user_config.agents.is_empty()
            {
                let row = state.ui.config_selected_row.min(state.system.user_config.agents.len() - 1);
                state.system.user_config.agents.remove(row);
                if state.ui.config_selected_row > 0 && state.ui.config_selected_row >= state.system.user_config.agents.len() {
                    state.ui.config_selected_row = state.system.user_config.agents.len().saturating_sub(1);
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
            // Convert KeyEvent to display string, update the hotkey
            handle_rebind(state, key_event);
        }
        Action::ConfigResetDefault => {
            handle_reset(state);
        }
        _ => {}
    }
}

fn max_rows(state: &AppState) -> usize {
    match state.ui.config_tab {
        ConfigTab::Agents => state.system.user_config.agents.len(),
        ConfigTab::Hotkeys => state.system.user_config.global_hotkeys.len(),
        ConfigTab::Scrollback => 3,
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

fn handle_reset(state: &mut AppState) {
    use crate::config::user_config::UserConfig;
    let defaults = UserConfig::default();

    match state.ui.config_tab {
        ConfigTab::Agents => {
            state.system.user_config.agents = defaults.agents;
        }
        ConfigTab::Hotkeys => {
            state.system.user_config.global_hotkeys = defaults.global_hotkeys;
        }
        ConfigTab::Scrollback => {
            state.system.user_config.scrollback_buffer_kb = defaults.scrollback_buffer_kb;
            state.system.user_config.replay_parser_rows = defaults.replay_parser_rows;
            state.system.user_config.live_scrollback_rows = defaults.live_scrollback_rows;
        }
    }
    let _ = save_user_config(&state.system.user_config);
}
```

**Step 2: Register module in `src/app/handlers/mod.rs`**

Add: `pub mod config;`

**Step 3: Dispatch config actions in `src/app/handler.rs`**

In `process_action()`, add match arms for all config actions. The simplest approach: add a catch-all at the end of the match:
```rust
Action::EnterConfigWindow | Action::ExitConfigWindow
| Action::ConfigSwitchTab(_) | Action::ConfigMoveUp | Action::ConfigMoveDown
| Action::ConfigMoveLeft | Action::ConfigMoveRight | Action::ConfigStartEdit
| Action::ConfigFinishEdit | Action::ConfigCancelEdit | Action::ConfigAddAgent
| Action::ConfigDeleteAgent | Action::ConfigReorderUp | Action::ConfigReorderDown
| Action::ConfigResetDefault | Action::ConfigInputChar(_) | Action::ConfigInputBackspace
| Action::ConfigStartRebind | Action::ConfigRebindKey(_) => {
    config::handle_config_action(state, action);
}
```

Import: `use super::handlers::config;` (if not already imported via the handlers module)

**Step 4: Build and verify**

Run: `cargo build --release 2>&1 | head -20`

**Step 5: Commit**

```bash
git add src/app/handlers/config.rs src/app/handlers/mod.rs src/app/handler.rs
git commit -m "feat: implement config action handlers with auto-save"
```

---

### Task 8: Wire agent creation from UserConfig instead of hardcoded enum

**Files:**
- Modify: `src/tui/event/handlers.rs:820-842` (agent_shortcut reads from config)
- Modify: `src/tui/components/create_session_dialog.rs:10-71` (dynamic agent list)
- Modify: `src/app/state/ui.rs` (parallel_task_agents from config)

**Step 1: Rewrite `agent_shortcut` to read from UserConfig**

In `src/tui/event/handlers.rs`, the `agent_shortcut` function currently hardcodes 1=Claude, 2=Gemini, etc. Change it to accept the user config:

```rust
pub(super) fn agent_shortcut(key: &KeyEvent, agents: &[crate::config::user_config::AgentConfig]) -> Option<(AgentType, bool, bool)> {
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

    // Check shift variants (e.g., '!' for '1')
    let shift_char = match key.code {
        KeyCode::Char('!') => Some("1"),
        KeyCode::Char('@') => Some("2"),
        KeyCode::Char('#') => Some("3"),
        KeyCode::Char('$') => Some("4"),
        KeyCode::Char('%') => Some("5"),
        KeyCode::Char('^') => Some("6"),
        KeyCode::Char('&') => Some("7"),
        KeyCode::Char('*') => Some("8"),
        KeyCode::Char('(') => Some("9"),
        _ => None,
    };

    for agent in agents {
        if !agent.enabled { continue; }
        if agent.hotkey == key_char || shift_char.map(|s| s == agent.hotkey).unwrap_or(false) {
            let agent_type = config_to_agent_type(agent);
            let skip_perms = shifted || shift_char.is_some();
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
```

Update all callers of `agent_shortcut` to pass `&state.system.user_config.agents`.

**Step 2: Update create_session_dialog.rs to render dynamically**

Replace the hardcoded agent list with a loop over `state.system.user_config.agents`:

```rust
pub fn render(frame: &mut Frame, state: &AppState) {
    let agents = &state.system.user_config.agents;
    let line_count = 7 + agents.len(); // header + footer + agent lines
    let height_pct = ((line_count * 100) / frame.area().height as usize).max(25).min(50) as u16;
    let area = centered_rect(40, height_pct, frame.area());
    frame.render_widget(Clear, area);

    let mut content = vec![
        Line::from(""),
        // ... workspace name ...
        Line::from(Span::styled("  Select an agent:", Style::default().fg(Color::White).add_modifier(Modifier::BOLD))),
        Line::from(""),
    ];

    for agent in agents {
        if !agent.enabled { continue; }
        content.push(Line::from(vec![
            Span::styled(format!("  [{}] ", agent.hotkey), Style::default().fg(Color::Cyan)),
            Span::styled(format!("[{}] ", agent.badge), Style::default().fg(Color::Magenta)),
            Span::raw(&agent.display_name),
        ]));
    }

    content.push(Line::from(""));
    content.push(Line::from(Span::styled("  [t] Terminal  |  Press Esc to cancel", Style::default().fg(Color::DarkGray))));

    // ... render block + paragraph as before ...
}
```

**Step 3: Update parallel_task_agents initialization**

In `src/app/state/ui.rs`, where `parallel_task_agents` is initialized (Default impl), read from user config if available, or keep default. This may need to happen when EnterParallelTaskMode is dispatched rather than at UIState construction.

**Step 4: Build and verify**

Run: `cargo build --release 2>&1 | head -20`

**Step 5: Commit**

```bash
git add src/tui/event/handlers.rs src/tui/components/create_session_dialog.rs src/app/state/ui.rs
git commit -m "feat: wire agent creation from UserConfig, dynamic session dialog"
```

---

### Task 9: Wire global hotkeys from UserConfig

**Files:**
- Modify: `src/tui/event/handlers.rs:9-31` (check_global_keys reads from config)

**Step 1: Rewrite `check_global_keys` to use UserConfig**

The current `check_global_keys` hardcodes Ctrl-z, Ctrl-x, etc. Change it to look up from the user config's global_hotkeys map:

```rust
fn check_global_keys(key: &KeyEvent, user_config: &crate::config::user_config::UserConfig) -> Option<Action> {
    use crate::config::keybindings::KeyCombo;
    let pressed = KeyCombo::new(key.code, key.modifiers);
    let pressed_str = pressed.display();

    for (action_name, key_str) in &user_config.global_hotkeys {
        if key_str == &pressed_str {
            return match action_name.as_str() {
                "CycleNextWorkspace" => Some(Action::CycleNextWorkspace),
                "CycleNextSession" => Some(Action::CycleNextSession),
                "InitiateQuit" => Some(Action::InitiateQuit),
                "EnterHelpMode" => Some(Action::EnterHelpMode),
                "ToggleDebugOverlay" => Some(Action::ToggleDebugOverlay),
                "EnterConfigWindow" => Some(Action::EnterConfigWindow),
                _ => None,
            };
        }
    }
    None
}
```

Update all callers to pass `&state.system.user_config`.

**Step 2: Also handle F11 in the output pane's F-key handler**

In `handle_output_pane_keys` (line 658), the F(n) match sends F-keys as terminal sequences. F11 should be intercepted before reaching the terminal:

After the `check_global_keys` call at the top of `handle_output_pane_keys`, add:
```rust
// F11 always opens config, even in output pane
if key.code == KeyCode::F(11) {
    return Action::EnterConfigWindow;
}
```

Similarly for `handle_pinned_terminal_keys`.

**Step 3: Build and verify**

Run: `cargo build --release 2>&1 | head -20`

**Step 4: Test manually**

- Start workbench, press F11, verify modal appears
- Press 1/2/3 to switch tabs, j/k to navigate
- Press Esc to close

**Step 5: Commit**

```bash
git add src/tui/event/handlers.rs
git commit -m "feat: wire global hotkeys from UserConfig, dynamic key dispatch"
```

---

### Task 10: End-to-end testing and polish

**Step 1: Verify agents tab**
- Open config (F11), on Agents tab
- Navigate to a field, press Enter, type new value, press Enter
- Verify value updates in the table
- Press 'a' to add agent, verify new row appears
- Press 'd' to delete agent, verify it's removed
- Close config, press 'n' in session list, verify new agent appears in dialog
- Create a session with the new agent, verify it works

**Step 2: Verify hotkeys tab**
- Switch to Hotkeys tab (2 or Tab)
- Navigate to "Cycle Workspace", press Enter
- Press a new key combo (e.g., Ctrl-a)
- Verify it updates
- Press Esc to close config
- Verify Ctrl-a now cycles workspace and Ctrl-z no longer does

**Step 3: Verify scrollback tab**
- Switch to Scrollback tab (3)
- Change raw buffer size to 1024
- Close config, start a new session
- Verify the session uses the new buffer size

**Step 4: Verify persistence**
- Make changes, close workbench, reopen
- Verify changes persisted (check `~/.config/workbench/user_config.toml`)

**Step 5: Final commit**

```bash
git add -A
git commit -m "feat: config window with agents, hotkeys, scrollback settings (F11)"
```
