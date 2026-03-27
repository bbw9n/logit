use crate::domain::{
    Issue, IssuePatch, IssueStatus, Priority, QueuedMutation, QueuedMutationKind, SyncState,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use std::{io, path::Path};

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("opening sqlite database at {}", path.display()))?;
        let store = Self { conn };
        store.migrate()?;
        store.seed()?;
        Ok(store)
    }

    pub fn list_issues(&self, unsynced_only: bool) -> Result<Vec<Issue>> {
        let mut sql = String::from(
            "SELECT local_id, remote_id, identifier, title, description, status, priority, assignee, sync_state, updated_at
             FROM issues",
        );
        if unsynced_only {
            sql.push_str(" WHERE sync_state != 'synced'");
        }
        sql.push_str(" ORDER BY updated_at DESC, local_id DESC");

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], map_issue_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn get_issue(&self, local_id: i64) -> Result<Option<Issue>> {
        self.conn
            .query_row(
                "SELECT local_id, remote_id, identifier, title, description, status, priority, assignee, sync_state, updated_at
                 FROM issues WHERE local_id = ?1",
                [local_id],
                map_issue_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn create_issue(&self, title: &str, description: &str) -> Result<Issue> {
        let now = Utc::now();
        self.conn.execute(
            "INSERT INTO issues (remote_id, identifier, title, description, status, priority, assignee, sync_state, updated_at)
             VALUES (NULL, '', ?1, ?2, 'todo', 'medium', NULL, 'pending_create', ?3)",
            params![title, description, now.to_rfc3339()],
        )?;
        let local_id = self.conn.last_insert_rowid();
        let identifier = format!("LOCAL-{local_id}");
        self.conn.execute(
            "UPDATE issues SET identifier = ?1 WHERE local_id = ?2",
            params![identifier, local_id],
        )?;
        self.enqueue(QueuedMutationKind::CreateIssue {
            issue_local_id: local_id,
        })?;

        self.get_issue(local_id)?
            .context("created issue missing from database")
    }

    pub fn update_issue(&self, local_id: i64, patch: &IssuePatch) -> Result<Issue> {
        let current = self
            .get_issue(local_id)?
            .with_context(|| format!("issue {local_id} not found"))?;
        let next_sync_state = if current.remote_id.is_some() {
            SyncState::PendingUpdate
        } else {
            SyncState::PendingCreate
        };

        self.conn.execute(
            "UPDATE issues
             SET title = ?1,
                 description = ?2,
                 status = ?3,
                 priority = ?4,
                 assignee = ?5,
                 sync_state = ?6,
                 updated_at = ?7
             WHERE local_id = ?8",
            params![
                patch.title.as_deref().unwrap_or(&current.title),
                patch.description.as_deref().unwrap_or(&current.description),
                encode_status(patch.status.as_ref().unwrap_or(&current.status)),
                encode_priority(patch.priority.as_ref().unwrap_or(&current.priority)),
                patch.assignee.clone().unwrap_or(current.assignee.clone()),
                encode_sync_state(&next_sync_state),
                Utc::now().to_rfc3339(),
                local_id,
            ],
        )?;

        if current.remote_id.is_some() {
            self.enqueue(QueuedMutationKind::UpdateIssue {
                issue_local_id: local_id,
                patch: patch.clone(),
            })?;
        }

        self.get_issue(local_id)?
            .with_context(|| format!("updated issue {local_id} missing from database"))
    }

    pub fn list_pending_mutations(&self) -> Result<Vec<QueuedMutation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind_json, created_at, attempt_count, last_error
             FROM queued_mutations
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let kind_json: String = row.get(1)?;
            Ok(QueuedMutation {
                id: row.get(0)?,
                kind: serde_json::from_str(&kind_json).map_err(to_sql_conversion_error)?,
                created_at: parse_dt(row.get::<_, String>(2)?).map_err(to_sql_conversion_error)?,
                attempt_count: row.get(3)?,
                last_error: row.get(4)?,
            })
        })?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn mark_mutation_attempt(&self, id: i64, error: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE queued_mutations
             SET attempt_count = attempt_count + 1, last_error = ?2
             WHERE id = ?1",
            params![id, error],
        )?;
        Ok(())
    }

    pub fn delete_mutation(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM queued_mutations WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn mark_issue_synced(
        &self,
        local_id: i64,
        remote_id: Option<&str>,
        identifier: Option<&str>,
    ) -> Result<()> {
        let existing = self
            .get_issue(local_id)?
            .with_context(|| format!("issue {local_id} not found"))?;

        self.conn.execute(
            "UPDATE issues
             SET remote_id = ?1,
                 identifier = ?2,
                 sync_state = 'synced',
                 updated_at = ?3
             WHERE local_id = ?4",
            params![
                remote_id.or(existing.remote_id.as_deref()),
                identifier.unwrap_or(&existing.identifier),
                Utc::now().to_rfc3339(),
                local_id
            ],
        )?;
        Ok(())
    }

    pub fn mark_issue_sync_error(&self, local_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE issues SET sync_state = 'sync_error', updated_at = ?2 WHERE local_id = ?1",
            params![local_id, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn retry_failed_mutations(&self) -> Result<usize> {
        let count = self.conn.execute(
            "UPDATE issues
             SET sync_state = CASE
                 WHEN remote_id IS NULL THEN 'pending_create'
                 ELSE 'pending_update'
             END
             WHERE sync_state = 'sync_error'",
            [],
        )?;
        Ok(count)
    }

    fn enqueue(&self, mutation: QueuedMutationKind) -> Result<()> {
        self.conn.execute(
            "INSERT INTO queued_mutations (kind_json, created_at, attempt_count, last_error)
             VALUES (?1, ?2, 0, NULL)",
            params![serde_json::to_string(&mutation)?, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS issues (
                local_id INTEGER PRIMARY KEY AUTOINCREMENT,
                remote_id TEXT,
                identifier TEXT NOT NULL,
                title TEXT NOT NULL,
                description TEXT NOT NULL,
                status TEXT NOT NULL,
                priority TEXT NOT NULL,
                assignee TEXT,
                sync_state TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS queued_mutations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                attempt_count INTEGER NOT NULL DEFAULT 0,
                last_error TEXT
            );",
        )?;
        Ok(())
    }

    fn seed(&self) -> Result<()> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM issues", [], |row| row.get(0))?;
        if count > 0 {
            return Ok(());
        }

        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO issues (remote_id, identifier, title, description, status, priority, assignee, sync_state, updated_at)
             VALUES
             ('lin_1001', 'ENG-12', 'Make offline triage feel instant', 'Seed issue that demonstrates synced state and richer detail copy.', 'in_progress', 'high', 'you', 'synced', ?1),
             (NULL, 'LOCAL-2', 'Draft local issue without network', 'This one starts as a queued local-only record to show the offline model.', 'todo', 'medium', NULL, 'pending_create', ?1)",
            [now],
        )?;
        self.enqueue(QueuedMutationKind::CreateIssue { issue_local_id: 2 })?;
        Ok(())
    }
}

fn map_issue_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Issue> {
    Ok(Issue {
        local_id: row.get(0)?,
        remote_id: row.get(1)?,
        identifier: row.get(2)?,
        title: row.get(3)?,
        description: row.get(4)?,
        status: decode_status(&row.get::<_, String>(5)?)?,
        priority: decode_priority(&row.get::<_, String>(6)?)?,
        assignee: row.get(7)?,
        sync_state: decode_sync_state(&row.get::<_, String>(8)?)?,
        updated_at: parse_dt(row.get::<_, String>(9)?).map_err(to_sql_conversion_error)?,
    })
}

fn parse_dt(value: String) -> std::result::Result<DateTime<Utc>, chrono::ParseError> {
    DateTime::parse_from_rfc3339(&value).map(|dt| dt.with_timezone(&Utc))
}

fn decode_status(value: &str) -> rusqlite::Result<IssueStatus> {
    match value {
        "todo" => Ok(IssueStatus::Todo),
        "in_progress" => Ok(IssueStatus::InProgress),
        "done" => Ok(IssueStatus::Done),
        other => Err(to_sql_conversion_error(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown issue status {other}"),
        ))),
    }
}

