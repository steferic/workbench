use crate::app::{Action, AppState, ConfigTreeNode, UtilityContentPayload, UtilityItem};
use crate::config::keybindings::get_user_config_path;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use tokio::task;

/// Load utility content based on the selected utility
pub fn load_utility_content(state: &mut AppState, action_tx: &mpsc::UnboundedSender<Action>) {
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
                let _ = action_tx.send(Action::UtilityContentLoaded(UtilityContentPayload {
                    request_id,
                    content,
                    pie_chart_data,
                    show_calendar: false,
                }));
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
                let _ = action_tx.send(Action::UtilityContentLoaded(UtilityContentPayload {
                    request_id,
                    content,
                    pie_chart_data: Vec::new(),
                    show_calendar: false,
                }));
            });
        }
        UtilityItem::FileTree => {
            state.ui.utility_content = loading_message("File Tree");
            let action_tx = action_tx.clone();
            task::spawn_blocking(move || {
                let content = build_file_tree(&workspace_path);
                let _ = action_tx.send(Action::UtilityContentLoaded(UtilityContentPayload {
                    request_id,
                    content,
                    pie_chart_data: Vec::new(),
                    show_calendar: false,
                }));
            });
        }
        UtilityItem::SuggestTodos => {
            load_suggest_todos_info(state);
        }
        UtilityItem::Keybindings => {
            load_keybindings_info(state);
        }
        UtilityItem::ToggleBanner => {
            // ToggleBanner is handled directly, not through content loading
            state.ui.utility_content = vec![
                "".to_string(),
                "  Banner Bar".to_string(),
                "  ==========".to_string(),
                "".to_string(),
                "  Press Enter to toggle the banner bar visibility.".to_string(),
                "".to_string(),
                if state.ui.banner_visible {
                    "  Status: Visible".to_string()
                } else {
                    "  Status: Hidden".to_string()
                },
            ];
        }
    }
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

/// Show info about the Suggest Todos utility
fn load_suggest_todos_info(state: &mut AppState) {
    let content = vec![
        "".to_string(),
        "  Suggest Todos".to_string(),
        "  ==============".to_string(),
        "".to_string(),
        "  This utility analyzes your codebase and suggests".to_string(),
        "  potential features, improvements, and refactoring".to_string(),
        "  opportunities as todo items.".to_string(),
        "".to_string(),
        "  Suggested todos appear with a (?) icon and can be:".to_string(),
        "  - Approved with 'y' to become pending todos".to_string(),
        "  - Deleted with 'd' if not relevant".to_string(),
        "".to_string(),
        "  Press Enter to analyze the codebase...".to_string(),
        "".to_string(),
        if state.ui.analyzer_session_id.is_some() {
            "  Status: Analysis in progress...".to_string()
        } else {
            "  Status: Ready".to_string()
        },
    ];
    state.ui.utility_content = content;
}

