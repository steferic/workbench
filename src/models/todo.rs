use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Difficulty {
    Easy,
    #[default]
    Med,
    Hard,
}

impl Difficulty {
    /// Parse from string like "EASY", "MED", "HARD"
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "EASY" => Some(Difficulty::Easy),
            "MED" | "MEDIUM" => Some(Difficulty::Med),
            "HARD" => Some(Difficulty::Hard),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Importance {
    Low,
    #[default]
    Med,
    High,
    Critical,
}

impl Importance {
    /// Parse from string like "LOW", "MED", "HIGH", "CRITICAL"
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "LOW" => Some(Importance::Low),
            "MED" | "MEDIUM" => Some(Importance::Med),
            "HIGH" => Some(Importance::High),
            "CRITICAL" | "CRIT" => Some(Importance::Critical),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum TodoStatus {
    /// Suggested by analyzer, needs approval
    Suggested,
    /// Waiting to be picked up by an agent
    #[default]
    Pending,
    /// Queued to run after current todo finishes
    Queued,
    /// Currently being worked on by a session
    InProgress { session_id: Uuid },
    /// Agent finished, awaiting user review
    ReadyForReview { session_id: Uuid },
    /// User marked as complete
    Done,
    /// Archived (hidden from main list but preserved)
    Archived,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Todo {
    pub id: Uuid,
    pub description: String,
    pub status: TodoStatus,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub difficulty: Option<Difficulty>,
    #[serde(default)]
    pub importance: Option<Importance>,
}

impl Todo {
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            description: description.into(),
            status: TodoStatus::Pending,
            created_at: Utc::now(),
            difficulty: None,
            importance: None,
        }
    }

    /// Create a suggested todo (from analyzer), parsing [DIFFICULTY] [IMPORTANCE] from description
    pub fn suggested(description: impl Into<String>) -> Self {
        let desc_str = description.into();
        let (difficulty, importance, clean_desc) = Self::parse_tags(&desc_str);

        Self {
            id: Uuid::new_v4(),
            description: clean_desc,
            status: TodoStatus::Suggested,
            created_at: Utc::now(),
            difficulty,
            importance,
        }
    }

    /// Parse [TAG] patterns from description, returning (difficulty, importance, cleaned_description)
    fn parse_tags(desc: &str) -> (Option<Difficulty>, Option<Importance>, String) {
        let mut difficulty = None;
        let mut importance = None;
        let mut clean = desc.to_string();

        // Find all [TAG] patterns
        let re_pattern = regex::Regex::new(r"\[([A-Z]+)\]").ok();

        if let Some(re) = re_pattern {
            for cap in re.captures_iter(desc) {
                if let Some(tag) = cap.get(1) {
                    let tag_str = tag.as_str();
                    // Try to parse as difficulty first, then importance
                    if difficulty.is_none() {
                        if let Some(d) = Difficulty::from_str(tag_str) {
                            difficulty = Some(d);
                            clean = clean.replace(&cap[0], "");
                            continue;
                        }
                    }
                    if importance.is_none() {
                        if let Some(i) = Importance::from_str(tag_str) {
                            importance = Some(i);
                            clean = clean.replace(&cap[0], "");
                        }
                    }
                }
            }
        }

        // Clean up extra whitespace
        let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");

        (difficulty, importance, clean)
    }

    pub fn is_suggested(&self) -> bool {
        matches!(self.status, TodoStatus::Suggested)
    }

    pub fn is_pending(&self) -> bool {
        matches!(self.status, TodoStatus::Pending)
    }

    pub fn is_queued(&self) -> bool {
        matches!(self.status, TodoStatus::Queued)
    }

    pub fn is_in_progress(&self) -> bool {
        matches!(self.status, TodoStatus::InProgress { .. })
    }

    pub fn is_ready_for_review(&self) -> bool {
        matches!(self.status, TodoStatus::ReadyForReview { .. })
    }

    pub fn is_done(&self) -> bool {
        matches!(self.status, TodoStatus::Done)
    }

    pub fn is_archived(&self) -> bool {
        matches!(self.status, TodoStatus::Archived)
    }

    /// Mark as queued (will run after current todo finishes)
    pub fn mark_queued(&mut self) {
        if self.is_pending() {
            self.status = TodoStatus::Queued;
        }
    }

    /// Assign to a session and mark as in progress
    pub fn assign_to(&mut self, session_id: Uuid) {
        self.status = TodoStatus::InProgress { session_id };
    }

    /// Mark as ready for review (agent finished)
    pub fn mark_ready_for_review(&mut self) {
        if let TodoStatus::InProgress { session_id } = self.status {
            self.status = TodoStatus::ReadyForReview { session_id };
        }
    }

    /// Mark as done
    pub fn mark_done(&mut self) {
        self.status = TodoStatus::Done;
    }

    /// Archive a todo (typically from review or done state)
    pub fn archive(&mut self) {
        self.status = TodoStatus::Archived;
    }

    /// Approve a suggested todo (converts to Pending)
    pub fn approve(&mut self) {
        if self.is_suggested() {
            self.status = TodoStatus::Pending;
        }
    }

    /// Get the session ID if currently assigned
    pub fn assigned_session_id(&self) -> Option<Uuid> {
        match &self.status {
            TodoStatus::InProgress { session_id } => Some(*session_id),
            TodoStatus::ReadyForReview { session_id } => Some(*session_id),
            _ => None,
        }
    }
}
