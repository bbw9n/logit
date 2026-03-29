# logit

`logit` is a Rust TUI for local-first work tracking inspired by Linear.

The current version is optimized around offline terminal workflows first: work through an inbox, track richer human/agent-facing issue states, capture scratch notes before they become formal issues, organize work with projects and labels, search, archive, and switch between saved views in the terminal. The Linear sync boundary exists in code, but real remote GraphQL integration is not finished yet.

## Current Status

What works today:

- Local issue creation and editing
- Terminal-native issue states like `ready for agent`, `agent running`, `needs review`, and `blocked`
- Scratch capture and promotion into full issues
- Local run tracking with run notes and evidence snippets
- SQLite-backed persistence
- Search across identifier, title, description, project, and labels
- Project and label organization
- Archive and restore flows
- Saved views for inbox, running, review, waiting, done, and scratch work
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

## Usage

### Navigation

- `j` / `k` or arrow keys: move through the issue list
- `q`: quit
- `?`: open help overlay
- `Esc`: close help or cancel the active modal

### Issue Workflows

- `n`: create a new issue
- `e`: edit the selected issue
- `x`: capture a scratch item
- `i`: promote the selected scratch item into a full issue
- `Enter`: save the current modal
- `Tab` / `Shift+Tab`: move between modal fields
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
- `a`: archive or restore the selected issue

### Views And Search

- `1`: inbox view
- `2`: running view
- `3`: review view
- `4`: waiting view
- `5`: done view
- `6`: scratch view
- `v`: toggle archived visibility in the current view
- `/`: open search
- `u`: clear search
- `f`: toggle unsynced-only filter

### Sync Actions

- `y`: attempt sync
- `r`: retry failed sync states

Note:

- Sync is still placeholder behavior right now. Without `LINEAR_API_KEY`, sync attempts mark queued items as failed. With a token set, the app exercises the sync path, but it does not yet perform real Linear API mutations.

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
- [src/domain.rs](src/domain.rs): issue, scratch, queue, and query types
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
3. Replace the placeholder sync layer with real Linear GraphQL integration.
