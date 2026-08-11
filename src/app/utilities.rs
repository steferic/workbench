use crate::app::{Action, AppState, UtilityContentPayload, UtilityItem};
use std::path::Path;
use tokio::sync::mpsc;
use tokio::task;

fn queue_utility_content(
    action_tx: &mpsc::UnboundedSender<Action>,
    payload: UtilityContentPayload,
    context: &str,
) {
    if let Err(err) = action_tx.send(Action::UtilityContentLoaded(payload)) {
        crate::logger::warn(format!("{context}: {err}"));
    }
}

/// Load utility content based on the selected utility
pub fn load_utility_content(state: &mut AppState, action_tx: &mpsc::UnboundedSender<Action>) {
    // Ahead of the workspace check, because the theme is the one utility that
    // is not about a project: it reads nothing off disk and applies to the
    // whole app, so "No workspace selected" would be both wrong and, on a
    // fresh install with nothing set up yet, the only thing you could see.
    if state.ui.selected_utility == UtilityItem::ToggleTheme {
        state.ui.utility_scroll_offset = 0;
        state.ui.utility_content = theme_picker_lines(state.ui.theme_mode);
        return;
    }

    let workspace_path = match state.selected_workspace() {
        Some(ws) => ws.path.clone(),
        None => {
            state.ui.utility_content = vec!["No workspace selected".to_string()];
            state.ui.pie_chart_data.clear();
            return;
        }
    };

    state.ui.utility_scroll_offset = 0;
    // Clear special view flags (only specific utilities set these)
    state.ui.pie_chart_data.clear();
    state.ui.show_calendar = false;
    state.ui.utility_request_id = state.ui.utility_request_id.wrapping_add(1);
    let request_id = state.ui.utility_request_id;

    match state.ui.selected_utility {
        UtilityItem::BrownNoise => {
            // Brown noise is a toggle, not a content utility
            // This shouldn't be called for toggles, but handle it gracefully
            state.ui.utility_content = vec![
                "".to_string(),
                "  Brown Noise".to_string(),
                "  ===========".to_string(),
                "".to_string(),
                "  Press Enter to toggle brown noise on/off.".to_string(),
            ];
        }
        UtilityItem::ClassicalRadio => {
            state.ui.utility_content = vec![
                "".to_string(),
                "  Classical Radio".to_string(),
                "  ===============".to_string(),
                "".to_string(),
                "  WRTI 90.1 - Philadelphia's classical music station.".to_string(),
                "".to_string(),
                "  Press Enter to toggle stream on/off.".to_string(),
            ];
        }
        UtilityItem::OceanWaves => {
            state.ui.utility_content = vec![
                "".to_string(),
                "  Ocean".to_string(),
                "  =====".to_string(),
                "".to_string(),
                "  Relaxing ocean and waterside sounds.".to_string(),
                "".to_string(),
                "  Press Enter to toggle on/off.".to_string(),
            ];
        }
        UtilityItem::WindChimes => {
            state.ui.utility_content = vec![
                "".to_string(),
                "  Chimes".to_string(),
                "  ======".to_string(),
                "".to_string(),
                "  Peaceful wind chime sounds.".to_string(),
                "".to_string(),
                "  Press Enter to toggle on/off.".to_string(),
            ];
        }
        UtilityItem::RainforestRain => {
            state.ui.utility_content = vec![
                "".to_string(),
                "  Rain".to_string(),
                "  ====".to_string(),
                "".to_string(),
                "  Soothing rainforest rain sounds.".to_string(),
                "".to_string(),
                "  Press Enter to toggle on/off.".to_string(),
            ];
        }
        UtilityItem::TopFiles => {
            state.ui.utility_content = loading_message("Top Files by Lines of Code");
            let action_tx = action_tx.clone();
            task::spawn_blocking(move || {
                let (content, pie_chart_data) = build_top_files(&workspace_path);
                queue_utility_content(
                    &action_tx,
                    UtilityContentPayload {
                        request_id,
                        content,
                        pie_chart_data,
                        show_calendar: false,
                    },
                    "failed to load top files utility content",
                );
            });
        }
        UtilityItem::Calendar => {
            load_calendar_content(state);
        }
        UtilityItem::GitHistory => {
            state.ui.utility_content = loading_message("Git History");
            let action_tx = action_tx.clone();
            task::spawn_blocking(move || {
                let content = build_git_history(&workspace_path);
                queue_utility_content(
                    &action_tx,
                    UtilityContentPayload {
                        request_id,
                        content,
                        pie_chart_data: Vec::new(),
                        show_calendar: false,
                    },
                    "failed to load git history utility content",
                );
            });
        }
        UtilityItem::Keybindings => {
            load_keybindings_info(state);
        }
        // Handled above the workspace check.
        UtilityItem::ToggleTheme => {}
    }
}

