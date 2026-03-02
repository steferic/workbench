# Config Window Design

## Overview

F11 opens a config modal with three tabbed sections: Agents, Hotkeys, and Scrollback. Changes apply live and auto-persist to `~/.config/workbench/config.toml`.

## Architecture

- **Trigger:** F11 sets `InputMode::ConfigSettings`, Esc exits
- **Layout:** Centered modal (70% x 80%), three tabs switched via Tab/1/2/3
- **Persistence:** New `UserConfig` struct in TOML format, loaded at startup, auto-saved on edit
- **State:** Config navigation state in `UIState`, config data in `SystemState`

## Agents Tab

Ordered table of configured agents:

```
 #  Key  Name     Command   Badge
 1   1   Claude   claude      C
 2   2   Gemini   gemini      G
 3   3   Codex    codex       X
 4   4   Grok     grok        K
```

- j/k navigate rows, h/l navigate fields
- Enter edits a field inline
- `a` adds new agent, `d` deletes (with confirm)
- Shift+j/k reorders
- AgentType gains `Custom { command, display_name, badge }` variant
- Session creation reads agents dynamically from config

## Hotkeys Tab

Editable global keybindings:

```
 Action                   Key
 Cycle Workspace          Ctrl-z
 Cycle Session            Ctrl-x
 Quit                     Ctrl-q
 Help                     F1
 Debug Overlay            F12
 Config                   F11
```

- j/k navigate, Enter to rebind (press new key combo)
- Conflict detection with swap/cancel prompt
- `r` resets single binding to default

## Scrollback Tab

Numeric settings:

```
 Setting                     Value
 Raw buffer size (KB)         512
 Replay parser rows           500
 Live parser scrollback       200
```

- Enter to edit value inline
- Applies to newly created sessions only

## Data Model

```rust
pub struct UserConfig {
    pub agents: Vec<AgentConfig>,
    pub global_hotkeys: HashMap<String, String>,
    pub scrollback_buffer_kb: usize,       // Default 512
    pub replay_parser_rows: u16,           // Default 500
    pub live_scrollback_rows: usize,       // Default 200
}

pub struct AgentConfig {
    pub command: String,
    pub display_name: String,
    pub badge: String,
    pub hotkey: String,
    pub enabled: bool,
}
```

Defaults match current hardcoded values. Generated from defaults on first run if config.toml doesn't exist.
