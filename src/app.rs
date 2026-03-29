use crate::{
    config::WorkspaceConfig,
    domain::{
        Issue, IssueDraft, IssuePatch, IssueQuery, IssueStatus, OwnerType, Priority, ScratchItem,
        ScratchSource, SyncState,
    },
    store::Store,
    sync::{LinearSyncService, SyncService},
};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy)]
pub enum EditorFocus {
    Title,
    Description,
    Project,
    Labels,
    Assignee,
}

impl EditorFocus {
    fn next(self) -> Self {
        match self {
            Self::Title => Self::Description,
            Self::Description => Self::Project,
            Self::Project => Self::Labels,
            Self::Labels => Self::Assignee,
            Self::Assignee => Self::Title,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Title => Self::Assignee,
            Self::Description => Self::Title,
            Self::Project => Self::Description,
            Self::Labels => Self::Project,
            Self::Assignee => Self::Labels,
        }
    }
}

#[derive(Debug, Clone)]
pub enum EditorMode {
    Create,
    Edit { local_id: i64 },
    Search,
    ScratchCapture,
}

#[derive(Debug, Clone)]
pub struct EditorState {
    pub mode: EditorMode,
    pub focus: EditorFocus,
    pub title: String,
    pub description: String,
    pub project: String,
    pub labels: String,
    pub assignee: String,
    pub status: IssueStatus,
    pub priority: Priority,
    pub search: String,
    pub scratch_source: ScratchSource,
}