/// Show keybindings config information
fn load_keybindings_info(state: &mut AppState) {
    let config_path = get_user_config_path();
    let kb = &state.system.keybindings;

    let mut content = vec![
        "".to_string(),
        "  Keybindings".to_string(),
        "  ===========".to_string(),
        "".to_string(),
        format!("  Config: {}", config_path.display()),
        "".to_string(),
        "  Global".to_string(),
        "  ------".to_string(),
    ];

    // Show global bindings
    let mut global_bindings: Vec<_> = kb.global.iter().collect();
    global_bindings.sort_by_key(|(k, _)| k.display());
    for (combo, action) in global_bindings {
        content.push(format!("  {:12}  {}", combo.display(), action));
    }

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
    content.push("  Todos Pane".to_string());
    content.push("  ----------".to_string());
    let mut todo_bindings: Vec<_> = kb.panel_todos_pane.iter().collect();
    todo_bindings.sort_by_key(|(k, _)| k.display());
    for (combo, action) in todo_bindings {
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
    content.push("  Edit the config file to customize keybindings.".to_string());
    content.push("  Restart workbench to apply changes.".to_string());

    state.ui.utility_content = content;
}

/// Load Claude Code config files (deprecated - now using tree view)
#[allow(dead_code)]
pub fn load_claude_config(state: &mut AppState) {
    let home = dirs::home_dir().unwrap_or_default();
    let claude_dir = home.join(".claude");

    let mut content = vec![
        "".to_string(),
        "  Claude Code Config".to_string(),
        "  ==================".to_string(),
        "".to_string(),
        format!("  Directory: {}", claude_dir.display()),
        "".to_string(),
    ];

    // Load settings.json
    let settings_path = claude_dir.join("settings.json");
    content.push("  settings.json".to_string());
    content.push("  -------------".to_string());
    if settings_path.exists() {
        match std::fs::read_to_string(&settings_path) {
            Ok(json) => {
                for line in json.lines() {
                    content.push(format!("  {}", line));
                }
            }
            Err(e) => content.push(format!("  Error: {}", e)),
        }
    } else {
        content.push("  (not found)".to_string());
    }

    content.push("".to_string());

    // Load settings.local.json
    let local_path = claude_dir.join("settings.local.json");
    content.push("  settings.local.json".to_string());
    content.push("  -------------------".to_string());
    if local_path.exists() {
        match std::fs::read_to_string(&local_path) {
            Ok(json) => {
                for line in json.lines() {
                    content.push(format!("  {}", line));
                }
            }
            Err(e) => content.push(format!("  Error: {}", e)),
        }
    } else {
        content.push("  (not found)".to_string());
    }

    content.push("".to_string());

    // Load global CLAUDE.md
    let claude_md_path = claude_dir.join("CLAUDE.md");
    content.push("  CLAUDE.md (global)".to_string());
    content.push("  ------------------".to_string());
    if claude_md_path.exists() {
        match std::fs::read_to_string(&claude_md_path) {
            Ok(md) => {
                if md.is_empty() {
                    content.push("  (empty)".to_string());
                } else {
                    for line in md.lines().take(50) {
                        content.push(format!("  {}", line));
                    }
                    let line_count = md.lines().count();
                    if line_count > 50 {
                        content.push(format!("  ... ({} more lines)", line_count - 50));
                    }
                }
            }
            Err(e) => content.push(format!("  Error: {}", e)),
        }
    } else {
        content.push("  (not found)".to_string());
    }

    content.push("".to_string());

    // List plugins
    let plugins_path = claude_dir.join("plugins").join("installed_plugins.json");
    content.push("  Installed Plugins".to_string());
    content.push("  -----------------".to_string());
    if plugins_path.exists() {
        match std::fs::read_to_string(&plugins_path) {
            Ok(json) => {
                for line in json.lines().take(30) {
                    content.push(format!("  {}", line));
                }
            }
            Err(e) => content.push(format!("  Error: {}", e)),
        }
    } else {
        content.push("  (not found)".to_string());
    }

    state.ui.utility_content = content;
    state.ui.utility_scroll_offset = 0;
    state.ui.pie_chart_data.clear();
    state.ui.show_calendar = false;
}

/// Load Gemini CLI config files (deprecated - now using tree view)
#[allow(dead_code)]
pub fn load_gemini_config(state: &mut AppState) {
    let home = dirs::home_dir().unwrap_or_default();
    let gemini_dir = home.join(".gemini");

    let mut content = vec![
        "".to_string(),
        "  Gemini CLI Config".to_string(),
        "  =================".to_string(),
        "".to_string(),
        format!("  Directory: {}", gemini_dir.display()),
        "".to_string(),
    ];

    // Load settings.json
    let settings_path = gemini_dir.join("settings.json");
    content.push("  settings.json".to_string());
    content.push("  -------------".to_string());
    if settings_path.exists() {
        match std::fs::read_to_string(&settings_path) {
            Ok(json) => {
                for line in json.lines() {
                    content.push(format!("  {}", line));
                }
            }
            Err(e) => content.push(format!("  Error: {}", e)),
        }
    } else {
        content.push("  (not found)".to_string());
    }

    content.push("".to_string());

    // Load global GEMINI.md
    let gemini_md_path = gemini_dir.join("GEMINI.md");
    content.push("  GEMINI.md (global)".to_string());
    content.push("  ------------------".to_string());
    if gemini_md_path.exists() {
        match std::fs::read_to_string(&gemini_md_path) {
            Ok(md) => {
                if md.is_empty() {
                    content.push("  (empty)".to_string());
                } else {
                    for line in md.lines().take(50) {
                        content.push(format!("  {}", line));
                    }
                    let line_count = md.lines().count();
                    if line_count > 50 {
                        content.push(format!("  ... ({} more lines)", line_count - 50));
                    }
                }
            }
            Err(e) => content.push(format!("  Error: {}", e)),
        }
    } else {
        content.push("  (not found)".to_string());
    }

    content.push("".to_string());

    // List extensions
    let extensions_path = gemini_dir.join("extensions");
    content.push("  Extensions".to_string());
    content.push("  ----------".to_string());
    if extensions_path.exists() {
        match std::fs::read_dir(&extensions_path) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    content.push(format!("  - {}", entry.file_name().to_string_lossy()));
                }
            }
            Err(e) => content.push(format!("  Error: {}", e)),
        }
    } else {
        content.push("  (not found)".to_string());
    }

    state.ui.utility_content = content;
    state.ui.utility_scroll_offset = 0;
    state.ui.pie_chart_data.clear();
    state.ui.show_calendar = false;
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

