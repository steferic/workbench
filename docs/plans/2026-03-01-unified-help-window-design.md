# Unified Help/Settings Window

## Summary

Merge the `?` help popup and F12 config window into a single unified window. Extend the existing `config_window.rs` with a new "Quick Ref" tab containing all help content in a scrollable page. Remove the `?` key trigger entirely.

## Trigger

- **F12** opens the unified window (only trigger)
- **?** key handler removed
- Per-pane help popups (`render_pane_help`) removed

## Tab Layout

```
  1 Quick Ref  |  2 Agents  |  3 Hotkeys  |  4 Memory
```

- Tab 1 (Quick Ref) is new
- Tabs 2-4 are the existing Agents, Hotkeys, Scrollback tabs (unchanged)
- Navigation via number keys `1/2/3/4` and left/right arrows

## Quick Reference Tab Content

Single scrollable page with section headers. Content consolidated from `help_popup.rs` (both `render` and `render_pane_help` functions):

```
  Navigation
  ──────────────────────────────────────────
  j/k, Up/Down        Move up/down in lists
  h/l, Left/Right     Switch between panels
  Tab                  Cycle focus between panels
  Shift+Left/Right     Focus left/right panel
  `                    Jump to next idle session

  Workspaces
  ──────────────────────────────────────────
  n                    Create/open workspace
  Enter                Select workspace
  w                    Toggle working/paused
  d                    Delete workspace

  Sessions
  ──────────────────────────────────────────
  1/2/3/4              New Claude/Gemini/Codex/Grok
  !/@ /#/$             Same but skip permissions
  Alt+1-4              Create in isolated worktree
  Alt+!/@ /#           Worktree + skip permissions
  t                    New terminal
  P                    Start parallel task
  Enter                Activate selected session
  s                    Stop session (graceful)
  x                    Kill session (force)
  d                    Delete session
  p                    Pin/unpin to side panel

  Worktrees
  ──────────────────────────────────────────
  w                    Open terminal in worktree
  m                    Merge worktree into main

  Todos
  ──────────────────────────────────────────
  n                    Create new todo
  Enter                Run todo with agent
  y/Y                  Accept suggested / Accept all
  x                    Mark as done
  X                    Archive todo
  d                    Delete todo
  Tab                  Switch tabs (Active/Archived/Reports)

  Todo Reports
  ──────────────────────────────────────────
  v                    View report details
  m                    Merge selected attempt
  d                    Discard attempt

  Utilities
  ──────────────────────────────────────────
  Tab                  Switch tabs (Util/Sounds/Cfg/Notes)
  Enter                Toggle/activate item

  Output Pane
  ──────────────────────────────────────────
  (type)               Send input to active session
  Ctrl+H               Return to session list
  Esc                  Send escape to agent
  Ctrl+C               Send interrupt signal

  General
  ──────────────────────────────────────────
  F12                  Open this window
  q                    Quit workbench
```

Scrollable via j/k when content exceeds visible area.

## Changes Required

### Files to modify:
- `src/tui/components/config_window.rs` — Add `QuickRef` variant handling, add `render_quickref_tab()`, update tab bar to show 4 tabs
- `src/app/state/types.rs` — Add `QuickRef` to `ConfigTab` enum
- `src/tui/event/handlers.rs` — Remove `?` key handling for help mode, add scroll state for Quick Ref tab, update tab switching to handle 4 tabs
- `src/app/handler.rs` — Remove `EnterHelpMode`/`ExitHelpMode` action handling
- `src/app/action.rs` — Remove help-related actions
- `src/app/state/ui.rs` — Add scroll offset for Quick Ref tab, remove help/pane_help state fields
- `src/tui/ui.rs` — Remove help popup rendering branch

### Files to delete (or gut):
- `src/tui/components/help_popup.rs` — No longer needed (content moves to config_window.rs)

### Files unaffected:
- Agents tab, Hotkeys tab, Memory tab — unchanged
- `config_window.rs` centered_rect helper — reused as-is
