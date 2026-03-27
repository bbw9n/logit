use crate::{
    app::{App, EditorFocus, EditorMode},
    domain::Issue,
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

pub fn render(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(frame.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(38),
            Constraint::Percentage(42),
            Constraint::Percentage(20),
        ])
        .split(chunks[0]);

    render_issue_list(frame, app, body[0]);
    render_issue_detail(frame, app, body[1]);
    render_sidebar(frame, app, body[2]);
    render_status_bar(frame, app, chunks[1]);
    render_editor(frame, app);
}

fn render_issue_list(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let items: Vec<ListItem<'_>> = if app.issues.is_empty() {
        vec![ListItem::new("No issues match the current filter")]
    } else {
        app.issues.iter().map(issue_list_item).collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(
                    " Issues ({}) ",
                    if app.query.include_archived {
                        "active + archived"
                    } else {
                        "active"
                    }
                ))
                .borders(Borders::ALL),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    let mut state = ListState::default();
    if !app.issues.is_empty() {
        state.select(Some(app.selected));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn issue_list_item(issue: &Issue) -> ListItem<'_> {
    ListItem::new(vec![
        Line::from(vec![
            Span::styled(
                format!("{:<10}", issue.identifier),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(issue.title.clone()),
        ]),
        Line::from(vec![
            Span::styled(
                format!(" {} ", issue.status.label()),
                Style::default().fg(status_color(issue)),
            ),
            Span::raw(" "),
            Span::styled(
                format!(" {} ", issue.sync_state.badge()),
                Style::default().fg(sync_color(issue)),
            ),
            if issue.is_archived {
                Span::styled(" archived ", Style::default().fg(Color::DarkGray))
            } else {
                Span::raw("")
            },
        ]),
    ])
}

fn render_issue_detail(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let text = if let Some(issue) = app.current_issue() {
        Text::from(vec![
            Line::from(vec![
                Span::styled(&issue.identifier, Style::default().fg(Color::Yellow)),
                Span::raw("  "),
                Span::styled(
                    issue.title.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            Line::from(format!("Status: {}", issue.status.label())),
            Line::from(format!("Priority: {}", issue.priority.label())),
            Line::from(format!(
                "Assignee: {}",
                issue.assignee.as_deref().unwrap_or("unassigned")
            )),
            Line::from(format!(
                "Archived: {}",
                if issue.is_archived { "yes" } else { "no" }
            )),
            Line::from(format!("Sync: {}", issue.sync_state.badge())),
            Line::from(format!(
                "Remote: {}",
                issue.remote_id.as_deref().unwrap_or("not synced yet")
            )),
            Line::from(format!(
                "Updated: {}",
                issue.updated_at.format("%Y-%m-%d %H:%M:%S UTC")
            )),
            Line::from(""),
            Line::from("Description"),
            Line::from(issue.description.clone()),
        ])
    } else {
        Text::from("No issue selected")
    };

    let paragraph = Paragraph::new(text)
        .block(Block::default().title(" Detail ").borders(Borders::ALL))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn render_sidebar(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let help = vec![
        Line::from(vec![
            Span::styled("Workspace: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(app.config.workspace_name.clone()),
        ]),
        Line::from(vec![
            Span::styled("Mode: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(app.offline_badge()),
        ]),
        Line::from(""),
        Line::from("Keys"),
        Line::from("j/k or arrows  move"),
        Line::from("n              new issue form"),
        Line::from("e              edit issue form"),
        Line::from("s / p          cycle status / priority"),
        Line::from("a              archive or restore"),
        Line::from("v              show archived"),
        Line::from("/              search"),
        Line::from("u              clear search"),
        Line::from("f              toggle unsynced"),
        Line::from("y              sync now"),
        Line::from("r              retry errors"),
        Line::from("q              quit"),
        Line::from(""),
        Line::from(app.query_summary()),
        Line::from(app.pending_summary()),
        Line::from(format!("DB: {}", app.config.database_path.display())),
        Line::from(format!("Data dir: {}", app.config.data_dir.display())),
    ];

    let sidebar = Paragraph::new(help)
        .block(
            Block::default()
                .title(" Command Center ")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(sidebar, area);
}

fn render_status_bar(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let bar = Paragraph::new(app.status_message.as_str())
        .style(Style::default().fg(Color::Black).bg(Color::White))
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(bar, area);
}

fn render_editor(frame: &mut Frame, app: &App) {
    let Some(editor) = &app.editor else {
        return;
    };

    let popup = centered_rect(70, 55, frame.area());
    frame.render_widget(Clear, popup);

    let (title, body) = match &editor.mode {
        EditorMode::Search => (
            " Search ",
            vec![
                Line::from("Type search text and press Enter."),
                Line::from("Esc cancels."),
                Line::from(""),
                Line::from(format!("query: {}", editor.search)),
            ],
        ),
        EditorMode::Create => (
            " New Issue ",
            issue_editor_lines(editor, "Create a fully local issue. Tab moves fields."),
        ),
        EditorMode::Edit { .. } => (
            " Edit Issue ",
            issue_editor_lines(editor, "Edit local issue fields. Tab moves fields."),
        ),
    };

    let paragraph = Paragraph::new(body)
        .block(Block::default().title(title).borders(Borders::ALL))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, popup);
}

fn issue_editor_lines(editor: &crate::app::EditorState, intro: &str) -> Vec<Line<'static>> {
    vec![
        Line::from(intro.to_string()),
        Line::from("Enter saves. Esc cancels. s/p cycle status and priority."),
        Line::from(""),
        field_line("title", editor.focus, EditorFocus::Title, &editor.title),
        field_line(
            "description",
            editor.focus,
            EditorFocus::Description,
            &editor.description,
        ),
        field_line(
            "assignee",
            editor.focus,
            EditorFocus::Assignee,
            &editor.assignee,
        ),
        Line::from(format!("status: {}", editor.status.label())),
        Line::from(format!("priority: {}", editor.priority.label())),
    ]
}

fn field_line(
    label: &str,
    current: EditorFocus,
    target: EditorFocus,
    value: &str,
) -> Line<'static> {
    let prefix = if matches_focus(current, target) {
        ">"
    } else {
        " "
    };
    let content = if value.is_empty() { "(empty)" } else { value };
    Line::from(format!("{prefix} {label}: {content}"))
}

fn matches_focus(current: EditorFocus, target: EditorFocus) -> bool {
    matches!(
        (current, target),
        (EditorFocus::Title, EditorFocus::Title)
            | (EditorFocus::Description, EditorFocus::Description)
            | (EditorFocus::Assignee, EditorFocus::Assignee)
    )
}

fn centered_rect(
    percent_x: u16,
    percent_y: u16,
    area: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn status_color(issue: &Issue) -> Color {
    match issue.status {
        crate::domain::IssueStatus::Todo => Color::Gray,
        crate::domain::IssueStatus::InProgress => Color::Blue,
        crate::domain::IssueStatus::Done => Color::Green,
    }
}

fn sync_color(issue: &Issue) -> Color {
    match issue.sync_state {
        crate::domain::SyncState::Synced => Color::Green,
        crate::domain::SyncState::PendingCreate | crate::domain::SyncState::PendingUpdate => {
            Color::Yellow
        }
        crate::domain::SyncState::SyncError => Color::Red,
        crate::domain::SyncState::Conflict => Color::Magenta,
    }
}
