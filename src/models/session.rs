use super::agent::AgentType;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    Running,
    Stopped,
    Errored,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub agent_type: AgentType,
    #[serde(default = "default_dangerously_skip_permissions")]
    pub dangerously_skip_permissions: bool,
    pub status: SessionStatus,
    pub started_at: DateTime<Utc>,
    pub stopped_at: Option<DateTime<Utc>>,
    /// Optional startup command for terminal sessions
    #[serde(default)]
    pub start_command: Option<String>,
}

impl Session {
    pub fn new(
        workspace_id: Uuid,
        agent_type: AgentType,
        dangerously_skip_permissions: bool,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            agent_type,
            dangerously_skip_permissions,
            status: SessionStatus::Running,
            started_at: Utc::now(),
            stopped_at: None,
            start_command: None,
        }
    }

    pub fn display_name(&self) -> String {
        format!(
            "{} ({})",
            self.agent_type.display_name(),
            &self.id.to_string()[..8]
        )
    }

    pub fn short_id(&self) -> String {
        self.id.to_string()[..8].to_string()
    }

    pub fn duration(&self) -> chrono::Duration {
        let end = self.stopped_at.unwrap_or_else(Utc::now);
        end - self.started_at
    }

    pub fn duration_string(&self) -> String {
        let duration = self.duration();
        let hours = duration.num_hours();
        let minutes = duration.num_minutes() % 60;
        let seconds = duration.num_seconds() % 60;

        if hours > 0 {
            format!("{}h {}m", hours, minutes)
        } else if minutes > 0 {
            format!("{}m {}s", minutes, seconds)
        } else {
            format!("{}s", seconds)
        }
    }

    pub fn status_icon(&self) -> &'static str {
        match self.status {
            SessionStatus::Running => "●",
            SessionStatus::Stopped => "○",
            SessionStatus::Errored => "✗",
        }
    }

    pub fn mark_stopped(&mut self) {
        self.status = SessionStatus::Stopped;
        self.stopped_at = Some(Utc::now());
    }
}

fn default_dangerously_skip_permissions() -> bool {
    true
}
