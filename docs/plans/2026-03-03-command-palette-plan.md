# Command Palette Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a fuzzy-searchable command palette (Ctrl+P) that lists all user-facing actions with their keybindings.

**Architecture:** New `CommandPalette` input mode with a centered overlay. A static list of `PaletteEntry` structs maps display names to `Action` variants. Substring filtering on the display name. The palette dispatches the selected action on Enter.

**Tech Stack:** Rust, ratatui, crossterm

---

### Task 1: Add CommandPalette to InputMode and UI state

**Files:**
- Modify: `src/app/state/types.rs`
- Modify: `src/app/state/ui.rs`

**Step 1: Add the InputMode variant**

In `src/app/state/types.rs`, add `CommandPalette` to the `InputMode` enum after `ConfigWindow`:

```rust
    ConfigWindow,  // F12 configuration window
    CommandPalette, // Ctrl+P command palette
```

**Step 2: Add UI state fields**

In `src/app/state/ui.rs`, add these fields to the `UIState` struct after `config_scroll_offset`:

```rust
    pub palette_query: String,
    pub palette_selected: usize,
```

**Step 3: Initialize in UIState::new()**

Add after `config_scroll_offset: 0,`:

```rust
            palette_query: String::new(),
            palette_selected: 0,
```

---

### Task 2: Add palette actions to Action enum

**Files:**
- Modify: `src/app/action.rs`

**Step 1: Add actions**

Add after the config window actions (after `ConfigRebindKey`):

```rust
    // Command palette
    EnterCommandPalette,
    ExitCommandPalette,
    CommandPaletteExecute,
    CommandPaletteDown,
    CommandPaletteUp,
    CommandPaletteInput(char),
    CommandPaletteBackspace,
```

---

### Task 3: Create the command palette component

**Files:**
- Create: `src/tui/components/command_palette.rs`
- Modify: `src/tui/components/mod.rs`

**Step 1: Add module declaration**

In `src/tui/components/mod.rs`, add:

```rust
pub mod command_palette;
```

**Step 2: Create the component file**

Create `src/tui/components/command_palette.rs`:

