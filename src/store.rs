use crate::domain::{
    ArtifactKind, ArtifactRecord, HandoffRecord, Issue, IssueDraft, IssuePatch, IssueQuery,
    IssueStatus, OwnerType, Priority, QueuedMutation, QueuedMutationKind, RunEventLevel,
    RunEventRecord, RunKind, RunRecord, RunStatus, ScratchItem, ScratchSource, SessionKind,
    SessionLink, SyncState, WorkContext,
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
            "SELECT local_id, remote_id, identifier, title, description, project, labels_json, status, priority, assignee, owner_type, owner_name, attention_reason, blocked_reason, closeout_summary, follow_up_needed, is_archived, sync_state, updated_at
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
            params.push(status.code().to_string());
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
                "SELECT local_id, remote_id, identifier, title, description, project, labels_json, status, priority, assignee, owner_type, owner_name, attention_reason, blocked_reason, closeout_summary, follow_up_needed, is_archived, sync_state, updated_at
                 FROM issues WHERE local_id = ?1",
                [local_id],
                map_issue_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_scratch_items(&self) -> Result<Vec<ScratchItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, body, source, created_at, promoted_issue_id
             FROM scratch_items
             ORDER BY created_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ScratchItem {
                id: row.get(0)?,
                body: row.get(1)?,
                source: decode_scratch_source(&row.get::<_, String>(2)?)?,
                created_at: parse_dt(row.get::<_, String>(3)?).map_err(to_sql_conversion_error)?,
                promoted_issue_id: row.get(4)?,
            })
        })?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn create_scratch_item(
        &self,
        body: impl Into<String>,
        source: ScratchSource,
    ) -> Result<ScratchItem> {
        let body = body.into();
        let created_at = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO scratch_items (body, source, created_at, promoted_issue_id)
             VALUES (?1, ?2, ?3, NULL)",
            params![body, encode_scratch_source(&source), created_at],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_scratch_item(id)?
            .context("created scratch item missing from database")
    }

    pub fn get_scratch_item(&self, id: i64) -> Result<Option<ScratchItem>> {
        self.conn
            .query_row(
                "SELECT id, body, source, created_at, promoted_issue_id
                 FROM scratch_items
                 WHERE id = ?1",
                [id],
                |row| {
                    Ok(ScratchItem {
                        id: row.get(0)?,
                        body: row.get(1)?,
                        source: decode_scratch_source(&row.get::<_, String>(2)?)?,
                        created_at: parse_dt(row.get::<_, String>(3)?)
                            .map_err(to_sql_conversion_error)?,
                        promoted_issue_id: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn promote_scratch_to_issue(&self, scratch_id: i64) -> Result<Issue> {
        let scratch = self
            .get_scratch_item(scratch_id)?
            .with_context(|| format!("scratch item {scratch_id} not found"))?;
        if let Some(issue_id) = scratch.promoted_issue_id {
            return self
                .get_issue(issue_id)?
                .with_context(|| format!("promoted issue {issue_id} not found"));
        }

        let title = scratch
            .body
            .lines()
            .find(|line| !line.trim().is_empty())
            .map(|line| line.trim().to_string())
            .unwrap_or_else(|| format!("Scratch item {}", scratch.id));
        let description = scratch.body.trim().to_string();

        let issue = self.create_issue(&IssueDraft::new(title, description))?;
        self.conn.execute(
            "UPDATE scratch_items SET promoted_issue_id = ?1 WHERE id = ?2",
            params![issue.local_id, scratch_id],
        )?;
        self.get_issue(issue.local_id)?
            .with_context(|| format!("promoted issue {} missing", issue.local_id))
    }

    pub fn list_runs_for_issue(&self, issue_local_id: i64) -> Result<Vec<RunRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, issue_local_id, kind, status, started_at, ended_at, summary, exit_code, session_ref
             FROM runs
             WHERE issue_local_id = ?1
             ORDER BY started_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([issue_local_id], map_run_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn latest_active_run_for_issue(&self, issue_local_id: i64) -> Result<Option<RunRecord>> {
        self.conn
            .query_row(
                "SELECT id, issue_local_id, kind, status, started_at, ended_at, summary, exit_code, session_ref
                 FROM runs
                 WHERE issue_local_id = ?1 AND status IN ('queued', 'running')
                 ORDER BY started_at DESC, id DESC
                 LIMIT 1",
                [issue_local_id],
                map_run_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_run_events_for_issue(&self, issue_local_id: i64) -> Result<Vec<RunEventRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT run_events.id, run_events.run_id, run_events.created_at, run_events.level, run_events.message
             FROM run_events
             INNER JOIN runs ON runs.id = run_events.run_id
             WHERE runs.issue_local_id = ?1
             ORDER BY run_events.created_at DESC, run_events.id DESC
             LIMIT 8",
        )?;
        let rows = stmt.query_map([issue_local_id], map_run_event_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn list_artifacts_for_issue(&self, issue_local_id: i64) -> Result<Vec<ArtifactRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, issue_local_id, run_id, kind, content_preview, location, created_at
             FROM artifacts
             WHERE issue_local_id = ?1
             ORDER BY created_at DESC, id DESC
             LIMIT 8",
        )?;
        let rows = stmt.query_map([issue_local_id], map_artifact_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn list_handoffs_for_issue(&self, issue_local_id: i64) -> Result<Vec<HandoffRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, issue_local_id, from_actor, to_actor, note, created_at
             FROM handoffs
             WHERE issue_local_id = ?1
             ORDER BY created_at DESC, id DESC
             LIMIT 8",
        )?;
        let rows = stmt.query_map([issue_local_id], map_handoff_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn create_handoff(
        &self,
        issue_local_id: i64,
        from_actor: &str,
        to_actor: &str,
        note: &str,
    ) -> Result<HandoffRecord> {
        self.conn.execute(
            "INSERT INTO handoffs (issue_local_id, from_actor, to_actor, note, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                issue_local_id,
                from_actor,
                to_actor,
                note,
                Utc::now().to_rfc3339()
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_handoff(id)?
            .context("created handoff missing from database")
    }

    pub fn get_active_work_context(&self, issue_local_id: i64) -> Result<Option<WorkContext>> {
        self.conn
            .query_row(
                "SELECT id, issue_local_id, repo_path, worktree_path, branch_name, is_active, created_at, updated_at
                 FROM work_contexts
                 WHERE issue_local_id = ?1 AND is_active = 1
                 ORDER BY updated_at DESC, id DESC
                 LIMIT 1",
                [issue_local_id],
                map_work_context_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn set_active_work_context(
        &self,
        issue_local_id: i64,
        repo_path: &str,
        worktree_path: Option<&str>,
        branch_name: Option<&str>,
    ) -> Result<WorkContext> {
        self.clear_active_work_context(issue_local_id)?;
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO work_contexts (issue_local_id, repo_path, worktree_path, branch_name, is_active, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5)",
            params![issue_local_id, repo_path, worktree_path, branch_name, now],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_work_context(id)?
            .context("created work context missing from database")
    }

    pub fn clear_active_work_context(&self, issue_local_id: i64) -> Result<usize> {
        let updated = self.conn.execute(
            "UPDATE work_contexts
             SET is_active = 0, updated_at = ?2
             WHERE issue_local_id = ?1 AND is_active = 1",
            params![issue_local_id, Utc::now().to_rfc3339()],
        )?;
        Ok(updated)
    }

    pub fn get_active_session_link(&self, issue_local_id: i64) -> Result<Option<SessionLink>> {
        self.conn
            .query_row(
                "SELECT id, issue_local_id, session_ref, session_kind, label, last_heartbeat_at, is_active, created_at
                 FROM session_links
                 WHERE issue_local_id = ?1 AND is_active = 1
                 ORDER BY last_heartbeat_at DESC, id DESC
                 LIMIT 1",
                [issue_local_id],
                map_session_link_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn set_active_session_link(
        &self,
        issue_local_id: i64,
        session_ref: &str,
        session_kind: SessionKind,
        label: &str,
    ) -> Result<SessionLink> {
        self.clear_active_session_link(issue_local_id)?;
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO session_links (issue_local_id, session_ref, session_kind, label, last_heartbeat_at, is_active, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?5)",
            params![issue_local_id, session_ref, session_kind.code(), label, now],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_session_link(id)?
            .context("created session link missing from database")
    }

    pub fn clear_active_session_link(&self, issue_local_id: i64) -> Result<usize> {
        let updated = self.conn.execute(
            "UPDATE session_links
             SET is_active = 0
             WHERE issue_local_id = ?1 AND is_active = 1",
            [issue_local_id],
        )?;
        Ok(updated)
    }

    pub fn create_run(
        &self,
        issue_local_id: i64,
        kind: RunKind,
        summary: Option<&str>,
    ) -> Result<RunRecord> {
        self.conn.execute(
            "INSERT INTO runs (issue_local_id, kind, status, started_at, ended_at, summary, exit_code, session_ref)
             VALUES (?1, ?2, 'running', ?3, NULL, ?4, NULL, NULL)",
            params![issue_local_id, kind.code(), Utc::now().to_rfc3339(), summary],
        )?;
        let run_id = self.conn.last_insert_rowid();
        self.get_run(run_id)?
            .context("created run missing from database")
    }

    pub fn append_run_event(
        &self,
        run_id: i64,
        level: RunEventLevel,
        message: &str,
    ) -> Result<RunEventRecord> {
        self.conn.execute(
            "INSERT INTO run_events (run_id, created_at, level, message)
             VALUES (?1, ?2, ?3, ?4)",
            params![run_id, Utc::now().to_rfc3339(), level.code(), message],
        )?;
        let event_id = self.conn.last_insert_rowid();
        self.get_run_event(event_id)?
            .context("created run event missing from database")
    }

    pub fn complete_run(
        &self,
        run_id: i64,
        status: RunStatus,
        summary: Option<&str>,
        exit_code: Option<i64>,
    ) -> Result<RunRecord> {
        self.conn.execute(
            "UPDATE runs
             SET status = ?2, ended_at = ?3, summary = COALESCE(?4, summary), exit_code = ?5
             WHERE id = ?1",
            params![
                run_id,
                status.code(),
                Utc::now().to_rfc3339(),
                summary,
                exit_code
            ],
        )?;
        self.get_run(run_id)?
            .context("completed run missing from database")
    }

    pub fn create_artifact(
        &self,
        issue_local_id: i64,
        run_id: Option<i64>,
        kind: ArtifactKind,
        content_preview: &str,
        location: Option<&str>,
    ) -> Result<ArtifactRecord> {
        self.conn.execute(
            "INSERT INTO artifacts (issue_local_id, run_id, kind, content_preview, location, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                issue_local_id,
                run_id,
                kind.code(),
                content_preview,
                location,
                Utc::now().to_rfc3339()
            ],
        )?;
        let artifact_id = self.conn.last_insert_rowid();
        self.get_artifact(artifact_id)?
            .context("created artifact missing from database")
    }

    pub fn create_issue(&self, draft: &IssueDraft) -> Result<Issue> {
        let now = Utc::now();
        self.conn.execute(
            "INSERT INTO issues (remote_id, identifier, title, description, project, labels_json, status, priority, assignee, owner_type, owner_name, attention_reason, blocked_reason, closeout_summary, follow_up_needed, is_archived, sync_state, updated_at)
             VALUES (NULL, '', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 0, 'pending_create', ?14)",
            params![
                draft.title,
                draft.description,
                draft.project,
                encode_labels(&draft.labels)?,
                draft.status.code(),
                encode_priority(&draft.priority),
                draft.assignee,
                draft.owner_type.code(),
                draft.owner_name,
                draft.attention_reason,
                draft.blocked_reason,
                draft.closeout_summary,
                if draft.follow_up_needed { 1 } else { 0 },
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
                 owner_type = ?8,
                 owner_name = ?9,
                 attention_reason = ?10,
                 blocked_reason = ?11,
                 closeout_summary = ?12,
                 follow_up_needed = ?13,
                 is_archived = ?14,
                 sync_state = ?15,
                 updated_at = ?16
             WHERE local_id = ?17",
            params![
                patch.title.as_deref().unwrap_or(&current.title),
                patch.description.as_deref().unwrap_or(&current.description),
                patch.project.clone().unwrap_or(current.project.clone()),
                encode_labels(patch.labels.as_ref().unwrap_or(&current.labels),)?,
                patch.status.as_ref().unwrap_or(&current.status).code(),
                encode_priority(patch.priority.as_ref().unwrap_or(&current.priority)),
                patch.assignee.clone().unwrap_or(current.assignee.clone()),
                patch
                    .owner_type
                    .as_ref()
                    .unwrap_or(&current.owner_type)
                    .code(),
                patch
                    .owner_name
                    .clone()
                    .unwrap_or(current.owner_name.clone()),
                patch
                    .attention_reason
                    .clone()
                    .unwrap_or(current.attention_reason.clone()),
                patch
                    .blocked_reason
                    .clone()
                    .unwrap_or(current.blocked_reason.clone()),
                patch
                    .closeout_summary
                    .clone()
                    .unwrap_or(current.closeout_summary.clone()),
                if patch.follow_up_needed.unwrap_or(current.follow_up_needed) {
                    1
                } else {
                    0
                },
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

    fn get_run(&self, id: i64) -> Result<Option<RunRecord>> {
        self.conn
            .query_row(
                "SELECT id, issue_local_id, kind, status, started_at, ended_at, summary, exit_code, session_ref
                 FROM runs
                 WHERE id = ?1",
                [id],
                map_run_row,
            )
            .optional()
            .map_err(Into::into)
    }

    fn get_run_event(&self, id: i64) -> Result<Option<RunEventRecord>> {
        self.conn
            .query_row(
                "SELECT id, run_id, created_at, level, message
                 FROM run_events
                 WHERE id = ?1",
                [id],
                map_run_event_row,
            )
            .optional()
            .map_err(Into::into)
    }

    fn get_artifact(&self, id: i64) -> Result<Option<ArtifactRecord>> {
        self.conn
            .query_row(
                "SELECT id, issue_local_id, run_id, kind, content_preview, location, created_at
                 FROM artifacts
                 WHERE id = ?1",
                [id],
                map_artifact_row,
            )
            .optional()
            .map_err(Into::into)
    }

    fn get_handoff(&self, id: i64) -> Result<Option<HandoffRecord>> {
        self.conn
            .query_row(
                "SELECT id, issue_local_id, from_actor, to_actor, note, created_at
                 FROM handoffs
                 WHERE id = ?1",
                [id],
                map_handoff_row,
            )
            .optional()
            .map_err(Into::into)
    }

    fn get_work_context(&self, id: i64) -> Result<Option<WorkContext>> {
        self.conn
            .query_row(
                "SELECT id, issue_local_id, repo_path, worktree_path, branch_name, is_active, created_at, updated_at
                 FROM work_contexts
                 WHERE id = ?1",
                [id],
                map_work_context_row,
            )
            .optional()
            .map_err(Into::into)
    }

    fn get_session_link(&self, id: i64) -> Result<Option<SessionLink>> {
        self.conn
            .query_row(
                "SELECT id, issue_local_id, session_ref, session_kind, label, last_heartbeat_at, is_active, created_at
                 FROM session_links
                 WHERE id = ?1",
                [id],
                map_session_link_row,
            )
            .optional()
            .map_err(Into::into)
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
                owner_type TEXT NOT NULL DEFAULT 'unassigned',
                owner_name TEXT,
                attention_reason TEXT,
                blocked_reason TEXT,
                closeout_summary TEXT,
                follow_up_needed INTEGER NOT NULL DEFAULT 0,
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
            );

            CREATE TABLE IF NOT EXISTS scratch_items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                body TEXT NOT NULL,
                source TEXT NOT NULL,
                created_at TEXT NOT NULL,
                promoted_issue_id INTEGER
            );

            CREATE TABLE IF NOT EXISTS runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                issue_local_id INTEGER NOT NULL,
                kind TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                summary TEXT,
                exit_code INTEGER,
                session_ref TEXT
            );

            CREATE TABLE IF NOT EXISTS run_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                level TEXT NOT NULL,
                message TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS artifacts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                issue_local_id INTEGER NOT NULL,
                run_id INTEGER,
                kind TEXT NOT NULL,
                content_preview TEXT NOT NULL,
                location TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS handoffs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                issue_local_id INTEGER NOT NULL,
                from_actor TEXT NOT NULL,
                to_actor TEXT NOT NULL,
                note TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS work_contexts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                issue_local_id INTEGER NOT NULL,
                repo_path TEXT NOT NULL,
                worktree_path TEXT,
                branch_name TEXT,
                is_active INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS session_links (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                issue_local_id INTEGER NOT NULL,
                session_ref TEXT NOT NULL,
                session_kind TEXT NOT NULL,
                label TEXT NOT NULL,
                last_heartbeat_at TEXT NOT NULL,
                is_active INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL
            );",
        )?;
        let issue_columns = self.table_columns("issues")?;
        if !issue_columns.iter().any(|column| column == "is_archived") {
            self.conn.execute(
                "ALTER TABLE issues ADD COLUMN is_archived INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !issue_columns.iter().any(|column| column == "project") {
            self.conn
                .execute("ALTER TABLE issues ADD COLUMN project TEXT", [])?;
        }
        if !issue_columns.iter().any(|column| column == "labels_json") {
            self.conn.execute(
                "ALTER TABLE issues ADD COLUMN labels_json TEXT NOT NULL DEFAULT '[]'",
                [],
            )?;
        }
        if !issue_columns.iter().any(|column| column == "owner_type") {
            self.conn.execute(
                "ALTER TABLE issues ADD COLUMN owner_type TEXT NOT NULL DEFAULT 'unassigned'",
                [],
            )?;
        }
        if !issue_columns.iter().any(|column| column == "owner_name") {
            self.conn
                .execute("ALTER TABLE issues ADD COLUMN owner_name TEXT", [])?;
        }
        if !issue_columns
            .iter()
            .any(|column| column == "attention_reason")
        {
            self.conn
                .execute("ALTER TABLE issues ADD COLUMN attention_reason TEXT", [])?;
        }
        if !issue_columns
            .iter()
            .any(|column| column == "blocked_reason")
        {
            self.conn
                .execute("ALTER TABLE issues ADD COLUMN blocked_reason TEXT", [])?;
        }
        if !issue_columns
            .iter()
            .any(|column| column == "closeout_summary")
        {
            self.conn
                .execute("ALTER TABLE issues ADD COLUMN closeout_summary TEXT", [])?;
        }
        if !issue_columns
            .iter()
            .any(|column| column == "follow_up_needed")
        {
            self.conn.execute(
                "ALTER TABLE issues ADD COLUMN follow_up_needed INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        let queued_columns = self.table_columns("queued_mutations")?;
        if !queued_columns
            .iter()
            .any(|column| column == "attempt_count")
        {
            self.conn.execute(
                "ALTER TABLE queued_mutations ADD COLUMN attempt_count INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !queued_columns.iter().any(|column| column == "last_error") {
            self.conn.execute(
                "ALTER TABLE queued_mutations ADD COLUMN last_error TEXT",
                [],
            )?;
        }
        let scratch_columns = self.table_columns("scratch_items")?;
        if !scratch_columns
            .iter()
            .any(|column| column == "promoted_issue_id")
        {
            self.conn.execute(
                "ALTER TABLE scratch_items ADD COLUMN promoted_issue_id INTEGER",
                [],
            )?;
        }
        let run_columns = self.table_columns("runs")?;
        if !run_columns.iter().any(|column| column == "summary") {
            self.conn
                .execute("ALTER TABLE runs ADD COLUMN summary TEXT", [])?;
        }
        if !run_columns.iter().any(|column| column == "exit_code") {
            self.conn
                .execute("ALTER TABLE runs ADD COLUMN exit_code INTEGER", [])?;
        }
        if !run_columns.iter().any(|column| column == "session_ref") {
            self.conn
                .execute("ALTER TABLE runs ADD COLUMN session_ref TEXT", [])?;
        }
        self.normalize_issue_rows()?;
        self.normalize_queued_mutations()?;
        Ok(())
    }

    fn table_columns(&self, table: &str) -> Result<Vec<String>> {
        let pragma = format!("PRAGMA table_info({table})");
        let columns = self
            .conn
            .prepare(&pragma)?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(columns)
    }

    fn normalize_issue_rows(&self) -> Result<()> {
        self.conn.execute(
            "UPDATE issues
             SET status = 'agent_running'
             WHERE status = 'in_progress'",
            [],
        )?;
        self.conn.execute(
            "UPDATE issues
             SET labels_json = '[]'
             WHERE labels_json IS NULL OR TRIM(labels_json) = ''",
            [],
        )?;
        self.conn.execute(
            "UPDATE issues
             SET identifier = 'LOCAL-' || local_id
             WHERE identifier IS NULL OR TRIM(identifier) = ''",
            [],
        )?;
        self.conn.execute(
            "UPDATE issues
             SET sync_state = CASE
                 WHEN remote_id IS NULL THEN 'pending_create'
                 ELSE 'pending_update'
             END
             WHERE sync_state IS NULL OR TRIM(sync_state) = ''",
            [],
        )?;
        self.conn.execute(
            "UPDATE issues
             SET owner_type = 'unassigned'
             WHERE owner_type IS NULL OR TRIM(owner_type) = ''",
            [],
        )?;
        self.conn.execute(
            "UPDATE issues
             SET attention_reason = CASE
                 WHEN status = 'ready_for_agent' THEN 'ready for agent pickup'
                 WHEN status = 'agent_running' THEN 'agent is currently working'
                 WHEN status = 'needs_human_input' THEN 'human decision needed'
                 WHEN status = 'needs_review' THEN 'review requested'
                 WHEN status = 'blocked' THEN 'blocked and waiting'
                 WHEN status = 'done' THEN 'closed loop'
                 ELSE 'needs triage'
             END
             WHERE attention_reason IS NULL OR TRIM(attention_reason) = ''",
            [],
        )?;
        self.conn.execute(
            "UPDATE issues
             SET follow_up_needed = 0
             WHERE follow_up_needed IS NULL",
            [],
        )?;
        Ok(())
    }

    fn normalize_queued_mutations(&self) -> Result<()> {
        self.conn.execute(
            "UPDATE queued_mutations
             SET kind_json = REPLACE(kind_json, '\"InProgress\"', '\"AgentRunning\"')
             WHERE kind_json LIKE '%\"InProgress\"%'",
            [],
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
            "INSERT INTO issues (remote_id, identifier, title, description, project, labels_json, status, priority, assignee, owner_type, owner_name, attention_reason, blocked_reason, closeout_summary, follow_up_needed, is_archived, sync_state, updated_at)
             VALUES
             ('lin_1001', 'ENG-12', 'Make offline triage feel instant', 'Seed issue that demonstrates synced state and richer detail copy.', 'Core', '[\"offline\",\"ux\"]', 'needs_review', 'high', 'you', 'human', 'reviewer', 'review requested', NULL, NULL, 0, 0, 'synced', ?1),
             (NULL, 'LOCAL-2', 'Draft local issue without network', 'This one starts as a queued local-only record to show the offline model.', 'Personal', '[\"draft\"]', 'todo', 'medium', NULL, 'unassigned', NULL, 'needs triage', NULL, NULL, 0, 0, 'pending_create', ?1)",
            [now.clone()],
        )?;
        self.enqueue(QueuedMutationKind::CreateIssue { issue_local_id: 2 })?;
        self.conn.execute(
            "INSERT INTO scratch_items (body, source, created_at, promoted_issue_id)
             VALUES
             ('Investigate why daily review loops still happen outside the tracker.', 'manual', ?1, NULL),
             ('Agent suggested splitting flaky sync QA into a separate follow-up.', 'agent', ?1, NULL)",
            [now],
        )?;
        self.conn.execute(
            "INSERT INTO runs (issue_local_id, kind, status, started_at, ended_at, summary, exit_code, session_ref)
             VALUES (1, 'manual', 'succeeded', ?1, ?1, 'Initial local triage run completed', 0, NULL)",
            [Utc::now().to_rfc3339()],
        )?;
        self.conn.execute(
            "INSERT INTO run_events (run_id, created_at, level, message)
             VALUES (1, ?1, 'info', 'Reviewed backlog shape and prepared next offline workflow step')",
            [Utc::now().to_rfc3339()],
        )?;
        self.conn.execute(
            "INSERT INTO artifacts (issue_local_id, run_id, kind, content_preview, location, created_at)
             VALUES (1, 1, 'note', 'Captured a short closeout note for the seed workflow.', NULL, ?1)",
            [Utc::now().to_rfc3339()],
        )?;
        self.conn.execute(
            "INSERT INTO handoffs (issue_local_id, from_actor, to_actor, note, created_at)
             VALUES (1, 'human', 'reviewer', 'Initial review requested for the seed workflow.', ?1)",
            [Utc::now().to_rfc3339()],
        )?;
        self.conn.execute(
            "INSERT INTO work_contexts (issue_local_id, repo_path, worktree_path, branch_name, is_active, created_at, updated_at)
             VALUES (1, '/tmp/logit-demo', '/tmp/logit-demo', 'feature/offline-inbox', 1, ?1, ?1)",
            [Utc::now().to_rfc3339()],
        )?;
        self.conn.execute(
            "INSERT INTO session_links (issue_local_id, session_ref, session_kind, label, last_heartbeat_at, is_active, created_at)
             VALUES (1, 'session-1', 'agent_session', 'Codex worker', ?1, 1, ?1)",
            [Utc::now().to_rfc3339()],
        )?;
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
        patch.status = Some(IssueStatus::AgentRunning);
        let updated = store.update_issue(created.local_id, &patch)?;
        assert_eq!(updated.title, "Write and test the CRUD layer");
        assert_eq!(updated.status, IssueStatus::AgentRunning);

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
            patch.status = Some(IssueStatus::AgentRunning);
            store.update_issue(local.local_id, &patch)?
        };
        let filtered = store.list_issues(&IssueQuery {
            status: Some(IssueStatus::AgentRunning),
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

    #[test]
    fn scratch_items_can_be_promoted_to_issues() -> Result<()> {
        let store = Store::open_in_memory()?;
        let scratch = store.create_scratch_item(
            "Follow up with customer about yesterday's blocked handoff",
            ScratchSource::Manual,
        )?;

        let promoted = store.promote_scratch_to_issue(scratch.id)?;
        assert_eq!(
            promoted.title,
            "Follow up with customer about yesterday's blocked handoff"
        );

        let scratch = store
            .get_scratch_item(scratch.id)?
            .expect("scratch item should still exist");
        assert_eq!(scratch.promoted_issue_id, Some(promoted.local_id));
        Ok(())
    }

    #[test]
    fn legacy_schema_is_migrated_across_tables() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE issues (
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
            CREATE TABLE queued_mutations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind_json TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE scratch_items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                body TEXT NOT NULL,
                source TEXT NOT NULL,
                created_at TEXT NOT NULL
            );",
        )?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO issues (remote_id, identifier, title, description, status, priority, assignee, sync_state, updated_at)
             VALUES (NULL, '', 'Legacy issue', 'Created before v2', 'in_progress', 'medium', NULL, 'pending_create', ?1)",
            [now.clone()],
        )?;
        conn.execute(
            "INSERT INTO queued_mutations (kind_json, created_at)
             VALUES (?1, ?2)",
            params![r#"{"CreateIssue":{"issue_local_id":1}}"#, now.clone()],
        )?;
        conn.execute(
            "INSERT INTO queued_mutations (kind_json, created_at)
             VALUES (?1, ?2)",
            params![
                r#"{"UpdateIssue":{"issue_local_id":1,"patch":{"title":null,"description":null,"project":null,"labels":null,"status":"InProgress","priority":null,"assignee":null,"is_archived":null}}}"#,
                now.clone()
            ],
        )?;
        conn.execute(
            "INSERT INTO scratch_items (body, source, created_at)
             VALUES ('Legacy scratch', 'manual', ?1)",
            [now],
        )?;

        let store = Store::from_connection(conn, false)?;

        let issue = store.get_issue(1)?.expect("legacy issue should migrate");
        assert_eq!(issue.status, IssueStatus::AgentRunning);
        assert_eq!(issue.identifier, "LOCAL-1");
        assert_eq!(issue.labels, Vec::<String>::new());

        let scratch = store
            .get_scratch_item(1)?
            .expect("legacy scratch should migrate");
        assert_eq!(scratch.promoted_issue_id, None);

        let pending = store.list_pending_mutations()?;
        assert_eq!(pending.len(), 2);
        match &pending[1].kind {
            QueuedMutationKind::UpdateIssue { patch, .. } => {
                assert_eq!(patch.status, Some(IssueStatus::AgentRunning));
            }
            other => panic!("expected migrated update mutation, got {other:?}"),
        }
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
        owner_type: decode_owner_type(&row.get::<_, String>(10)?)?,
        owner_name: row.get(11)?,
        attention_reason: row.get(12)?,
        blocked_reason: row.get(13)?,
        closeout_summary: row.get(14)?,
        follow_up_needed: row.get::<_, i64>(15)? != 0,
        is_archived: row.get::<_, i64>(16)? != 0,
        sync_state: decode_sync_state(&row.get::<_, String>(17)?)?,
        updated_at: parse_dt(row.get::<_, String>(18)?).map_err(to_sql_conversion_error)?,
    })
}

fn map_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRecord> {
    Ok(RunRecord {
        id: row.get(0)?,
        issue_local_id: row.get(1)?,
        kind: decode_run_kind(&row.get::<_, String>(2)?)?,
        status: decode_run_status(&row.get::<_, String>(3)?)?,
        started_at: parse_dt(row.get::<_, String>(4)?).map_err(to_sql_conversion_error)?,
        ended_at: row
            .get::<_, Option<String>>(5)?
            .map(parse_dt)
            .transpose()
            .map_err(to_sql_conversion_error)?,
        summary: row.get(6)?,
        exit_code: row.get(7)?,
        session_ref: row.get(8)?,
    })
}

fn map_run_event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunEventRecord> {
    Ok(RunEventRecord {
        id: row.get(0)?,
        run_id: row.get(1)?,
        created_at: parse_dt(row.get::<_, String>(2)?).map_err(to_sql_conversion_error)?,
        level: decode_run_event_level(&row.get::<_, String>(3)?)?,
        message: row.get(4)?,
    })
}

fn map_artifact_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactRecord> {
    Ok(ArtifactRecord {
        id: row.get(0)?,
        issue_local_id: row.get(1)?,
        run_id: row.get(2)?,
        kind: decode_artifact_kind(&row.get::<_, String>(3)?)?,
        content_preview: row.get(4)?,
        location: row.get(5)?,
        created_at: parse_dt(row.get::<_, String>(6)?).map_err(to_sql_conversion_error)?,
    })
}

fn map_handoff_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HandoffRecord> {
    Ok(HandoffRecord {
        id: row.get(0)?,
        issue_local_id: row.get(1)?,
        from_actor: row.get(2)?,
        to_actor: row.get(3)?,
        note: row.get(4)?,
        created_at: parse_dt(row.get::<_, String>(5)?).map_err(to_sql_conversion_error)?,
    })
}

fn map_work_context_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkContext> {
    Ok(WorkContext {
        id: row.get(0)?,
        issue_local_id: row.get(1)?,
        repo_path: row.get(2)?,
        worktree_path: row.get(3)?,
        branch_name: row.get(4)?,
        is_active: row.get::<_, i64>(5)? != 0,
        created_at: parse_dt(row.get::<_, String>(6)?).map_err(to_sql_conversion_error)?,
        updated_at: parse_dt(row.get::<_, String>(7)?).map_err(to_sql_conversion_error)?,
    })
}

fn map_session_link_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionLink> {
    Ok(SessionLink {
        id: row.get(0)?,
        issue_local_id: row.get(1)?,
        session_ref: row.get(2)?,
        session_kind: decode_session_kind(&row.get::<_, String>(3)?)?,
        label: row.get(4)?,
        last_heartbeat_at: parse_dt(row.get::<_, String>(5)?).map_err(to_sql_conversion_error)?,
        is_active: row.get::<_, i64>(6)? != 0,
        created_at: parse_dt(row.get::<_, String>(7)?).map_err(to_sql_conversion_error)?,
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
        "ready_for_agent" => Ok(IssueStatus::ReadyForAgent),
        "agent_running" => Ok(IssueStatus::AgentRunning),
        "needs_human_input" => Ok(IssueStatus::NeedsHumanInput),
        "needs_review" => Ok(IssueStatus::NeedsReview),
        "blocked" => Ok(IssueStatus::Blocked),
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

fn decode_owner_type(value: &str) -> rusqlite::Result<OwnerType> {
    match value {
        "human" => Ok(OwnerType::Human),
        "agent" => Ok(OwnerType::Agent),
        "unassigned" => Ok(OwnerType::Unassigned),
        other => Err(to_sql_conversion_error(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown owner type {other}"),
        ))),
    }
}

fn decode_run_kind(value: &str) -> rusqlite::Result<RunKind> {
    match value {
        "manual" => Ok(RunKind::Manual),
        "agent" => Ok(RunKind::Agent),
        "shell" => Ok(RunKind::Shell),
        "script" => Ok(RunKind::Script),
        other => Err(to_sql_conversion_error(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown run kind {other}"),
        ))),
    }
}

fn decode_run_status(value: &str) -> rusqlite::Result<RunStatus> {
    match value {
        "queued" => Ok(RunStatus::Queued),
        "running" => Ok(RunStatus::Running),
        "succeeded" => Ok(RunStatus::Succeeded),
        "failed" => Ok(RunStatus::Failed),
        "cancelled" => Ok(RunStatus::Cancelled),
        other => Err(to_sql_conversion_error(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown run status {other}"),
        ))),
    }
}

fn decode_run_event_level(value: &str) -> rusqlite::Result<RunEventLevel> {
    match value {
        "info" => Ok(RunEventLevel::Info),
        "warn" => Ok(RunEventLevel::Warn),
        "error" => Ok(RunEventLevel::Error),
        other => Err(to_sql_conversion_error(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown run event level {other}"),
        ))),
    }
}

fn decode_artifact_kind(value: &str) -> rusqlite::Result<ArtifactKind> {
    match value {
        "note" => Ok(ArtifactKind::Note),
        "output" => Ok(ArtifactKind::Output),
        "link" => Ok(ArtifactKind::Link),
        "file" => Ok(ArtifactKind::FileRef),
        other => Err(to_sql_conversion_error(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown artifact kind {other}"),
        ))),
    }
}

fn decode_session_kind(value: &str) -> rusqlite::Result<SessionKind> {
    match value {
        "human_terminal" => Ok(SessionKind::HumanTerminal),
        "agent_session" => Ok(SessionKind::AgentSession),
        "background_job" => Ok(SessionKind::BackgroundJob),
        other => Err(to_sql_conversion_error(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown session kind {other}"),
        ))),
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

fn encode_scratch_source(source: &ScratchSource) -> &'static str {
    match source {
        ScratchSource::Manual => "manual",
        ScratchSource::Agent => "agent",
        ScratchSource::RunFailure => "run_failure",
        ScratchSource::Pasted => "pasted",
    }
}

fn decode_scratch_source(value: &str) -> rusqlite::Result<ScratchSource> {
    match value {
        "manual" => Ok(ScratchSource::Manual),
        "agent" => Ok(ScratchSource::Agent),
        "run_failure" => Ok(ScratchSource::RunFailure),
        "pasted" => Ok(ScratchSource::Pasted),
        other => Err(to_sql_conversion_error(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown scratch source {other}"),
        ))),
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
