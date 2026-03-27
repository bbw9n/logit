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
    InProgress,
    Done,
}

impl IssueStatus {
    pub fn cycle(&self) -> Self {
        match self {
            Self::Todo => Self::InProgress,
            Self::InProgress => Self::Done,
            Self::Done => Self::Todo,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::InProgress => "in progress",
            Self::Done => "done",
        }
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Issue {
    pub local_id: i64,
    pub remote_id: Option<String>,
    pub identifier: String,
    pub title: String,
    pub description: String,
    pub status: IssueStatus,
    pub priority: Priority,
    pub assignee: Option<String>,
    pub sync_state: SyncState,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssuePatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<IssueStatus>,
    pub priority: Option<Priority>,
    pub assignee: Option<Option<String>>,
}

impl IssuePatch {
    pub fn empty() -> Self {
        Self {
            title: None,
            description: None,
            status: None,
            priority: None,
            assignee: None,
        }
    }
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
