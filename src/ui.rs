use crate::{
    app::{App, EditorFocus, EditorMode},
    config::ThemePreset,
    domain::Issue,
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

#[derive(Clone, Copy)]
struct Palette {
    bg: Color,
    panel: Color,
    panel_alt: Color,
    border: Color,
    title: Color,
    accent: Color,
    soft: Color,
    muted: Color,
    success_bg: Color,
    warn_bg: Color,
    error_bg: Color,
    info_bg: Color,
    select_bg: Color,
    select_fg: Color,
    white: Color,
    todo: Color,
    progress: Color,
    done: Color,
    none_priority: Color,
    low_priority: Color,
    medium_priority: Color,
    high_priority: Color,
    urgent_priority: Color,
    conflict: Color,
}

fn palette(theme: Option<ThemePreset>) -> Palette {
    match theme {
        None => Palette {
            bg: Color::Reset,
            panel: Color::Reset,
            panel_alt: Color::Reset,
            border: Color::DarkGray,
            title: Color::Cyan,
            accent: Color::Yellow,
            soft: Color::Reset,
            muted: Color::Gray,
            success_bg: Color::Black,
            warn_bg: Color::Black,
            error_bg: Color::Black,
            info_bg: Color::Black,
            select_bg: Color::Cyan,
            select_fg: Color::Black,
            white: Color::White,
            todo: Color::Gray,
            progress: Color::Blue,
            done: Color::Green,
            none_priority: Color::Gray,
            low_priority: Color::Green,
            medium_priority: Color::Yellow,
            high_priority: Color::LightRed,
            urgent_priority: Color::Red,
            conflict: Color::Magenta,
        },
        Some(ThemePreset::Nord) => Palette {
            bg: Color::Rgb(76, 86, 106),                // nord3
            panel: Color::Rgb(67, 76, 94),              // nord2
            panel_alt: Color::Rgb(59, 66, 82),          // nord1
            border: Color::Rgb(76, 86, 106),            // nord3
            title: Color::Rgb(136, 192, 208),           // nord8
            accent: Color::Rgb(235, 203, 139),          // nord13
            soft: Color::Rgb(229, 233, 240),            // nord5
            muted: Color::Rgb(216, 222, 233),           // nord4
            success_bg: Color::Rgb(59, 66, 82),         // nord1 chip background
            warn_bg: Color::Rgb(59, 66, 82),            // nord1 chip background
            error_bg: Color::Rgb(59, 66, 82),           // nord1 chip background
            info_bg: Color::Rgb(59, 66, 82),            // nord1 chip background
            select_bg: Color::Rgb(136, 192, 208),       // nord8
            select_fg: Color::Rgb(46, 52, 64),          // nord0
            white: Color::Rgb(236, 239, 244),           // nord6
            todo: Color::Rgb(216, 222, 233),            // nord4
            progress: Color::Rgb(129, 161, 193),        // nord9
            done: Color::Rgb(163, 190, 140),            // nord14
            none_priority: Color::Rgb(216, 222, 233),   // nord4
            low_priority: Color::Rgb(143, 188, 187),    // nord7
            medium_priority: Color::Rgb(235, 203, 139), // nord13
            high_priority: Color::Rgb(208, 135, 112),   // nord12
            urgent_priority: Color::Rgb(191, 97, 106),  // nord11
            conflict: Color::Rgb(180, 142, 173),        // nord15
        },
        Some(ThemePreset::Sunset) => Palette {
            bg: Color::Rgb(31, 16, 18),
            panel: Color::Rgb(47, 24, 28),
            panel_alt: Color::Rgb(58, 31, 35),
            border: Color::Rgb(145, 91, 82),
            title: Color::Rgb(255, 184, 108),
            accent: Color::Rgb(255, 225, 138),
            soft: Color::Rgb(239, 214, 205),
            muted: Color::Rgb(184, 141, 131),
            success_bg: Color::Rgb(68, 50, 52),
            warn_bg: Color::Rgb(68, 50, 52),
            error_bg: Color::Rgb(68, 50, 52),
            info_bg: Color::Rgb(68, 50, 52),
            select_bg: Color::Rgb(173, 92, 77),
            select_fg: Color::Rgb(255, 247, 240),
            white: Color::Rgb(255, 247, 240),
            todo: Color::Rgb(196, 169, 160),
            progress: Color::Rgb(255, 166, 92),
            done: Color::Rgb(122, 214, 146),
            none_priority: Color::Rgb(184, 141, 131),
            low_priority: Color::Rgb(122, 214, 146),
            medium_priority: Color::Rgb(255, 196, 87),
            high_priority: Color::Rgb(255, 138, 76),
            urgent_priority: Color::Rgb(255, 92, 122),
            conflict: Color::Rgb(214, 143, 222),
        },
        Some(ThemePreset::Forest) => Palette {
            bg: Color::Rgb(16, 24, 18),
            panel: Color::Rgb(24, 36, 27),
            panel_alt: Color::Rgb(31, 45, 33),
            border: Color::Rgb(84, 108, 88),
            title: Color::Rgb(146, 220, 166),
            accent: Color::Rgb(240, 214, 112),
            soft: Color::Rgb(194, 212, 196),
            muted: Color::Rgb(128, 150, 131),
            success_bg: Color::Rgb(41, 54, 44),
            warn_bg: Color::Rgb(41, 54, 44),
            error_bg: Color::Rgb(41, 54, 44),
            info_bg: Color::Rgb(41, 54, 44),
            select_bg: Color::Rgb(73, 113, 84),
            select_fg: Color::Rgb(245, 251, 245),
            white: Color::Rgb(245, 251, 245),
            todo: Color::Rgb(170, 180, 171),
            progress: Color::Rgb(90, 184, 170),
            done: Color::Rgb(110, 223, 144),
            none_priority: Color::Rgb(128, 150, 131),
            low_priority: Color::Rgb(110, 223, 144),
            medium_priority: Color::Rgb(240, 214, 112),
            high_priority: Color::Rgb(232, 156, 82),
            urgent_priority: Color::Rgb(236, 107, 112),
            conflict: Color::Rgb(169, 153, 230),
        },
    }
}

pub fn render(frame: &mut Frame, app: &App) {
    let palette = palette(app.config.theme);
    if palette.bg != Color::Reset {
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.bg)),
            frame.area(),
        );
    }

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

    render_issue_list(frame, app, body[0], palette);
    render_issue_detail(frame, app, body[1], palette);
    render_sidebar(frame, app, body[2], palette);
    render_status_bar(frame, app, chunks[1], palette);
    render_editor(frame, app, palette);
    render_help(frame, app, palette);
}