/// Load file tree for the workspace using git ls-files (respects .gitignore)
fn build_file_tree(workspace_path: &Path) -> Vec<String> {
    use std::collections::BTreeMap;

    let mut content = vec![
        "".to_string(),
        "  File Tree".to_string(),
        "  =========".to_string(),
        "".to_string(),
    ];

    // Get workspace name for root
    let ws_name = workspace_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".");

    // Try git ls-files first (respects .gitignore)
    let output = std::process::Command::new("git")
        .args(["ls-files"])
        .current_dir(workspace_path)
        .output();

    let files: Vec<String> = match output {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|s| s.to_string())
                .collect()
        }
        _ => {
            // Fallback: manual directory walk (limited)
            content.push(format!("  {}/", ws_name));
            content.push("  (not a git repository)".to_string());
            return content;
        }
    };

    if files.is_empty() {
        content.push(format!("  {}/", ws_name));
        content.push("  (no tracked files)".to_string());
        return content;
    }

    // Build tree structure: path -> children (BTreeMap for sorted order)
    #[derive(Default)]
    struct TreeNode {
        children: BTreeMap<String, TreeNode>,
        is_file: bool,
    }

    let mut root = TreeNode::default();

    for file_path in &files {
        let parts: Vec<&str> = file_path.split('/').collect();
        let mut current = &mut root;

        for (i, part) in parts.iter().enumerate() {
            let is_last = i == parts.len() - 1;
            current = current.children.entry(part.to_string()).or_default();
            if is_last {
                current.is_file = true;
            }
        }
    }

    // Render tree with visual characters
    content.push(format!("  {}/", ws_name));

    fn render_tree(
        node: &TreeNode,
        prefix: &str,
        content: &mut Vec<String>,
    ) {
        let entries: Vec<_> = node.children.iter().collect();
        let count = entries.len();

        for (i, (name, child)) in entries.iter().enumerate() {
            let is_last = i == count - 1;
            let connector = if is_last { "└── " } else { "├── " };
            let child_prefix = if is_last { "    " } else { "│   " };

            let display_name = if child.is_file && child.children.is_empty() {
                name.to_string()
            } else {
                format!("{}/", name)
            };

            content.push(format!("  {}{}{}", prefix, connector, display_name));

            // Recursively render children (but limit depth to avoid huge trees)
            if !child.children.is_empty() && prefix.len() < 40 {
                render_tree(child, &format!("{}{}", prefix, child_prefix), content);
            }
        }
    }

    render_tree(&root, "", &mut content);

    // Add file count
    content.push("".to_string());
    content.push(format!("  {} files tracked", files.len()));

    content
}

/// Load top 20 files by lines of code with pie chart visualization
fn build_top_files(
    workspace_path: &Path,
) -> (Vec<String>, Vec<(String, f64, ratatui::style::Color)>) {
    use ratatui::style::Color;
    use std::fs::File;
    use std::io::{BufRead, BufReader};

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
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|s| s.to_string())
                .collect()
        }
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
        if let Ok(file) = File::open(&full_path) {
            let reader = BufReader::new(file);
            let line_count = reader.lines().count();
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
        let label = path
            .split('/')
            .next_back()
            .unwrap_or(path)
            .to_string();
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
    content.push(format!("  Total: {} lines across {} files", all_total, files.len()));

    (content, pie_chart_data)
}

