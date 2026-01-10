use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentType {
    Claude,
    Gemini,
    Codex,
    Grok,
    Terminal(String), // Named terminal with custom name
}

impl AgentType {
    pub fn command(&self) -> &str {
        match self {
            AgentType::Claude => "claude",
            AgentType::Gemini => "gemini",
            AgentType::Codex => "codex",
            AgentType::Grok => "grok",
            AgentType::Terminal(_) => {
                // Use $SHELL or default to bash
                std::env::var("SHELL").ok()
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
            AgentType::Terminal(name) => name.clone(),
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            AgentType::Claude => "⚡",
            AgentType::Gemini => "⚡",
            AgentType::Codex => "⚡",
            AgentType::Grok => "⚡",
            AgentType::Terminal(_) => "🖥",
        }
    }

    pub fn badge(&self) -> &'static str {
        match self {
            AgentType::Claude => "C",
            AgentType::Gemini => "G",
            AgentType::Codex => "X",
            AgentType::Grok => "K",
            AgentType::Terminal(_) => "T",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, AgentType::Terminal(_))
    }

    pub fn is_agent(&self) -> bool {
        !self.is_terminal()
    }
}

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}