```rust
use crate::app::{Action, AppState};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

pub struct PaletteEntry {
    pub name: &'static str,
    pub action: Action,
    pub keybinding: &'static str,
}

fn palette_entries() -> Vec<PaletteEntry> {
    vec![
        PaletteEntry { name: "Create Claude Session", action: Action::CreateSession(crate::models::AgentType::Claude, false, false), keybinding: "1" },
        PaletteEntry { name: "Create Gemini Session", action: Action::CreateSession(crate::models::AgentType::Gemini, false, false), keybinding: "2" },
        PaletteEntry { name: "Create Codex Session", action: Action::CreateSession(crate::models::AgentType::Codex, false, false), keybinding: "3" },
        PaletteEntry { name: "Create Grok Session", action: Action::CreateSession(crate::models::AgentType::Grok, false, false), keybinding: "4" },
        PaletteEntry { name: "Create Terminal", action: Action::CreateTerminal, keybinding: "t" },
        PaletteEntry { name: "Start Parallel Task", action: Action::EnterParallelTaskMode, keybinding: "P" },
        PaletteEntry { name: "New Workspace", action: Action::EnterWorkspaceActionMode, keybinding: "n" },
        PaletteEntry { name: "Toggle Split View", action: Action::ToggleSplitView, keybinding: "\\" },
        PaletteEntry { name: "Cycle Next Workspace", action: Action::CycleNextWorkspace, keybinding: "Ctrl+Z" },
        PaletteEntry { name: "Cycle Next Session", action: Action::CycleNextSession, keybinding: "Ctrl+X" },
        PaletteEntry { name: "Help & Settings", action: Action::EnterConfigWindow, keybinding: "F12" },
        PaletteEntry { name: "Toggle Debug Overlay", action: Action::ToggleDebugOverlay, keybinding: "" },
        PaletteEntry { name: "Quit", action: Action::InitiateQuit, keybinding: "q" },
    ]
}

pub fn filtered_entries(query: &str) -> Vec<PaletteEntry> {
    let query_lower = query.to_lowercase();
    palette_entries()
        .into_iter()
        .filter(|e| query.is_empty() || e.name.to_lowercase().contains(&query_lower))
        .collect()
}

pub fn render(frame: &mut Frame, state: &AppState) {
    let area = centered_rect(50, 60, frame.area());
    frame.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);

    // Search input
    let input_block = Block::default()
        .title(" Command Palette ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));

    let input_text = format!("> {}_", state.ui.palette_query);
    let input_paragraph = Paragraph::new(Line::from(vec![
        Span::styled(input_text, Style::default().fg(Color::White)),
    ]))
    .block(input_block);

    frame.render_widget(input_paragraph, chunks[0]);

    // Results list
    let entries = filtered_entries(&state.ui.palette_query);

    let items: Vec<ListItem> = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let is_selected = i == state.ui.palette_selected;
            let name_style = if is_selected {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            let key_style = Style::default().fg(Color::DarkGray);

            let key_display = if entry.keybinding.is_empty() {
                String::new()
            } else {
                format!("[{}]", entry.keybinding)
            };

            // Pad name to push keybinding to the right
            let available_width = chunks[1].width.saturating_sub(6) as usize;
            let name_len = entry.name.len();
            let key_len = key_display.len();
            let padding = available_width.saturating_sub(name_len + key_len);

            ListItem::new(Line::from(vec![
                Span::styled(format!("  {}", entry.name), name_style),
                Span::raw(" ".repeat(padding)),
                Span::styled(key_display, key_style),
            ]))
        })
        .collect();

    let list_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .style(Style::default().bg(Color::Black));

    let list = List::new(items).block(list_block);

    let mut list_state = ListState::default();
    if !entries.is_empty() {
        list_state.select(Some(state.ui.palette_selected.min(entries.len().saturating_sub(1))));
    }

    frame.render_stateful_widget(list, chunks[1], &mut list_state);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
```

---

### Task 4: Wire up event handling for the palette

**Files:**
- Modify: `src/tui/event/handlers.rs`

**Step 1: Add Ctrl+P global handler**

In the `handle_key_event` method, after the pending quit check block (~line 235) and before the global window navigation block, add:

```rust
        // Global Ctrl+P - command palette
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
            return Action::EnterCommandPalette;
        }
```

**Step 2: Add CommandPalette input mode handler**

In the input mode match block (inside `handle_key_event`, near the top where `InputMode::ConfigWindow` etc. are handled), add before `InputMode::Normal => {}`:

```rust
            InputMode::CommandPalette => {
                return match key.code {
                    KeyCode::Esc => Action::ExitCommandPalette,
                    KeyCode::Enter => Action::CommandPaletteExecute,
                    KeyCode::Char('j') | KeyCode::Down => Action::CommandPaletteDown,
                    KeyCode::Char('k') | KeyCode::Up => Action::CommandPaletteUp,
                    KeyCode::Backspace => Action::CommandPaletteBackspace,
                    KeyCode::Char(c) => Action::CommandPaletteInput(c),
                    _ => Action::Tick,
                };
            }
```

---

### Task 5: Wire up action handling

**Files:**
- Modify: `src/app/handlers/input.rs`
- Modify: `src/app/handler.rs`

**Step 1: Add palette action handling in input.rs**

In `handle_input_action()`, add these match arms:

