use serde::{Deserialize, Serialize};

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
