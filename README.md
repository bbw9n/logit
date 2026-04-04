# logit

`logit` is a Rust TUI for local-first work tracking inspired by Linear.

The current version is optimized around offline terminal workflows first: work through an inbox, track richer human/agent-facing issue states, capture scratch notes before they become formal issues, organize work with projects and labels, search, archive, and switch between saved views in the terminal. The Linear sync boundary exists in code, but real remote GraphQL integration is not finished yet.

## Current Status

What works today:

- Local issue creation and editing
- Terminal-native issue states like `ready for agent`, `agent running`, `needs review`, and `blocked`
- Scratch capture and promotion into full issues
- Local run tracking with run notes and evidence snippets
- Active worktree and session context per issue
- Git snapshot context on attached worktrees
- Parent/sub-issue dispatch for parallel work
- Parallel dispatch summaries for parent issues and sibling progress
- Structured agent requests attached to issues
- Closeout summaries with follow-up tracking
- Handoff history and reopen flow
- SQLite-backed persistence
- Search across identifier, title, description, project, and labels
- Project and label organization
- Archive and restore flows
- Saved views for inbox, running, review, waiting, done, and scratch work
- Structured interruption queue for open agent requests
- Parent-only dispatch board for supervising active graphs
- Default home screen centered on the inbox
- Lightweight active-agent roster derived from session/worktree/run state
- Offline mutation queue and tested sync boundary behavior
- GitHub Actions CI for format, check, and test

What is not finished yet:

- Real Linear GraphQL API sync
- Rich form widgets and multi-line editing UX
- Comments, attachments, cycles, projects as first-class screens

## Build And Run

Requirements:

- Rust stable
- Cargo

Run locally:

```bash
cargo run
```

Check and test:

```bash
cargo check --locked
cargo test --locked
```

Hook-oriented execution updates:

```bash
cargo run -- hook start-run --issue LOCAL-1 --kind agent --summary "worker started" --session-ref worker-a --session-kind agent_session --session-label "Worker A"
cargo run -- hook note --issue LOCAL-1 --message "checkpoint reached" --level info
cargo run -- hook finish-run --issue LOCAL-1 --status succeeded --summary "worker finished cleanly"
```

## Usage

### Navigation

- `j` / `k` or arrow keys: move through the issue list
- `q`: quit
- `?`: open help overlay
- `Esc`: close help or cancel the active modal

### Issue Workflows

- `n`: create a new issue
- `e`: edit the selected issue
- `D`: dispatch a sub-issue from the selected issue
- `R`: open a structured agent request
- `Q`: resolve the latest open agent request
- `P`: jump from a child issue to its parent
- `C`: move through the current dispatch graph
- `V`: approve all review-ready child issues from a parent graph
- `J`: requeue stalled child issues back to agents from a parent graph
- `A`: acknowledge the selected interruption and requeue its issue to an agent
- `E`: resolve all open interruptions across the selected dispatch graph
- `S`: snooze the selected interruption for 30 minutes
- `X`: escalate the selected interruption to the top of the queue
- `H`: snooze all review interruptions in the selected graph
- `B`: escalate all blocker interruptions in the selected graph
- `x`: capture a scratch item
- `i`: promote the selected scratch item into a full issue
- `Enter`: save the current modal
- `Tab` / `Shift+Tab`: move between modal fields
- `Left` / `Right` / `Home` / `End`: move the cursor inside the active modal field
- `Ctrl+J` or `Shift+Enter`: insert a newline inside the active modal field
- `s`: cycle issue status
- `p`: cycle issue priority
- `h`: send the selected issue to an agent
- `m`: mark the selected issue as needing human input
- `w`: mark the selected issue as needing review
- `b`: mark the selected issue as blocked
- `t`: start a local run for the selected issue
- `g`: mark the latest active run as succeeded
- `z`: mark the latest active run as failed
- `l`: attach a note to the latest active run
- `o`: attach an evidence note to the selected issue
- `c`: close the selected issue with a summary
- `Shift+O`: reopen the selected done issue into the inbox
- `a`: archive or restore the selected issue
- `]`: attach repo/worktree/branch context to the selected issue
- `[`: attach session context to the selected issue
- `}`: clear the active work context
- `{`: clear the active session link

### Views And Search

- `1`: inbox view
- `2`: running view
- `3`: review view
- `4`: waiting view
- `5`: done view
- `6`: scratch view
- `7`: interruption queue
- `8`: dispatch board
- `v`: toggle archived visibility in the current view
- `/`: open search
- `u`: clear search
- `f`: toggle unsynced-only filter

### Sync Actions

- `y`: attempt sync
- `r`: retry failed sync states

### Hook Commands

`logit` also exposes a small non-TUI execution hook surface for shells, wrappers, and agent runtimes:

- `logit hook start-run --issue LOCAL-1 [--kind agent|manual|shell|script] [--summary ...] [--session-ref ...] [--session-kind ...] [--session-label ...] [--repo-path ...] [--worktree-path ...] [--branch ...] [--git-status ...]`
- `logit hook finish-run --issue LOCAL-1 [--status succeeded|failed|cancelled] [--summary ...] [--exit-code ...]`
- `logit hook note --issue LOCAL-1 --message ... [--level info|warn|error]`
- `logit hook heartbeat --issue LOCAL-1 --session-ref ... [--session-kind ...] [--session-label ...]`

These commands write directly into the same SQLite store as the TUI, so hook-driven execution shows up in runs, session state, and the supervision surfaces automatically.

