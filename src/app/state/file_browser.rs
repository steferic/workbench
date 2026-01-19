use std::path::{Path, PathBuf};

use super::AppState;

impl AppState {
    pub fn refresh_file_browser(&mut self) {
        self.ui.file_browser_all_entries.clear();
        self.ui.file_browser_entries.clear();
        self.ui.file_browser_selected = 0;
        self.ui.file_browser_scroll = 0;

        if let Ok(entries) = std::fs::read_dir(&self.ui.file_browser_path) {
            let mut dirs: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .filter(|p| {
                    // Filter out hidden directories (starting with .)
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| !n.starts_with('.'))
                        .unwrap_or(false)
                })
                .collect();

            // Sort alphabetically
            dirs.sort_by(|a, b| {
                a.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_lowercase()
                    .cmp(
                        &b.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .to_lowercase(),
                    )
            });

            self.ui.file_browser_all_entries = dirs;
        }
        self.apply_file_browser_filter();
    }

    pub fn file_browser_enter_selected(&mut self) {
        if let Some(path) = self.ui.file_browser_entries.get(self.ui.file_browser_selected).cloned() {
            self.ui.file_browser_path = path;
            self.ui.file_browser_query.clear();
            self.refresh_file_browser();
        }
    }

    pub fn file_browser_go_up(&mut self) {
        if let Some(parent) = self.ui.file_browser_path.parent() {
            self.ui.file_browser_path = parent.to_path_buf();
            self.ui.file_browser_query.clear();
            self.refresh_file_browser();
        }
    }

    pub fn apply_file_browser_filter(&mut self) {
        let query = self.ui.file_browser_query.trim();
        if query.is_empty() {
            self.ui.file_browser_entries = self.ui.file_browser_all_entries.clone();
            self.ui.file_browser_selected = 0;
            self.ui.file_browser_scroll = 0;
            return;
        }

        let query_lower = query.to_ascii_lowercase();
        if let Some(path) = resolve_query_path(&self.ui.file_browser_path, query) {
            self.ui.file_browser_path = path;
            self.ui.file_browser_query.clear();
            self.refresh_file_browser();
            return;
        }

        let mut matches: Vec<(usize, String, PathBuf)> = Vec::new();
        let use_absolute = query.starts_with('/');

        for path in &self.ui.file_browser_all_entries {
            let candidate = if use_absolute {
                path.to_string_lossy().to_string()
            } else {
                shorten_home_path(path)
            };
            if let Some(score) = fuzzy_score(&query_lower, &candidate) {
                matches.push((score, candidate.to_ascii_lowercase(), path.clone()));
            }
        }

        matches.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        self.ui.file_browser_entries = matches.into_iter().map(|(_, _, path)| path).collect();
        self.ui.file_browser_selected = 0;
        self.ui.file_browser_scroll = 0;
    }
}

pub fn shorten_home_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let (Some(home_str), Some(path_str)) = (home.to_str(), path.to_str()) {
            if let Some(stripped) = path_str.strip_prefix(home_str) {
                return format!("~{}", stripped);
            }
        }
    }
    path.to_string_lossy().to_string()
}

fn resolve_query_path(base: &Path, query: &str) -> Option<PathBuf> {
    if !is_path_like(query) {
        return None;
    }

    let expanded = if let Some(rest) = query.strip_prefix("~/") {
        dirs::home_dir().map(|home| home.join(rest))?
    } else {
        PathBuf::from(query)
    };

    let candidates = if expanded.is_absolute() {
        vec![expanded]
    } else {
        let mut list = vec![base.join(&expanded)];
        if let Some(home) = dirs::home_dir() {
            list.push(home.join(&expanded));
        }
        list
    };

    candidates.into_iter().find(|c| c.exists() && c.is_dir())
}

fn is_path_like(query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return false;
    }
    if query.starts_with('/') {
        return true;
    }
    if query.starts_with('~') || query.starts_with('.') {
        return query.len() > 1;
    }
    query.contains('/')
}

fn fuzzy_score(query_lower: &str, candidate: &str) -> Option<usize> {
    if query_lower.is_empty() {
        return Some(0);
    }

    let candidate_lower = candidate.to_ascii_lowercase();
    let mut score = 0usize;
    let mut last_match: Option<usize> = None;
    let mut search_start = 0usize;

    for qch in query_lower.chars() {
        let mut found = None;
        for (idx, cch) in candidate_lower[search_start..].char_indices() {
            if cch == qch {
                found = Some(search_start + idx);
                break;
            }
        }

        let match_idx = found?;
        if let Some(prev) = last_match {
            score += match_idx.saturating_sub(prev + 1);
        } else {
            score += match_idx;
        }
        last_match = Some(match_idx);
        search_start = match_idx + 1;
    }

    Some(score)
}