/// The theme list, with a marker on the one in use.
///
/// Enter steps to the next theme rather than opening a chooser, so this pane is
/// the chooser: it is the list you are stepping through, and the marker is
/// where you are in it.
fn theme_picker_lines(current: crate::theme::ThemeMode) -> Vec<String> {
    let mut lines = vec![
        String::new(),
        "  Theme".to_string(),
        "  =====".to_string(),
        String::new(),
        "  Press Enter to step to the next theme.".to_string(),
        String::new(),
    ];
    // Grouped by ground, which is the first thing you are choosing; `ALL` is in
    // Enter's stepping order, which interleaves the two.
    for (heading, dark) in [("  On a dark ground", true), ("  On a light ground", false)] {
        lines.push(heading.to_string());
        for theme in crate::theme::ThemeMode::ALL
            .iter()
            .filter(|t| t.is_dark() == dark)
        {
            lines.push(format!(
                "  {} {:<11} {}",
                if *theme == current { "▶" } else { " " },
                theme.label(),
                theme.detail(),
            ));
        }
        lines.push(String::new());
    }
    lines
}

fn loading_message(title: &str) -> Vec<String> {
    vec![
        "".to_string(),
        format!("  {}", title),
        format!("  {}", "=".repeat(title.len())),
        "".to_string(),
        "  Loading...".to_string(),
    ]
}

/// Show keybindings config information
fn load_keybindings_info(state: &mut AppState) {
    let config_path = crate::config::user_config::get_user_config_path()
        .unwrap_or_else(|_| std::path::PathBuf::from("~/.config/workbench/user_config.toml"));
    let kb = &state.system.keybindings;
    let hotkeys = &state.system.user_config.global_hotkeys;

    let mut content = vec![
        "".to_string(),
        "  Keybindings".to_string(),
        "  ===========".to_string(),
        "".to_string(),
        format!("  Global hotkeys: {}", config_path.display()),
        "".to_string(),
        "  Global".to_string(),
        "  ------".to_string(),
    ];

    // Show global bindings
    for action in crate::config::user_config::ordered_global_hotkey_actions(hotkeys) {
        let key = hotkeys
            .get(&action)
            .map(|binding| binding.as_str())
            .unwrap_or("");
        content.push(format!("  {:12}  {}", key, action));
    }

    content.push("".to_string());
    content.push("  Built-in Panel Navigation".to_string());
    content.push("  -------------------------".to_string());
    content.push("".to_string());
    content.push("  Workspace List".to_string());
    content.push("  --------------".to_string());
    let mut ws_bindings: Vec<_> = kb.panel_workspace_list.iter().collect();
    ws_bindings.sort_by_key(|(k, _)| k.display());
    for (combo, action) in ws_bindings {
        content.push(format!("  {:12}  {}", combo.display(), action));
    }

    content.push("".to_string());
    content.push("  Session List".to_string());
    content.push("  ------------".to_string());
    let mut sess_bindings: Vec<_> = kb.panel_session_list.iter().collect();
    sess_bindings.sort_by_key(|(k, _)| k.display());
    for (combo, action) in sess_bindings {
        content.push(format!("  {:12}  {}", combo.display(), action));
    }

    content.push("".to_string());
    content.push("  Tasks Pane".to_string());
    content.push("  ----------".to_string());
    let mut task_bindings: Vec<_> = kb.panel_tasks_pane.iter().collect();
    task_bindings.sort_by_key(|(k, _)| k.display());
    for (combo, action) in task_bindings {
        content.push(format!("  {:12}  {}", combo.display(), action));
    }

    content.push("".to_string());
    content.push("  Utilities Pane".to_string());
    content.push("  --------------".to_string());
    let mut util_bindings: Vec<_> = kb.panel_utilities_pane.iter().collect();
    util_bindings.sort_by_key(|(k, _)| k.display());
    for (combo, action) in util_bindings {
        content.push(format!("  {:12}  {}", combo.display(), action));
    }

    content.push("".to_string());
    content.push("  Output Pane".to_string());
    content.push("  -----------".to_string());
    let mut out_bindings: Vec<_> = kb.panel_output_pane.iter().collect();
    out_bindings.sort_by_key(|(k, _)| k.display());
    for (combo, action) in out_bindings {
        content.push(format!("  {:12}  {}", combo.display(), action));
    }

    content.push("".to_string());
    content.push("  Pinned Terminal".to_string());
    content.push("  ---------------".to_string());
    let mut pinned_bindings: Vec<_> = kb.panel_pinned_terminal.iter().collect();
    pinned_bindings.sort_by_key(|(k, _)| k.display());
    for (combo, action) in pinned_bindings {
        content.push(format!("  {:12}  {}", combo.display(), action));
    }

    content.push("".to_string());
    content.push("  Global hotkeys are configured in user_config.toml.".to_string());
    content.push("  Panel bindings listed here are built-in defaults.".to_string());

    state.ui.utility_content = content;
}