```rust
        Action::EnterCommandPalette => {
            state.ui.input_mode = InputMode::CommandPalette;
            state.ui.palette_query.clear();
            state.ui.palette_selected = 0;
        }
        Action::ExitCommandPalette => {
            state.ui.input_mode = InputMode::Normal;
            state.ui.palette_query.clear();
            state.ui.palette_selected = 0;
        }
        Action::CommandPaletteInput(c) => {
            state.ui.palette_query.push(c);
            state.ui.palette_selected = 0; // Reset selection on new input
        }
        Action::CommandPaletteBackspace => {
            state.ui.palette_query.pop();
            state.ui.palette_selected = 0;
        }
        Action::CommandPaletteDown => {
            let count = crate::tui::components::command_palette::filtered_entries(&state.ui.palette_query).len();
            if count > 0 && state.ui.palette_selected + 1 < count {
                state.ui.palette_selected += 1;
            }
        }
        Action::CommandPaletteUp => {
            if state.ui.palette_selected > 0 {
                state.ui.palette_selected -= 1;
            }
        }
```

**Step 2: Handle CommandPaletteExecute**

This is special — it needs to close the palette and return the selected action for dispatch. Add in input.rs:

```rust
        Action::CommandPaletteExecute => {
            let entries = crate::tui::components::command_palette::filtered_entries(&state.ui.palette_query);
            if let Some(entry) = entries.into_iter().nth(state.ui.palette_selected) {
                state.ui.input_mode = InputMode::Normal;
                state.ui.palette_query.clear();
                state.ui.palette_selected = 0;
                // Store the action to be dispatched on the next tick
                state.ui.pending_palette_action = Some(entry.action);
            }
        }
```

This requires adding `pending_palette_action` to UIState. Add in `src/app/state/ui.rs`:

Field: `pub pending_palette_action: Option<Action>,`
Init: `pending_palette_action: None,`

Then in `src/app/handler.rs`, at the start of `handle_action()` (or in the tick handler), check for and dispatch the pending action:

```rust
// Check for pending palette action
if let Some(palette_action) = state.ui.pending_palette_action.take() {
    // Re-dispatch the action
    return self.handle_action(state, palette_action, pty_manager, pty_tx);
}
```

**Step 3: Add palette actions to the dispatch group in handler.rs**

In `handler.rs`, find the input actions dispatch group and add the palette actions:

```rust
Action::EnterCommandPalette | Action::ExitCommandPalette |
Action::CommandPaletteExecute | Action::CommandPaletteDown |
Action::CommandPaletteUp | Action::CommandPaletteInput(_) |
Action::CommandPaletteBackspace |
```

---

### Task 6: Wire up rendering

**Files:**
- Modify: `src/tui/ui.rs`
- Modify: `src/tui/components/status_bar.rs`

**Step 1: Add palette rendering in ui.rs**

In the modal overlay match block in `src/tui/ui.rs`, add:

```rust
        InputMode::CommandPalette => {
            command_palette::render(frame, state);
        }
```

And add `command_palette` to the imports at the top.

**Step 2: Add CommandPalette status bar entry**

In `src/tui/components/status_bar.rs`, add a match arm for `InputMode::CommandPalette`:

```rust
        InputMode::CommandPalette => (
            vec![Span::styled(
                " COMMAND PALETTE ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )],
            vec![Span::styled(
                "Type to filter  Enter: execute  Esc: close",
                Style::default().fg(Color::Gray),
            )],
        ),
```

---

### Task 7: Build, test, and fix

**Step 1: Build**

Run: `cargo build 2>&1`
Expected: Clean compile.

**Step 2: Fix any compile errors**

Common issues: missing imports, exhaustive match patterns, unused variables.

**Step 3: Manual test**

Run: `cargo run`
- Press Ctrl+P — palette should open
- Type "claude" — should filter to "Create Claude Session"
- Press Enter — should create a Claude session
- Press Ctrl+P again, press Esc — should close
- Press Ctrl+P, use j/k to navigate, Enter to select

**Step 4: Commit**

```bash
git add -A
git commit -m "feat: add command palette (Ctrl+P)

Searchable overlay listing all actions with keybindings.
Substring filtering, j/k navigation, Enter to execute."
```
