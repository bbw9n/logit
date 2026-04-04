use crate::{
    config::WorkspaceConfig,
    domain::{
        AgentRequest, AgentRequestKind, AgentRequestStatus, ArtifactKind, ArtifactRecord,
        HandoffRecord, Issue, IssueDraft, IssuePatch, IssueQuery, IssueStatus, OwnerType, Priority,
        RunEventLevel, RunEventRecord, RunKind, RunRecord, RunStatus, ScratchItem, ScratchSource,
        SessionKind, SessionLink, SyncState, WorkContext,
    },
    store::Store,
    sync::{LinearSyncService, SyncService},
};
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{collections::HashMap, env, path::PathBuf, process::Command};

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
    Edit {
        local_id: i64,
    },
    DispatchSubissue {
        parent_local_id: i64,
    },
    AgentRequest {
        issue_local_id: i64,
    },
    Search,
    ScratchCapture,
    RunNote {
        run_id: i64,
    },
    Closeout {
        local_id: i64,
    },
    ArtifactNote {
        issue_local_id: i64,
        run_id: Option<i64>,
    },
    WorkContext {
        issue_local_id: i64,
    },
    SessionLink {
        issue_local_id: i64,
    },
}

#[derive(Debug, Clone)]
pub struct EditorState {
    pub mode: EditorMode,
    pub focus: EditorFocus,
    pub title: String,
    pub title_cursor: usize,
    pub description: String,
    pub description_cursor: usize,
    pub project: String,
    pub project_cursor: usize,
    pub labels: String,
    pub labels_cursor: usize,
    pub assignee: String,
    pub assignee_cursor: usize,
    pub status: IssueStatus,
    pub priority: Priority,
    pub search: String,
    pub search_cursor: usize,
    pub scratch_source: ScratchSource,
    pub follow_up_needed: bool,
}

impl EditorState {
    fn insert_char(&mut self, ch: char) {
        match self.mode {
            EditorMode::Search => insert_char_at(&mut self.search, &mut self.search_cursor, ch),
            _ => match self.focus {
                EditorFocus::Title => insert_char_at(&mut self.title, &mut self.title_cursor, ch),
                EditorFocus::Description => {
                    insert_char_at(&mut self.description, &mut self.description_cursor, ch)
                }
                EditorFocus::Project => {
                    insert_char_at(&mut self.project, &mut self.project_cursor, ch)
                }
                EditorFocus::Labels => {
                    insert_char_at(&mut self.labels, &mut self.labels_cursor, ch)
                }
                EditorFocus::Assignee => {
                    insert_char_at(&mut self.assignee, &mut self.assignee_cursor, ch)
                }
            },
        }
    }

    fn backspace(&mut self) {
        match self.mode {
            EditorMode::Search => backspace_at(&mut self.search, &mut self.search_cursor),
            _ => match self.focus {
                EditorFocus::Title => backspace_at(&mut self.title, &mut self.title_cursor),
                EditorFocus::Description => {
                    backspace_at(&mut self.description, &mut self.description_cursor)
                }
                EditorFocus::Project => backspace_at(&mut self.project, &mut self.project_cursor),
                EditorFocus::Labels => backspace_at(&mut self.labels, &mut self.labels_cursor),
                EditorFocus::Assignee => {
                    backspace_at(&mut self.assignee, &mut self.assignee_cursor)
                }
            },
        }
    }

    fn move_left(&mut self) {
        match self.mode {
            EditorMode::Search => self.search_cursor = self.search_cursor.saturating_sub(1),
            _ => match self.focus {
                EditorFocus::Title => self.title_cursor = self.title_cursor.saturating_sub(1),
                EditorFocus::Description => {
                    self.description_cursor = self.description_cursor.saturating_sub(1)
                }
                EditorFocus::Project => self.project_cursor = self.project_cursor.saturating_sub(1),
                EditorFocus::Labels => self.labels_cursor = self.labels_cursor.saturating_sub(1),
                EditorFocus::Assignee => {
                    self.assignee_cursor = self.assignee_cursor.saturating_sub(1)
                }
            },
        }
    }

    fn move_right(&mut self) {
        match self.mode {
            EditorMode::Search => {
                self.search_cursor = move_right_cursor(&self.search, self.search_cursor)
            }
            _ => match self.focus {
                EditorFocus::Title => {
                    self.title_cursor = move_right_cursor(&self.title, self.title_cursor)
                }
                EditorFocus::Description => {
                    self.description_cursor =
                        move_right_cursor(&self.description, self.description_cursor)
                }
                EditorFocus::Project => {
                    self.project_cursor = move_right_cursor(&self.project, self.project_cursor)
                }
                EditorFocus::Labels => {
                    self.labels_cursor = move_right_cursor(&self.labels, self.labels_cursor)
                }
                EditorFocus::Assignee => {
                    self.assignee_cursor = move_right_cursor(&self.assignee, self.assignee_cursor)
                }
            },
        }
    }

    fn move_home(&mut self) {
        match self.mode {
            EditorMode::Search => self.search_cursor = 0,
            _ => match self.focus {
                EditorFocus::Title => self.title_cursor = 0,
                EditorFocus::Description => self.description_cursor = 0,
                EditorFocus::Project => self.project_cursor = 0,
                EditorFocus::Labels => self.labels_cursor = 0,
                EditorFocus::Assignee => self.assignee_cursor = 0,
            },
        }
    }

    fn move_end(&mut self) {
        match self.mode {
            EditorMode::Search => self.search_cursor = self.search.chars().count(),
            _ => match self.focus {
                EditorFocus::Title => self.title_cursor = self.title.chars().count(),
                EditorFocus::Description => {
                    self.description_cursor = self.description.chars().count()
                }
                EditorFocus::Project => self.project_cursor = self.project.chars().count(),
                EditorFocus::Labels => self.labels_cursor = self.labels.chars().count(),
                EditorFocus::Assignee => self.assignee_cursor = self.assignee.chars().count(),
            },
        }
    }

    fn insert_newline(&mut self) {
        if !matches!(self.mode, EditorMode::Search) {
            self.insert_char('\n');
        }
    }
}

pub struct App {
    pub config: WorkspaceConfig,
    pub issues: Vec<Issue>,
    pub scratch_items: Vec<ScratchItem>,
    pub runs: Vec<RunRecord>,
    pub run_events: Vec<RunEventRecord>,
    pub artifacts: Vec<ArtifactRecord>,
    pub handoffs: Vec<HandoffRecord>,
    pub subissues: Vec<Issue>,
    pub agent_requests: Vec<AgentRequest>,
    pub interruptions: Vec<InterruptionItem>,
    pub active_work_context: Option<WorkContext>,
    pub active_session_link: Option<SessionLink>,
    pub agent_roster: Vec<AgentRosterEntry>,
    pub dispatch_summaries: HashMap<i64, DispatchSummary>,
    pub needs_human_count: usize,
    pub stale_agent_count: usize,
    pub open_agent_request_count: usize,
    pub escalated_interruptions_count: usize,
    pub snoozed_interruptions_count: usize,
    pub due_soon_interruptions_count: usize,
    pub next_interruption_due_label: Option<String>,
    pub parallel_parent_count: usize,
    pub parallel_subissue_count: usize,
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

#[derive(Debug, Clone)]
struct GitContextPrefill {
    repo_path: String,
    worktree_path: Option<String>,
    branch_name: Option<String>,
    git_status_summary: Option<String>,
    dirty_file_count: i64,
    staged_file_count: i64,
    ahead_count: i64,
    behind_count: i64,
}

#[derive(Debug, Clone)]
pub struct AgentRosterEntry {
    pub issue_local_id: i64,
    pub identifier: String,
    pub session_label: String,
    pub session_kind: SessionKind,
    pub branch_name: Option<String>,
    pub last_activity_at: DateTime<Utc>,
    pub is_stale: bool,
}

#[derive(Debug, Clone)]
pub struct InterruptionItem {
    pub request: AgentRequest,
    pub issue: Issue,
}

#[derive(Debug, Clone, Default)]
struct InterruptionOverview {
    escalated_count: usize,
    snoozed_count: usize,
    due_soon_count: usize,
    next_due_label: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DispatchSummary {
    pub total_children: usize,
    pub todo_count: usize,
    pub ready_count: usize,
    pub running_count: usize,
    pub waiting_count: usize,
    pub review_count: usize,
    pub done_count: usize,
    pub archived_count: usize,
    pub open_request_count: usize,
}

impl DispatchSummary {
    fn new() -> Self {
        Self::default()
    }

    fn record_issue(&mut self, issue: &Issue, open_requests: usize) {
        self.total_children += 1;
        self.open_request_count += open_requests;
        if issue.is_archived {
            self.archived_count += 1;
        }
        match issue.status {
            IssueStatus::Todo => self.todo_count += 1,
            IssueStatus::ReadyForAgent => self.ready_count += 1,
            IssueStatus::AgentRunning => self.running_count += 1,
            IssueStatus::NeedsHumanInput | IssueStatus::Blocked => self.waiting_count += 1,
            IssueStatus::NeedsReview => self.review_count += 1,
            IssueStatus::Done => self.done_count += 1,
        }
    }