/// Load calendar with work history
fn load_calendar_content(state: &mut AppState) {
    // Set flag to show calendar widget
    state.ui.show_calendar = true;

    // The calendar widget will be rendered in output_pane
    // We just need some minimal content for the legend/info section
    let mut content = vec![
        "".to_string(),
        "  Work History".to_string(),
        "  ============".to_string(),
        "".to_string(),
    ];

    // Show last active for each workspace
    for ws in &state.data.workspaces {
        let status_icon = match ws.status {
            crate::models::WorkspaceStatus::Working => "●",
            crate::models::WorkspaceStatus::Paused => "○",
        };
        let last_active = ws.last_active_display();
        content.push(format!("  {} {} - {}", status_icon, ws.name, last_active));
    }

    if state.data.workspaces.is_empty() {
        content.push("  No workspaces yet".to_string());
    }

    content.push("".to_string());
    content.push("  ● = Working, ○ = Paused".to_string());
    content.push("  Today is highlighted in blue".to_string());

    state.ui.utility_content = content;
}

/// Load git history for the workspace
fn build_git_history(workspace_path: &Path) -> Vec<String> {
    let output = std::process::Command::new("git")
        .args(["log", "--oneline", "-30"])
        .current_dir(workspace_path)
        .output();

    let mut content = vec![
        "".to_string(),
        "  Git History (last 30 commits)".to_string(),
        "  =============================".to_string(),
        "".to_string(),
    ];

    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                content.push(format!("  {}", line));
            }
            if stdout.is_empty() {
                content.push("  No commits yet".to_string());
            }
        }
        Ok(_) => {
            content.push("  Not a git repository".to_string());
        }
        Err(e) => {
            content.push(format!("  Error: {}", e));
        }
    }

    content
}

