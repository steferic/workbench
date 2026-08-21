use serde::{Deserialize, Serialize};

/// What to call a model on screen.
///
/// Agents journal an id built for an API rather than for a status bar —
/// `claude-opus-5`, `claude-haiku-4-5-20251001`, `gpt-5.6-sol`. What is worth
/// the width is the family and the version, so that is what comes back:
/// "Opus 5", "Haiku 4.5", "GPT-5.6 Sol".
///
/// Deliberately not a table of known ids. A model this does not recognise is
/// the one case that matters — it is the new one, and printing it tidily
/// without being taught is the whole job.
pub fn model_label(raw: &str) -> String {
    // `claude-opus-5[1m]` — a context-window variant, not another model, and
    // noise in a column this narrow.
    let trimmed = raw.split('[').next().unwrap_or(raw).trim();
    // An agent that routes through a gateway names the vendor ahead of a slash
    // — `google/gemini-3.7-flash`, `z-ai/glm-5v-turbo`. Same argument as the
    // `claude-` prefix below: which company made it is not what a column this
    // narrow is for. A trailing slash names nothing, so keep the whole id.
    let trimmed = trimmed
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(trimmed);
    let mut parts: Vec<&str> = trimmed.split('-').filter(|p| !p.is_empty()).collect();
    // A trailing 8-digit date is how a snapshot gets pinned. The version above
    // it already says which model this is.
    if parts
        .last()
        .is_some_and(|p| p.len() == 8 && p.chars().all(|c| c.is_ascii_digit()))
    {
        parts.pop();
    }
    // The vendor is already obvious from the agent it is running in.
    if parts.first() == Some(&"claude") {
        parts.remove(0);
    }
    if parts.is_empty() {
        return trimmed.to_string();
    }

    let mut out: Vec<String> = Vec::new();
    for part in parts {
        let numeric = part.chars().all(|c| c.is_ascii_digit() || c == '.');
        let follows_number = out
            .last()
            .is_some_and(|last: &String| last.chars().any(|c| c.is_ascii_digit()));
        if numeric && follows_number {
            // `haiku-4-5` is one version number that the id split with its own
            // separator; put it back together as 4.5 rather than "4 5".
            let last = out.last_mut().expect("checked by follows_number");
            last.push('.');
            last.push_str(part);
        } else {
            out.push(match part {
                "gpt" => "GPT".to_string(),
                other => capitalise(other),
            });
        }
    }
    // GPT wears its version on a hyphen, which is how OpenAI writes it.
    if out.len() > 1 && out[0] == "GPT" && out[1].starts_with(|c: char| c.is_ascii_digit()) {
        let version = out.remove(1);
        out[0] = format!("GPT-{version}");
    }
    out.join(" ")
}

fn capitalise(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentType {
    Claude,
    Gemini,
    Codex,
    Grok,
    Custom {
        command: String,
        display_name: String,
        badge: String,
    },
    Terminal(String), // Named terminal with custom name
}

impl AgentType {
    pub fn command(&self) -> &str {
        match self {
            AgentType::Claude => "claude",
            AgentType::Gemini => "gemini",
            AgentType::Codex => "codex",
            AgentType::Grok => "grok",
            AgentType::Custom { command, .. } => command.as_str(),
            AgentType::Terminal(_) => {
                // Use $SHELL or default to bash
                std::env::var("SHELL")
                    .ok()
                    .map(|_| "shell") // Placeholder, actual shell resolved at spawn time
                    .unwrap_or("bash")
            }
        }
    }

    pub fn display_name(&self) -> String {
        match self {
            AgentType::Claude => "Claude".to_string(),
            AgentType::Gemini => "Gemini".to_string(),
            AgentType::Codex => "Codex".to_string(),
            AgentType::Grok => "Grok".to_string(),
            AgentType::Custom { display_name, .. } => display_name.clone(),
            AgentType::Terminal(name) => name.clone(),
        }
    }

    pub fn badge(&self) -> String {
        match self {
            AgentType::Claude => "C".to_string(),
            AgentType::Gemini => "G".to_string(),
            AgentType::Codex => "X".to_string(),
            AgentType::Grok => "K".to_string(),
            AgentType::Custom { badge, .. } => badge.clone(),
            AgentType::Terminal(_) => "T".to_string(),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, AgentType::Terminal(_))
    }

    pub fn is_agent(&self) -> bool {
        !self.is_terminal()
    }

    pub fn is_codex_like(&self) -> bool {
        match self {
            AgentType::Codex => true,
            AgentType::Custom { command, .. } => command == "codex",
            _ => false,
        }
    }

    pub fn is_redraw_style(&self) -> bool {
        match self {
            AgentType::Claude | AgentType::Codex => true,
            AgentType::Custom { command, .. } => command == "claude" || command == "codex",
            _ => false,
        }
    }
}

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

#[cfg(test)]
mod model_label_tests {
    use super::model_label;

    #[test]
    fn it_prints_the_family_and_version_a_status_bar_has_room_for() {
        // The ask: "I want to see Opus 5, not just generic Claude".
        assert_eq!(model_label("claude-opus-5"), "Opus 5");
        assert_eq!(model_label("claude-sonnet-5"), "Sonnet 5");
        // A version the id split with its own separator is one number.
        assert_eq!(model_label("claude-haiku-4-5"), "Haiku 4.5");
        // A pinned snapshot's date says nothing the version does not.
        assert_eq!(model_label("claude-haiku-4-5-20251001"), "Haiku 4.5");
        // A context-window variant is the same model.
        assert_eq!(model_label("claude-opus-5[1m]"), "Opus 5");
    }

    /// An agent pointed at a gateway journals `vendor/model`, which would
    /// otherwise print as "Google/gemini 3.7 Flash".
    #[test]
    fn a_gateway_id_loses_the_vendor_the_way_claude_ids_do() {
        assert_eq!(model_label("google/gemini-3.7-flash"), "Gemini 3.7 Flash");
        assert_eq!(model_label("z-ai/glm-5v-turbo"), "Glm 5v Turbo");
        // The vendor segment goes before the `claude-` prefix does, so a
        // Claude model reached through a gateway reads like a local one.
        assert_eq!(model_label("anthropic/claude-sonnet-5"), "Sonnet 5");
        // Nothing after the slash to use means the slash named no model, so
        // the whole id is kept rather than trimmed away to nothing.
        assert_eq!(model_label("google/"), "Google/");
    }

    #[test]
    fn it_handles_the_other_agent_without_being_taught_each_one() {
        assert_eq!(model_label("gpt-5.6-sol"), "GPT-5.6 Sol");
        assert_eq!(model_label("gpt-5"), "GPT-5");
        assert_eq!(model_label("o3"), "O3");
    }

    /// The case that matters most is the model this has never seen, because
    /// that is the new one — it should come out tidy without a table entry.
    #[test]
    fn an_unknown_id_still_comes_out_readable() {
        assert_eq!(model_label("claude-quartz-7"), "Quartz 7");
        assert_eq!(model_label("some-future-model-2"), "Some Future Model 2");
        // And nothing it cannot make sense of is lost.
        assert_eq!(model_label("mystery"), "Mystery");
        assert_eq!(model_label(""), "");
    }
}