    pub fn compact_label(&self) -> String {
        format!(
            "{} child{} | ready {} | running {} | review {} | waiting {} | done {}{}",
            self.total_children,
            if self.total_children == 1 { "" } else { "ren" },
            self.ready_count,
            self.running_count,
            self.review_count,
            self.waiting_count,
            self.done_count,
            if self.open_request_count > 0 {
                format!(" | req {}", self.open_request_count)
            } else {
                String::new()
            }
        )
    }
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
            runs: Vec::new(),
            run_events: Vec::new(),
            artifacts: Vec::new(),
            handoffs: Vec::new(),
            subissues: Vec::new(),
            agent_requests: Vec::new(),
            interruptions: Vec::new(),
            active_work_context: None,
            active_session_link: None,
            agent_roster: Vec::new(),
            dispatch_summaries: HashMap::new(),
            needs_human_count: 0,
            stale_agent_count: 0,
            open_agent_request_count: 0,
            escalated_interruptions_count: 0,
            snoozed_interruptions_count: 0,
            due_soon_interruptions_count: 0,
            next_interruption_due_label: None,
            parallel_parent_count: 0,
            parallel_subissue_count: 0,
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
        if matches!(
            self.saved_view,
            SavedView::Scratch | SavedView::Interruptions
        ) {
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

    pub fn current_interruption(&self) -> Option<&InterruptionItem> {
        if self.saved_view != SavedView::Interruptions {
            return None;
        }
        self.interruptions.get(self.selected)
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
            KeyCode::Char('D') => self.begin_dispatch_editor(),
            KeyCode::Char('R') => self.begin_agent_request_editor(),
            KeyCode::Char('Q') => self.resolve_latest_agent_request()?,
            KeyCode::Char('P') => self.jump_to_parent_issue()?,
            KeyCode::Char('C') => self.jump_to_next_graph_issue()?,
            KeyCode::Char('V') => self.approve_review_children()?,
            KeyCode::Char('J') => self.requeue_stalled_children()?,
            KeyCode::Char('A') => self.acknowledge_interruption_and_requeue()?,
            KeyCode::Char('E') => self.resolve_graph_interruptions()?,
            KeyCode::Char('S') => self.snooze_selected_interruption()?,
            KeyCode::Char('X') => self.escalate_selected_interruption()?,
            KeyCode::Char('H') => self.snooze_graph_review_interruptions()?,
            KeyCode::Char('B') => self.escalate_graph_blockers()?,
            KeyCode::Char('x') => self.begin_scratch_editor(),
            KeyCode::Char('i') => self.promote_selected_scratch()?,
            KeyCode::Char('s') => self.cycle_status()?,
            KeyCode::Char('p') => self.cycle_priority()?,
            KeyCode::Char('a') => self.toggle_archive_current_issue()?,
            KeyCode::Char('h') => self.send_current_issue_to_agent()?,
            KeyCode::Char('m') => self.request_human_input()?,
            KeyCode::Char('w') => self.request_review()?,
            KeyCode::Char('b') => self.mark_current_issue_blocked()?,
            KeyCode::Char('t') => self.start_run()?,
            KeyCode::Char('g') => self.complete_latest_run_success()?,
            KeyCode::Char('z') => self.complete_latest_run_failure()?,
            KeyCode::Char('l') => self.begin_run_note_editor(),
            KeyCode::Char('o') => self.begin_artifact_editor(),
            KeyCode::Char('c') => self.begin_closeout_editor(),
            KeyCode::Char('O') => self.reopen_current_issue()?,
            KeyCode::Char(']') => self.begin_work_context_editor(),
            KeyCode::Char('[') => self.begin_session_link_editor(),
            KeyCode::Char('}') => self.clear_work_context()?,
            KeyCode::Char('{') => self.clear_session_link()?,
            KeyCode::Char('v') => self.toggle_archived_visibility(),
            KeyCode::Char('1') => self.set_saved_view(SavedView::Inbox),
            KeyCode::Char('2') => self.set_saved_view(SavedView::Running),
            KeyCode::Char('3') => self.set_saved_view(SavedView::Review),
            KeyCode::Char('4') => self.set_saved_view(SavedView::Waiting),
            KeyCode::Char('5') => self.set_saved_view(SavedView::Done),
            KeyCode::Char('6') => self.set_saved_view(SavedView::Scratch),
            KeyCode::Char('7') => self.set_saved_view(SavedView::Interruptions),
            KeyCode::Char('8') => self.set_saved_view(SavedView::DispatchBoard),
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
            title_cursor: 0,
            description: String::new(),
            description_cursor: 0,
            project: String::new(),
            project_cursor: 0,
            labels: String::new(),
            labels_cursor: 0,
            assignee: String::new(),
            assignee_cursor: 0,
            status: IssueStatus::Todo,
            priority: Priority::Medium,
            search: String::new(),
            search_cursor: 0,
            scratch_source: ScratchSource::Manual,
            follow_up_needed: false,
        });
        self.status_message =
            "Creating a local issue. Tab moves fields, arrows move the cursor, Ctrl+J inserts a newline."
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
            title_cursor: issue.title.chars().count(),
            description: issue.description.clone(),
            description_cursor: issue.description.chars().count(),
            project: issue.project.clone().unwrap_or_default(),
            project_cursor: issue.project.as_deref().unwrap_or_default().chars().count(),
            labels: issue.labels.join(", "),
            labels_cursor: issue.labels.join(", ").chars().count(),
            assignee: issue.assignee.clone().unwrap_or_default(),
            assignee_cursor: issue
                .assignee
                .as_deref()
                .unwrap_or_default()
                .chars()
                .count(),
            status: issue.status.clone(),
            priority: issue.priority.clone(),
            search: self.query.search.clone().unwrap_or_default(),
            search_cursor: self
                .query
                .search
                .as_deref()
                .unwrap_or_default()
                .chars()
                .count(),
            scratch_source: ScratchSource::Manual,
            follow_up_needed: issue.follow_up_needed,
        });
        self.status_message = format!("Editing {}", issue.identifier);
    }

    pub fn begin_dispatch_editor(&mut self) {
        let Some(issue) = self.current_issue().cloned() else {
            self.status_message = "No issue selected to dispatch".into();
            return;
        };
        self.editor = Some(EditorState {
            mode: EditorMode::DispatchSubissue {
                parent_local_id: issue.local_id,
            },
            focus: EditorFocus::Title,
            title: format!("{} / subtask", issue.title),
            title_cursor: format!("{} / subtask", issue.title).chars().count(),
            description: String::new(),
            description_cursor: 0,
            project: "ready_for_agent".into(),
            project_cursor: "ready_for_agent".chars().count(),
            labels: String::new(),
            labels_cursor: 0,
            assignee: String::new(),
            assignee_cursor: 0,
            status: IssueStatus::ReadyForAgent,
            priority: issue.priority,
            search: String::new(),
            search_cursor: 0,
            scratch_source: ScratchSource::Manual,
            follow_up_needed: false,
        });
        self.status_message =
            "Dispatch a sub-issue. Fields are title, description, initial status.".into();
    }

    pub fn begin_agent_request_editor(&mut self) {
        let Some(issue) = self.current_issue().cloned() else {
            self.status_message = "No issue selected for agent request".into();
            return;
        };
        self.editor = Some(EditorState {
            mode: EditorMode::AgentRequest {
                issue_local_id: issue.local_id,
            },
            focus: EditorFocus::Title,
            title: "Need human decision".into(),
            title_cursor: "Need human decision".chars().count(),
            description: String::new(),
            description_cursor: 0,
            project: "question".into(),
            project_cursor: "question".chars().count(),
            labels: String::new(),
            labels_cursor: 0,
            assignee: String::new(),
            assignee_cursor: 0,
            status: IssueStatus::NeedsHumanInput,
            priority: issue.priority,
            search: String::new(),
            search_cursor: 0,
            scratch_source: ScratchSource::Manual,
            follow_up_needed: false,
        });
        self.status_message =
            "Create a structured agent request. Fields are title, body, kind.".into();
    }

    pub fn begin_search_editor(&mut self) {
        self.editor = Some(EditorState {
            mode: EditorMode::Search,
            focus: EditorFocus::Title,
            title: String::new(),
            title_cursor: 0,
            description: String::new(),
            description_cursor: 0,
            project: String::new(),
            project_cursor: 0,
            labels: String::new(),
            labels_cursor: 0,
            assignee: String::new(),
            assignee_cursor: 0,
            status: IssueStatus::Todo,
            priority: Priority::Medium,
            search: self.query.search.clone().unwrap_or_default(),
            search_cursor: self
                .query
                .search
                .as_deref()
                .unwrap_or_default()
                .chars()
                .count(),
            scratch_source: ScratchSource::Manual,
            follow_up_needed: false,
        });
        self.status_message =
            "Search issues by title, identifier, description, project, or labels".into();
    }

    pub fn begin_scratch_editor(&mut self) {
        self.editor = Some(EditorState {
            mode: EditorMode::ScratchCapture,
            focus: EditorFocus::Title,
            title: String::new(),
            title_cursor: 0,
            description: String::new(),
            description_cursor: 0,
            project: String::new(),
            project_cursor: 0,
            labels: String::new(),
            labels_cursor: 0,
            assignee: String::new(),
            assignee_cursor: 0,
            status: IssueStatus::Todo,
            priority: Priority::Medium,
            search: String::new(),
            search_cursor: 0,
            scratch_source: ScratchSource::Manual,
            follow_up_needed: false,
        });
        self.status_message =
            "Capturing scratch work. Use the title field for a quick note and Ctrl+O to cycle the source."
                .into();
    }

    pub fn begin_run_note_editor(&mut self) {
        let Some(issue) = self.current_issue() else {
            self.status_message = "No issue selected for a run note".into();
            return;
        };
        let Ok(Some(run)) = self.store.latest_active_run_for_issue(issue.local_id) else {
            self.status_message = "No active run available. Press t to start one first.".into();
            return;
        };
        self.editor = Some(EditorState {
            mode: EditorMode::RunNote { run_id: run.id },
            focus: EditorFocus::Title,
            title: String::new(),
            title_cursor: 0,
            description: String::new(),
            description_cursor: 0,
            project: String::new(),
            project_cursor: 0,
            labels: String::new(),
            labels_cursor: 0,
            assignee: String::new(),
            assignee_cursor: 0,
            status: IssueStatus::Todo,
            priority: Priority::Medium,
            search: String::new(),
            search_cursor: 0,
            scratch_source: ScratchSource::Manual,
            follow_up_needed: false,
        });
        self.status_message = "Write a run note, then press Enter to attach it.".into();
    }

    pub fn begin_artifact_editor(&mut self) {
        let Some(issue) = self.current_issue() else {
            self.status_message = "No issue selected for evidence".into();
            return;
        };
        let run_id = self
            .store
            .latest_active_run_for_issue(issue.local_id)
            .ok()
            .flatten()
            .map(|run| run.id);
        self.editor = Some(EditorState {
            mode: EditorMode::ArtifactNote {
                issue_local_id: issue.local_id,
                run_id,
            },
            focus: EditorFocus::Title,
            title: String::new(),
            title_cursor: 0,
            description: String::new(),
            description_cursor: 0,
            project: String::new(),
            project_cursor: 0,
            labels: String::new(),
            labels_cursor: 0,
            assignee: String::new(),
            assignee_cursor: 0,
            status: IssueStatus::Todo,
            priority: Priority::Medium,
            search: String::new(),
            search_cursor: 0,
            scratch_source: ScratchSource::Manual,
            follow_up_needed: false,
        });
        self.status_message = "Write an evidence note, then press Enter to attach it.".into();
    }

    pub fn begin_closeout_editor(&mut self) {
        let Some(issue) = self.current_issue().cloned() else {
            self.status_message = "No issue selected to close out".into();
            return;
        };
        let closeout_summary = issue.closeout_summary.clone().unwrap_or_default();
        self.editor = Some(EditorState {
            mode: EditorMode::Closeout {
                local_id: issue.local_id,
            },
            focus: EditorFocus::Title,
            title: closeout_summary.clone(),
            title_cursor: closeout_summary.chars().count(),
            description: String::new(),
            description_cursor: 0,
            project: String::new(),
            project_cursor: 0,
            labels: String::new(),
            labels_cursor: 0,
            assignee: String::new(),
            assignee_cursor: 0,
            status: IssueStatus::Done,
            priority: issue.priority,
            search: String::new(),
            search_cursor: 0,
            scratch_source: ScratchSource::Manual,
            follow_up_needed: issue.follow_up_needed,
        });
        self.status_message =
            "Write a closeout summary. Ctrl+F toggles follow-up, arrows move the cursor, Ctrl+J inserts a newline.".into();
    }

    pub fn begin_work_context_editor(&mut self) {
        let Some(issue) = self.current_issue().cloned() else {
            self.status_message = "No issue selected for work context".into();
            return;
        };
        let current = self.active_work_context.clone();
        let detected = current
            .as_ref()
            .map(|ctx| GitContextPrefill {
                repo_path: ctx.repo_path.clone(),
                worktree_path: ctx.worktree_path.clone(),
                branch_name: ctx.branch_name.clone(),
                git_status_summary: ctx.git_status_summary.clone(),
                dirty_file_count: ctx.dirty_file_count,
                staged_file_count: ctx.staged_file_count,
                ahead_count: ctx.ahead_count,
                behind_count: ctx.behind_count,
            })
            .or_else(detect_git_context_prefill);
        let repo_path = detected
            .as_ref()
            .map(|ctx| ctx.repo_path.clone())
            .unwrap_or_default();
        let worktree_path = detected
            .as_ref()
            .and_then(|ctx| ctx.worktree_path.clone())
            .unwrap_or_default();
        let branch_name = detected
            .as_ref()
            .and_then(|ctx| ctx.branch_name.clone())
            .unwrap_or_default();
        self.editor = Some(EditorState {
            mode: EditorMode::WorkContext {
                issue_local_id: issue.local_id,
            },
            focus: EditorFocus::Title,
            title: repo_path.clone(),
            title_cursor: repo_path.chars().count(),
            description: worktree_path.clone(),
            description_cursor: worktree_path.chars().count(),
            project: branch_name.clone(),
            project_cursor: branch_name.chars().count(),
            labels: String::new(),
            labels_cursor: 0,
            assignee: String::new(),
            assignee_cursor: 0,
            status: IssueStatus::Todo,
            priority: issue.priority,
            search: String::new(),
            search_cursor: 0,
            scratch_source: ScratchSource::Manual,
            follow_up_needed: false,
        });
        self.status_message =
            "Attach repo/worktree/branch context. Git values are prefilled when available.".into();
    }

    pub fn begin_session_link_editor(&mut self) {
        let Some(issue) = self.current_issue().cloned() else {
            self.status_message = "No issue selected for session link".into();
            return;
        };
        let current = self.active_session_link.clone();
        let detected = current
            .as_ref()
            .map(|link| {
                (
                    link.label.clone(),
                    link.session_kind.code().to_string(),
                    link.session_ref.clone(),
                )
            })
            .unwrap_or_else(default_session_prefill);
        self.editor = Some(EditorState {
            mode: EditorMode::SessionLink {
                issue_local_id: issue.local_id,
            },
            focus: EditorFocus::Title,
            title: detected.0.clone(),
            title_cursor: detected.0.chars().count(),
            description: detected.1.clone(),
            description_cursor: detected.1.chars().count(),
            project: detected.2.clone(),
            project_cursor: detected.2.chars().count(),
            labels: String::new(),
            labels_cursor: 0,
            assignee: String::new(),
            assignee_cursor: 0,
            status: IssueStatus::Todo,
            priority: issue.priority,
            search: String::new(),
            search_cursor: 0,
            scratch_source: ScratchSource::Manual,
            follow_up_needed: false,
        });
        self.status_message =
            "Attach session context. Local terminal defaults are prefilled when available.".into();
    }

    pub fn reopen_current_issue(&mut self) -> Result<()> {
        let Some(issue) = self.current_issue().cloned() else {
            return Ok(());
        };
        let mut patch = IssuePatch::empty();
        patch.status = Some(IssueStatus::Todo);
        patch.attention_reason = Some(Some("reopened for follow-up".into()));
        let updated = self.store.update_issue(issue.local_id, &patch)?;
        self.store.create_handoff(
            issue.local_id,
            issue
                .owner_name
                .as_deref()
                .unwrap_or(issue.owner_type.label()),
            "inbox",
            "Issue reopened after closeout",
        )?;
        self.saved_view = SavedView::Inbox;
        self.reload()?;
        self.select_issue(updated.local_id);
        self.status_message = format!("Reopened {}", updated.identifier);
        Ok(())
    }

    pub fn clear_work_context(&mut self) -> Result<()> {
        let Some(issue) = self.current_issue().cloned() else {
            return Ok(());
        };
        self.store.clear_active_work_context(issue.local_id)?;
        self.reload()?;
        self.select_issue(issue.local_id);
        self.status_message = "Cleared active work context".into();
        Ok(())
    }

    pub fn clear_session_link(&mut self) -> Result<()> {
        let Some(issue) = self.current_issue().cloned() else {
            return Ok(());
        };
        self.store.clear_active_session_link(issue.local_id)?;
        self.reload()?;
        self.select_issue(issue.local_id);
        self.status_message = "Cleared active session link".into();
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

    pub fn start_run(&mut self) -> Result<()> {
        let Some(issue) = self.current_issue().cloned() else {
            return Ok(());
        };
        self.ensure_terminal_context(issue.local_id)?;
        let session_ref = self
            .active_session_link
            .as_ref()
            .map(|session| session.session_ref.clone());
        let run = self.store.create_run(
            issue.local_id,
            if issue.owner_type == OwnerType::Agent {
                RunKind::Agent
            } else {
                RunKind::Manual
            },
            Some("Started from TUI"),
            session_ref.as_deref(),
        )?;
        if let Some(note) = build_run_context_note(
            self.active_work_context.as_ref(),
            self.active_session_link.as_ref(),
        ) {
            let _ = self
                .store
                .append_run_event(run.id, RunEventLevel::Info, &note)?;
        }
        let mut patch = IssuePatch::empty();
        patch.status = Some(IssueStatus::AgentRunning);
        patch.attention_reason = Some(Some("execution in progress".into()));
        let _ = self.store.update_issue(issue.local_id, &patch)?;
        self.saved_view = SavedView::Running;
        self.reload()?;
        self.status_message = format!("Started run #{} for {}", run.id, issue.identifier);
        Ok(())
    }

    pub fn complete_latest_run_success(&mut self) -> Result<()> {
        self.complete_latest_run(
            RunStatus::Succeeded,
            Some("Run completed successfully"),
            None,
            IssueStatus::NeedsReview,
            "run succeeded; review requested",
        )
    }

    pub fn complete_latest_run_failure(&mut self) -> Result<()> {
        self.complete_latest_run(
            RunStatus::Failed,
            Some("Run failed"),
            Some(1),
            IssueStatus::NeedsHumanInput,
            "run failed; human follow-up needed",
        )
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
        let all_dispatch_issues = self.store.list_issues(&IssueQuery {
            include_archived: true,
            ..IssueQuery::default()
        })?;
        self.needs_human_count = issues
            .iter()
            .filter(|issue| issue_needs_human_attention(issue))
            .count();
        self.open_agent_request_count = issues.iter().try_fold(0usize, |acc, issue| {
            Ok::<usize, anyhow::Error>(
                acc + self
                    .store
                    .list_agent_requests_for_issue(issue.local_id)?
                    .into_iter()
                    .filter(|request| request.status == AgentRequestStatus::Open)
                    .count(),
            )
        })?;
        let interruption_overview = self.build_interruption_overview(&issues)?;
        self.escalated_interruptions_count = interruption_overview.escalated_count;
        self.snoozed_interruptions_count = interruption_overview.snoozed_count;
        self.due_soon_interruptions_count = interruption_overview.due_soon_count;
        self.next_interruption_due_label = interruption_overview.next_due_label;
        self.agent_roster = self.build_agent_roster(&issues)?;
        self.stale_agent_count = self
            .agent_roster
            .iter()
            .filter(|entry| entry.is_stale)
            .count();
        self.dispatch_summaries = self.build_dispatch_summaries(&all_dispatch_issues)?;
        self.parallel_parent_count = self.dispatch_summaries.len();
        self.parallel_subissue_count = self
            .dispatch_summaries
            .values()
            .map(|summary| summary.total_children)
            .sum();
        self.interruptions = self.build_interruption_queue(&issues)?;
        self.issues = self.filter_issues_for_view(issues);
        self.scratch_items = self.store.list_scratch_items()?;
        self.refresh_activity()?;
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
            KeyCode::Enter if key.modifiers == KeyModifiers::SHIFT => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.insert_newline();
                }
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
                    editor.backspace();
                }
            }
            KeyCode::Left => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.move_left();
                }
            }
            KeyCode::Right => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.move_right();
                }
            }
            KeyCode::Home => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.move_home();
                }
            }
            KeyCode::End => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.move_end();
                }
            }
            KeyCode::Char('j') if key.modifiers == KeyModifiers::CONTROL => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.insert_newline();
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
            KeyCode::Char('f') if key.modifiers == KeyModifiers::CONTROL => {
                if let Some(editor) = self.editor.as_mut() {
                    if matches!(editor.mode, EditorMode::Closeout { .. }) {
                        editor.follow_up_needed = !editor.follow_up_needed;
                    }
                }
            }
            KeyCode::Char(ch)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                if let Some(editor) = self.editor.as_mut() {
                    editor.insert_char(ch);
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
            EditorMode::DispatchSubissue { parent_local_id } => {
                let title = if editor.title.trim().is_empty() {
                    "Dispatched subtask".to_string()
                } else {
                    editor.title.trim().to_string()
                };
                let description = if editor.description.trim().is_empty() {
                    "Subtask dispatched from parent issue.".to_string()
                } else {
                    editor.description.trim().to_string()
                };
                let kind_status = match editor.project.trim() {
                    "todo" => IssueStatus::Todo,
                    "agent_running" => IssueStatus::AgentRunning,
                    "needs_review" => IssueStatus::NeedsReview,
                    "blocked" => IssueStatus::Blocked,
                    _ => IssueStatus::ReadyForAgent,
                };
                let mut draft = IssueDraft::new(title, description);
                draft.parent_id = Some(parent_local_id);
                draft.status = kind_status.clone();
                draft.priority = editor.priority;
                draft.owner_type = if matches!(
                    kind_status,
                    IssueStatus::ReadyForAgent | IssueStatus::AgentRunning
                ) {
                    OwnerType::Agent
                } else {
                    OwnerType::Unassigned
                };
                draft.attention_reason = Some(format!("dispatched from LOCAL-{parent_local_id}"));
                let issue = self.store.create_issue(&draft)?;
                self.saved_view = preferred_view_for_status(issue.status.clone());
                self.reload()?;
                self.select_issue(issue.local_id);
                self.status_message = format!("Dispatched sub-issue {}", issue.identifier);
            }
            EditorMode::AgentRequest { issue_local_id } => {
                let kind = match editor.project.trim() {
                    "review" => AgentRequestKind::Review,
                    "blocker" => AgentRequestKind::Blocker,
                    _ => AgentRequestKind::Question,
                };
                let title = if editor.title.trim().is_empty() {
                    "Need human input".to_string()
                } else {
                    editor.title.trim().to_string()
                };
                let body = if editor.description.trim().is_empty() {
                    "No extra context provided.".to_string()
                } else {
                    editor.description.trim().to_string()
                };
                let requested_by = self
                    .active_session_link
                    .as_ref()
                    .map(|session| session.label.clone())
                    .unwrap_or_else(|| "agent".into());
                let _request = self.store.create_agent_request(
                    issue_local_id,
                    kind.clone(),
                    &title,
                    &body,
                    &requested_by,
                )?;
                let mut patch = IssuePatch::empty();
                patch.status = Some(match kind {
                    AgentRequestKind::Review => IssueStatus::NeedsReview,
                    AgentRequestKind::Blocker => IssueStatus::Blocked,
                    AgentRequestKind::Question => IssueStatus::NeedsHumanInput,
                });
                patch.attention_reason =
                    Some(Some(format!("agent request: {}", title.to_lowercase())));
                if matches!(kind, AgentRequestKind::Blocker) {
                    patch.blocked_reason = Some(Some(title.clone()));
                }
                let issue = self.store.update_issue(issue_local_id, &patch)?;
                self.store.create_handoff(
                    issue_local_id,
                    requested_by.as_str(),
                    "human",
                    &format!("Agent request opened: {title}"),
                )?;
                self.saved_view = preferred_view_for_status(issue.status.clone());
                self.reload()?;
                self.select_issue(issue.local_id);
                self.status_message = format!("Opened agent request on {}", issue.identifier);
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
            EditorMode::RunNote { run_id } => {
                let message = if editor.title.trim().is_empty() {
                    "Captured run note".to_string()
                } else {
                    editor.title.trim().to_string()
                };
                self.store
                    .append_run_event(run_id, RunEventLevel::Info, &message)?;
                self.reload()?;
                self.status_message = "Attached note to active run".into();
            }
            EditorMode::Closeout { local_id } => {
                let mut patch = IssuePatch::empty();
                patch.status = Some(IssueStatus::Done);
                patch.closeout_summary = Some(empty_to_none(&editor.title));
                patch.follow_up_needed = Some(editor.follow_up_needed);
                patch.attention_reason = Some(Some(if editor.follow_up_needed {
                    "closed; follow-up still needed".into()
                } else {
                    "closed loop".into()
                }));
                let issue = self.store.update_issue(local_id, &patch)?;
                self.store.create_handoff(
                    local_id,
                    "active work",
                    "done",
                    if editor.follow_up_needed {
                        "Closed with follow-up still needed"
                    } else {
                        "Closed with summary"
                    },
                )?;
                self.saved_view = SavedView::Done;
                self.reload()?;
                self.select_issue(issue.local_id);
                self.status_message = format!("Closed {}", issue.identifier);
            }
            EditorMode::ArtifactNote {
                issue_local_id,
                run_id,
            } => {
                let note = if editor.title.trim().is_empty() {
                    "Captured evidence note".to_string()
                } else {
                    editor.title.trim().to_string()
                };
                self.store.create_artifact(
                    issue_local_id,
                    run_id,
                    ArtifactKind::Note,
                    &note,
                    None,
                )?;
                self.reload()?;
                self.select_issue(issue_local_id);
                self.status_message = "Attached evidence note".into();
            }
            EditorMode::WorkContext { issue_local_id } => {
                let repo_path = if editor.title.trim().is_empty() {
                    ".".to_string()
                } else {
                    editor.title.trim().to_string()
                };
                let git_snapshot = detect_git_context_prefill_for_path(
                    editor
                        .description
                        .trim()
                        .split_once('\n')
                        .map(|(first, _)| first)
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or(repo_path.as_str()),
                );
                self.store.set_active_work_context(
                    issue_local_id,
                    &repo_path,
                    empty_to_none(&editor.description).as_deref(),
                    empty_to_none(&editor.project).as_deref(),
                    git_snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.git_status_summary.as_deref()),
                    git_snapshot
                        .as_ref()
                        .map(|snapshot| snapshot.dirty_file_count)
                        .unwrap_or(0),
                    git_snapshot
                        .as_ref()
                        .map(|snapshot| snapshot.staged_file_count)
                        .unwrap_or(0),
                    git_snapshot
                        .as_ref()
                        .map(|snapshot| snapshot.ahead_count)
                        .unwrap_or(0),
                    git_snapshot
                        .as_ref()
                        .map(|snapshot| snapshot.behind_count)
                        .unwrap_or(0),
                )?;
                self.reload()?;
                self.select_issue(issue_local_id);
                self.status_message = "Attached work context".into();
            }
            EditorMode::SessionLink { issue_local_id } => {
                let label = if editor.title.trim().is_empty() {
                    "session".to_string()
                } else {
                    editor.title.trim().to_string()
                };
                let kind = parse_session_kind(&editor.description);
                let session_ref = if editor.project.trim().is_empty() {
                    "session-ref".to_string()
                } else {
                    editor.project.trim().to_string()
                };
                self.store
                    .set_active_session_link(issue_local_id, &session_ref, kind, &label)?;
                self.reload()?;
                self.select_issue(issue_local_id);
                self.status_message = "Attached session link".into();
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
        } else if matches!(view, SavedView::Interruptions) {
            self.status_message = "Switched to interruption queue".into();
        } else if matches!(view, SavedView::Done) {
            self.status_message = "Switched to done issues".into();
        } else if matches!(view, SavedView::Inbox) {
            self.status_message = "Switched to inbox".into();
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
            let _ = self.refresh_activity();
        }
    }

    fn focus_issue_any_view(&mut self, local_id: i64) -> Result<()> {
        let Some(issue) = self.store.get_issue(local_id)? else {
            self.status_message = "Target issue no longer exists".into();
            return Ok(());
        };

        self.saved_view = preferred_view_for_status(issue.status.clone());
        self.query.unsynced_only = false;
        self.query.archived_only = false;
        self.query.include_archived = issue.is_archived;
        self.reload()?;
        self.select_issue(local_id);
        Ok(())
    }

    fn select_scratch(&mut self, scratch_id: i64) {
        if let Some(index) = self
            .scratch_items
            .iter()
            .position(|scratch| scratch.id == scratch_id)
        {
            self.selected = index;
            let _ = self.refresh_activity();
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
            "queue: {} | pending: {} | errors: {} | scratch: {} | open-requests: {}",
            self.queued_mutation_count,
            pending,
            errors,
            self.scratch_items
                .iter()
                .filter(|item| item.promoted_issue_id.is_none())
                .count(),
            self.open_agent_request_count
        )
    }

    pub fn is_scratch_view(&self) -> bool {
        self.saved_view == SavedView::Scratch
    }

    pub fn is_interruptions_view(&self) -> bool {
        self.saved_view == SavedView::Interruptions
    }

    pub fn list_title(&self) -> &'static str {
        match self.saved_view {
            SavedView::Inbox => "Inbox",
            SavedView::Running => "Running",
            SavedView::Review => "Review",
            SavedView::Waiting => "Waiting",
            SavedView::Done => "Done",
            SavedView::Scratch => "Scratch",
            SavedView::Interruptions => "Interruptions",
            SavedView::DispatchBoard => "Dispatch Board",
        }
    }

    pub fn attention_summary(&self) -> String {
        let agents_running = self
            .agent_roster
            .iter()
            .filter(|entry| entry.session_kind == SessionKind::AgentSession)
            .count();
        format!(
            "needs-human: {} | active-agents: {} | stale-agents: {} | parallel: {} parent / {} child | roster: {}",
            self.needs_human_count,
            agents_running,
            self.stale_agent_count,
            self.parallel_parent_count,
            self.parallel_subissue_count,
            self.agent_roster.len()
        )
    }

    pub fn interruption_glance_summary(&self) -> String {
        format!(
            "open: {} | escalated: {} | snoozed: {} | due-soon: {}",
            self.open_agent_request_count,
            self.escalated_interruptions_count,
            self.snoozed_interruptions_count,
            self.due_soon_interruptions_count
        )
    }

    pub fn interruption_due_summary(&self) -> String {
        self.next_interruption_due_label
            .clone()
            .unwrap_or_else(|| "next due: none".into())
    }

    pub fn dispatch_summary_for_issue(&self, local_id: i64) -> Option<&DispatchSummary> {
        self.dispatch_summaries.get(&local_id)
    }

    pub fn parallel_context_summary(&self, issue: &Issue) -> Option<String> {
        if let Some(summary) = self.dispatch_summary_for_issue(issue.local_id) {
            return Some(format!("dispatch owner | {}", summary.compact_label()));
        }
        if let Some(parent_id) = issue.parent_id {
            if let Some(summary) = self.dispatch_summary_for_issue(parent_id) {
                return Some(format!(
                    "child of LOCAL-{} | sibling graph: {}",
                    parent_id,
                    summary.compact_label()
                ));
            }
            return Some(format!("child of LOCAL-{parent_id}"));
        }
        None
    }

    pub fn issue_is_stale(&self, local_id: i64) -> bool {
        self.agent_roster
            .iter()
            .any(|entry| entry.issue_local_id == local_id && entry.is_stale)
    }

    pub fn graph_navigation_hint(&self, issue: &Issue) -> Option<String> {
        if let Some(parent_id) = issue.parent_id {
            return Some(format!("P parent LOCAL-{parent_id} | C next sibling"));
        }
        if self.dispatch_summary_for_issue(issue.local_id).is_some() {
            return Some("C next child needing attention".into());
        }
        None
    }

    fn visible_len(&self) -> usize {
        if self.saved_view == SavedView::Scratch {
            self.scratch_items.len()
        } else if self.saved_view == SavedView::Interruptions {
            self.interruptions.len()
        } else {
            self.issues.len()
        }
    }

    fn filter_issues_for_view(&self, issues: Vec<Issue>) -> Vec<Issue> {
        issues
            .into_iter()
            .filter(|issue| match self.saved_view {
                SavedView::Inbox => issue_needs_human_attention(issue),
                SavedView::Running => {
                    matches!(
                        issue.status,
                        IssueStatus::ReadyForAgent | IssueStatus::AgentRunning
                    )
                }
                SavedView::Review => issue.status == IssueStatus::NeedsReview,
                SavedView::Waiting => {
                    matches!(
                        issue.status,
                        IssueStatus::Blocked | IssueStatus::NeedsHumanInput
                    )
                }
                SavedView::Done => issue.status == IssueStatus::Done,
                SavedView::Scratch => false,
                SavedView::Interruptions => false,
                SavedView::DispatchBoard => {
                    self.dispatch_summary_for_issue(issue.local_id).is_some()
                }
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
        let to_actor = owner_name
            .as_deref()
            .unwrap_or(owner_type.label())
            .to_string();
        let mut patch = IssuePatch::empty();
        patch.status = Some(status);
        patch.owner_type = Some(owner_type);
        patch.owner_name = Some(owner_name);
        patch.attention_reason = Some(attention_reason);
        patch.blocked_reason = Some(blocked_reason);
        let updated = self.store.update_issue(issue.local_id, &patch)?;
        self.store.create_handoff(
            issue.local_id,
            issue
                .owner_name
                .as_deref()
                .unwrap_or(issue.owner_type.label()),
            &to_actor,
            message,
        )?;
        self.saved_view = preferred_view_for_status(updated.status.clone());
        self.reload()?;
        self.select_issue(updated.local_id);
        self.status_message = format!("{message}: {}", updated.identifier);
        Ok(())
    }

    fn refresh_activity(&mut self) -> Result<()> {
        if let Some(issue) = self.current_issue().cloned() {
            self.runs = self.store.list_runs_for_issue(issue.local_id)?;
            self.run_events = self.store.list_run_events_for_issue(issue.local_id)?;
            self.artifacts = self.store.list_artifacts_for_issue(issue.local_id)?;
            self.handoffs = self.store.list_handoffs_for_issue(issue.local_id)?;
            self.subissues = self.store.list_subissues(issue.local_id)?;
            self.agent_requests = self.store.list_agent_requests_for_issue(issue.local_id)?;
            self.active_work_context = self.store.get_active_work_context(issue.local_id)?;
            self.active_session_link = self.store.get_active_session_link(issue.local_id)?;
        } else if let Some(interruption) = self.current_interruption().cloned() {
            self.runs = self
                .store
                .list_runs_for_issue(interruption.issue.local_id)?;
            self.run_events = self
                .store
                .list_run_events_for_issue(interruption.issue.local_id)?;
            self.artifacts = self
                .store
                .list_artifacts_for_issue(interruption.issue.local_id)?;
            self.handoffs = self
                .store
                .list_handoffs_for_issue(interruption.issue.local_id)?;
            self.subissues = self.store.list_subissues(interruption.issue.local_id)?;
            self.agent_requests = self
                .store
                .list_agent_requests_for_issue(interruption.issue.local_id)?;
            self.active_work_context = self
                .store
                .get_active_work_context(interruption.issue.local_id)?;
            self.active_session_link = self
                .store
                .get_active_session_link(interruption.issue.local_id)?;
        } else {
            self.runs.clear();
            self.run_events.clear();
            self.artifacts.clear();
            self.handoffs.clear();
            self.subissues.clear();
            self.agent_requests.clear();
            self.active_work_context = None;
            self.active_session_link = None;
        }
        Ok(())
    }

    fn resolve_latest_agent_request(&mut self) -> Result<()> {
        if let Some(interruption) = self.current_interruption().cloned() {
            let _ = self.store.resolve_agent_request(interruption.request.id)?;
            self.store.create_handoff(
                interruption.issue.local_id,
                "human",
                interruption.request.requested_by.as_str(),
                &format!("Resolved interruption: {}", interruption.request.title),
            )?;
            self.reload()?;
            self.status_message =
                format!("Resolved interruption on {}", interruption.issue.identifier);
            return Ok(());
        }
        let Some(issue) = self.current_issue().cloned() else {
            return Ok(());
        };
        let open_request = self
            .store
            .list_agent_requests_for_issue(issue.local_id)?
            .into_iter()
            .find(|request| request.status == AgentRequestStatus::Open);
        let Some(request) = open_request else {
            self.status_message = "No open agent request to resolve".into();
            return Ok(());
        };
        let _ = self.store.resolve_agent_request(request.id)?;
        self.store.create_handoff(
            issue.local_id,
            "human",
            request.requested_by.as_str(),
            &format!("Resolved agent request: {}", request.title),
        )?;
        self.reload()?;
        self.select_issue(issue.local_id);
        self.status_message = format!("Resolved request on {}", issue.identifier);
        Ok(())
    }

    fn acknowledge_interruption_and_requeue(&mut self) -> Result<()> {
        let Some(interruption) = self.current_interruption().cloned() else {
            return Ok(());
        };
        let _ = self.store.resolve_agent_request(interruption.request.id)?;
        let mut patch = IssuePatch::empty();
        patch.status = Some(IssueStatus::ReadyForAgent);
        patch.owner_type = Some(OwnerType::Agent);
        patch.owner_name = Some(Some("agent".into()));
        patch.attention_reason = Some(Some("requeued after interruption acknowledgement".into()));
        patch.blocked_reason = Some(None);
        self.store
            .update_issue(interruption.issue.local_id, &patch)?;
        self.store.create_handoff(
            interruption.issue.local_id,
            "human",
            "agent",
            "Acknowledged interruption and requeued to agent",
        )?;
        self.reload()?;
        self.status_message = format!(
            "Acknowledged interruption and requeued {}",
            interruption.issue.identifier
        );
        Ok(())
    }

    fn resolve_graph_interruptions(&mut self) -> Result<()> {
        let Some(interruption) = self.current_interruption().cloned() else {
            return Ok(());
        };
        let graph_root_id = interruption
            .issue
            .parent_id
            .unwrap_or(interruption.issue.local_id);
        let mut affected_issue_ids = vec![graph_root_id];
        affected_issue_ids.extend(
            self.store
                .list_subissues(graph_root_id)?
                .into_iter()
                .map(|issue| issue.local_id),
        );

        let mut resolved = 0usize;
        for issue_id in affected_issue_ids {
            for request in self
                .store
                .list_agent_requests_for_issue(issue_id)?
                .into_iter()
                .filter(|request| request.status == AgentRequestStatus::Open)
            {
                let _ = self.store.resolve_agent_request(request.id)?;
                resolved += 1;
            }
        }

        if resolved == 0 {
            self.status_message = "No open interruptions in this graph".into();
            return Ok(());
        }

        self.store.create_handoff(
            graph_root_id,
            "human",
            "graph",
            "Resolved open interruptions across dispatch graph",
        )?;
        self.reload()?;
        self.status_message = format!("Resolved {resolved} interruption(s) across the graph");
        Ok(())
    }

    fn snooze_selected_interruption(&mut self) -> Result<()> {
        let Some(interruption) = self.current_interruption().cloned() else {
            return Ok(());
        };
        let until = Utc::now() + Duration::minutes(30);
        let _ = self
            .store
            .snooze_agent_request(interruption.request.id, until)?;
        self.store.create_handoff(
            interruption.issue.local_id,
            "human",
            "later",
            "Snoozed interruption for 30 minutes",
        )?;
        self.reload()?;
        self.status_message = format!(
            "Snoozed interruption on {} until {}",
            interruption.issue.identifier,
            until.format("%H:%M UTC")
        );
        Ok(())
    }

    fn escalate_selected_interruption(&mut self) -> Result<()> {
        let Some(interruption) = self.current_interruption().cloned() else {
            return Ok(());
        };
        let Some(updated_request) = self.store.escalate_agent_request(interruption.request.id)?
        else {
            self.status_message = "Selected interruption no longer exists".into();
            return Ok(());
        };
        let mut patch = IssuePatch::empty();
        patch.attention_reason = Some(Some(format!(
            "escalated interruption: {} (level {})",
            updated_request.title.to_lowercase(),
            updated_request.escalation_level
        )));
        self.store
            .update_issue(interruption.issue.local_id, &patch)?;
        self.store.create_handoff(
            interruption.issue.local_id,
            "human",
            "escalated",
            &format!(
                "Escalated interruption to level {}",
                updated_request.escalation_level
            ),
        )?;
        self.reload()?;
        self.status_message = format!(
            "Escalated interruption on {} to level {}",
            interruption.issue.identifier, updated_request.escalation_level
        );
        Ok(())
    }

    fn snooze_graph_review_interruptions(&mut self) -> Result<()> {
        let Some(interruption) = self.current_interruption().cloned() else {
            return Ok(());
        };
        let graph_root_id = interruption
            .issue
            .parent_id
            .unwrap_or(interruption.issue.local_id);
        let until = Utc::now() + Duration::hours(1);
        let mut affected_issue_ids = vec![graph_root_id];
        affected_issue_ids.extend(
            self.store
                .list_subissues(graph_root_id)?
                .into_iter()
                .map(|issue| issue.local_id),
        );

        let mut snoozed = 0usize;
        for issue_id in affected_issue_ids {
            for request in self
                .store
                .list_agent_requests_for_issue(issue_id)?
                .into_iter()
                .filter(|request| {
                    request.status == AgentRequestStatus::Open
                        && request.kind == AgentRequestKind::Review
                })
            {
                let _ = self.store.snooze_agent_request(request.id, until)?;
                snoozed += 1;
            }
        }

        if snoozed == 0 {
            self.status_message = "No open review interruptions in this graph".into();
            return Ok(());
        }

        self.store.create_handoff(
            graph_root_id,
            "human",
            "later",
            "Snoozed review interruptions across dispatch graph",
        )?;
        self.reload()?;
        self.status_message = format!(
            "Snoozed {snoozed} review interruption(s) until {}",
            until.format("%H:%M UTC")
        );
        Ok(())
    }

    fn escalate_graph_blockers(&mut self) -> Result<()> {
        let Some(interruption) = self.current_interruption().cloned() else {
            return Ok(());
        };
        let graph_root_id = interruption
            .issue
            .parent_id
            .unwrap_or(interruption.issue.local_id);
        let mut affected_issue_ids = vec![graph_root_id];
        affected_issue_ids.extend(
            self.store
                .list_subissues(graph_root_id)?
                .into_iter()
                .map(|issue| issue.local_id),
        );

        let mut escalated = 0usize;
        for issue_id in affected_issue_ids {
            for request in self
                .store
                .list_agent_requests_for_issue(issue_id)?
                .into_iter()
                .filter(|request| {
                    request.status == AgentRequestStatus::Open
                        && request.kind == AgentRequestKind::Blocker
                })
            {
                let _ = self.store.escalate_agent_request(request.id)?;
                escalated += 1;
            }
        }

        if escalated == 0 {
            self.status_message = "No open blocker interruptions in this graph".into();
            return Ok(());
        }

        self.store.create_handoff(
            graph_root_id,
            "human",
            "escalated",
            "Escalated blocker interruptions across dispatch graph",
        )?;
        self.reload()?;
        self.status_message =
            format!("Escalated {escalated} blocker interruption(s) across the graph");
        Ok(())
    }

    fn jump_to_parent_issue(&mut self) -> Result<()> {
        let Some(issue) = self.current_issue().cloned() else {
            return Ok(());
        };
        let Some(parent_id) = issue.parent_id else {
            self.status_message = "Selected issue is already the graph root".into();
            return Ok(());
        };
        self.focus_issue_any_view(parent_id)?;
        self.status_message = format!("Jumped to parent LOCAL-{parent_id}");
        Ok(())
    }

    fn jump_to_next_graph_issue(&mut self) -> Result<()> {
        let Some(issue) = self.current_issue().cloned() else {
            return Ok(());
        };

        if issue.parent_id.is_none() {
            let children = self.store.list_subissues(issue.local_id)?;
            if children.is_empty() {
                self.status_message = "No dispatched children to jump into".into();
                return Ok(());
            }
            let target = children
                .into_iter()
                .min_by_key(graph_issue_sort_key)
                .expect("children already checked as non-empty");
            self.focus_issue_any_view(target.local_id)?;
            self.status_message = format!("Jumped to child {}", target.identifier);
            return Ok(());
        }

        let parent_id = issue.parent_id.expect("child issue should have parent");
        let siblings = self.store.list_subissues(parent_id)?;
        if siblings.len() <= 1 {
            self.status_message = "No sibling issues in this dispatch graph".into();
            return Ok(());
        }

        let mut ordered = siblings;
        ordered.sort_by_key(graph_issue_sort_key);
        let Some(current_index) = ordered
            .iter()
            .position(|candidate| candidate.local_id == issue.local_id)
        else {
            self.status_message = "Current issue is missing from its sibling graph".into();
            return Ok(());
        };
        let next_index = (current_index + 1) % ordered.len();
        let target = ordered[next_index].clone();
        self.focus_issue_any_view(target.local_id)?;
        self.status_message = format!("Jumped to sibling {}", target.identifier);
        Ok(())
    }

    fn approve_review_children(&mut self) -> Result<()> {
        let Some(parent) = self.current_issue().cloned() else {
            return Ok(());
        };
        let children = self.store.list_subissues(parent.local_id)?;
        if children.is_empty() {
            self.status_message = "No child issues in this dispatch graph".into();
            return Ok(());
        }

        let mut approved = 0usize;
        for child in children
            .into_iter()
            .filter(|child| child.status == IssueStatus::NeedsReview)
        {
            let mut patch = IssuePatch::empty();
            patch.status = Some(IssueStatus::Done);
            patch.attention_reason = Some(Some("approved from parent dispatch graph".into()));
            patch.owner_type = Some(OwnerType::Human);
            patch.owner_name = Some(Some("reviewer".into()));
            patch.blocked_reason = Some(None);
            self.store.update_issue(child.local_id, &patch)?;
            for request in self
                .store
                .list_agent_requests_for_issue(child.local_id)?
                .into_iter()
                .filter(|request| {
                    request.status == AgentRequestStatus::Open
                        && request.kind == AgentRequestKind::Review
                })
            {
                let _ = self.store.resolve_agent_request(request.id)?;
            }
            self.store.create_handoff(
                child.local_id,
                "review",
                "done",
                "Approved from parent dispatch graph",
            )?;
            approved += 1;
        }

        if approved == 0 {
            self.status_message = "No review-ready child issues to approve".into();
            return Ok(());
        }

        self.reload()?;
        self.select_issue(parent.local_id);
        self.status_message = format!("Approved {approved} child issue(s) from graph");
        Ok(())
    }

    fn requeue_stalled_children(&mut self) -> Result<()> {
        let Some(parent) = self.current_issue().cloned() else {
            return Ok(());
        };
        let children = self.store.list_subissues(parent.local_id)?;
        if children.is_empty() {
            self.status_message = "No child issues in this dispatch graph".into();
            return Ok(());
        }

        let mut requeued = 0usize;
        for child in children.into_iter().filter(|child| {
            matches!(
                child.status,
                IssueStatus::Todo | IssueStatus::NeedsHumanInput | IssueStatus::Blocked
            )
        }) {
            let mut patch = IssuePatch::empty();
            patch.status = Some(IssueStatus::ReadyForAgent);
            patch.owner_type = Some(OwnerType::Agent);
            patch.owner_name = Some(Some("agent".into()));
            patch.attention_reason = Some(Some("requeued from parent dispatch graph".into()));
            patch.blocked_reason = Some(None);
            self.store.update_issue(child.local_id, &patch)?;
            self.store.create_handoff(
                child.local_id,
                "parent graph",
                "agent",
                "Requeued from parent dispatch graph",
            )?;
            requeued += 1;
        }

        if requeued == 0 {
            self.status_message = "No stalled child issues to requeue".into();
            return Ok(());
        }

        self.reload()?;
        self.select_issue(parent.local_id);
        self.status_message = format!("Requeued {requeued} child issue(s) to agents");
        Ok(())
    }

    fn complete_latest_run(
        &mut self,
        status: RunStatus,
        summary: Option<&str>,
        exit_code: Option<i64>,
        next_issue_status: IssueStatus,
        attention_reason: &str,
    ) -> Result<()> {
        let Some(issue) = self.current_issue().cloned() else {
            return Ok(());
        };
        let Some(run) = self.store.latest_active_run_for_issue(issue.local_id)? else {
            self.status_message = "No active run to complete".into();
            return Ok(());
        };
        let updated_run = self
            .store
            .complete_run(run.id, status.clone(), summary, exit_code)?;
        let mut patch = IssuePatch::empty();
        patch.status = Some(next_issue_status.clone());
        patch.attention_reason = Some(Some(attention_reason.into()));
        if status == RunStatus::Failed {
            patch.blocked_reason = Some(Some("latest run failed".into()));
        } else {
            patch.blocked_reason = Some(None);
        }
        let _ = self.store.update_issue(issue.local_id, &patch)?;
        self.store.create_handoff(
            issue.local_id,
            "run",
            if status == RunStatus::Failed {
                "human"
            } else {
                "review"
            },
            attention_reason,
        )?;
        self.saved_view = preferred_view_for_status(next_issue_status);
        self.reload()?;
        self.status_message = format!(
            "Run #{} marked {}",
            updated_run.id,
            updated_run.status.label()
        );
        Ok(())
    }

    fn ensure_terminal_context(&mut self, issue_local_id: i64) -> Result<()> {
        if self.active_work_context.is_none() {
            if let Some(prefill) = detect_git_context_prefill() {
                let context = self.store.set_active_work_context(
                    issue_local_id,
                    &prefill.repo_path,
                    prefill.worktree_path.as_deref(),
                    prefill.branch_name.as_deref(),
                    prefill.git_status_summary.as_deref(),
                    prefill.dirty_file_count,
                    prefill.staged_file_count,
                    prefill.ahead_count,
                    prefill.behind_count,
                )?;
                self.active_work_context = Some(context);
            }
        }

        if self.active_session_link.is_none() {
            let (label, kind, session_ref) = default_session_prefill();
            let session = self.store.set_active_session_link(
                issue_local_id,
                &session_ref,
                parse_session_kind(&kind),
                &label,
            )?;
            self.active_session_link = Some(session);
        }

        Ok(())
    }

    fn build_agent_roster(&self, issues: &[Issue]) -> Result<Vec<AgentRosterEntry>> {
        let mut roster = Vec::new();
        for issue in issues.iter().filter(|issue| {
            matches!(
                issue.status,
                IssueStatus::ReadyForAgent | IssueStatus::AgentRunning | IssueStatus::NeedsReview
            ) || issue.owner_type == OwnerType::Agent
        }) {
            let active_session = self.store.get_active_session_link(issue.local_id)?;
            let active_context = self.store.get_active_work_context(issue.local_id)?;
            let active_run = self.store.latest_active_run_for_issue(issue.local_id)?;
            if let Some(session) = active_session {
                let session_kind = session.session_kind.clone();
                let last_activity_at = latest_activity_at(
                    issue.updated_at,
                    Some(session.last_heartbeat_at),
                    active_context.as_ref().map(|ctx| ctx.updated_at),
                    active_run
                        .as_ref()
                        .map(|run| run.ended_at.unwrap_or(run.started_at)),
                );
                roster.push(AgentRosterEntry {
                    issue_local_id: issue.local_id,
                    identifier: issue.identifier.clone(),
                    session_label: session.label,
                    session_kind: session_kind.clone(),
                    branch_name: active_context
                        .as_ref()
                        .and_then(|ctx| ctx.branch_name.clone()),
                    last_activity_at,
                    is_stale: is_stale_agent(
                        session_kind,
                        issue.status.clone(),
                        last_activity_at,
                        Utc::now(),
                    ),
                });
            }
        }
        roster.sort_by(|left, right| left.identifier.cmp(&right.identifier));
        Ok(roster)
    }

    fn build_dispatch_summaries(&self, issues: &[Issue]) -> Result<HashMap<i64, DispatchSummary>> {
        let mut summaries = HashMap::new();
        for issue in issues.iter().filter(|issue| issue.parent_id.is_some()) {
            let Some(parent_id) = issue.parent_id else {
                continue;
            };
            let open_requests = self
                .store
                .list_agent_requests_for_issue(issue.local_id)?
                .into_iter()
                .filter(|request| request.status == AgentRequestStatus::Open)
                .count();
            summaries
                .entry(parent_id)
                .or_insert_with(DispatchSummary::new)
                .record_issue(issue, open_requests);
        }
        Ok(summaries)
    }

    fn build_interruption_queue(&self, issues: &[Issue]) -> Result<Vec<InterruptionItem>> {
        let mut queue = Vec::new();
        let now = Utc::now();
        for issue in issues {
            for request in self
                .store
                .list_agent_requests_for_issue(issue.local_id)?
                .into_iter()
                .filter(|request| {
                    request.status == AgentRequestStatus::Open
                        && request
                            .snoozed_until
                            .map(|until| until <= now)
                            .unwrap_or(true)
                })
            {
                queue.push(InterruptionItem {
                    request,
                    issue: issue.clone(),
                });
            }
        }
        queue.sort_by(|left, right| {
            right
                .request
                .escalation_level
                .cmp(&left.request.escalation_level)
                .then_with(|| right.request.created_at.cmp(&left.request.created_at))
        });
        Ok(queue)
    }

    fn build_interruption_overview(&self, issues: &[Issue]) -> Result<InterruptionOverview> {
        let now = Utc::now();
        let mut overview = InterruptionOverview::default();
        let mut next_due: Option<DateTime<Utc>> = None;

        for issue in issues {
            for request in self
                .store
                .list_agent_requests_for_issue(issue.local_id)?
                .into_iter()
                .filter(|request| request.status == AgentRequestStatus::Open)
            {
                if request.escalation_level > 0 {
                    overview.escalated_count += 1;
                }
                if let Some(until) = request.snoozed_until {
                    if until > now {
                        overview.snoozed_count += 1;
                        if until <= now + Duration::hours(1) {
                            overview.due_soon_count += 1;
                        }
                        next_due = Some(match next_due {
                            Some(current) => current.min(until),
                            None => until,
                        });
                    }
                }
            }
        }

        overview.next_due_label =
            next_due.map(|due| format!("next due: {}", relative_future_label(now, due)));
        Ok(overview)
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
    Interruptions,
    DispatchBoard,
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
            Self::Interruptions => "interruptions",
            Self::DispatchBoard => "dispatch-board",
        }
    }
}

