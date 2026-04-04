mod app;
mod config;
mod domain;
mod store;
mod sync;
mod ui;

use anyhow::{Context, Result, anyhow};
use app::App;
use config::WorkspaceConfig;
use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use domain::{RunEventLevel, RunKind, RunStatus, SessionKind};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{collections::HashMap, io, time::Duration};
use store::Store;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if matches!(args.get(1).map(String::as_str), Some("hook")) {
        return run_hook_cli(&args[2..]);
    }
    let mut terminal = setup_terminal()?;
    let result = run(&mut terminal);
    restore_terminal(&mut terminal)?;
    result
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut app = App::bootstrap()?;

    loop {
        terminal.draw(|frame| ui::render(frame, &app))?;

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            if app.handle_key(key)? {
                break;
            }
        }
    }

    Ok(())
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run_hook_cli(args: &[String]) -> Result<()> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(anyhow!(
            "missing hook command. try: start-run, finish-run, note, heartbeat"
        ));
    };

    let options = parse_flag_options(&args[1..])?;
    let config = WorkspaceConfig::load()?;
    let store = Store::open(&config.database_path)?;

    match command {
        "start-run" => hook_start_run(&store, &options),
        "finish-run" => hook_finish_run(&store, &options),
        "note" => hook_note(&store, &options),
        "heartbeat" => hook_heartbeat(&store, &options),
        other => Err(anyhow!("unknown hook command: {other}")),
    }
}

fn hook_start_run(store: &Store, options: &HashMap<String, String>) -> Result<()> {
    let issue = resolve_issue(store, required_option(options, "issue")?)?;

    if let Some(repo_path) = options.get("repo-path") {
        let _ = store.set_active_work_context(
            issue.local_id,
            repo_path,
            options.get("worktree-path").map(String::as_str),
            options.get("branch").map(String::as_str),
            options.get("git-status").map(String::as_str),
            0,
            0,
            0,
            0,
        )?;
    }

    if let Some(session_ref) = options.get("session-ref") {
        let session_kind = options
            .get("session-kind")
            .map(|value| parse_session_kind(value))
            .transpose()?
            .unwrap_or(SessionKind::BackgroundJob);
        let label = options
            .get("session-label")
            .map(String::as_str)
            .unwrap_or(session_ref.as_str());
        let _ = store.set_active_session_link(issue.local_id, session_ref, session_kind, label)?;
    }

    let run = store.create_run(
        issue.local_id,
        options
            .get("kind")
            .map(|value| parse_run_kind(value))
            .transpose()?
            .unwrap_or(RunKind::Manual),
        options.get("summary").map(String::as_str),
        options.get("session-ref").map(String::as_str),
    )?;
    println!("started run {} for {}", run.id, issue.identifier);
    Ok(())
}

fn hook_finish_run(store: &Store, options: &HashMap<String, String>) -> Result<()> {
    let issue = resolve_issue(store, required_option(options, "issue")?)?;
    let run = store
        .latest_active_run_for_issue(issue.local_id)?
        .with_context(|| format!("no active run found for {}", issue.identifier))?;
    let status = options
        .get("status")
        .map(|value| parse_run_status(value))
        .transpose()?
        .unwrap_or(RunStatus::Succeeded);
    let run = store.complete_run(
        run.id,
        status,
        options.get("summary").map(String::as_str),
        options
            .get("exit-code")
            .map(|value| value.parse::<i64>())
            .transpose()
            .context("invalid exit-code")?,
    )?;
    println!("finished run {} as {}", run.id, run.status.label());
    Ok(())
}

fn hook_note(store: &Store, options: &HashMap<String, String>) -> Result<()> {
    let issue = resolve_issue(store, required_option(options, "issue")?)?;
    let run = store
        .latest_active_run_for_issue(issue.local_id)?
        .with_context(|| format!("no active run found for {}", issue.identifier))?;
    let event = store.append_run_event(
        run.id,
        options
            .get("level")
            .map(|value| parse_run_event_level(value))
            .transpose()?
            .unwrap_or(RunEventLevel::Info),
        required_option(options, "message")?,
    )?;
    println!("attached {} note to run {}", event.level.label(), run.id);
    Ok(())
}