fn decode_priority(value: &str) -> rusqlite::Result<Priority> {
    match value {
        "none" => Ok(Priority::None),
        "low" => Ok(Priority::Low),
        "medium" => Ok(Priority::Medium),
        "high" => Ok(Priority::High),
        "urgent" => Ok(Priority::Urgent),
        other => Err(to_sql_conversion_error(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown priority {other}"),
        ))),
    }
}

fn decode_sync_state(value: &str) -> rusqlite::Result<SyncState> {
    match value {
        "synced" => Ok(SyncState::Synced),
        "pending_create" => Ok(SyncState::PendingCreate),
        "pending_update" => Ok(SyncState::PendingUpdate),
        "sync_error" => Ok(SyncState::SyncError),
        "conflict" => Ok(SyncState::Conflict),
        other => Err(to_sql_conversion_error(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown sync state {other}"),
        ))),
    }
}

fn encode_status(status: &IssueStatus) -> &'static str {
    match status {
        IssueStatus::Todo => "todo",
        IssueStatus::InProgress => "in_progress",
        IssueStatus::Done => "done",
    }
}

fn encode_priority(priority: &Priority) -> &'static str {
    match priority {
        Priority::None => "none",
        Priority::Low => "low",
        Priority::Medium => "medium",
        Priority::High => "high",
        Priority::Urgent => "urgent",
    }
}

fn encode_sync_state(sync_state: &SyncState) -> &'static str {
    match sync_state {
        SyncState::Synced => "synced",
        SyncState::PendingCreate => "pending_create",
        SyncState::PendingUpdate => "pending_update",
        SyncState::SyncError => "sync_error",
        SyncState::Conflict => "conflict",
    }
}

fn to_sql_conversion_error(
    error: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}
