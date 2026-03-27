use crate::domain::{
    Issue, IssueDraft, IssuePatch, IssueQuery, IssueStatus, Priority, QueuedMutation,
    QueuedMutationKind, SyncState,
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
        Self::from_connection(conn, true)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?, false)
    }

    pub fn list_issues(&self, query: &IssueQuery) -> Result<Vec<Issue>> {
        let mut sql = String::from(
            "SELECT local_id, remote_id, identifier, title, description, project, labels_json, status, priority, assignee, is_archived, sync_state, updated_at
             FROM issues WHERE 1 = 1",
        );
        let mut params = Vec::new();

        if !query.include_archived {
            sql.push_str(" AND is_archived = 0");
        }
        if query.archived_only {
            sql.push_str(" AND is_archived = 1");
        }
        if query.unsynced_only {
            sql.push_str(" AND sync_state != 'synced'");
        }
        if let Some(status) = &query.status {
            sql.push_str(" AND status = ?");
            params.push(encode_status(status).to_string());
        }
        if let Some(search) = query
            .search
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            sql.push_str(" AND (identifier LIKE ? OR title LIKE ? OR description LIKE ? OR COALESCE(project, '') LIKE ? OR labels_json LIKE ?)");
            let pattern = format!("%{}%", search.trim());
            params.push(pattern.clone());
            params.push(pattern.clone());
            params.push(pattern.clone());
            params.push(pattern.clone());
            params.push(pattern);
        }
        sql.push_str(" ORDER BY updated_at DESC, local_id DESC");

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), map_issue_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn get_issue(&self, local_id: i64) -> Result<Option<Issue>> {
        self.conn
            .query_row(
                "SELECT local_id, remote_id, identifier, title, description, project, labels_json, status, priority, assignee, is_archived, sync_state, updated_at
                 FROM issues WHERE local_id = ?1",
                [local_id],
                map_issue_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn create_issue(&self, draft: &IssueDraft) -> Result<Issue> {
        let now = Utc::now();
        self.conn.execute(
            "INSERT INTO issues (remote_id, identifier, title, description, project, labels_json, status, priority, assignee, is_archived, sync_state, updated_at)
             VALUES (NULL, '', ?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 'pending_create', ?8)",
            params![
                draft.title,
                draft.description,
                draft.project,
                encode_labels(&draft.labels)?,
                encode_status(&draft.status),
                encode_priority(&draft.priority),
                draft.assignee,
                now.to_rfc3339()
            ],
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
                 project = ?3,
                 labels_json = ?4,
                 status = ?5,
                 priority = ?6,
                 assignee = ?7,
                 is_archived = ?8,
                 sync_state = ?9,
                 updated_at = ?10
             WHERE local_id = ?11",
            params![
                patch.title.as_deref().unwrap_or(&current.title),
                patch.description.as_deref().unwrap_or(&current.description),
                patch.project.clone().unwrap_or(current.project.clone()),
                encode_labels(patch.labels.as_ref().unwrap_or(&current.labels),)?,
                encode_status(patch.status.as_ref().unwrap_or(&current.status)),
                encode_priority(patch.priority.as_ref().unwrap_or(&current.priority)),
                patch.assignee.clone().unwrap_or(current.assignee.clone()),
                patch.is_archived.unwrap_or(current.is_archived),
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

    #[cfg(test)]
    pub fn delete_issue(&self, local_id: i64) -> Result<bool> {
        for mutation in self.list_pending_mutations()? {
            let matches_issue = match mutation.kind {
                QueuedMutationKind::CreateIssue { issue_local_id } => issue_local_id == local_id,
                QueuedMutationKind::UpdateIssue { issue_local_id, .. } => {
                    issue_local_id == local_id
                }
            };

            if matches_issue {
                self.delete_mutation(mutation.id)?;
            }
        }

        let deleted = self
            .conn
            .execute("DELETE FROM issues WHERE local_id = ?1", [local_id])?;
        Ok(deleted > 0)
    }

    pub fn archive_issue(&self, local_id: i64, archived: bool) -> Result<Issue> {
        let mut patch = IssuePatch::empty();
        patch.is_archived = Some(archived);
        self.update_issue(local_id, &patch)
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

    fn from_connection(conn: Connection, seed: bool) -> Result<Self> {
        let store = Self { conn };
        store.migrate()?;
        if seed {
            store.seed()?;
        }
        Ok(store)
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
                project TEXT,
                labels_json TEXT NOT NULL DEFAULT '[]',
                status TEXT NOT NULL,
                priority TEXT NOT NULL,
                assignee TEXT,
                is_archived INTEGER NOT NULL DEFAULT 0,
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
        let columns = self
            .conn
            .prepare("PRAGMA table_info(issues)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .collect::<Vec<_>>();
        if !columns.iter().any(|column| column == "is_archived") {
            self.conn.execute(
                "ALTER TABLE issues ADD COLUMN is_archived INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !columns.iter().any(|column| column == "project") {
            self.conn
                .execute("ALTER TABLE issues ADD COLUMN project TEXT", [])?;
        }
        if !columns.iter().any(|column| column == "labels_json") {
            self.conn.execute(
                "ALTER TABLE issues ADD COLUMN labels_json TEXT NOT NULL DEFAULT '[]'",
                [],
            )?;
        }
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
            "INSERT INTO issues (remote_id, identifier, title, description, project, labels_json, status, priority, assignee, is_archived, sync_state, updated_at)
             VALUES
             ('lin_1001', 'ENG-12', 'Make offline triage feel instant', 'Seed issue that demonstrates synced state and richer detail copy.', 'Core', '[\"offline\",\"ux\"]', 'in_progress', 'high', 'you', 0, 'synced', ?1),
             (NULL, 'LOCAL-2', 'Draft local issue without network', 'This one starts as a queued local-only record to show the offline model.', 'Personal', '[\"draft\"]', 'todo', 'medium', NULL, 0, 'pending_create', ?1)",
            [now],
        )?;
        self.enqueue(QueuedMutationKind::CreateIssue { issue_local_id: 2 })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_crud_round_trip_works() -> Result<()> {
        let store = Store::open_in_memory()?;
        let draft = IssueDraft::new("Write the CRUD layer", "Back the TUI with SQLite.");

        let created = store.create_issue(&draft)?;
        assert_eq!(created.identifier, "LOCAL-1");
        assert_eq!(created.sync_state, SyncState::PendingCreate);

        let fetched = store
            .get_issue(created.local_id)?
            .expect("issue should exist");
        assert_eq!(fetched.title, draft.title);

        let mut patch = IssuePatch::empty();
        patch.title = Some("Write and test the CRUD layer".into());
        patch.status = Some(IssueStatus::InProgress);
        let updated = store.update_issue(created.local_id, &patch)?;
        assert_eq!(updated.title, "Write and test the CRUD layer");
        assert_eq!(updated.status, IssueStatus::InProgress);

        assert!(store.delete_issue(created.local_id)?);
        assert!(store.get_issue(created.local_id)?.is_none());
        assert!(store.list_pending_mutations()?.is_empty());
        Ok(())
    }

    #[test]
    fn issue_query_filters_unsynced_status_and_search() -> Result<()> {
        let store = Store::open_in_memory()?;
        let local = store.create_issue(&IssueDraft::new(
            "Offline draft",
            "Only local until Linear sync is enabled.",
        ))?;
        let synced = store.create_issue(&IssueDraft::new(
            "Remote-backed issue",
            "Already synced to Linear.",
        ))?;
        store.mark_issue_synced(synced.local_id, Some("lin_1"), Some("ENG-1"))?;

        let unsynced = store.list_issues(&IssueQuery {
            unsynced_only: true,
            ..IssueQuery::default()
        })?;
        assert_eq!(unsynced.len(), 1);
        assert_eq!(unsynced[0].local_id, local.local_id);

        let in_progress = {
            let mut patch = IssuePatch::empty();
            patch.status = Some(IssueStatus::InProgress);
            store.update_issue(local.local_id, &patch)?
        };
        let filtered = store.list_issues(&IssueQuery {
            status: Some(IssueStatus::InProgress),
            search: Some("offline".into()),
            ..IssueQuery::default()
        })?;
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].local_id, in_progress.local_id);
        Ok(())
    }

    #[test]
    fn deleting_issue_cleans_up_queued_mutations() -> Result<()> {
        let store = Store::open_in_memory()?;
        let issue = store.create_issue(&IssueDraft::new("Delete me", "Queued for creation"))?;
        assert_eq!(store.list_pending_mutations()?.len(), 1);

        assert!(store.delete_issue(issue.local_id)?);
        assert!(store.list_pending_mutations()?.is_empty());
        Ok(())
    }

    #[test]
    fn updating_synced_issue_creates_update_mutation() -> Result<()> {
        let store = Store::open_in_memory()?;
        let issue = store.create_issue(&IssueDraft::new("Synced issue", "Already in Linear"))?;
        store.mark_issue_synced(issue.local_id, Some("lin_42"), Some("ENG-42"))?;
        for mutation in store.list_pending_mutations()? {
            store.delete_mutation(mutation.id)?;
        }
        assert!(store.list_pending_mutations()?.is_empty());

        let mut patch = IssuePatch::empty();
        patch.priority = Some(Priority::High);
        let updated = store.update_issue(issue.local_id, &patch)?;

        assert_eq!(updated.sync_state, SyncState::PendingUpdate);
        let pending = store.list_pending_mutations()?;
        assert_eq!(pending.len(), 1);
        match &pending[0].kind {
            QueuedMutationKind::UpdateIssue {
                issue_local_id,
                patch,
            } => {
                assert_eq!(*issue_local_id, issue.local_id);
                assert_eq!(patch.priority, Some(Priority::High));
            }
            other => panic!("expected update mutation, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn retry_failed_sync_restores_expected_pending_state() -> Result<()> {
        let store = Store::open_in_memory()?;
        let local = store.create_issue(&IssueDraft::new("Local", "Still local"))?;
        let synced = store.create_issue(&IssueDraft::new("Remote", "Was synced before"))?;
        store.mark_issue_synced(synced.local_id, Some("lin_77"), Some("ENG-77"))?;

        store.mark_issue_sync_error(local.local_id)?;
        store.mark_issue_sync_error(synced.local_id)?;
        let retried = store.retry_failed_mutations()?;
        assert_eq!(retried, 2);

        let local_issue = store
            .get_issue(local.local_id)?
            .expect("local issue missing");
        let synced_issue = store
            .get_issue(synced.local_id)?
            .expect("synced issue missing");

        assert_eq!(local_issue.sync_state, SyncState::PendingCreate);
        assert_eq!(synced_issue.sync_state, SyncState::PendingUpdate);
        Ok(())
    }

    #[test]
    fn archived_issues_are_hidden_until_requested() -> Result<()> {
        let store = Store::open_in_memory()?;
        let issue = store.create_issue(&IssueDraft::new("Archive me", "Local archive flow"))?;
        store.archive_issue(issue.local_id, true)?;

        let default_view = store.list_issues(&IssueQuery::default())?;
        assert!(default_view.is_empty());

        let all_view = store.list_issues(&IssueQuery {
            include_archived: true,
            ..IssueQuery::default()
        })?;
        assert_eq!(all_view.len(), 1);
        assert!(all_view[0].is_archived);
        Ok(())
    }

    #[test]
    fn archived_only_query_returns_only_archived_items() -> Result<()> {
        let store = Store::open_in_memory()?;
        let active = store.create_issue(&IssueDraft::new("Active", "Visible by default"))?;
        let archived = store.create_issue(&IssueDraft::new("Archived", "Hidden by default"))?;
        store.archive_issue(archived.local_id, true)?;

        let archived_only = store.list_issues(&IssueQuery {
            include_archived: true,
            archived_only: true,
            ..IssueQuery::default()
        })?;
        assert_eq!(archived_only.len(), 1);
        assert_eq!(archived_only[0].local_id, archived.local_id);
        assert_ne!(archived_only[0].local_id, active.local_id);
        Ok(())
    }

    #[test]
    fn project_and_labels_round_trip_and_search() -> Result<()> {
        let store = Store::open_in_memory()?;
        let mut draft = IssueDraft::new("Plan local project", "Track work offline");
        draft.project = Some("Ops".into());
        draft.labels = vec!["cli".into(), "planning".into()];

        let created = store.create_issue(&draft)?;
        assert_eq!(created.project.as_deref(), Some("Ops"));
        assert_eq!(
            created.labels,
            vec!["cli".to_string(), "planning".to_string()]
        );

        let by_project = store.list_issues(&IssueQuery {
            search: Some("Ops".into()),
            ..IssueQuery::default()
        })?;
        assert_eq!(by_project.len(), 1);
        assert_eq!(by_project[0].local_id, created.local_id);

        let by_label = store.list_issues(&IssueQuery {
            search: Some("planning".into()),
            ..IssueQuery::default()
        })?;
        assert_eq!(by_label.len(), 1);
        assert_eq!(by_label[0].local_id, created.local_id);
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
        project: row.get(5)?,
        labels: decode_labels(&row.get::<_, String>(6)?).map_err(to_sql_conversion_error)?,
        status: decode_status(&row.get::<_, String>(7)?)?,
        priority: decode_priority(&row.get::<_, String>(8)?)?,
        assignee: row.get(9)?,
        is_archived: row.get::<_, i64>(10)? != 0,
        sync_state: decode_sync_state(&row.get::<_, String>(11)?)?,
        updated_at: parse_dt(row.get::<_, String>(12)?).map_err(to_sql_conversion_error)?,
    })
}

fn encode_labels(labels: &[String]) -> Result<String> {
    Ok(serde_json::to_string(labels)?)
}

fn decode_labels(value: &str) -> std::result::Result<Vec<String>, serde_json::Error> {
    serde_json::from_str(value)
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
