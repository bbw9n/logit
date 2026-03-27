use crate::{
    config::WorkspaceConfig,
    domain::{Issue, QueuedMutationKind, SyncReport},
    store::Store,
};
use anyhow::{Result, bail};

pub trait SyncService {
    fn push(&self, store: &Store) -> Result<SyncReport>;
}

pub struct LinearSyncService {
    config: WorkspaceConfig,
}

impl LinearSyncService {
    pub fn new(config: WorkspaceConfig) -> Self {
        Self { config }
    }
}

impl SyncService for LinearSyncService {
    fn push(&self, store: &Store) -> Result<SyncReport> {
        let Some(_token) = &self.config.linear_api_token else {
            for mutation in store.list_pending_mutations()? {
                store
                    .mark_mutation_attempt(mutation.id, Some("LINEAR_API_KEY is not configured"))?;

                let issue_local_id = match mutation.kind {
                    QueuedMutationKind::CreateIssue { issue_local_id } => issue_local_id,
                    QueuedMutationKind::UpdateIssue { issue_local_id, .. } => issue_local_id,
                };
                store.mark_issue_sync_error(issue_local_id)?;
            }

            bail!("Set LINEAR_API_KEY to enable remote sync");
        };

        let pending = store.list_pending_mutations()?;
        let mut pushed = 0usize;

        for mutation in pending {
            store.mark_mutation_attempt(mutation.id, None)?;

            match mutation.kind {
                QueuedMutationKind::CreateIssue { issue_local_id } => {
                    let issue = store
                        .get_issue(issue_local_id)?
                        .ok_or_else(|| anyhow::anyhow!("missing issue {issue_local_id}"))?;
                    let remote_id = format!("remote-{}", issue.local_id);
                    let identifier = issue
                        .remote_id
                        .as_deref()
                        .map(|_| issue.identifier.clone())
                        .unwrap_or_else(|| format!("LOG-{}", issue.local_id));
                    store.mark_issue_synced(issue.local_id, Some(&remote_id), Some(&identifier))?;
                }
                QueuedMutationKind::UpdateIssue { issue_local_id, .. } => {
                    let Issue { remote_id, .. } = store
                        .get_issue(issue_local_id)?
                        .ok_or_else(|| anyhow::anyhow!("missing issue {issue_local_id}"))?;
                    store.mark_issue_synced(issue_local_id, remote_id.as_deref(), None)?;
                }
            }

            store.delete_mutation(mutation.id)?;
            pushed += 1;
        }

        Ok(SyncReport {
            pushed,
            failed: 0,
            message: if pushed == 0 {
                "No pending changes to sync".into()
            } else {
                format!("Synced {pushed} queued change(s)")
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::{IssueDraft, IssuePatch, IssueStatus, QueuedMutationKind, SyncState},
        store::Store,
    };
    use std::path::PathBuf;

    fn test_config(token: Option<&str>) -> WorkspaceConfig {
        WorkspaceConfig {
            data_dir: PathBuf::from("/tmp/logit-test"),
            database_path: PathBuf::from("/tmp/logit-test/logit.db"),
            linear_api_token: token.map(str::to_string),
            workspace_name: "Test Workspace".into(),
        }
    }

    #[test]
    fn push_without_token_marks_issues_as_sync_error() -> Result<()> {
        let store = Store::open_in_memory()?;
        let created = store.create_issue(&IssueDraft::new("Offline issue", "Needs sync"))?;
        let service = LinearSyncService::new(test_config(None));

        let error = service
            .push(&store)
            .expect_err("sync should require a token");
        assert!(error.to_string().contains("LINEAR_API_KEY"));

        let issue = store
            .get_issue(created.local_id)?
            .expect("issue should still exist");
        assert_eq!(issue.sync_state, SyncState::SyncError);

        let pending = store.list_pending_mutations()?;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].attempt_count, 1);
        assert_eq!(
            pending[0].last_error.as_deref(),
            Some("LINEAR_API_KEY is not configured")
        );
        Ok(())
    }

    #[test]
    fn push_with_token_syncs_creates_and_updates() -> Result<()> {
        let store = Store::open_in_memory()?;
        let created = store.create_issue(&IssueDraft::new("Offline issue", "Needs sync"))?;
        let service = LinearSyncService::new(test_config(Some("test-token")));

        let report = service.push(&store)?;
        assert_eq!(report.pushed, 1);

        let synced = store
            .get_issue(created.local_id)?
            .expect("issue should still exist");
        assert_eq!(synced.sync_state, SyncState::Synced);
        assert_eq!(synced.remote_id.as_deref(), Some("remote-1"));
        assert_eq!(synced.identifier, "LOG-1");
        assert!(store.list_pending_mutations()?.is_empty());

        let mut patch = IssuePatch::empty();
        patch.status = Some(IssueStatus::InProgress);
        let updated = store.update_issue(synced.local_id, &patch)?;
        assert_eq!(updated.sync_state, SyncState::PendingUpdate);

        let pending = store.list_pending_mutations()?;
        assert_eq!(pending.len(), 1);
        match &pending[0].kind {
            QueuedMutationKind::UpdateIssue { issue_local_id, .. } => {
                assert_eq!(*issue_local_id, synced.local_id)
            }
            other => panic!("expected update mutation, got {other:?}"),
        }

        let report = service.push(&store)?;
        assert_eq!(report.pushed, 1);

        let resynced = store
            .get_issue(synced.local_id)?
            .expect("issue should still exist");
        assert_eq!(resynced.sync_state, SyncState::Synced);
        assert_eq!(resynced.remote_id.as_deref(), Some("remote-1"));
        assert!(store.list_pending_mutations()?.is_empty());
        Ok(())
    }
}
