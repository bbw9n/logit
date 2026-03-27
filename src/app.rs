use crate::{
    config::WorkspaceConfig,
    domain::{Issue, IssuePatch, SyncState},
    store::Store,
    sync::{LinearSyncService, SyncService},
};
use anyhow::Result;

pub struct App {
    pub config: WorkspaceConfig,
    pub issues: Vec<Issue>,
    pub selected: usize,
    pub unsynced_only: bool,
    pub status_message: String,
    pub queued_mutation_count: usize,
    store: Store,
    sync_service: LinearSyncService,
}

impl App {
    pub fn bootstrap() -> Result<Self> {
        let config = WorkspaceConfig::load()?;
        let store = Store::open(&config.database_path)?;
        let sync_service = LinearSyncService::new(config.clone());
        let mut app = Self {
            config,
            issues: Vec::new(),
            selected: 0,
            unsynced_only: false,
            status_message: String::from("Offline-first issue tracking ready"),
            queued_mutation_count: 0,
            store,
            sync_service,
        };
        app.reload()?;
        Ok(app)
    }

    pub fn current_issue(&self) -> Option<&Issue> {
        self.issues.get(self.selected)
    }

    pub fn select_next(&mut self) {
        if self.issues.is_empty() {
            self.selected = 0;
        } else {
            self.selected = (self.selected + 1).min(self.issues.len() - 1);
        }
    }

    pub fn select_previous(&mut self) {
        if self.issues.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.saturating_sub(1);
        }
    }

    pub fn toggle_filter(&mut self) {
        self.unsynced_only = !self.unsynced_only;
        if let Err(error) = self.reload() {
            self.status_message = format!("Failed to reload issues: {error:#}");
        }
    }

    pub fn create_issue(&mut self) -> Result<()> {
        let count = self.queued_mutation_count + 1;
        let issue = self.store.create_issue(
            &format!("Local draft issue #{count}"),
            "Created from the TUI while offline. Press y to attempt sync once LINEAR_API_KEY is available.",
        )?;
        self.reload()?;
        self.select_issue(issue.local_id);
        self.status_message = format!("Created {} and queued it for sync", issue.identifier);
        Ok(())
    }

    pub fn cycle_status(&mut self) -> Result<()> {
        let Some(issue) = self.current_issue().cloned() else {
            return Ok(());
        };
        let mut patch = IssuePatch::empty();
        patch.status = Some(issue.status.cycle());
        let updated = self.store.update_issue(issue.local_id, &patch)?;
        self.reload()?;
        self.select_issue(updated.local_id);
        self.status_message = format!(
            "Updated {} to {}",
            updated.identifier,
            updated.status.label()
        );
        Ok(())
    }

    pub fn touch_title(&mut self) -> Result<()> {
        let Some(issue) = self.current_issue().cloned() else {
            return Ok(());
        };
        let mut patch = IssuePatch::empty();
        patch.title = Some(format!("{} [edited]", issue.title));
        let updated = self.store.update_issue(issue.local_id, &patch)?;
        self.reload()?;
        self.select_issue(updated.local_id);
        self.status_message = format!("Queued local edit for {}", updated.identifier);
        Ok(())
    }

    pub fn sync_now(&mut self) -> Result<()> {
        match self.sync_service.push(&self.store) {
            Ok(report) => {
                self.reload()?;
                self.status_message = format!(
                    "{} | pushed={}, failed={}",
                    report.message, report.pushed, report.failed
                );
                Ok(())
            }
            Err(error) => {
                self.reload()?;
                self.status_message = format!("Sync failed: {error:#}");
                Ok(())
            }
        }
    }

    pub fn retry_failed_sync(&mut self) -> Result<()> {
        let retried = self.store.retry_failed_mutations()?;
        self.reload()?;
        self.status_message = if retried == 0 {
            "No failed issues to retry".into()
        } else {
            format!("Moved {retried} issue(s) back to pending sync")
        };
        Ok(())
    }

    fn reload(&mut self) -> Result<()> {
        self.issues = self.store.list_issues(self.unsynced_only)?;
        self.queued_mutation_count = self.store.list_pending_mutations()?.len();
        if self.issues.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.issues.len() - 1);
        }
        Ok(())
    }

    fn select_issue(&mut self, local_id: i64) {
        if let Some(index) = self
            .issues
            .iter()
            .position(|issue| issue.local_id == local_id)
        {
            self.selected = index;
        }
    }

    pub fn offline_badge(&self) -> &'static str {
        if self.config.linear_api_token.is_some() {
            "sync ready"
        } else {
            "offline"
        }
    }

    pub fn pending_summary(&self) -> String {
        let pending = self
            .issues
            .iter()
            .filter(|issue| {
                matches!(
                    issue.sync_state,
                    SyncState::PendingCreate | SyncState::PendingUpdate
                )
            })
            .count();
        let errors = self
            .issues
            .iter()
            .filter(|issue| issue.sync_state == SyncState::SyncError)
            .count();
        format!(
            "queue: {} | pending: {} | errors: {}",
            self.queued_mutation_count, pending, errors
        )
    }
}
