use crate::app::{AppState, UtilityItem};
use std::path::PathBuf;

/// Load utility content based on the selected utility
pub fn load_utility_content(state: &mut AppState) {
    let workspace_path = match state.selected_workspace() {
        Some(ws) => ws.path.clone(),
        None => {
            state.utility_content = vec!["No workspace selected".to_string()];
            return;
        }
    };

    state.utility_scroll_offset = 0;

    match state.selected_utility {
        UtilityItem::BrownNoise => {
            // Brown noise is a toggle, not a content utility
            // This shouldn't be called for toggles, but handle it gracefully
            state.utility_content = vec![
                "".to_string(),
                "  Brown Noise".to_string(),
                "  ===========".to_string(),
                "".to_string(),
                "  Press Enter to toggle brown noise on/off.".to_string(),
            ];
        }
        UtilityItem::TopFiles => {
            load_top_files(&workspace_path, state);
        }
        UtilityItem::Calendar => {
            load_calendar_content(state);
        }
        UtilityItem::GitHistory => {
            load_git_history(&workspace_path, state);
        }
        UtilityItem::FileTree => {
            load_file_tree(&workspace_path, state);
        }
    }
}

/// Load calendar showing when workspace was worked on
fn load_calendar_content(state: &mut AppState) {
    let mut content = vec![
        "".to_string(),
        "  Work History".to_string(),
        "  ============".to_string(),
        "".to_string(),
    ];

    // Show last active for each workspace
    for ws in &state.workspaces {
        let status_icon = match ws.status {
            crate::models::WorkspaceStatus::Working => "[W]",
            crate::models::WorkspaceStatus::Paused => "[P]",
        };
        let last_active = ws.last_active_display();
        content.push(format!("  {} {} - {}", status_icon, ws.name, last_active));
    }

    if state.workspaces.is_empty() {
        content.push("  No workspaces yet".to_string());
    }

    content.push("".to_string());
    content.push("  [W] = Working, [P] = Paused".to_string());

    state.utility_content = content;
}

/// Load git history for the workspace
fn load_git_history(workspace_path: &PathBuf, state: &mut AppState) {
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

    state.utility_content = content;
}

/// Load file tree for the workspace using git ls-files (respects .gitignore)
fn load_file_tree(workspace_path: &PathBuf, state: &mut AppState) {
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
            state.utility_content = content;
            return;
        }
    };

    if files.is_empty() {
        content.push(format!("  {}/", ws_name));
        content.push("  (no tracked files)".to_string());
        state.utility_content = content;
        return;
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

    state.utility_content = content;
}

/// Load top 20 files by lines of code
fn load_top_files(workspace_path: &PathBuf, state: &mut AppState) {
    use std::fs::File;
    use std::io::{BufRead, BufReader};

    let mut content = vec![
        "".to_string(),
        "  Top 20 Files by Lines of Code".to_string(),
        "  ==============================".to_string(),
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
            state.utility_content = content;
            return;
        }
    };

    if files.is_empty() {
        content.push("  (no tracked files)".to_string());
        state.utility_content = content;
        return;
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

    // Take top 20
    let top_files: Vec<_> = file_lines.into_iter().take(20).collect();

    if top_files.is_empty() {
        content.push("  (no files found)".to_string());
        state.utility_content = content;
        return;
    }

    // Find max line count for padding
    let max_lines = top_files.first().map(|(_, c)| *c).unwrap_or(0);
    let line_width = max_lines.to_string().len();

    // Render the list
    for (i, (path, lines)) in top_files.iter().enumerate() {
        let rank = i + 1;
        content.push(format!(
            "  {:>2}. {:>width$} lines  {}",
            rank,
            lines,
            path,
            width = line_width
        ));
    }

    // Total lines
    let total_lines: usize = files.iter()
        .filter_map(|f| {
            let full_path = workspace_path.join(f);
            File::open(&full_path).ok()
                .map(|file| BufReader::new(file).lines().count())
        })
        .sum();

    content.push("".to_string());
    content.push(format!("  Total: {} lines across {} files", total_lines, files.len()));

    state.utility_content = content;
}
