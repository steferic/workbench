# Command Palette Design

## Summary

A fuzzy-searchable action list triggered by Ctrl+P. Provides discoverability for all available actions with their keybindings.

## Trigger

`Ctrl+P` — global, works from any panel in Normal mode. Does not activate when in text input modes (CreateTodo, SetStartCommand, etc).

## Behavior

1. Opens centered overlay (50% width, 60% height)
2. Text input at top with cursor
3. Filtered list of actions below, updating as user types
4. `j/k` or `Up/Down` to navigate, `Enter` to execute, `Esc` to close
5. Executing an action closes the palette and dispatches the action

## Action List

All dispatchable actions with human-readable names. Each entry shows:
- Display name (e.g. "Create Claude Session")
- Keybinding hint right-aligned (e.g. `[1]`)

Actions sourced from a static list covering all user-facing actions in the app. Not auto-generated from the keybinding system (since many actions are panel-specific and context matters).

## Filtering

Simple case-insensitive substring match on the display name. No external fuzzy library needed — the action list is small (~30-40 items).

## Data Model

```rust
pub struct CommandPaletteEntry {
    pub name: &'static str,      // "Create Claude Session"
    pub action: Action,           // Action::CreateClaude
    pub keybinding: &'static str, // "1"
}
```

## UI State

```rust
// In InputMode enum
CommandPalette,

// In UIState
pub command_palette_query: String,
pub command_palette_selected: usize,
```

## Layout

```
┌─ Command Palette ──────────────────────┐
│ > search query_                        │
│────────────────────────────────────────│
│ > Create Claude Session           [1]  │
│   Create Gemini Session           [2]  │
│   Create Codex Session            [3]  │
│   Create Terminal                 [t]  │
│   Toggle Split View               [\]  │
│   Quit                            [q]  │
│   Help & Settings                [F12] │
│   ...                                  │
└────────────────────────────────────────┘
```

## Files to Create/Modify

- Create: `src/tui/components/command_palette.rs`
- Modify: `src/app/state/types.rs` — add `CommandPalette` to `InputMode`
- Modify: `src/app/state/ui.rs` — add palette state fields
- Modify: `src/app/action.rs` — add `EnterCommandPalette`, `CommandPaletteExecute`
- Modify: `src/tui/event/handlers.rs` — add Ctrl+P handler, palette input handling
- Modify: `src/app/handlers/input.rs` — handle palette actions
- Modify: `src/tui/ui.rs` — render palette overlay
- Modify: `src/tui/components/mod.rs` — add module
