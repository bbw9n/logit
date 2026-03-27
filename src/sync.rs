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