pub struct App {
    pub config: WorkspaceConfig,
    pub issues: Vec<Issue>,
    pub scratch_items: Vec<ScratchItem>,
    pub selected: usize,
    pub query: IssueQuery,
    pub saved_view: SavedView,
    pub status_message: String,
    pub queued_mutation_count: usize,
    pub editor: Option<EditorState>,
    pub show_help: bool,
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
            scratch_items: Vec::new(),
            selected: 0,
            query: IssueQuery::default(),
            saved_view: SavedView::Inbox,
            status_message: String::from("Offline-first issue tracking ready"),
            queued_mutation_count: 0,
            editor: None,
            show_help: false,
            store,
            sync_service,
        };
        app.reload()?;
        Ok(app)
    }

    pub fn current_issue(&self) -> Option<&Issue> {
        if self.saved_view == SavedView::Scratch {
            return None;
        }
        self.issues.get(self.selected)
    }

    pub fn current_scratch(&self) -> Option<&ScratchItem> {
        if self.saved_view != SavedView::Scratch {
            return None;
        }
        self.scratch_items.get(self.selected)
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.editor.is_some() {
            return self.handle_editor_key(key);
        }
        if self.show_help && key.code == KeyCode::Esc {
            self.toggle_help();
            return Ok(false);
        }

        match key.code {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_previous(),
            KeyCode::Char('n') => self.begin_create_editor(),
            KeyCode::Char('e') => self.begin_edit_editor(),
            KeyCode::Char('x') => self.begin_scratch_editor(),
            KeyCode::Char('i') => self.promote_selected_scratch()?,
            KeyCode::Char('s') => self.cycle_status()?,
            KeyCode::Char('p') => self.cycle_priority()?,
            KeyCode::Char('a') => self.toggle_archive_current_issue()?,
            KeyCode::Char('h') => self.send_current_issue_to_agent()?,
            KeyCode::Char('m') => self.request_human_input()?,
            KeyCode::Char('w') => self.request_review()?,
            KeyCode::Char('b') => self.mark_current_issue_blocked()?,
            KeyCode::Char('v') => self.toggle_archived_visibility(),
            KeyCode::Char('1') => self.set_saved_view(SavedView::Inbox),
            KeyCode::Char('2') => self.set_saved_view(SavedView::Running),
            KeyCode::Char('3') => self.set_saved_view(SavedView::Review),
            KeyCode::Char('4') => self.set_saved_view(SavedView::Waiting),
            KeyCode::Char('5') => self.set_saved_view(SavedView::Done),
            KeyCode::Char('6') => self.set_saved_view(SavedView::Scratch),
            KeyCode::Char('?') => self.toggle_help(),
            KeyCode::Char('y') => self.sync_now()?,
            KeyCode::Char('r') => self.retry_failed_sync()?,
            KeyCode::Char('/') => self.begin_search_editor(),
            KeyCode::Char('u') => self.clear_search(),
            KeyCode::Char('f') => self.toggle_unsynced_filter(),
            _ => {}
        }

        Ok(false)
    }

    pub fn select_next(&mut self) {
        let len = self.visible_len();
        if len == 0 {
            self.selected = 0;
        } else {
            self.selected = (self.selected + 1).min(len - 1);
        }
    }

    pub fn select_previous(&mut self) {
        if self.visible_len() == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.saturating_sub(1);
        }
    }

    pub fn toggle_unsynced_filter(&mut self) {
        self.query.unsynced_only = !self.query.unsynced_only;
        if self.query.unsynced_only {
            self.query.archived_only = false;
        }
        if let Err(error) = self.reload() {
            self.status_message = format!("Failed to reload issues: {error:#}");
        } else {
            self.status_message = if self.query.unsynced_only {
                "Showing only unsynced issues".into()
            } else {
                "Showing synced and unsynced issues".into()
            };
        }
    }

    pub fn begin_create_editor(&mut self) {
        self.editor = Some(EditorState {
            mode: EditorMode::Create,
            focus: EditorFocus::Title,
            title: String::new(),
            description: String::new(),
            project: String::new(),
            labels: String::new(),
            assignee: String::new(),
            status: IssueStatus::Todo,
            priority: Priority::Medium,
            search: String::new(),
            scratch_source: ScratchSource::Manual,
        });
        self.status_message =
            "Creating a local issue. Tab moves fields, Ctrl+S/Ctrl+P cycle status and priority."
                .into();
    }

    pub fn begin_edit_editor(&mut self) {
        let Some(issue) = self.current_issue().cloned() else {
            self.status_message = "No issue selected to edit".into();
            return;
        };
        self.editor = Some(EditorState {
            mode: EditorMode::Edit {
                local_id: issue.local_id,
            },
            focus: EditorFocus::Title,
            title: issue.title.clone(),
            description: issue.description.clone(),
            project: issue.project.clone().unwrap_or_default(),
            labels: issue.labels.join(", "),
            assignee: issue.assignee.clone().unwrap_or_default(),
            status: issue.status.clone(),
            priority: issue.priority.clone(),
            search: self.query.search.clone().unwrap_or_default(),
            scratch_source: ScratchSource::Manual,
        });
        self.status_message = format!("Editing {}", issue.identifier);
    }

    pub fn begin_search_editor(&mut self) {
        self.editor = Some(EditorState {
            mode: EditorMode::Search,
            focus: EditorFocus::Title,
            title: String::new(),
            description: String::new(),
            project: String::new(),
            labels: String::new(),
            assignee: String::new(),
            status: IssueStatus::Todo,
            priority: Priority::Medium,
            search: self.query.search.clone().unwrap_or_default(),
            scratch_source: ScratchSource::Manual,
        });
        self.status_message =
            "Search issues by title, identifier, description, project, or labels".into();
    }

    pub fn begin_scratch_editor(&mut self) {
        self.editor = Some(EditorState {
            mode: EditorMode::ScratchCapture,
            focus: EditorFocus::Title,
            title: String::new(),
            description: String::new(),
            project: String::new(),
            labels: String::new(),
            assignee: String::new(),
            status: IssueStatus::Todo,
            priority: Priority::Medium,
            search: String::new(),
            scratch_source: ScratchSource::Manual,
        });
        self.status_message =
            "Capturing scratch work. Use the title field for a quick note and Ctrl+O to cycle the source."
                .into();
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

    pub fn cycle_priority(&mut self) -> Result<()> {
        let Some(issue) = self.current_issue().cloned() else {
            return Ok(());
        };
        let mut patch = IssuePatch::empty();
        patch.priority = Some(issue.priority.cycle());
        let updated = self.store.update_issue(issue.local_id, &patch)?;
        self.reload()?;
        self.select_issue(updated.local_id);
        self.status_message = format!(
            "Updated {} priority to {}",
            updated.identifier,
            updated.priority.label()
        );
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

    pub fn toggle_archive_current_issue(&mut self) -> Result<()> {
        if self.saved_view == SavedView::Scratch {
            self.status_message = "Scratch items are promoted, not archived".into();
            return Ok(());
        }
        let Some(issue) = self.current_issue().cloned() else {
            return Ok(());
        };
        let updated = self
            .store
            .archive_issue(issue.local_id, !issue.is_archived)?;
        self.reload()?;
        self.status_message = if updated.is_archived {
            format!("Archived {}", updated.identifier)
        } else {
            format!("Restored {}", updated.identifier)
        };
        Ok(())
    }

    pub fn send_current_issue_to_agent(&mut self) -> Result<()> {
        self.apply_handoff_transition(
            IssueStatus::ReadyForAgent,
            OwnerType::Agent,
            Some("agent".into()),
            Some("ready for agent pickup".into()),
            None,
            "Sent issue to agent",
        )
    }

    pub fn request_human_input(&mut self) -> Result<()> {
        self.apply_handoff_transition(
            IssueStatus::NeedsHumanInput,
            OwnerType::Human,
            Some("human".into()),
            Some("human decision needed".into()),
            None,
            "Marked issue as needing human input",
        )
    }

    pub fn request_review(&mut self) -> Result<()> {
        self.apply_handoff_transition(
            IssueStatus::NeedsReview,
            OwnerType::Human,
            Some("reviewer".into()),
            Some("review requested".into()),
            None,
            "Marked issue as needing review",
        )
    }

    pub fn mark_current_issue_blocked(&mut self) -> Result<()> {
        self.apply_handoff_transition(
            IssueStatus::Blocked,
            OwnerType::Unassigned,
            None,
            Some("blocked and waiting".into()),
            Some("awaiting follow-up".into()),
            "Marked issue as blocked",
        )
    }

    pub fn toggle_archived_visibility(&mut self) {
        self.query.include_archived = !self.query.include_archived;
        if !self.query.include_archived {
            self.query.archived_only = false;
        }
        if let Err(error) = self.reload() {
            self.status_message = format!("Failed to reload issues: {error:#}");
        } else if self.query.include_archived {
            self.status_message = "Showing archived issues".into();
        } else {
            self.status_message = "Hiding archived issues".into();
        }
    }

    pub fn clear_search(&mut self) {
        self.query.search = None;
        if let Err(error) = self.reload() {
            self.status_message = format!("Failed to clear search: {error:#}");
        } else {
            self.status_message = "Cleared search filter".into();
        }
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
        self.status_message = if self.show_help {
            "Help overlay open. Press ? or Esc to close it.".into()
        } else {
            "Help overlay closed".into()
        };
    }

    pub fn query_summary(&self) -> String {
        let search = self.query.search.as_deref().unwrap_or("none");
        format!(
            "view: {} | archived: {} | search: {}",
            self.saved_view.label(),
            if self.query.archived_only {
                "only"
            } else if self.query.include_archived {
                "shown"
            } else {
                "hidden"
            },
            search
        )
    }

    fn reload(&mut self) -> Result<()> {
        let issues = self.store.list_issues(&self.query)?;
        self.issues = self.filter_issues_for_view(issues);
        self.scratch_items = self.store.list_scratch_items()?;
        self.queued_mutation_count = self.store.list_pending_mutations()?.len();
        let len = self.visible_len();
        if len == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(len - 1);
        }
        Ok(())
    }

    fn handle_editor_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.editor = None;
                self.status_message = "Cancelled input".into();
                return Ok(false);
            }
            KeyCode::Enter => {
                self.submit_editor()?;
                return Ok(false);
            }
            KeyCode::Tab => {
                if let Some(editor) = self.editor.as_mut() {
                    if !matches!(editor.mode, EditorMode::Search) {
                        editor.focus = editor.focus.next();
                    }
                }
            }
            KeyCode::BackTab => {
                if let Some(editor) = self.editor.as_mut() {
                    if !matches!(editor.mode, EditorMode::Search) {
                        editor.focus = editor.focus.previous();
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(editor) = self.editor.as_mut() {
                    match editor.mode {
                        EditorMode::Search => {
                            editor.search.pop();
                        }
                        _ => match editor.focus {
                            EditorFocus::Title => {
                                editor.title.pop();
                            }
                            EditorFocus::Description => {
                                editor.description.pop();
                            }
                            EditorFocus::Project => {
                                editor.project.pop();
                            }
                            EditorFocus::Labels => {
                                editor.labels.pop();
                            }
                            EditorFocus::Assignee => {
                                editor.assignee.pop();
                            }
                        },
                    }
                }
            }
            KeyCode::Char('s') if key.modifiers == KeyModifiers::CONTROL => {
                if let Some(editor) = self.editor.as_mut() {
                    if !matches!(editor.mode, EditorMode::Search) {
                        editor.status = editor.status.cycle();
                    }
                }
            }
            KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                if let Some(editor) = self.editor.as_mut() {
                    if !matches!(editor.mode, EditorMode::Search) {
                        editor.priority = editor.priority.cycle();
                    }
                }
            }
            KeyCode::Char('o') if key.modifiers == KeyModifiers::CONTROL => {
                if let Some(editor) = self.editor.as_mut() {
                    if matches!(editor.mode, EditorMode::ScratchCapture) {
                        editor.scratch_source = match editor.scratch_source {
                            ScratchSource::Manual => ScratchSource::Agent,
                            ScratchSource::Agent => ScratchSource::RunFailure,
                            ScratchSource::RunFailure => ScratchSource::Pasted,
                            ScratchSource::Pasted => ScratchSource::Manual,
                        };
                    }
                }
            }
            KeyCode::Char(ch)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                if let Some(editor) = self.editor.as_mut() {
                    match editor.mode {
                        EditorMode::Search => editor.search.push(ch),
                        _ => match editor.focus {
                            EditorFocus::Title => editor.title.push(ch),
                            EditorFocus::Description => editor.description.push(ch),
                            EditorFocus::Project => editor.project.push(ch),
                            EditorFocus::Labels => editor.labels.push(ch),
                            EditorFocus::Assignee => editor.assignee.push(ch),
                        },
                    }
                }
            }
            _ => {}
        }

        Ok(false)
    }

    fn submit_editor(&mut self) -> Result<()> {
        let Some(editor) = self.editor.clone() else {
            return Ok(());
        };

        match editor.mode {
            EditorMode::Search => {
                let search = editor.search.trim().to_string();
                self.query.search = if search.is_empty() {
                    None
                } else {
                    Some(search)
                };
                self.reload()?;
                self.status_message = if self.query.search.is_some() {
                    "Applied search filter".into()
                } else {
                    "Cleared search filter".into()
                };
            }
            EditorMode::Create => {
                let title = if editor.title.trim().is_empty() {
                    format!("Local draft issue #{}", self.queued_mutation_count + 1)
                } else {
                    editor.title.trim().to_string()
                };
                let description = if editor.description.trim().is_empty() {
                    "Local issue created from the TUI.".to_string()
                } else {
                    editor.description.trim().to_string()
                };
                let mut draft = IssueDraft::new(title, description);
                draft.status = editor.status;
                draft.priority = editor.priority;
                draft.project = empty_to_none(&editor.project);
                draft.labels = parse_labels(&editor.labels);
                draft.assignee = empty_to_none(&editor.assignee);
                let issue = self.store.create_issue(&draft)?;
                self.reload()?;
                self.select_issue(issue.local_id);
                self.status_message = format!("Created {}", issue.identifier);
            }
            EditorMode::Edit { local_id } => {
                let Some(existing) = self.store.get_issue(local_id)? else {
                    self.editor = None;
                    self.status_message = "Selected issue no longer exists".into();
                    return Ok(());
                };
                let mut patch = IssuePatch::empty();
                patch.title = Some(if editor.title.trim().is_empty() {
                    existing.title
                } else {
                    editor.title.trim().to_string()
                });
                patch.description = Some(editor.description.trim().to_string());
                patch.project = Some(empty_to_none(&editor.project));
                patch.labels = Some(parse_labels(&editor.labels));
                patch.assignee = Some(empty_to_none(&editor.assignee));
                patch.status = Some(editor.status);
                patch.priority = Some(editor.priority);
                let issue = self.store.update_issue(local_id, &patch)?;
                self.reload()?;
                self.select_issue(issue.local_id);
                self.status_message = format!("Saved local edits for {}", issue.identifier);
            }
            EditorMode::ScratchCapture => {
                let body = if editor.title.trim().is_empty() {
                    "Scratch note".to_string()
                } else {
                    editor.title.trim().to_string()
                };
                let scratch = self
                    .store
                    .create_scratch_item(body, editor.scratch_source.clone())?;
                self.reload()?;
                self.saved_view = SavedView::Scratch;
                self.select_scratch(scratch.id);
                self.status_message = format!("Captured scratch item #{}", scratch.id);
            }
        }

        self.editor = None;
        Ok(())
    }

    fn set_saved_view(&mut self, view: SavedView) {
        self.saved_view = view;
        self.query.unsynced_only = false;
        self.query.include_archived = false;
        self.query.archived_only = false;
        if matches!(view, SavedView::Scratch) {
            self.status_message = "Switched to scratch inbox".into();
        } else if matches!(view, SavedView::Done) {
            self.status_message = "Switched to done issues".into();
        } else {
            self.status_message = format!("Switched to {}", view.label());
        }
        if let Err(error) = self.reload() {
            self.status_message = format!("Failed to switch view: {error:#}");
        }
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

    fn select_scratch(&mut self, scratch_id: i64) {
        if let Some(index) = self
            .scratch_items
            .iter()
            .position(|scratch| scratch.id == scratch_id)
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
            "queue: {} | pending: {} | errors: {} | scratch: {}",
            self.queued_mutation_count,
            pending,
            errors,
            self.scratch_items
                .iter()
                .filter(|item| item.promoted_issue_id.is_none())
                .count()
        )
    }

    pub fn is_scratch_view(&self) -> bool {
        self.saved_view == SavedView::Scratch
    }

    pub fn list_title(&self) -> &'static str {
        if self.is_scratch_view() {
            "Scratch"
        } else {
            "Inbox"
        }
    }

    fn visible_len(&self) -> usize {
        if self.saved_view == SavedView::Scratch {
            self.scratch_items.len()
        } else {
            self.issues.len()
        }
    }

    fn filter_issues_for_view(&self, issues: Vec<Issue>) -> Vec<Issue> {
        issues
            .into_iter()
            .filter(|issue| match self.saved_view {
                SavedView::Inbox => issue.status.is_inbox_relevant(),
                SavedView::Running => issue.status == IssueStatus::AgentRunning,
                SavedView::Review => issue.status == IssueStatus::NeedsReview,
                SavedView::Waiting => {
                    matches!(
                        issue.status,
                        IssueStatus::Blocked | IssueStatus::NeedsHumanInput
                    )
                }
                SavedView::Done => issue.status == IssueStatus::Done,
                SavedView::Scratch => false,
            })
            .collect()
    }

    fn promote_selected_scratch(&mut self) -> Result<()> {
        let Some(scratch) = self.current_scratch().cloned() else {
            self.status_message = "No scratch item selected to promote".into();
            return Ok(());
        };
        let issue = self.store.promote_scratch_to_issue(scratch.id)?;
        self.saved_view = SavedView::Inbox;
        self.reload()?;
        self.select_issue(issue.local_id);
        self.status_message = format!("Promoted scratch #{} into {}", scratch.id, issue.identifier);
        Ok(())
    }

    fn apply_handoff_transition(
        &mut self,
        status: IssueStatus,
        owner_type: OwnerType,
        owner_name: Option<String>,
        attention_reason: Option<String>,
        blocked_reason: Option<String>,
        message: &str,
    ) -> Result<()> {
        let Some(issue) = self.current_issue().cloned() else {
            return Ok(());
        };
        let mut patch = IssuePatch::empty();
        patch.status = Some(status);
        patch.owner_type = Some(owner_type);
        patch.owner_name = Some(owner_name);
        patch.attention_reason = Some(attention_reason);
        patch.blocked_reason = Some(blocked_reason);
        let updated = self.store.update_issue(issue.local_id, &patch)?;
        self.reload()?;
        self.select_issue(updated.local_id);
        self.status_message = format!("{message}: {}", updated.identifier);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavedView {
    Inbox,
    Running,
    Review,
    Waiting,
    Done,
    Scratch,
}

impl SavedView {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Running => "running",
            Self::Review => "review",
            Self::Waiting => "waiting",
            Self::Done => "done",
            Self::Scratch => "scratch",
        }
    }
}