fn issue_needs_human_attention(issue: &Issue) -> bool {
    matches!(
        issue.status,
        IssueStatus::Todo
            | IssueStatus::NeedsHumanInput
            | IssueStatus::NeedsReview
            | IssueStatus::Blocked
    )
}

fn preferred_view_for_status(status: IssueStatus) -> SavedView {
    match status {
        IssueStatus::ReadyForAgent | IssueStatus::AgentRunning => SavedView::Running,
        IssueStatus::NeedsReview => SavedView::Review,
        IssueStatus::Blocked => SavedView::Waiting,
        IssueStatus::Done => SavedView::Done,
        IssueStatus::Todo | IssueStatus::NeedsHumanInput => SavedView::Inbox,
    }
}

fn graph_issue_sort_key(issue: &Issue) -> (u8, String) {
    (graph_issue_rank(&issue.status), issue.identifier.clone())
}

fn graph_issue_rank(status: &IssueStatus) -> u8 {
    match status {
        IssueStatus::NeedsHumanInput => 0,
        IssueStatus::Blocked => 1,
        IssueStatus::NeedsReview => 2,
        IssueStatus::Todo => 3,
        IssueStatus::ReadyForAgent => 4,
        IssueStatus::AgentRunning => 5,
        IssueStatus::Done => 6,
    }
}

