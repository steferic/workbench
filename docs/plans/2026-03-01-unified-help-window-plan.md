# Unified Help/Settings Window Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Merge the `?` help popup and F12 config window into a single unified window with tabs: Quick Ref | Agents | Hotkeys | Memory.

**Architecture:** Add a `QuickRef` variant to `ConfigTab`, add a `render_quickref_tab()` function to `config_window.rs` with all help content in a scrollable page, add a `config_scroll_offset` field to UIState for scrolling. Remove `InputMode::Help`, `PaneHelp`, and the `help_popup.rs` component. Remap `?`/`h`/`F1` keys to open the config window instead.

**Tech Stack:** Rust, ratatui, crossterm

---

### Task 1: Add `QuickRef` variant to `ConfigTab` enum

**Files:**
- Modify: `src/app/state/types.rs:28-33`

**Step 1: Add QuickRef as the default variant**

In the `ConfigTab` enum, add `QuickRef` as the first variant and make it `#[default]`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfigTab {
    #[default]
    QuickRef,
    Agents,
    Hotkeys,
    Scrollback,
}
```

**Step 2: Build to verify it compiles (expect errors from exhaustive matches)**

Run: `cargo build 2>&1 | head -40`
Expected: Errors about non-exhaustive patterns in match statements for `ConfigTab`. This is expected and will be fixed in subsequent tasks.

---

### Task 2: Add `config_scroll_offset` to UIState

**Files:**
- Modify: `src/app/state/ui.rs:113-120` (struct fields)
- Modify: `src/app/state/ui.rs:200-207` (initializer)

**Step 1: Add the scroll offset field**

Add after `config_rebinding` (line 119):

```rust
    pub config_scroll_offset: usize,
```

**Step 2: Initialize it in `UIState::new()`**

Add after `config_rebinding: false,` (line 207):

```rust
            config_scroll_offset: 0,
```

---

### Task 3: Update tab bar and tab switching in config_window.rs

**Files:**
- Modify: `src/tui/components/config_window.rs:21-25` (match in render)
- Modify: `src/tui/components/config_window.rs:31-35` (tab bar definition)
- Modify: `src/tui/components/config_window.rs:63-64` (title)

**Step 1: Update the render function to handle QuickRef**

In the `render()` function, update the match block:

```rust
    match state.ui.config_tab {
        ConfigTab::QuickRef => render_quickref_tab(frame, chunks[1], state),
        ConfigTab::Agents => render_agents_tab(frame, chunks[1], state),
        ConfigTab::Hotkeys => render_hotkeys_tab(frame, chunks[1], state),
        ConfigTab::Scrollback => render_scrollback_tab(frame, chunks[1], state),
    }
```

**Step 2: Update the tab bar definition**

Change the `tabs` vec in `render_tab_bar()`:

```rust
    let tabs = vec![
        ("1", "Quick Ref", ConfigTab::QuickRef),
        ("2", "Agents", ConfigTab::Agents),
        ("3", "Hotkeys", ConfigTab::Hotkeys),
        ("4", "Memory", ConfigTab::Scrollback),
    ];