/// Load top 20 files by lines of code with pie chart visualization
fn build_top_files(
    workspace_path: &Path,
) -> (Vec<String>, Vec<(String, f64, ratatui::style::Color)>) {
    use ratatui::style::Color;

    let mut content = vec![
        "".to_string(),
        "  Top Files by Lines of Code".to_string(),
        "  ==========================".to_string(),
        "".to_string(),
    ];

    // Get tracked files using git ls-files
    let output = std::process::Command::new("git")
        .args(["ls-files"])
        .current_dir(workspace_path)
        .output();

    let files: Vec<String> = match output {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|s| s.to_string())
            .collect(),
        _ => {
            content.push("  (not a git repository)".to_string());
            return (content, Vec::new());
        }
    };

    if files.is_empty() {
        content.push("  (no tracked files)".to_string());
        return (content, Vec::new());
    }

    // Count lines for each file
    let mut file_lines: Vec<(String, usize)> = Vec::new();

    for file_path in &files {
        let full_path = workspace_path.join(file_path);
        if let Ok(bytes) = std::fs::read(&full_path) {
            let line_count = bytes.iter().filter(|&&b| b == b'\n').count();
            file_lines.push((file_path.clone(), line_count));
        }
    }

    // Sort by line count descending
    file_lines.sort_by(|a, b| b.1.cmp(&a.1));

    // Take top 10 for pie chart
    let top_files: Vec<_> = file_lines.iter().take(10).cloned().collect();

    if top_files.is_empty() {
        content.push("  (no files found)".to_string());
        return (content, Vec::new());
    }

    // Colors for the pie chart slices
    let colors = [
        Color::Cyan,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Red,
        Color::LightCyan,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
    ];

    // Calculate total for top files and "other"
    let top_total: usize = top_files.iter().map(|(_, c)| c).sum();
    let all_total: usize = file_lines.iter().map(|(_, c)| c).sum();
    let other_total = all_total.saturating_sub(top_total);

    // Populate pie chart data
    let mut pie_chart_data = Vec::new();
    for (i, (path, lines)) in top_files.iter().enumerate() {
        // Get file name only for label
        let label = path.split('/').next_back().unwrap_or(path).to_string();
        pie_chart_data.push((label, *lines as f64, colors[i % colors.len()]));
    }

    // Add "Other" slice if there are more files
    if other_total > 0 {
        pie_chart_data.push(("Other".to_string(), other_total as f64, Color::DarkGray));
    }

    // Text summary below the chart
    content.push("  Legend:".to_string());
    content.push("".to_string());

    // Find max line count for padding
    let max_lines = top_files.first().map(|(_, c)| *c).unwrap_or(0);
    let line_width = max_lines.to_string().len();

    // Render the list with color indicators
    for (i, (path, lines)) in top_files.iter().enumerate() {
        let color_char = match colors[i % colors.len()] {
            Color::Cyan => "●",
            Color::Green => "●",
            Color::Yellow => "●",
            Color::Blue => "●",
            Color::Magenta => "●",
            Color::Red => "●",
            Color::LightCyan => "○",
            Color::LightGreen => "○",
            Color::LightYellow => "○",
            Color::LightBlue => "○",
            _ => "●",
        };
        let pct = (*lines as f64 / all_total as f64 * 100.0) as usize;
        content.push(format!(
            "  {} {:>width$} ({:>2}%)  {}",
            color_char,
            lines,
            pct,
            path,
            width = line_width
        ));
    }

    if other_total > 0 {
        let pct = (other_total as f64 / all_total as f64 * 100.0) as usize;
        content.push(format!(
            "  ● {:>width$} ({:>2}%)  Other ({} files)",
            other_total,
            pct,
            file_lines.len().saturating_sub(10),
            width = line_width
        ));
    }

    content.push("".to_string());
    content.push(format!(
        "  Total: {} lines across {} files",
        all_total,
        files.len()
    ));

    (content, pie_chart_data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeMode;

    #[test]
    fn the_picker_lists_every_theme_and_marks_the_one_in_use() {
        let lines = theme_picker_lines(ThemeMode::Botan);
        let body = lines.join("\n");

        for theme in ThemeMode::ALL {
            assert!(
                body.contains(theme.label()),
                "{} is missing from the picker",
                theme.label(),
            );
        }
        // Exactly one marker, on the theme actually in use.
        let marked: Vec<&str> = lines
            .iter()
            .filter(|l| l.contains('▶'))
            .map(String::as_str)
            .collect();
        assert_eq!(marked.len(), 1, "expected one marker, got {marked:?}");
        assert!(
            marked[0].contains("Botan"),
            "marker sits on {:?}",
            marked[0]
        );

        // Both grounds are named, so the list reads as two groups.
        assert!(body.contains("On a dark ground") && body.contains("On a light ground"));
    }

    /// The theme is the one utility that applies to the whole app rather than
    /// to a project, and it has to work on a fresh install with nothing set up.
    #[test]
    fn the_theme_list_does_not_need_a_workspace() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut state = AppState::new();
        assert!(state.selected_workspace().is_none());

        state.ui.selected_utility = UtilityItem::ToggleTheme;
        load_utility_content(&mut state, &tx);

        let body = state.ui.utility_content.join("\n");
        assert!(!body.contains("No workspace selected"), "got: {body}");
        assert!(body.contains("Kimidori"), "got: {body}");
    }
}