fn empty_to_none(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn parse_labels(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::WorkspaceConfig, store::Store};
    use std::path::PathBuf;

    fn test_app() -> Result<App> {
        let config = WorkspaceConfig {
            data_dir: PathBuf::from("/tmp/logit-test"),
            database_path: PathBuf::from("/tmp/logit-test/logit.db"),
            linear_api_token: None,
            workspace_name: "Test Workspace".into(),
            theme: None,
        };
        let store = Store::open_in_memory()?;
        let sync_service = LinearSyncService::new(config.clone());
        Ok(App {
            config,
            issues: Vec::new(),
            scratch_items: Vec::new(),
            selected: 0,
            query: IssueQuery::default(),
            saved_view: SavedView::Inbox,
            status_message: String::new(),
            queued_mutation_count: 0,
            editor: None,
            show_help: false,
            store,
            sync_service,
        })
    }

    #[test]
    fn typing_s_in_editor_inserts_text() -> Result<()> {
        let mut app = test_app()?;
        app.begin_create_editor();

        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))?;

        let editor = app.editor.expect("editor should still be open");
        assert_eq!(editor.title, "s");
        Ok(())
    }

    #[test]
    fn ctrl_s_cycles_status_in_editor() -> Result<()> {
        let mut app = test_app()?;
        app.begin_create_editor();

        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))?;

        let editor = app.editor.expect("editor should still be open");
        assert_eq!(editor.status, crate::domain::IssueStatus::ReadyForAgent);
        Ok(())
    }

    #[test]
    fn scratch_capture_creates_scratch_item() -> Result<()> {
        let mut app = test_app()?;
        app.begin_scratch_editor();

        for ch in "follow up with support".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;

        assert_eq!(app.saved_view, SavedView::Scratch);
        assert_eq!(app.scratch_items.len(), 1);
        assert_eq!(app.scratch_items[0].body, "follow up with support");
        Ok(())
    }

    #[test]
    fn inbox_views_filter_by_terminal_state() -> Result<()> {
        let mut app = test_app()?;
        let mut running = IssueDraft::new("Agent run", "Agent is working");
        running.status = IssueStatus::AgentRunning;
        let mut review = IssueDraft::new("Needs review", "Waiting on approval");
        review.status = IssueStatus::NeedsReview;
        let mut done = IssueDraft::new("Closed loop", "Already finished");
        done.status = IssueStatus::Done;

        app.store.create_issue(&running)?;
        app.store.create_issue(&review)?;
        app.store.create_issue(&done)?;

        app.set_saved_view(SavedView::Running);
        assert_eq!(app.issues.len(), 1);
        assert_eq!(app.issues[0].status, IssueStatus::AgentRunning);

        app.set_saved_view(SavedView::Review);
        assert_eq!(app.issues.len(), 1);
        assert_eq!(app.issues[0].status, IssueStatus::NeedsReview);

        app.set_saved_view(SavedView::Done);
        assert_eq!(app.issues.len(), 1);
        assert_eq!(app.issues[0].status, IssueStatus::Done);
        Ok(())
    }

    #[test]
    fn handoff_actions_persist_owner_and_attention() -> Result<()> {
        let mut app = test_app()?;
        let issue = app
            .store
            .create_issue(&IssueDraft::new("Handoff", "Track next actor"))?;
        app.reload()?;
        app.select_issue(issue.local_id);

        app.send_current_issue_to_agent()?;
        let issue = app.store.get_issue(issue.local_id)?.expect("issue missing");
        assert_eq!(issue.status, IssueStatus::ReadyForAgent);
        assert_eq!(issue.owner_type, OwnerType::Agent);
        assert_eq!(
            issue.attention_reason.as_deref(),
            Some("ready for agent pickup")
        );

        app.request_review()?;
        let issue = app.store.get_issue(issue.local_id)?.expect("issue missing");
        assert_eq!(issue.status, IssueStatus::NeedsReview);
        assert_eq!(issue.owner_type, OwnerType::Human);
        assert_eq!(issue.owner_name.as_deref(), Some("reviewer"));
        Ok(())
    }
}