```

**Step 3: Update the title**

Change the block title from `" Settings (F12) "` to `" Help & Settings (F12) "`.

---

### Task 4: Add `render_quickref_tab()` to config_window.rs

**Files:**
- Modify: `src/tui/components/config_window.rs` (add new function)

**Step 1: Add the render function**

Add this function after `render_tab_bar()` and before `render_agents_tab()`:

```rust
fn render_quickref_tab(frame: &mut Frame, area: Rect, state: &AppState) {
    let scroll_offset = state.ui.config_scroll_offset;

    // Build help content as a helper to keep things clean
    let mut lines: Vec<Line> = Vec::new();

    let section_style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let key_style = Style::default().fg(Color::Cyan);
    let sep_style = Style::default().fg(Color::DarkGray);

    let separator = Line::from(Span::styled(
        "  ──────────────────────────────────────────────────────",
        sep_style,
    ));

    // -- Navigation --
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Navigation", section_style)));
    lines.push(separator.clone());
    lines.push(Line::from(vec![
        Span::styled("  j/k, Up/Down       ", key_style),
        Span::raw("Move up/down in lists"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  h/l, Left/Right    ", key_style),
        Span::raw("Switch between panels"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Tab                ", key_style),
        Span::raw("Cycle focus between panels"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Shift+Left/Right   ", key_style),
        Span::raw("Focus left/right panel"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  `                  ", key_style),
        Span::raw("Jump to next idle session"),
    ]));

    // -- Workspaces --
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Workspaces", section_style)));
    lines.push(separator.clone());
    lines.push(Line::from(vec![
        Span::styled("  n                  ", key_style),
        Span::raw("Create/open workspace"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Enter              ", key_style),
        Span::raw("Select workspace"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  w                  ", key_style),
        Span::raw("Toggle working/paused"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  d                  ", key_style),
        Span::raw("Delete workspace"),
    ]));

    // -- Sessions --
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Sessions", section_style)));
    lines.push(separator.clone());
    lines.push(Line::from(vec![
        Span::styled("  1/2/3/4            ", key_style),
        Span::raw("New Claude/Gemini/Codex/Grok"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  !/@ /#/$           ", key_style),
        Span::raw("Same but skip permissions"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Alt+1-4            ", key_style),
        Span::raw("Create in isolated worktree"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Alt+!/@ /#         ", key_style),
        Span::raw("Worktree + skip permissions"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  t                  ", key_style),
        Span::raw("New terminal"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  P                  ", key_style),
        Span::raw("Start parallel task"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Enter              ", key_style),
        Span::raw("Activate selected session"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  s                  ", key_style),
        Span::raw("Stop session (graceful)"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  x                  ", key_style),
        Span::raw("Kill session (force)"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  d                  ", key_style),
        Span::raw("Delete session"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  p                  ", key_style),
        Span::raw("Pin/unpin to side panel"),
    ]));

    // -- Worktrees --
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Worktrees", section_style)));
    lines.push(separator.clone());
    lines.push(Line::from(vec![
        Span::styled("  w                  ", key_style),
        Span::raw("Open terminal in worktree"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  m                  ", key_style),
        Span::raw("Merge worktree into main"),
    ]));

    // -- Todos --
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Todos", section_style)));
    lines.push(separator.clone());
    lines.push(Line::from(vec![
        Span::styled("  n                  ", key_style),
        Span::raw("Create new todo"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Enter              ", key_style),
        Span::raw("Run todo with agent"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  y / Y              ", key_style),
        Span::raw("Accept suggested / Accept all"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  x                  ", key_style),
        Span::raw("Mark as done"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  X                  ", key_style),
        Span::raw("Archive todo"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  d                  ", key_style),
        Span::raw("Delete todo"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Tab                ", key_style),
        Span::raw("Switch tabs (Active/Archived/Reports)"),
    ]));

    // -- Todo Reports --
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Todo Reports", section_style)));
    lines.push(separator.clone());
    lines.push(Line::from(vec![
        Span::styled("  v                  ", key_style),
        Span::raw("View report details"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  m                  ", key_style),
        Span::raw("Merge selected attempt"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  d                  ", key_style),
        Span::raw("Discard attempt"),
    ]));

    // -- Utilities --
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Utilities", section_style)));
    lines.push(separator.clone());
    lines.push(Line::from(vec![
        Span::styled("  Tab                ", key_style),
        Span::raw("Switch tabs (Util/Sounds/Cfg/Notes)"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Enter              ", key_style),
        Span::raw("Toggle/activate item"),
    ]));

    // -- Output Pane --
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Output Pane", section_style)));
    lines.push(separator.clone());
    lines.push(Line::from(vec![
        Span::styled("  (type)             ", key_style),
        Span::raw("Send input to active session"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Ctrl+H             ", key_style),
        Span::raw("Return to session list"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Esc                ", key_style),
        Span::raw("Send escape to agent (interrupt)"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Ctrl+C             ", key_style),
        Span::raw("Send interrupt signal"),
    ]));

    // -- General --
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  General", section_style)));
    lines.push(separator.clone());
    lines.push(Line::from(vec![
        Span::styled("  F12                ", key_style),
        Span::raw("Open this window"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  q                  ", key_style),
        Span::raw("Quit workbench"),
    ]));
    lines.push(Line::from(""));

    // Footer
    lines.push(Line::from(Span::styled(
        "  ──────────────────────────────────────────────────────",
        sep_style,
    )));
    lines.push(Line::from(vec![
        Span::styled("  [j/k]", Style::default().fg(Color::Cyan)),
        Span::raw(" Scroll"),
    ]));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .style(Style::default().bg(Color::Black));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset as u16, 0));
    frame.render_widget(paragraph, area);
}
```

**Step 2: Build to verify**

Run: `cargo build 2>&1 | head -20`
Expected: Should compile (remaining errors will be from other files not yet updated).

---

### Task 5: Update tab switching in event handlers

**Files:**
- Modify: `src/tui/event/handlers.rs:190-214`

**Step 1: Update the ConfigWindow key handling**

Change the number key mappings and Tab cycling to include `QuickRef`:

```rust
                return match key.code {
                    KeyCode::Esc => Action::ExitConfigWindow,
                    KeyCode::Char('1') => Action::ConfigSwitchTab(ConfigTab::QuickRef),
                    KeyCode::Char('2') => Action::ConfigSwitchTab(ConfigTab::Agents),
                    KeyCode::Char('3') => Action::ConfigSwitchTab(ConfigTab::Hotkeys),
                    KeyCode::Char('4') => Action::ConfigSwitchTab(ConfigTab::Scrollback),
                    KeyCode::Tab => {
                        let next = match state.ui.config_tab {
                            ConfigTab::QuickRef => ConfigTab::Agents,
                            ConfigTab::Agents => ConfigTab::Hotkeys,
                            ConfigTab::Hotkeys => ConfigTab::Scrollback,
                            ConfigTab::Scrollback => ConfigTab::QuickRef,
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
```

---

### Task 6: Update config handler for QuickRef scroll behavior

**Files:**
- Modify: `src/app/handlers/config.rs:7-13` (ConfigSwitchTab — reset scroll offset)
- Modify: `src/app/handlers/config.rs:14-23` (ConfigMoveDown/Up — scroll on QuickRef tab)
- Modify: `src/app/handlers/config.rs:179-184` (max_rows — add QuickRef)

**Step 1: Reset scroll offset on tab switch**

In the `ConfigSwitchTab` handler, add scroll offset reset:

```rust
        Action::ConfigSwitchTab(tab) => {
            state.ui.config_tab = tab;
            state.ui.config_selected_row = 0;
            state.ui.config_selected_col = 0;
            state.ui.config_editing = false;
            state.ui.config_rebinding = false;
            state.ui.config_scroll_offset = 0;
        }
```

**Step 2: Handle scroll for QuickRef in MoveDown/MoveUp**

Update `ConfigMoveDown`:

```rust
        Action::ConfigMoveDown => {
            if state.ui.config_tab == ConfigTab::QuickRef {
                state.ui.config_scroll_offset = state.ui.config_scroll_offset.saturating_add(1);
            } else {
                let max = max_rows(state);
                if max > 0 && state.ui.config_selected_row + 1 < max {
                    state.ui.config_selected_row += 1;
                }
            }
        }
```

Update `ConfigMoveUp`:

```rust
        Action::ConfigMoveUp => {
            if state.ui.config_tab == ConfigTab::QuickRef {
                state.ui.config_scroll_offset = state.ui.config_scroll_offset.saturating_sub(1);
            } else {
                if state.ui.config_selected_row > 0 {
                    state.ui.config_selected_row -= 1;
                }
            }
        }
```

**Step 3: Add QuickRef to max_rows**

```rust
fn max_rows(state: &AppState) -> usize {
    match state.ui.config_tab {
        ConfigTab::QuickRef => 0,  // Scroll-based, not row-based
        ConfigTab::Agents => state.system.user_config.agents.len(),
        ConfigTab::Hotkeys => state.system.user_config.global_hotkeys.len(),
        ConfigTab::Scrollback => 3,
    }
}
```

**Step 4: Add QuickRef to ConfigResetDefault match**

In the `ConfigResetDefault` handler (~line 159), add the QuickRef arm:

```rust
                ConfigTab::QuickRef => {} // Nothing to reset
```

**Step 5: Add QuickRef to ConfigStartEdit match**

In the `ConfigStartEdit` handler (~line 38), add the QuickRef arm:

```rust
                ConfigTab::QuickRef => {} // Not editable
```

---

### Task 7: Remap `?`, `h`, and `F1` keys to open config window

**Files:**
- Modify: `src/config/defaults.toml` (multiple sections)
- Modify: `src/config/user_config.rs:46-48`
- Modify: `src/tui/event/handlers.rs` (multiple places where `?` and `h` map to help actions)

**Step 1: Update defaults.toml**

Remove `[mode.help]` section entirely (lines 26-30).

Change `F1` global from `EnterHelpMode` to `EnterConfigWindow`:
```toml
"F1" = "EnterConfigWindow"
```

In each panel section, change `"?" = "EnterHelpMode"` to `"?" = "EnterConfigWindow"` and change `"h" = "ShowPaneHelp"` to `"h" = "EnterConfigWindow"` for panels that use it (workspace_list, session_list, todos_pane, utilities_pane).

In `[panel.output_pane]`, change `"?" = "EnterHelpMode"` to `"?" = "EnterConfigWindow"`.

**Step 2: Update user_config.rs default global hotkeys**

In the `default_global_hotkeys()` function (~line 46), change:
```rust
    m.insert("EnterConfigWindow".into(), "F1".into());
```
And remove the old `EnterHelpMode` entry. Keep F12 as `ToggleDebugOverlay` or also map to `EnterConfigWindow` — check the current mapping. Looking at the code, F12 is hardcoded in `handlers.rs:252` as `Action::EnterConfigWindow`, so the defaults.toml `F12 = ToggleDebugOverlay` is overridden. Update defaults.toml to reflect reality:
```toml
"F12" = "EnterConfigWindow"
```

**Step 3: Update event handler string-to-action mapping**

In `src/tui/event/handlers.rs` line 21, the string `"EnterHelpMode"` maps to `Some(Action::EnterHelpMode)`. Change this to map to `Action::EnterConfigWindow`:
```rust
                    "EnterHelpMode" => Some(Action::EnterConfigWindow),
```
Or better yet, just remove the mapping entirely since defaults.toml will now use `"EnterConfigWindow"` directly. But keep it as a fallback alias for users who may have custom configs.

---

### Task 8: Remove Help mode and PaneHelp infrastructure

**Files:**
- Modify: `src/app/state/types.rs` — Remove `InputMode::Help` variant, remove `PaneHelp` enum
- Modify: `src/app/state/ui.rs` — Remove `pane_help` field
- Modify: `src/app/action.rs` — Remove `EnterHelpMode`, `ShowPaneHelp`, `DismissPaneHelp` actions
- Modify: `src/app/handlers/input.rs` — Remove `EnterHelpMode` and `ShowPaneHelp`/`DismissPaneHelp` handlers
- Modify: `src/app/handler.rs:334-342` — Remove help/pane-help action dispatching
- Modify: `src/app/mod.rs:20` — Remove `PaneHelp` from re-exports
- Modify: `src/tui/event/handlers.rs` — Remove `InputMode::Help` match arm, remove pane_help check, remove `PaneHelp` import, remove all `ShowPaneHelp(PaneHelp::X)` mappings, remove `EnterHelpMode` mappings
- Modify: `src/tui/ui.rs:139-141` — Remove `InputMode::Help` rendering branch
- Modify: `src/tui/ui.rs:176-179` — Remove pane help rendering
- Modify: `src/tui/components/status_bar.rs:114-126` — Remove `InputMode::Help` status bar display
- Modify: `src/tui/components/config_window.rs:198` — Remove/update `EnterHelpMode` format mapping
- Delete: `src/tui/components/help_popup.rs` — No longer needed
- Modify: `src/tui/components/mod.rs` — Remove `pub mod help_popup;`
- Modify: `src/tui/ui.rs:3` — Remove `help_popup` from imports

**Step 1: Remove types**

In `src/app/state/types.rs`:
- Remove `Help` from `InputMode` enum
- Remove the entire `PaneHelp` enum (lines 99-106)

In `src/app/state/ui.rs`:
- Remove `PaneHelp` from imports
- Remove the `pane_help: Option<PaneHelp>` field
- Remove `pane_help: None` from initializer

**Step 2: Remove actions**

In `src/app/action.rs`:
- Remove `use crate::app::state::PaneHelp;` import (if it becomes unused)
- Remove `EnterHelpMode` (line 78)
- Remove `ShowPaneHelp(PaneHelp)` (line 82)
- Remove `DismissPaneHelp` (line 83)

**Step 3: Remove handlers**

In `src/app/handlers/input.rs`:
- Remove the `Action::EnterHelpMode` arm
- Remove the `Action::ShowPaneHelp` and `Action::DismissPaneHelp` arms

In `src/app/handler.rs`:
- Remove `Action::EnterHelpMode` from the input actions dispatch group
- Remove `Action::ShowPaneHelp(_) | Action::DismissPaneHelp` from the input actions dispatch group

**Step 4: Remove event handling**

In `src/tui/event/handlers.rs`:
- Remove `PaneHelp` from the imports line
- Remove `InputMode::Help` match arm (lines 34-40)
- Remove the `pane_help` check block (lines 236-240)
- Remove all `Action::ShowPaneHelp(PaneHelp::X)` from panel key handlers — replace `h` key with `Action::EnterConfigWindow` in workspace_list, session_list, todos_pane, utilities_pane
- Remove all `Action::EnterHelpMode` from panel key handlers — replace `?` key with `Action::EnterConfigWindow`
- Change `"EnterHelpMode" => Some(Action::EnterHelpMode)` to `"EnterHelpMode" => Some(Action::EnterConfigWindow)` (backwards compat)

**Step 5: Remove rendering**

In `src/tui/ui.rs`:
- Remove `InputMode::Help => { help_popup::render(frame, state); }` from the match
- Remove the pane help rendering block
- Remove `help_popup` from the imports

In `src/tui/components/status_bar.rs`:
- Remove the `InputMode::Help` match arm

In `src/tui/components/mod.rs`:
- Remove `pub mod help_popup;`

**Step 6: Delete help_popup.rs**

Run: `rm src/tui/components/help_popup.rs`

**Step 7: Update config_window.rs format_action_name**

Change `"EnterHelpMode" => "Help"` to `"EnterConfigWindow" => "Help & Settings"` (or remove if no longer needed).

---

### Task 9: Build, test, and fix any remaining issues

**Step 1: Full build**

Run: `cargo build 2>&1`
Expected: Clean compile with no errors.

**Step 2: Fix any remaining compile errors**

Address any exhaustive match errors or unused imports that were missed.

**Step 3: Run the app**

Run: `cargo run`
- Press F12 — should open the unified window on the Quick Ref tab
- Press 1/2/3/4 — should switch between tabs
- Press j/k on Quick Ref tab — should scroll content
- Press ? from any panel — should open the config window
- Press Esc — should close the window
- Verify Agents, Hotkeys, and Memory tabs still work as before

**Step 4: Commit**

```bash
git add -A
git commit -m "feat: merge help popup and config window into unified help/settings window

Add Quick Ref tab with all keybinding help content. Remove separate
help popup and pane-specific help popups. F12 and ? both open the
unified window."
```
