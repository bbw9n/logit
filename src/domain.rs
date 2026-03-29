use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SyncState {
    Synced,
    PendingCreate,
    PendingUpdate,
    SyncError,
    Conflict,
}

impl SyncState {
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Synced => "synced",
            Self::PendingCreate => "pending-create",
            Self::PendingUpdate => "pending-update",
            Self::SyncError => "sync-error",
            Self::Conflict => "conflict",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum IssueStatus {
    Todo,
    ReadyForAgent,
    #[serde(alias = "InProgress")]
    AgentRunning,
    NeedsHumanInput,
    NeedsReview,
    Blocked,
    Done,
}

impl IssueStatus {
    pub fn cycle(&self) -> Self {
        match self {
            Self::Todo => Self::ReadyForAgent,
            Self::ReadyForAgent => Self::AgentRunning,
            Self::AgentRunning => Self::NeedsHumanInput,
            Self::NeedsHumanInput => Self::NeedsReview,
            Self::NeedsReview => Self::Blocked,
            Self::Blocked => Self::Done,
            Self::Done => Self::Todo,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::ReadyForAgent => "ready for agent",
            Self::AgentRunning => "agent running",
            Self::NeedsHumanInput => "needs human input",
            Self::NeedsReview => "needs review",
            Self::Blocked => "blocked",
            Self::Done => "done",
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::ReadyForAgent => "ready_for_agent",
            Self::AgentRunning => "agent_running",
            Self::NeedsHumanInput => "needs_human_input",
            Self::NeedsReview => "needs_review",
            Self::Blocked => "blocked",
            Self::Done => "done",
        }
    }

    pub fn is_inbox_relevant(&self) -> bool {
        !matches!(self, Self::Done)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Priority {
    None,
    Low,
    Medium,
    High,
    Urgent,
}

impl Priority {
    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Urgent => "urgent",
        }
    }

    pub fn cycle(&self) -> Self {
        match self {
            Self::None => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::Urgent,
            Self::Urgent => Self::None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum OwnerType {
    Human,
    Agent,
    Unassigned,
}

impl OwnerType {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
            Self::Unassigned => "unassigned",
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
            Self::Unassigned => "unassigned",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Issue {
    pub local_id: i64,
    pub remote_id: Option<String>,
    pub identifier: String,
    pub title: String,
    pub description: String,
    pub project: Option<String>,
    pub labels: Vec<String>,
    pub status: IssueStatus,
    pub priority: Priority,
    pub assignee: Option<String>,
    pub owner_type: OwnerType,
    pub owner_name: Option<String>,
    pub attention_reason: Option<String>,
    pub blocked_reason: Option<String>,
    pub is_archived: bool,
    pub sync_state: SyncState,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueDraft {
    pub title: String,
    pub description: String,
    pub project: Option<String>,
    pub labels: Vec<String>,
    pub status: IssueStatus,
    pub priority: Priority,
    pub assignee: Option<String>,
    pub owner_type: OwnerType,
    pub owner_name: Option<String>,
    pub attention_reason: Option<String>,
    pub blocked_reason: Option<String>,
}

impl IssueDraft {
    pub fn new(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: description.into(),
            project: None,
            labels: Vec::new(),
            status: IssueStatus::Todo,
            priority: Priority::Medium,
            assignee: None,
            owner_type: OwnerType::Unassigned,
            owner_name: None,
            attention_reason: Some("new local task".into()),
            blocked_reason: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssuePatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub project: Option<Option<String>>,
    pub labels: Option<Vec<String>>,
    pub status: Option<IssueStatus>,
    pub priority: Option<Priority>,
    pub assignee: Option<Option<String>>,
    pub owner_type: Option<OwnerType>,
    pub owner_name: Option<Option<String>>,
    pub attention_reason: Option<Option<String>>,
    pub blocked_reason: Option<Option<String>>,
    pub is_archived: Option<bool>,
}

impl IssuePatch {
    pub fn empty() -> Self {
        Self {
            title: None,
            description: None,
            project: None,
            labels: None,
            status: None,
            priority: None,
            assignee: None,
            owner_type: None,
            owner_name: None,
            attention_reason: None,
            blocked_reason: None,
            is_archived: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct IssueQuery {
    pub unsynced_only: bool,
    pub include_archived: bool,
    pub archived_only: bool,
    pub status: Option<IssueStatus>,
    pub search: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScratchSource {
    Manual,
    Agent,
    RunFailure,
    Pasted,
}

impl ScratchSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Agent => "agent",
            Self::RunFailure => "run failure",
            Self::Pasted => "pasted",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScratchItem {
    pub id: i64,
    pub body: String,
    pub source: ScratchSource,
    pub created_at: DateTime<Utc>,
    pub promoted_issue_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QueuedMutationKind {
    CreateIssue {
        issue_local_id: i64,
    },
    UpdateIssue {
        issue_local_id: i64,
        patch: IssuePatch,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedMutation {
    pub id: i64,
    pub kind: QueuedMutationKind,
    pub created_at: DateTime<Utc>,
    pub attempt_count: i64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SyncReport {
    pub pushed: usize,
    pub failed: usize,
    pub message: String,
}
