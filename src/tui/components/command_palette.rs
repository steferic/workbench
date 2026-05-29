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
        PaletteEntry {
            name: "Create Claude Session",
            action: Action::CreateSession(crate::models::AgentType::Claude, false, false),
            keybinding: "1",
        },
        PaletteEntry {
            name: "Create Gemini Session",
            action: Action::CreateSession(crate::models::AgentType::Gemini, false, false),
            keybinding: "2",
        },
        PaletteEntry {
            name: "Create Codex Session",
            action: Action::CreateSession(crate::models::AgentType::Codex, false, false),
            keybinding: "3",
        },
        PaletteEntry {
            name: "Create Grok Session",
            action: Action::CreateSession(crate::models::AgentType::Grok, false, false),
            keybinding: "4",
        },
        PaletteEntry {
            name: "Create Terminal",
            action: Action::CreateTerminal,
            keybinding: "t",
        },
        PaletteEntry {
            name: "Start Parallel Task",
            action: Action::EnterParallelTaskMode,
            keybinding: "P",
        },
        PaletteEntry {
            name: "New Workspace",
            action: Action::EnterWorkspaceActionMode,
            keybinding: "n",
        },
        PaletteEntry {
            name: "Toggle Split View",
            action: Action::ToggleSplitView,
            keybinding: "\\",
        },
        PaletteEntry {
            name: "Cycle Next Workspace",
            action: Action::CycleNextWorkspace,
            keybinding: "F7",
        },
        PaletteEntry {
            name: "Cycle Prev Workspace",
            action: Action::CyclePrevWorkspace,
            keybinding: "F6",
        },
        PaletteEntry {
            name: "Cycle Next Session",
            action: Action::CycleNextSession,
            keybinding: "F9",
        },
        PaletteEntry {
            name: "Cycle Prev Session",
            action: Action::CyclePrevSession,
            keybinding: "F8",
        },
        PaletteEntry {
            name: "Help & Settings",
            action: Action::EnterConfigWindow,
            keybinding: "F1",
        },
        PaletteEntry {
            name: "Toggle Debug Overlay",
            action: Action::ToggleDebugOverlay,
            keybinding: "",
        },
        PaletteEntry {
            name: "Quit",
            action: Action::InitiateQuit,
            keybinding: "q",
        },
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

    let input_text = format!("> {}_", state.ui.palette.query);
    let input_paragraph = Paragraph::new(Line::from(vec![Span::styled(
        input_text,
        Style::default().fg(Color::White),
    )]))
    .block(input_block);

    frame.render_widget(input_paragraph, chunks[0]);

    // Results list
    let entries = filtered_entries(&state.ui.palette.query);

    let items: Vec<ListItem> = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let is_selected = i == state.ui.palette.selected;
            let name_style = if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
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
        list_state.select(Some(
            state
                .ui
                .palette.selected
                .min(entries.len().saturating_sub(1)),
        ));
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