fn hook_heartbeat(store: &Store, options: &HashMap<String, String>) -> Result<()> {
    let issue = resolve_issue(store, required_option(options, "issue")?)?;
    let session_ref = required_option(options, "session-ref")?;
    let session_kind = options
        .get("session-kind")
        .map(|value| parse_session_kind(value))
        .transpose()?
        .unwrap_or(SessionKind::BackgroundJob);
    let label = options
        .get("session-label")
        .map(String::as_str)
        .unwrap_or(session_ref);
    let session =
        store.set_active_session_link(issue.local_id, session_ref, session_kind, label)?;
    println!(
        "heartbeat recorded for {} on {}",
        session.label, issue.identifier
    );
    Ok(())
}

fn parse_flag_options(args: &[String]) -> Result<HashMap<String, String>> {
    let mut options = HashMap::new();
    let mut index = 0usize;
    while index < args.len() {
        let key = args[index]
            .strip_prefix("--")
            .ok_or_else(|| anyhow!("expected flag starting with --, got {}", args[index]))?;
        let value = args
            .get(index + 1)
            .ok_or_else(|| anyhow!("missing value for --{key}"))?;
        options.insert(key.to_string(), value.clone());
        index += 2;
    }
    Ok(options)
}

fn required_option<'a>(options: &'a HashMap<String, String>, key: &str) -> Result<&'a str> {
    options
        .get(key)
        .map(String::as_str)
        .with_context(|| format!("missing required --{key}"))
}

fn resolve_issue(store: &Store, issue_ref: &str) -> Result<domain::Issue> {
    if let Ok(local_id) = issue_ref.parse::<i64>() {
        return store
            .get_issue(local_id)?
            .with_context(|| format!("issue {issue_ref} not found"));
    }

    store
        .get_issue_by_identifier(issue_ref)?
        .with_context(|| format!("issue {issue_ref} not found"))
}

fn parse_run_kind(value: &str) -> Result<RunKind> {
    match value {
        "manual" => Ok(RunKind::Manual),
        "agent" => Ok(RunKind::Agent),
        "shell" => Ok(RunKind::Shell),
        "script" => Ok(RunKind::Script),
        other => Err(anyhow!("unknown run kind: {other}")),
    }
}

fn parse_run_status(value: &str) -> Result<RunStatus> {
    match value {
        "queued" => Ok(RunStatus::Queued),
        "running" => Ok(RunStatus::Running),
        "succeeded" => Ok(RunStatus::Succeeded),
        "failed" => Ok(RunStatus::Failed),
        "cancelled" => Ok(RunStatus::Cancelled),
        other => Err(anyhow!("unknown run status: {other}")),
    }
}

fn parse_session_kind(value: &str) -> Result<SessionKind> {
    match value {
        "human_terminal" => Ok(SessionKind::HumanTerminal),
        "agent_session" => Ok(SessionKind::AgentSession),
        "background_job" => Ok(SessionKind::BackgroundJob),
        other => Err(anyhow!("unknown session kind: {other}")),
    }
}

fn parse_run_event_level(value: &str) -> Result<RunEventLevel> {
    match value {
        "info" => Ok(RunEventLevel::Info),
        "warn" => Ok(RunEventLevel::Warn),
        "error" => Ok(RunEventLevel::Error),
        other => Err(anyhow!("unknown event level: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_flag_options_reads_pairs() -> Result<()> {
        let args = vec![
            "--issue".to_string(),
            "LOCAL-1".to_string(),
            "--status".to_string(),
            "succeeded".to_string(),
        ];
        let options = parse_flag_options(&args)?;
        assert_eq!(options.get("issue").map(String::as_str), Some("LOCAL-1"));
        assert_eq!(options.get("status").map(String::as_str), Some("succeeded"));
        Ok(())
    }

    #[test]
    fn resolve_issue_accepts_identifier() -> Result<()> {
        let store = Store::open_in_memory()?;
        let issue =
            store.create_issue(&domain::IssueDraft::new("Hook issue", "Created for hook"))?;
        let resolved = resolve_issue(&store, &issue.identifier)?;
        assert_eq!(resolved.local_id, issue.local_id);
        Ok(())
    }

    #[test]
    fn parse_run_status_accepts_failed() -> Result<()> {
        assert_eq!(parse_run_status("failed")?, RunStatus::Failed);
        Ok(())
    }
}
