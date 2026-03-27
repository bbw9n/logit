use crate::{app::App, domain::Issue};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
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
                    if app.unsynced_only { "unsynced" } else { "all" }
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
        Line::from("n              new issue"),
        Line::from("s              cycle status"),
        Line::from("e              edit title"),
        Line::from("d              delete issue"),
        Line::from("y              sync now"),
        Line::from("r              retry errors"),
        Line::from("/              toggle filter"),
        Line::from("q              quit"),
        Line::from(""),
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