/// Initialize the config tree with CLI agent config directories
pub fn init_config_tree(state: &mut AppState) {
    let home = dirs::home_dir().unwrap_or_default();

    let mut nodes = Vec::new();

    // Claude root
    let claude_path = home.join(".claude");
    if claude_path.exists() {
        nodes.push(ConfigTreeNode::Root {
            name: "Claude".to_string(),
            path: claude_path,
            expanded: false,
        });
    }

    // Gemini root
    let gemini_path = home.join(".gemini");
    if gemini_path.exists() {
        nodes.push(ConfigTreeNode::Root {
            name: "Gemini".to_string(),
            path: gemini_path,
            expanded: false,
        });
    }

    // Codex root
    let codex_path = home.join(".codex");
    if codex_path.exists() {
        nodes.push(ConfigTreeNode::Root {
            name: "Codex".to_string(),
            path: codex_path,
            expanded: false,
        });
    }

    // Grok root
    let grok_path = home.join(".grok");
    if grok_path.exists() {
        nodes.push(ConfigTreeNode::Root {
            name: "Grok".to_string(),
            path: grok_path,
            expanded: false,
        });
    }

    state.ui.config_tree_nodes = nodes;
    state.ui.config_tree_selected = 0;
}

/// Toggle expansion of a tree node (root or directory)
/// Unused - keeping for potential tree view
#[allow(dead_code)]
pub fn toggle_config_tree_node(state: &mut AppState) {
    if state.ui.config_tree_nodes.is_empty() {
        return;
    }

    let selected = state.ui.config_tree_selected;
    if selected >= state.ui.config_tree_nodes.len() {
        return;
    }

    // Check if selected node is expandable
    let node = &state.ui.config_tree_nodes[selected];
    if !node.is_expandable() {
        return;
    }

    let is_expanded = node.is_expanded();
    let node_path = node.path().to_path_buf();
    let node_depth = node.depth();

    if is_expanded {
        // Collapse: remove all children (nodes after this one with greater depth)
        // Toggle the expanded flag first
        toggle_expanded(&mut state.ui.config_tree_nodes[selected]);

        // Find how many children to remove
        let mut remove_count = 0;
        for i in (selected + 1)..state.ui.config_tree_nodes.len() {
            if state.ui.config_tree_nodes[i].depth() > node_depth {
                remove_count += 1;
            } else {
                break;
            }
        }

        // Remove children
        for _ in 0..remove_count {
            state.ui.config_tree_nodes.remove(selected + 1);
        }
    } else {
        // Expand: add children right after this node
        toggle_expanded(&mut state.ui.config_tree_nodes[selected]);

        let children = get_directory_children(&node_path, node_depth + 1);

        // Insert children after selected node
        let insert_pos = selected + 1;
        for (i, child) in children.into_iter().enumerate() {
            state.ui.config_tree_nodes.insert(insert_pos + i, child);
        }
    }
}

#[allow(dead_code)]
fn toggle_expanded(node: &mut ConfigTreeNode) {
    match node {
        ConfigTreeNode::Root { expanded, .. } => *expanded = !*expanded,
        ConfigTreeNode::Directory { expanded, .. } => *expanded = !*expanded,
        ConfigTreeNode::File { .. } => {}
    }
}

/// Get children of a directory as tree nodes
#[allow(dead_code)]
fn get_directory_children(path: &Path, depth: usize) -> Vec<ConfigTreeNode> {
    let mut children = Vec::new();

    if let Ok(entries) = std::fs::read_dir(path) {
        let mut dirs = Vec::new();
        let mut files = Vec::new();

        for entry in entries.flatten() {
            let entry_path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            // Skip hidden files/directories except specific config files
            if name.starts_with('.') && !matches!(name.as_str(), ".gitignore" | ".env" | ".env.local") {
                continue;
            }

            if entry_path.is_dir() {
                dirs.push(ConfigTreeNode::Directory {
                    name,
                    path: entry_path,
                    expanded: false,
                    depth,
                });
            } else {
                files.push(ConfigTreeNode::File {
                    name,
                    path: entry_path,
                    depth,
                });
            }
        }

        // Sort: directories first (alphabetically), then files (alphabetically)
        dirs.sort_by(|a: &ConfigTreeNode, b: &ConfigTreeNode| a.name().cmp(b.name()));
        files.sort_by(|a: &ConfigTreeNode, b: &ConfigTreeNode| a.name().cmp(b.name()));

        children.extend(dirs);
        children.extend(files);
    }

    children
}

/// Get the path of the currently selected config tree node (if it's a file)
#[allow(dead_code)]
pub fn get_selected_config_file_path(state: &AppState) -> Option<PathBuf> {
    if state.ui.config_tree_nodes.is_empty() {
        return None;
    }

    let selected = state.ui.config_tree_selected;
    if selected >= state.ui.config_tree_nodes.len() {
        return None;
    }

    let node = &state.ui.config_tree_nodes[selected];
    if node.is_file() {
        Some(node.path().to_path_buf())
    } else {
        None
    }
}