Note:

- Sync is still placeholder behavior right now. Without `LINEAR_API_KEY`, sync attempts mark queued items as failed. With a token set, the app exercises the sync path, but it does not yet perform real Linear API mutations.

### Terminal-Native Context

Each issue can carry the local execution context where work is happening:

- Work context:
  - repo path
  - worktree path
  - branch name
- Session link:
  - label
  - kind: `human_terminal`, `agent_session`, or `background_job`
  - session reference

This is local-first metadata for terminal coordination. It is useful for coding workflows with multiple worktrees and for agent-driven tasks where the active worker lives in a shell session.

When you open the work-context modal, `logit` now tries to prefill:

- repo root from `git rev-parse --show-toplevel`
- current branch from `git branch --show-current`
- current working directory as the worktree path when it differs from the repo root
- lightweight git status snapshot counts from `git status --porcelain --branch`

Outside a git repo, it falls back gracefully to your current working directory.

When you open the session-link modal without an existing session attached, it prefills a local terminal session with:

- label: `local terminal`
- kind: `human_terminal`
- session ref: `pid:<current-process-id>`

When you start a run with `t`, `logit` now also tries to:

- ensure there is an active work context and session link
- store the active session reference on the run record
- append a run note summarizing the current repo/branch/git snapshot/session context

The inbox is intentionally human-attention-first: it prioritizes items in `todo`, `needs human input`, `needs review`, and `blocked`, while the sidebar shows a compact roster of active agent sessions and their current branches.

## Agentic Control Loop

`logit` now supports a tighter supervision loop for parallel work:

- Dispatch:
  - split a selected issue into a child sub-issue with `D`
  - child issues keep a `parent_id` link so parallel work stays attached to the original outcome
- Interrupt:
  - raise a structured agent request with `R`
  - requests can be `question`, `review`, or `blocker`
  - requests update the parent issue’s attention state so they surface in the inbox/review/waiting views
- Resolve:
  - resolve the latest open request with `Q`
  - request history stays attached to the issue for later review

The detail pane shows parent/root status, dispatched sub-issues, and structured agent requests alongside runs, evidence, and handoffs.

The inbox and sidebar also surface a compact parallel-work summary, so parent issues are easier to scan as coordination nodes rather than plain tasks.

The interruption queue gives humans a dedicated place to process open agent questions, blockers, and review requests without relying on the currently selected issue.
It now supports lightweight triage too: snoozed interruptions drop out of the queue until they are due again, and escalated interruptions bubble to the top.
The sidebar now mirrors that urgency with interruption summary lines for open, escalated, snoozed, due-soon, and next-due state.

The dispatch board is the graph-first counterpart: it shows parent issues with active child graphs so a human can supervise and rebalance parallel work from one place.

You can now move through a dispatch graph directly from the terminal:

- `P` jumps from a child issue back to its parent/root
- `C` jumps from a parent to the most actionable child, or from a child to the next sibling in the graph
- `V` approves all review-ready children from the selected parent issue
- `J` requeues stalled children (`todo`, `needs human input`, `blocked`) back to `ready for agent`
- `A` acknowledges the selected interruption and sends that issue back to an agent
- `E` resolves all open interruptions across the selected graph
- `S` snoozes the selected interruption for 30 minutes
- `X` escalates the selected interruption to the top of the queue
- `H` snoozes all review interruptions in the selected graph
- `B` escalates all blocker interruptions in the selected graph

## Data Storage

`logit` stores local state in a SQLite database under your local app data directory.

On macOS this is typically:

```text
~/Library/Application Support/logit/logit.db
```

The exact path is determined in [src/config.rs](src/config.rs) using the platform-local data directory.

Optional environment variable:

- `LINEAR_API_KEY`: enables the current sync code path

Optional config file:

- `~/.config/logit/config.toml`

Example:

```toml
workspace_name = "Personal Workspace"
theme = "nord"
```

If `theme` is omitted, `logit` falls back to your terminal’s native colors.

Available theme presets:

- `nord`
- `sunset`
- `forest`

## Architecture

High-level structure:

- [src/main.rs](src/main.rs): terminal bootstrap and event loop
- [src/app.rs](src/app.rs): app state and keyboard workflows
- [src/ui.rs](src/ui.rs): layout, panes, modals, help overlay
- [src/store.rs](src/store.rs): SQLite CRUD, query, archive, queue logic
- [src/domain.rs](src/domain.rs): issue, scratch, run, handoff, work-context, and query types
- [src/sync.rs](src/sync.rs): sync boundary and placeholder service

## Tests

The current test suite covers:

- CRUD round trips
- Search and filtering
- Archive visibility rules
- Project and label persistence
- Scratch capture and promotion
- Inbox view filtering by terminal-native issue state
- Run lifecycle, run notes, and evidence capture
- Work context and session-link persistence
- Git snapshot and run session-ref persistence
- Parent/sub-issue and agent-request persistence
- Closeout summaries and follow-up flags
- Handoff history and reopen transitions
- Mutation queue cleanup
- Sync-state transitions
- Placeholder sync success and failure paths

Run:

```bash
cargo test --locked
```

## Roadmap

Near-term directions:

1. Add richer local workflows like handoffs, run history, and evidence-based closure.
2. Improve local editor UX with richer text input and more obvious field affordances.
3. Add lightweight git/session autofill on top of the new work-context model.
4. Replace the placeholder sync layer with real Linear GraphQL integration.