fn render_issue_list(frame: &mut Frame, app: &App, area: ratatui::layout::Rect, palette: Palette) {
    let query_summary = app.query_summary();
    let items: Vec<ListItem<'_>> = if app.issues.is_empty() {
        vec![ListItem::new(empty_state_copy(app))]
    } else {
        app.issues
            .iter()
            .map(|issue| issue_list_item(issue, palette))
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(styled_title("Issues", &query_summary, palette))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(palette.border))
                .style(Style::default().bg(palette.panel)),
        )
        .highlight_style(
            Style::default()
                .fg(palette.select_fg)
                .bg(palette.select_bg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("  ");

    let mut state = ListState::default();
    if !app.issues.is_empty() {
        state.select(Some(app.selected));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn issue_list_item(issue: &Issue, palette: Palette) -> ListItem<'_> {
    let status = status_color(issue, palette);
    ListItem::new(vec![
        Line::from(vec![
            Span::styled("● ", Style::default().fg(status)),
            Span::styled(
                format!("{:<10}", issue.identifier),
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(issue.title.clone()),
        ]),
        Line::from(vec![
            badge(issue.status.label(), status, badge_bg(status, palette)),
            Span::raw(" "),
            badge(
                issue.sync_state.badge(),
                palette.white,
                badge_bg(sync_color(issue, palette), palette),
            ),
            if issue.is_archived {
                Span::styled(
                    " archived ",
                    Style::default()
                        .fg(palette.muted)
                        .add_modifier(Modifier::ITALIC),
                )
            } else {
                Span::raw("")
            },
        ]),
    ])
}

fn render_issue_detail(
    frame: &mut Frame,
    app: &App,
    area: ratatui::layout::Rect,
    palette: Palette,
) {
    let text = if let Some(issue) = app.current_issue() {
        let labels = if issue.labels.is_empty() {
            "none".to_string()
        } else {
            issue.labels.join(", ")
        };
        let updated = issue.updated_at.format("%Y-%m-%d %H:%M:%S UTC").to_string();
        Text::from(vec![
            Line::from(vec![
                Span::styled(&issue.identifier, Style::default().fg(palette.accent)),
                Span::raw("  "),
                Span::styled(
                    issue.title.clone(),
                    Style::default()
                        .fg(palette.white)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            meta_line("Status", issue.status.label(), palette),
            meta_line("Priority", issue.priority.label(), palette),
            meta_line(
                "Project",
                issue.project.as_deref().unwrap_or("none"),
                palette,
            ),
            meta_line("Labels", labels, palette),
            meta_line(
                "Assignee",
                issue.assignee.as_deref().unwrap_or("unassigned"),
                palette,
            ),
            meta_line(
                "Archived",
                if issue.is_archived { "yes" } else { "no" },
                palette,
            ),
            meta_line("Sync", issue.sync_state.badge(), palette),
            meta_line(
                "Remote",
                issue.remote_id.as_deref().unwrap_or("not synced yet"),
                palette,
            ),
            meta_line("Updated", updated, palette),
            Line::from(""),
            Line::from(Span::styled(
                "Description",
                Style::default()
                    .fg(palette.title)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                issue.description.clone(),
                Style::default().fg(palette.soft),
            )),
        ])
    } else {
        Text::from(Span::styled(
            "No issue selected",
            Style::default().fg(palette.muted),
        ))
    };

    let paragraph = Paragraph::new(text)
        .block(
            Block::default()
                .title(styled_title("Detail", "selected issue", palette))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(palette.border))
                .style(Style::default().bg(palette.panel_alt)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn render_sidebar(frame: &mut Frame, app: &App, area: ratatui::layout::Rect, palette: Palette) {
    let help = vec![
        Line::from(vec![
            Span::styled(
                "Workspace ",
                Style::default()
                    .fg(palette.title)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                app.config.workspace_name.clone(),
                Style::default().fg(palette.white),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "Mode ",
                Style::default()
                    .fg(palette.title)
                    .add_modifier(Modifier::BOLD),
            ),
            badge(app.offline_badge(), palette.white, palette.info_bg),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Keys",
            Style::default()
                .fg(palette.title)
                .add_modifier(Modifier::BOLD),
        )),
        key_line("j/k or arrows", "move", palette),
        key_line("n", "new issue form", palette),
        key_line("e", "edit issue form", palette),
        key_line("s / p", "cycle status / priority", palette),
        key_line("a", "archive or restore", palette),
        key_line("v", "show archived", palette),
        key_line("1 / 2 / 3", "active / unsynced / archived", palette),
        key_line("/", "search", palette),
        key_line("u", "clear search", palette),
        key_line("f", "toggle unsynced", palette),
        key_line("?", "help overlay", palette),
        key_line("y", "sync now", palette),
        key_line("r", "retry errors", palette),
        key_line("q", "quit", palette),
        Line::from(""),
        Line::from(Span::styled(
            "Context",
            Style::default()
                .fg(palette.title)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            app.query_summary(),
            Style::default().fg(palette.soft),
        )),
        Line::from(Span::styled(
            app.pending_summary(),
            Style::default().fg(palette.soft),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Storage",
            Style::default()
                .fg(palette.title)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("DB: {}", app.config.database_path.display()),
            Style::default().fg(palette.muted),
        )),
        Line::from(Span::styled(
            format!("Data dir: {}", app.config.data_dir.display()),
            Style::default().fg(palette.muted),
        )),
        Line::from(Span::styled(
            format!(
                "Theme: {}",
                app.config
                    .theme
                    .map(|theme| theme.as_str())
                    .unwrap_or("terminal")
            ),
            Style::default().fg(palette.muted),
        )),
    ];

    let sidebar = Paragraph::new(help)
        .block(
            Block::default()
                .title(styled_title("Command Center", "workflow map", palette))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(palette.border))
                .style(Style::default().bg(palette.panel)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(sidebar, area);
}

fn render_status_bar(frame: &mut Frame, app: &App, area: ratatui::layout::Rect, palette: Palette) {
    let bar = Paragraph::new(app.status_message.as_str())
        .style(
            Style::default()
                .fg(palette.white)
                .add_modifier(Modifier::BOLD),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(palette.border)),
        );
    frame.render_widget(bar, area);
}

fn render_editor(frame: &mut Frame, app: &App, palette: Palette) {
    let Some(editor) = &app.editor else {
        return;
    };

    let popup = centered_rect(70, 55, frame.area());
    frame.render_widget(Clear, popup);

    let (title, body) = match &editor.mode {
        EditorMode::Search => (
            " Search ",
            vec![
                Line::from(Span::styled(
                    "Type search text and press Enter.",
                    Style::default().fg(palette.soft),
                )),
                Line::from(Span::styled(
                    "Esc cancels.",
                    Style::default().fg(palette.muted),
                )),
                Line::from(""),
                field_line(
                    "query",
                    EditorFocus::Title,
                    EditorFocus::Title,
                    &editor.search,
                    palette,
                ),
            ],
        ),
        EditorMode::Create => (
            " New Issue ",
            issue_editor_lines(
                editor,
                "Create a fully local issue. Tab moves fields.",
                palette,
            ),
        ),
        EditorMode::Edit { .. } => (
            " Edit Issue ",
            issue_editor_lines(
                editor,
                "Edit local issue fields. Tab moves fields.",
                palette,
            ),
        ),
    };

    let paragraph = Paragraph::new(body)
        .block(
            Block::default()
                .title(Span::styled(
                    title,
                    Style::default()
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(palette.accent))
                .style(Style::default().bg(palette.panel_alt)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, popup);
}

fn render_help(frame: &mut Frame, app: &App, palette: Palette) {
    if !app.show_help {
        return;
    }

    let popup = centered_rect(72, 62, frame.area());
    frame.render_widget(Clear, popup);
    let body = vec![
        Line::from("Basic loop"),
        Line::from("1. Move with j/k or arrow keys."),
        Line::from("2. Press n to create or e to edit."),
        Line::from("3. Use Tab to move fields, Enter to save, Esc to cancel."),
        Line::from("4. Organize with project, labels, assignee, status, and priority."),
        Line::from("5. Search with / and switch saved views with 1, 2, and 3."),
        Line::from(""),
        Line::from("Views"),
        Line::from("1 active issues"),
        Line::from("2 unsynced issues"),
        Line::from("3 archived issues"),
        Line::from("v show or hide archived alongside active issues"),
        Line::from("f quick unsynced toggle"),
        Line::from(""),
        Line::from("Issue actions"),
        Line::from("s cycle status"),
        Line::from("p cycle priority"),
        Line::from("a archive or restore selected issue"),
        Line::from("y attempt sync"),
        Line::from("r retry failed sync states"),
        Line::from(""),
        Line::from("Press ? or Esc to close this help."),
    ];
    let widget = Paragraph::new(body)
        .block(
            Block::default()
                .title(Span::styled(
                    " Help ",
                    Style::default()
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(palette.accent))
                .style(Style::default().bg(palette.panel_alt)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(widget, popup);
}

fn issue_editor_lines(
    editor: &crate::app::EditorState,
    intro: &str,
    palette: Palette,
) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            intro.to_string(),
            Style::default().fg(palette.soft),
        )),
        Line::from(Span::styled(
            "Enter saves. Esc cancels. Ctrl+S/Ctrl+P cycle status and priority.",
            Style::default().fg(palette.muted),
        )),
        Line::from(""),
        field_line(
            "title",
            editor.focus,
            EditorFocus::Title,
            &editor.title,
            palette,
        ),
        field_line(
            "description",
            editor.focus,
            EditorFocus::Description,
            &editor.description,
            palette,
        ),
        field_line(
            "project",
            editor.focus,
            EditorFocus::Project,
            &editor.project,
            palette,
        ),
        field_line(
            "labels",
            editor.focus,
            EditorFocus::Labels,
            &editor.labels,
            palette,
        ),
        field_line(
            "assignee",
            editor.focus,
            EditorFocus::Assignee,
            &editor.assignee,
            palette,
        ),
        Line::from(vec![
            Span::styled("status ", Style::default().fg(palette.title)),
            badge(
                editor.status.label(),
                palette.white,
                badge_bg(
                    status_color_for_label(editor.status.label(), palette),
                    palette,
                ),
            ),
        ]),
        Line::from(vec![
            Span::styled("priority ", Style::default().fg(palette.title)),
            badge(
                editor.priority.label(),
                palette.white,
                badge_bg(priority_color(editor.priority.label(), palette), palette),
            ),
        ]),
    ]
}

fn field_line(
    label: &str,
    current: EditorFocus,
    target: EditorFocus,
    value: &str,
    palette: Palette,
) -> Line<'static> {
    let prefix = if matches_focus(current, target) {
        ">"
    } else {
        " "
    };
    let content = if value.is_empty() { "(empty)" } else { value };
    Line::from(vec![
        Span::styled(
            format!("{prefix} "),
            Style::default().fg(if matches_focus(current, target) {
                palette.accent
            } else {
                palette.muted
            }),
        ),
        Span::styled(
            format!("{label}: "),
            Style::default()
                .fg(palette.title)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(content.to_string(), Style::default().fg(palette.soft)),
    ])
}

fn matches_focus(current: EditorFocus, target: EditorFocus) -> bool {
    matches!(
        (current, target),
        (EditorFocus::Title, EditorFocus::Title)
            | (EditorFocus::Description, EditorFocus::Description)
            | (EditorFocus::Project, EditorFocus::Project)
            | (EditorFocus::Labels, EditorFocus::Labels)
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

fn empty_state_copy(app: &App) -> &'static str {
    if app.query.archived_only {
        "No archived issues in this view. Press n to create one or 1 to go back to active work."
    } else if app.query.unsynced_only {
        "No unsynced issues right now. Press n to create a local issue or 1 to browse all active work."
    } else {
        "No active issues yet. Press n to create your first local issue."
    }
}

fn status_color(issue: &Issue, palette: Palette) -> Color {
    match issue.status {
        crate::domain::IssueStatus::Todo => palette.todo,
        crate::domain::IssueStatus::InProgress => palette.progress,
        crate::domain::IssueStatus::Done => palette.done,
    }
}

fn sync_color(issue: &Issue, palette: Palette) -> Color {
    match issue.sync_state {
        crate::domain::SyncState::Synced => palette.done,
        crate::domain::SyncState::PendingCreate | crate::domain::SyncState::PendingUpdate => {
            palette.medium_priority
        }
        crate::domain::SyncState::SyncError => palette.urgent_priority,
        crate::domain::SyncState::Conflict => palette.conflict,
    }
}

fn styled_title<'a>(label: &'a str, detail: &'a str, palette: Palette) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!(" {label} "),
            Style::default()
                .fg(palette.title)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(detail.to_string(), Style::default().fg(palette.muted)),
    ])
}

fn badge<'a>(text: &'a str, fg: Color, bg: Color) -> Span<'a> {
    Span::styled(
        format!(" {text} "),
        Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
    )
}

fn badge_bg(color: Color, palette: Palette) -> Color {
    if color == palette.done {
        palette.success_bg
    } else if color == palette.medium_priority {
        palette.warn_bg
    } else if color == palette.urgent_priority {
        palette.error_bg
    } else if color == palette.progress {
        palette.info_bg
    } else {
        palette.panel_alt
    }
}

fn key_line<'a>(keys: &'a str, action: &'a str, palette: Palette) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!("{keys:<14}"),
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(action.to_string(), Style::default().fg(palette.soft)),
    ])
}

fn meta_line(label: &str, value: impl Into<String>, palette: Palette) -> Line<'static> {
    let value = value.into();
    Line::from(vec![
        Span::styled(
            format!("{label}: "),
            Style::default()
                .fg(palette.title)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(value, Style::default().fg(palette.soft)),
    ])
}

fn status_color_for_label(label: &str, palette: Palette) -> Color {
    match label {
        "todo" => palette.todo,
        "in progress" => palette.progress,
        "done" => palette.done,
        _ => palette.soft,
    }
}

fn priority_color(label: &str, palette: Palette) -> Color {
    match label {
        "none" => palette.none_priority,
        "low" => palette.low_priority,
        "medium" => palette.medium_priority,
        "high" => palette.high_priority,
        "urgent" => palette.urgent_priority,
        _ => palette.soft,
    }
}