fn relative_future_label(now: DateTime<Utc>, due: DateTime<Utc>) -> String {
    let delta = due - now;
    if delta <= Duration::zero() {
        "now".into()
    } else if delta < Duration::hours(1) {
        format!("in {}m", delta.num_minutes().max(1))
    } else if delta < Duration::days(1) {
        format!("in {}h", delta.num_hours().max(1))
    } else {
        format!("in {}d", delta.num_days().max(1))
    }
}

fn latest_activity_at(
    issue_updated_at: DateTime<Utc>,
    session_at: Option<DateTime<Utc>>,
    context_at: Option<DateTime<Utc>>,
    run_at: Option<DateTime<Utc>>,
) -> DateTime<Utc> {
    [Some(issue_updated_at), session_at, context_at, run_at]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(issue_updated_at)
}

fn is_stale_agent(
    session_kind: SessionKind,
    issue_status: IssueStatus,
    last_activity_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> bool {
    if session_kind != SessionKind::AgentSession {
        return false;
    }
    if !matches!(
        issue_status,
        IssueStatus::ReadyForAgent | IssueStatus::AgentRunning | IssueStatus::NeedsReview
    ) {
        return false;
    }
    now.signed_duration_since(last_activity_at) >= Duration::minutes(30)
}

pub(crate) fn relative_age_label(last_activity_at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let age = now.signed_duration_since(last_activity_at);
    if age < Duration::minutes(1) {
        "just now".into()
    } else if age < Duration::hours(1) {
        format!("{}m ago", age.num_minutes())
    } else if age < Duration::days(1) {
        format!("{}h ago", age.num_hours())
    } else {
        format!("{}d ago", age.num_days())
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

fn parse_session_kind(value: &str) -> SessionKind {
    match value.trim() {
        "agent_session" | "agent" => SessionKind::AgentSession,
        "background_job" | "job" => SessionKind::BackgroundJob,
        _ => SessionKind::HumanTerminal,
    }
}

fn detect_git_context_prefill() -> Option<GitContextPrefill> {
    let cwd = env::current_dir().ok()?;
    detect_git_context_prefill_for_path(&cwd.display().to_string())
}

fn detect_git_context_prefill_for_path(path: &str) -> Option<GitContextPrefill> {
    let cwd = PathBuf::from(path);
    let cwd_display = cwd.display().to_string();
    let repo_root = command_stdout(cwd.as_path(), "git", &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.clone());
    let branch_name = command_stdout(&cwd, "git", &["branch", "--show-current"]);
    let status_snapshot = command_stdout(&cwd, "git", &["status", "--porcelain", "--branch"])
        .as_deref()
        .map(parse_git_status_snapshot)
        .unwrap_or_default();
    let worktree_path = if repo_root != cwd {
        Some(cwd_display)
    } else {
        None
    };

    Some(GitContextPrefill {
        repo_path: repo_root.display().to_string(),
        worktree_path,
        branch_name,
        git_status_summary: status_snapshot.summary,
        dirty_file_count: status_snapshot.dirty_file_count,
        staged_file_count: status_snapshot.staged_file_count,
        ahead_count: status_snapshot.ahead_count,
        behind_count: status_snapshot.behind_count,
    })
}

fn command_stdout(cwd: &std::path::Path, program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    normalize_command_output(String::from_utf8_lossy(&output.stdout).as_ref())
}

fn normalize_command_output(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn default_session_prefill() -> (String, String, String) {
    (
        "local terminal".into(),
        "human_terminal".into(),
        format!("pid:{}", std::process::id()),
    )
}

#[derive(Debug, Default)]
struct GitStatusSnapshot {
    summary: Option<String>,
    dirty_file_count: i64,
    staged_file_count: i64,
    ahead_count: i64,
    behind_count: i64,
}

fn parse_git_status_snapshot(output: &str) -> GitStatusSnapshot {
    let mut snapshot = GitStatusSnapshot::default();

    for (index, line) in output.lines().enumerate() {
        if index == 0 && line.starts_with("## ") {
            if let Some((ahead, behind)) = parse_branch_counts(line) {
                snapshot.ahead_count = ahead;
                snapshot.behind_count = behind;
            }
            continue;
        }

        let chars: Vec<char> = line.chars().collect();
        if chars.len() < 2 {
            continue;
        }
        let staged = chars[0];
        let unstaged = chars[1];
        if staged != ' ' && staged != '?' {
            snapshot.staged_file_count += 1;
        }
        if unstaged != ' ' || (staged == '?' && unstaged == '?') {
            snapshot.dirty_file_count += 1;
        }
    }

    let mut parts = Vec::new();
    if snapshot.dirty_file_count > 0 {
        parts.push(format!("dirty {}", snapshot.dirty_file_count));
    }
    if snapshot.staged_file_count > 0 {
        parts.push(format!("staged {}", snapshot.staged_file_count));
    }
    if snapshot.ahead_count > 0 {
        parts.push(format!("ahead {}", snapshot.ahead_count));
    }
    if snapshot.behind_count > 0 {
        parts.push(format!("behind {}", snapshot.behind_count));
    }
    if parts.is_empty() {
        parts.push("clean".into());
    }
    snapshot.summary = Some(parts.join(" | "));
    snapshot
}

fn parse_branch_counts(line: &str) -> Option<(i64, i64)> {
    let mut ahead = 0;
    let mut behind = 0;
    let start = line.find('[')?;
    let end = line[start..].find(']')? + start;
    let payload = &line[start + 1..end];
    for part in payload.split(',') {
        let trimmed = part.trim();
        if let Some(value) = trimmed.strip_prefix("ahead ") {
            ahead = value.parse().ok()?;
        } else if let Some(value) = trimmed.strip_prefix("behind ") {
            behind = value.parse().ok()?;
        }
    }
    Some((ahead, behind))
}

fn build_run_context_note(
    work_context: Option<&WorkContext>,
    session_link: Option<&SessionLink>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(context) = work_context {
        parts.push(format!(
            "repo={} branch={}",
            context.repo_path,
            context.branch_name.as_deref().unwrap_or("none")
        ));
        if let Some(summary) = &context.git_status_summary {
            parts.push(format!("git={summary}"));
        }
    }
    if let Some(session) = session_link {
        parts.push(format!(
            "session={} [{}]",
            session.label,
            session.session_kind.label()
        ));
    }
    if parts.is_empty() {
        None
    } else {
        Some(format!("Run context: {}", parts.join(" | ")))
    }
}

fn insert_char_at(value: &mut String, cursor: &mut usize, ch: char) {
    let byte_index = char_to_byte_index(value, *cursor);
    value.insert(byte_index, ch);
    *cursor += 1;
}

fn backspace_at(value: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let start = char_to_byte_index(value, cursor.saturating_sub(1));
    let end = char_to_byte_index(value, *cursor);
    value.replace_range(start..end, "");
    *cursor = cursor.saturating_sub(1);
}

fn move_right_cursor(value: &str, cursor: usize) -> usize {
    let max = value.chars().count();
    (cursor + 1).min(max)
}

fn char_to_byte_index(value: &str, char_index: usize) -> usize {
    value
        .char_indices()
        .nth(char_index)
        .map(|(index, _)| index)
        .unwrap_or_else(|| value.len())
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
            runs: Vec::new(),
            run_events: Vec::new(),
            artifacts: Vec::new(),
            handoffs: Vec::new(),
            subissues: Vec::new(),
            agent_requests: Vec::new(),
            interruptions: Vec::new(),
            active_work_context: None,
            active_session_link: None,
            selected: 0,
            query: IssueQuery::default(),
            agent_roster: Vec::new(),
            dispatch_summaries: HashMap::new(),
            needs_human_count: 0,
            stale_agent_count: 0,
            open_agent_request_count: 0,
            escalated_interruptions_count: 0,
            snoozed_interruptions_count: 0,
            due_soon_interruptions_count: 0,
            next_interruption_due_label: None,
            parallel_parent_count: 0,
            parallel_subissue_count: 0,
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
        let todo = IssueDraft::new("Needs triage", "Human should see this");
        let mut running = IssueDraft::new("Agent run", "Agent is working");
        running.status = IssueStatus::AgentRunning;
        let mut review = IssueDraft::new("Needs review", "Waiting on approval");
        review.status = IssueStatus::NeedsReview;
        let mut done = IssueDraft::new("Closed loop", "Already finished");
        done.status = IssueStatus::Done;

        app.store.create_issue(&todo)?;
        app.store.create_issue(&running)?;
        app.store.create_issue(&review)?;
        app.store.create_issue(&done)?;

        app.set_saved_view(SavedView::Inbox);
        assert_eq!(app.issues.len(), 2);
        assert!(
            app.issues
                .iter()
                .all(|issue| matches!(issue.status, IssueStatus::Todo | IssueStatus::NeedsReview))
        );

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
    fn roster_surfaces_active_agent_sessions() -> Result<()> {
        let mut app = test_app()?;
        let mut issue = IssueDraft::new("Parallel worker", "Should appear in roster");
        issue.status = IssueStatus::AgentRunning;
        issue.owner_type = OwnerType::Agent;
        let issue = app.store.create_issue(&issue)?;
        app.store.set_active_session_link(
            issue.local_id,
            "agent-1",
            SessionKind::AgentSession,
            "Worker A",
        )?;
        app.store.set_active_work_context(
            issue.local_id,
            "/repo",
            Some("/repo/wt-a"),
            Some("feature/a"),
            Some("ahead 1"),
            0,
            0,
            1,
            0,
        )?;

        app.reload()?;

        assert_eq!(app.agent_roster.len(), 1);
        assert_eq!(app.agent_roster[0].identifier, issue.identifier);
        assert_eq!(app.agent_roster[0].session_label, "Worker A");
        assert_eq!(
            app.agent_roster[0].branch_name.as_deref(),
            Some("feature/a")
        );
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

    #[test]
    fn run_loop_tracks_runs_notes_and_evidence() -> Result<()> {
        let mut app = test_app()?;
        let issue = app
            .store
            .create_issue(&IssueDraft::new("Ship run", "Need execution trail"))?;
        app.reload()?;
        app.select_issue(issue.local_id);

        app.start_run()?;
        assert_eq!(app.runs.len(), 1);
        assert!(app.runs[0].status.is_active());

        app.begin_run_note_editor();
        for ch in "checked output".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;
        assert!(!app.run_events.is_empty());
        assert!(
            app.run_events
                .iter()
                .any(|event| event.message.contains("checked output"))
        );
        assert!(
            app.run_events
                .iter()
                .any(|event| event.message.contains("Run context:"))
        );

        app.begin_artifact_editor();
        for ch in "proof saved".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;
        assert_eq!(app.artifacts.len(), 1);
        assert!(app.artifacts[0].content_preview.contains("proof saved"));

        app.complete_latest_run_success()?;
        let issue = app.store.get_issue(issue.local_id)?.expect("issue missing");
        assert_eq!(issue.status, IssueStatus::NeedsReview);
        assert_eq!(app.runs[0].status, RunStatus::Succeeded);
        Ok(())
    }

    #[test]
    fn closeout_editor_persists_summary_and_follow_up() -> Result<()> {
        let mut app = test_app()?;
        let issue = app
            .store
            .create_issue(&IssueDraft::new("Close me", "Need a wrap-up"))?;
        app.reload()?;
        app.select_issue(issue.local_id);

        app.begin_closeout_editor();
        for ch in "Wrapped with notes".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
        }
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL))?;
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;

        let issue = app.store.get_issue(issue.local_id)?.expect("issue missing");
        assert_eq!(issue.status, IssueStatus::Done);
        assert_eq!(
            issue.closeout_summary.as_deref(),
            Some("Wrapped with notes")
        );
        assert!(issue.follow_up_needed);
        Ok(())
    }

    #[test]
    fn handoff_history_is_recorded_for_actor_transitions() -> Result<()> {
        let mut app = test_app()?;
        let issue = app
            .store
            .create_issue(&IssueDraft::new("Handoff trail", "Need auditability"))?;
        app.reload()?;
        app.select_issue(issue.local_id);

        app.send_current_issue_to_agent()?;
        assert_eq!(app.handoffs.len(), 1);
        assert_eq!(app.handoffs[0].to_actor, "agent");

        app.request_review()?;
        assert!(!app.handoffs.is_empty());
        assert!(
            app.handoffs
                .iter()
                .any(|handoff| handoff.note.contains("review"))
        );
        Ok(())
    }

    #[test]
    fn reopen_creates_handoff_and_restores_inbox_status() -> Result<()> {
        let mut app = test_app()?;
        let issue = app
            .store
            .create_issue(&IssueDraft::new("Reopen me", "Was closed too early"))?;
        app.reload()?;
        app.select_issue(issue.local_id);

        app.begin_closeout_editor();
        for ch in "closed for now".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;

        app.reopen_current_issue()?;
        let issue = app.store.get_issue(issue.local_id)?.expect("issue missing");
        assert_eq!(issue.status, IssueStatus::Todo);
        assert!(
            app.handoffs
                .iter()
                .any(|handoff| handoff.note.contains("reopened"))
        );
        Ok(())
    }

    #[test]
    fn work_context_and_session_link_attach_and_clear() -> Result<()> {
        let mut app = test_app()?;
        let issue = app
            .store
            .create_issue(&IssueDraft::new("Context", "Need terminal context"))?;
        app.reload()?;
        app.select_issue(issue.local_id);

        app.begin_work_context_editor();
        for _ in 0..app
            .editor
            .as_ref()
            .map(|editor| editor.title.len())
            .unwrap_or_default()
        {
            app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))?;
        }
        for ch in "/repo".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
        }
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))?;
        for _ in 0..app
            .editor
            .as_ref()
            .map(|editor| editor.description.len())
            .unwrap_or_default()
        {
            app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))?;
        }
        for ch in "/repo-wt".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
        }
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))?;
        for _ in 0..app
            .editor
            .as_ref()
            .map(|editor| editor.project.len())
            .unwrap_or_default()
        {
            app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))?;
        }
        for ch in "feature/x".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;
        assert_eq!(
            app.active_work_context
                .as_ref()
                .and_then(|ctx| ctx.branch_name.clone()),
            Some("feature/x".into())
        );

        app.begin_session_link_editor();
        for _ in 0..app
            .editor
            .as_ref()
            .map(|editor| editor.title.len())
            .unwrap_or_default()
        {
            app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))?;
        }
        for ch in "Worker".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
        }
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))?;
        for _ in 0..app
            .editor
            .as_ref()
            .map(|editor| editor.description.len())
            .unwrap_or_default()
        {
            app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))?;
        }
        for ch in "agent_session".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
        }
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))?;
        for _ in 0..app
            .editor
            .as_ref()
            .map(|editor| editor.project.len())
            .unwrap_or_default()
        {
            app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))?;
        }
        for ch in "sess-1".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;
        assert_eq!(
            app.active_session_link
                .as_ref()
                .map(|link| link.label.clone()),
            Some("Worker".into())
        );

        app.clear_work_context()?;
        app.clear_session_link()?;
        assert!(app.active_work_context.is_none());
        assert!(app.active_session_link.is_none());
        Ok(())
    }

    #[test]
    fn work_context_editor_prefills_from_current_environment() -> Result<()> {
        let mut app = test_app()?;
        let issue = app.store.create_issue(&IssueDraft::new(
            "Context prefill",
            "Should use cwd or git values",
        ))?;
        app.reload()?;
        app.select_issue(issue.local_id);

        app.begin_work_context_editor();

        let editor = app.editor.expect("editor should be open");
        assert!(!editor.title.is_empty());
        Ok(())
    }

    #[test]
    fn session_link_editor_prefills_local_terminal_defaults() -> Result<()> {
        let mut app = test_app()?;
        let issue = app.store.create_issue(&IssueDraft::new(
            "Session prefill",
            "Should use local terminal defaults",
        ))?;
        app.reload()?;
        app.select_issue(issue.local_id);

        app.begin_session_link_editor();

        let editor = app.editor.expect("editor should be open");
        assert_eq!(editor.title, "local terminal");
        assert_eq!(editor.description, "human_terminal");
        assert!(editor.project.starts_with("pid:"));
        Ok(())
    }

    #[test]
    fn normalize_command_output_trims_and_drops_empty_values() {
        assert_eq!(
            normalize_command_output("  feature/test \n"),
            Some("feature/test".into())
        );
        assert_eq!(normalize_command_output(" \n\t "), None);
    }

    #[test]
    fn agent_sessions_become_stale_after_threshold() {
        let now = Utc::now();
        assert!(is_stale_agent(
            SessionKind::AgentSession,
            IssueStatus::AgentRunning,
            now - Duration::minutes(45),
            now
        ));
        assert!(!is_stale_agent(
            SessionKind::AgentSession,
            IssueStatus::AgentRunning,
            now - Duration::minutes(5),
            now
        ));
        assert!(!is_stale_agent(
            SessionKind::HumanTerminal,
            IssueStatus::AgentRunning,
            now - Duration::minutes(45),
            now
        ));
    }

    #[test]
    fn relative_age_label_formats_recent_and_older_activity() {
        let now = Utc::now();
        assert_eq!(relative_age_label(now, now), "just now");
        assert_eq!(
            relative_age_label(now - Duration::minutes(12), now),
            "12m ago"
        );
    }

    #[test]
    fn dispatch_editor_creates_subissue() -> Result<()> {
        let mut app = test_app()?;
        let issue = app
            .store
            .create_issue(&IssueDraft::new("Parent", "Dispatch from here"))?;
        app.reload()?;
        app.select_issue(issue.local_id);

        app.begin_dispatch_editor();
        app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE))?;
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))?;
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))?;
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))?;
        for ch in "child".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;

        let subissues = app.store.list_subissues(issue.local_id)?;
        assert_eq!(subissues.len(), 1);
        assert_eq!(subissues[0].parent_id, Some(issue.local_id));
        Ok(())
    }

    #[test]
    fn dispatch_summaries_track_parallel_child_progress() -> Result<()> {
        let mut app = test_app()?;
        let parent = app
            .store
            .create_issue(&IssueDraft::new("Parent graph", "Track parallel work"))?;

        let mut ready = IssueDraft::new("Ready child", "Agent can pick this up");
        ready.parent_id = Some(parent.local_id);
        ready.status = IssueStatus::ReadyForAgent;
        app.store.create_issue(&ready)?;

        let mut review = IssueDraft::new("Review child", "Needs review");
        review.parent_id = Some(parent.local_id);
        review.status = IssueStatus::NeedsReview;
        let review = app.store.create_issue(&review)?;
        app.store.create_agent_request(
            review.local_id,
            AgentRequestKind::Review,
            "Check output",
            "Need a reviewer",
            "worker-a",
        )?;

        let mut done = IssueDraft::new("Done child", "Already finished");
        done.parent_id = Some(parent.local_id);
        done.status = IssueStatus::Done;
        app.store.create_issue(&done)?;

        app.reload()?;

        let summary = app
            .dispatch_summary_for_issue(parent.local_id)
            .expect("dispatch summary should exist");
        assert_eq!(summary.total_children, 3);
        assert_eq!(summary.ready_count, 1);
        assert_eq!(summary.review_count, 1);
        assert_eq!(summary.done_count, 1);
        assert_eq!(summary.open_request_count, 1);
        assert_eq!(app.parallel_parent_count, 1);
        assert_eq!(app.parallel_subissue_count, 3);
        Ok(())
    }

    #[test]
    fn graph_navigation_jumps_from_parent_to_most_actionable_child() -> Result<()> {
        let mut app = test_app()?;
        let parent = app
            .store
            .create_issue(&IssueDraft::new("Parent nav", "Move through graph"))?;

        let mut running = IssueDraft::new("Running child", "Still executing");
        running.parent_id = Some(parent.local_id);
        running.status = IssueStatus::AgentRunning;
        let running = app.store.create_issue(&running)?;

        let mut blocked = IssueDraft::new("Blocked child", "Needs attention first");
        blocked.parent_id = Some(parent.local_id);
        blocked.status = IssueStatus::Blocked;
        let blocked = app.store.create_issue(&blocked)?;

        app.reload()?;
        app.select_issue(parent.local_id);

        app.jump_to_next_graph_issue()?;

        assert_eq!(
            app.current_issue().map(|issue| issue.local_id),
            Some(blocked.local_id)
        );
        assert_eq!(app.saved_view, SavedView::Waiting);
        assert_ne!(running.local_id, blocked.local_id);
        Ok(())
    }

    #[test]
    fn graph_navigation_jumps_to_parent_and_next_sibling() -> Result<()> {
        let mut app = test_app()?;
        let parent = app
            .store
            .create_issue(&IssueDraft::new("Parent nav", "Move through graph"))?;

        let mut first = IssueDraft::new("First child", "Sibling one");
        first.parent_id = Some(parent.local_id);
        first.status = IssueStatus::NeedsReview;
        let first = app.store.create_issue(&first)?;

        let mut second = IssueDraft::new("Second child", "Sibling two");
        second.parent_id = Some(parent.local_id);
        second.status = IssueStatus::Done;
        let second = app.store.create_issue(&second)?;

        app.focus_issue_any_view(first.local_id)?;
        app.jump_to_parent_issue()?;
        assert_eq!(
            app.current_issue().map(|issue| issue.local_id),
            Some(parent.local_id)
        );

        app.focus_issue_any_view(first.local_id)?;
        app.jump_to_next_graph_issue()?;
        assert_eq!(
            app.current_issue().map(|issue| issue.local_id),
            Some(second.local_id)
        );
        assert_eq!(app.saved_view, SavedView::Done);
        Ok(())
    }

    #[test]
    fn parent_graph_can_approve_review_children() -> Result<()> {
        let mut app = test_app()?;
        let parent = app
            .store
            .create_issue(&IssueDraft::new("Parent approve", "Review everything here"))?;

        let mut review = IssueDraft::new("Review child", "Needs sign-off");
        review.parent_id = Some(parent.local_id);
        review.status = IssueStatus::NeedsReview;
        let review = app.store.create_issue(&review)?;
        app.store.create_agent_request(
            review.local_id,
            AgentRequestKind::Review,
            "Please review",
            "Looks good from the worker side",
            "worker-a",
        )?;

        let mut waiting = IssueDraft::new("Blocked child", "Should stay blocked");
        waiting.parent_id = Some(parent.local_id);
        waiting.status = IssueStatus::Blocked;
        let waiting = app.store.create_issue(&waiting)?;

        app.reload()?;
        app.select_issue(parent.local_id);

        app.approve_review_children()?;

        let review = app
            .store
            .get_issue(review.local_id)?
            .expect("review child missing");
        let waiting = app
            .store
            .get_issue(waiting.local_id)?
            .expect("waiting child missing");
        assert_eq!(review.status, IssueStatus::Done);
        assert_eq!(
            review.attention_reason.as_deref(),
            Some("approved from parent dispatch graph")
        );
        assert_eq!(waiting.status, IssueStatus::Blocked);
        assert!(
            app.store
                .list_agent_requests_for_issue(review.local_id)?
                .into_iter()
                .all(|request| request.status == AgentRequestStatus::Resolved)
        );
        Ok(())
    }

    #[test]
    fn parent_graph_can_requeue_stalled_children() -> Result<()> {
        let mut app = test_app()?;
        let parent = app.store.create_issue(&IssueDraft::new(
            "Parent requeue",
            "Send stalled work back out",
        ))?;

        let mut todo = IssueDraft::new("Todo child", "Should requeue");
        todo.parent_id = Some(parent.local_id);
        todo.status = IssueStatus::Todo;
        let todo = app.store.create_issue(&todo)?;

        let mut blocked = IssueDraft::new("Blocked child", "Should requeue");
        blocked.parent_id = Some(parent.local_id);
        blocked.status = IssueStatus::Blocked;
        blocked.blocked_reason = Some("needs retry".into());
        let blocked = app.store.create_issue(&blocked)?;

        let mut running = IssueDraft::new("Running child", "Should stay running");
        running.parent_id = Some(parent.local_id);
        running.status = IssueStatus::AgentRunning;
        let running = app.store.create_issue(&running)?;

        app.reload()?;
        app.select_issue(parent.local_id);

        app.requeue_stalled_children()?;

        let todo = app
            .store
            .get_issue(todo.local_id)?
            .expect("todo child missing");
        let blocked = app
            .store
            .get_issue(blocked.local_id)?
            .expect("blocked child missing");
        let running = app
            .store
            .get_issue(running.local_id)?
            .expect("running child missing");
        assert_eq!(todo.status, IssueStatus::ReadyForAgent);
        assert_eq!(blocked.status, IssueStatus::ReadyForAgent);
        assert_eq!(blocked.blocked_reason, None);
        assert_eq!(todo.owner_type, OwnerType::Agent);
        assert_eq!(running.status, IssueStatus::AgentRunning);
        Ok(())
    }

    #[test]
    fn agent_request_flow_creates_and_resolves_request() -> Result<()> {
        let mut app = test_app()?;
        let mut issue = IssueDraft::new("Agent task", "Need a structured interruption");
        issue.status = IssueStatus::AgentRunning;
        issue.owner_type = OwnerType::Agent;
        let issue = app.store.create_issue(&issue)?;
        app.reload()?;
        app.set_saved_view(SavedView::Running);
        app.select_issue(issue.local_id);

        app.begin_agent_request_editor();
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))?;
        for ch in "Need approval".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
        }
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))?;
        for _ in 0..app
            .editor
            .as_ref()
            .map(|editor| editor.project.len())
            .unwrap_or_default()
        {
            app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))?;
        }
        for ch in "review".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;

        assert_eq!(app.agent_requests.len(), 1);
        assert_eq!(app.agent_requests[0].status, AgentRequestStatus::Open);

        app.resolve_latest_agent_request()?;
        assert_eq!(app.agent_requests[0].status, AgentRequestStatus::Resolved);
        Ok(())
    }

    #[test]
    fn interruption_queue_lists_open_requests_and_resolves_selected_item() -> Result<()> {
        let mut app = test_app()?;
        let mut issue = IssueDraft::new("Queue me", "Needs interruption queue coverage");
        issue.status = IssueStatus::AgentRunning;
        issue.owner_type = OwnerType::Agent;
        let issue = app.store.create_issue(&issue)?;
        app.store.create_agent_request(
            issue.local_id,
            AgentRequestKind::Question,
            "Need a decision",
            "Should we continue?",
            "worker-a",
        )?;

        app.reload()?;
        app.set_saved_view(SavedView::Interruptions);

        assert_eq!(app.interruptions.len(), 1);
        assert_eq!(
            app.current_interruption().map(|item| item.issue.local_id),
            Some(issue.local_id)
        );

        app.resolve_latest_agent_request()?;

        assert!(app.interruptions.is_empty());
        assert!(
            app.store
                .list_agent_requests_for_issue(issue.local_id)?
                .into_iter()
                .all(|request| request.status == AgentRequestStatus::Resolved)
        );
        Ok(())
    }

    #[test]
    fn dispatch_board_lists_parent_graphs_only() -> Result<()> {
        let mut app = test_app()?;
        let parent = app
            .store
            .create_issue(&IssueDraft::new("Board parent", "Should appear on board"))?;
        let standalone = app
            .store
            .create_issue(&IssueDraft::new("Standalone", "Should not appear on board"))?;

        let mut child = IssueDraft::new("Board child", "Attach to parent");
        child.parent_id = Some(parent.local_id);
        child.status = IssueStatus::ReadyForAgent;
        app.store.create_issue(&child)?;

        app.reload()?;
        app.set_saved_view(SavedView::DispatchBoard);

        assert_eq!(app.issues.len(), 1);
        assert_eq!(app.issues[0].local_id, parent.local_id);
        assert_ne!(app.issues[0].local_id, standalone.local_id);
        Ok(())
    }

    #[test]
    fn interruption_queue_bulk_actions_ack_and_resolve_graph() -> Result<()> {
        let mut app = test_app()?;
        let parent = app.store.create_issue(&IssueDraft::new(
            "Graph root",
            "Supervise interruption graph",
        ))?;

        let mut child = IssueDraft::new("Interrupted child", "Needs follow-up");
        child.parent_id = Some(parent.local_id);
        child.status = IssueStatus::NeedsHumanInput;
        child.owner_type = OwnerType::Agent;
        let child = app.store.create_issue(&child)?;
        app.store.create_agent_request(
            parent.local_id,
            AgentRequestKind::Blocker,
            "Root blocker",
            "Need parent-level decision",
            "worker-b",
        )?;
        app.store.create_agent_request(
            child.local_id,
            AgentRequestKind::Question,
            "Need answer",
            "Should I continue?",
            "worker-a",
        )?;

        app.reload()?;
        app.set_saved_view(SavedView::Interruptions);

        app.acknowledge_interruption_and_requeue()?;
        let child = app.store.get_issue(child.local_id)?.expect("child missing");
        assert_eq!(child.status, IssueStatus::ReadyForAgent);

        app.resolve_graph_interruptions()?;
        assert!(
            app.store
                .list_agent_requests_for_issue(parent.local_id)?
                .into_iter()
                .all(|request| request.status == AgentRequestStatus::Resolved)
        );
        assert!(
            app.store
                .list_agent_requests_for_issue(child.local_id)?
                .into_iter()
                .all(|request| request.status == AgentRequestStatus::Resolved)
        );
        Ok(())
    }

    #[test]
    fn snoozed_interruptions_leave_queue_until_due() -> Result<()> {
        let mut app = test_app()?;
        let mut issue = IssueDraft::new("Snooze me", "Should disappear from queue");
        issue.status = IssueStatus::AgentRunning;
        issue.owner_type = OwnerType::Agent;
        let issue = app.store.create_issue(&issue)?;
        app.store.create_agent_request(
            issue.local_id,
            AgentRequestKind::Question,
            "Later please",
            "This can wait",
            "worker-a",
        )?;

        app.reload()?;
        app.set_saved_view(SavedView::Interruptions);
        app.snooze_selected_interruption()?;

        assert!(app.interruptions.is_empty());
        let request = app
            .store
            .list_agent_requests_for_issue(issue.local_id)?
            .into_iter()
            .next()
            .expect("request missing");
        assert!(request.snoozed_until.is_some());
        Ok(())
    }

    #[test]
    fn escalated_interruptions_sort_to_top() -> Result<()> {
        let mut app = test_app()?;
        let mut older = IssueDraft::new("Older request", "Should bubble after escalation");
        older.status = IssueStatus::AgentRunning;
        older.owner_type = OwnerType::Agent;
        let older = app.store.create_issue(&older)?;
        app.store.create_agent_request(
            older.local_id,
            AgentRequestKind::Question,
            "Older",
            "First request",
            "worker-a",
        )?;

        let mut newer = IssueDraft::new("Newer request", "Starts ahead by recency");
        newer.status = IssueStatus::AgentRunning;
        newer.owner_type = OwnerType::Agent;
        let newer = app.store.create_issue(&newer)?;
        app.store.create_agent_request(
            newer.local_id,
            AgentRequestKind::Question,
            "Newer",
            "Second request",
            "worker-b",
        )?;

        app.reload()?;
        app.set_saved_view(SavedView::Interruptions);
        assert_eq!(
            app.current_interruption().map(|item| item.issue.local_id),
            Some(newer.local_id)
        );

        app.selected = 1;
        app.escalate_selected_interruption()?;

        assert_eq!(
            app.interruptions.first().map(|item| item.issue.local_id),
            Some(older.local_id)
        );
        assert_eq!(app.interruptions[0].request.escalation_level, 1);
        Ok(())
    }

    #[test]
    fn interruption_overview_tracks_escalation_and_due_soon() -> Result<()> {
        let mut app = test_app()?;
        let mut issue = IssueDraft::new("Overview", "Sidebar interruption summary");
        issue.status = IssueStatus::AgentRunning;
        issue.owner_type = OwnerType::Agent;
        let issue = app.store.create_issue(&issue)?;
        let snoozed_request = app.store.create_agent_request(
            issue.local_id,
            AgentRequestKind::Question,
            "Soon due",
            "This will be snoozed briefly",
            "worker-a",
        )?;
        app.store
            .snooze_agent_request(snoozed_request.id, Utc::now() + Duration::minutes(20))?;
        let escalated_request = app.store.create_agent_request(
            issue.local_id,
            AgentRequestKind::Blocker,
            "Escalated",
            "This should count as escalated",
            "worker-b",
        )?;
        app.store.escalate_agent_request(escalated_request.id)?;

        app.reload()?;

        assert_eq!(app.escalated_interruptions_count, 1);
        assert_eq!(app.snoozed_interruptions_count, 1);
        assert_eq!(app.due_soon_interruptions_count, 1);
        assert!(
            app.interruption_due_summary().contains("next due: in")
                || app.interruption_due_summary().contains("next due: now")
        );
        Ok(())
    }

    #[test]
    fn graph_review_interruptions_can_be_snoozed_in_bulk() -> Result<()> {
        let mut app = test_app()?;
        let parent = app
            .store
            .create_issue(&IssueDraft::new("Root", "Graph review queue"))?;
        let mut child = IssueDraft::new("Review child", "Needs review interruption");
        child.parent_id = Some(parent.local_id);
        child.status = IssueStatus::NeedsReview;
        let child = app.store.create_issue(&child)?;
        app.store.create_agent_request(
            child.local_id,
            AgentRequestKind::Review,
            "Review me",
            "Please check this",
            "worker-a",
        )?;

        app.reload()?;
        app.set_saved_view(SavedView::Interruptions);
        app.snooze_graph_review_interruptions()?;

        let request = app
            .store
            .list_agent_requests_for_issue(child.local_id)?
            .into_iter()
            .next()
            .expect("request missing");
        assert!(request.snoozed_until.is_some());
        Ok(())
    }

    #[test]
    fn graph_blocker_interruptions_can_be_escalated_in_bulk() -> Result<()> {
        let mut app = test_app()?;
        let parent = app
            .store
            .create_issue(&IssueDraft::new("Root", "Graph blocker queue"))?;
        let mut child = IssueDraft::new("Blocked child", "Needs blocker escalation");
        child.parent_id = Some(parent.local_id);
        child.status = IssueStatus::Blocked;
        let child = app.store.create_issue(&child)?;
        app.store.create_agent_request(
            child.local_id,
            AgentRequestKind::Blocker,
            "Blocker",
            "This is blocked",
            "worker-a",
        )?;

        app.reload()?;
        app.set_saved_view(SavedView::Interruptions);
        app.escalate_graph_blockers()?;

        let request = app
            .store
            .list_agent_requests_for_issue(child.local_id)?
            .into_iter()
            .next()
            .expect("request missing");
        assert_eq!(request.escalation_level, 1);
        Ok(())
    }
}
