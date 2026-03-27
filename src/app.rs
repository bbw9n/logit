use crate::{
    config::WorkspaceConfig,
    domain::{Issue, IssueDraft, IssuePatch, IssueQuery, Priority, SyncState},
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
    pub status: crate::domain::IssueStatus,
    pub priority: Priority,
    pub search: String,
}

pub struct App {
    pub config: WorkspaceConfig,
    pub issues: Vec<Issue>,
    pub selected: usize,
    pub query: IssueQuery,
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
            selected: 0,
            query: IssueQuery::default(),
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
        self.issues.get(self.selected)
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
            KeyCode::Char('s') => self.cycle_status()?,
            KeyCode::Char('p') => self.cycle_priority()?,
            KeyCode::Char('a') => self.toggle_archive_current_issue()?,
            KeyCode::Char('v') => self.toggle_archived_visibility(),
            KeyCode::Char('1') => self.set_saved_view(SavedView::Active),
            KeyCode::Char('2') => self.set_saved_view(SavedView::Unsynced),
            KeyCode::Char('3') => self.set_saved_view(SavedView::Archived),
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
            status: crate::domain::IssueStatus::Todo,
            priority: Priority::Medium,
            search: String::new(),
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
            status: crate::domain::IssueStatus::Todo,
            priority: Priority::Medium,
            search: self.query.search.clone().unwrap_or_default(),
        });
        self.status_message =
            "Search issues by title, identifier, description, project, or labels".into();
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
            self.saved_view_label(),
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
        self.issues = self.store.list_issues(&self.query)?;
        self.queued_mutation_count = self.store.list_pending_mutations()?.len();
        if self.issues.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.issues.len() - 1);
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
        }

        self.editor = None;
        Ok(())
    }

    fn set_saved_view(&mut self, view: SavedView) {
        match view {
            SavedView::Active => {
                self.query.unsynced_only = false;
                self.query.include_archived = false;
                self.query.archived_only = false;
                self.status_message = "Switched to active issues".into();
            }
            SavedView::Unsynced => {
                self.query.unsynced_only = true;
                self.query.include_archived = false;
                self.query.archived_only = false;
                self.status_message = "Switched to unsynced issues".into();
            }
            SavedView::Archived => {
                self.query.unsynced_only = false;
                self.query.include_archived = true;
                self.query.archived_only = true;
                self.status_message = "Switched to archived issues".into();
            }
        }
        if let Err(error) = self.reload() {
            self.status_message = format!("Failed to switch view: {error:#}");
        }
    }

    fn saved_view_label(&self) -> &'static str {
        if self.query.archived_only {
            "archived"
        } else if self.query.unsynced_only {
            "unsynced"
        } else {
            "active"
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

#[derive(Debug, Clone, Copy)]
enum SavedView {
    Active,
    Unsynced,
    Archived,
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
            selected: 0,
            query: IssueQuery::default(),
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
        assert_eq!(editor.status, crate::domain::IssueStatus::InProgress);
        Ok(())
    }
}
