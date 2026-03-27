# logit

`logit` is a Rust TUI for local-first issue tracking inspired by Linear.

The current version is optimized around offline workflows first: browse issues, create and edit them locally, organize work with projects and labels, search, archive, and switch between saved views in the terminal. The Linear sync boundary exists in code, but real remote GraphQL integration is not finished yet.

## Current Status

What works today:

- Local issue creation and editing
- SQLite-backed persistence
- Search across identifier, title, description, project, and labels
- Project and label organization
- Archive and restore flows
- Saved views for active, unsynced, and archived work
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
- `Enter`: save the current modal
- `Tab` / `Shift+Tab`: move between modal fields
- `s`: cycle issue status
- `p`: cycle issue priority
- `a`: archive or restore the selected issue

### Views And Search

- `1`: active issues view
- `2`: unsynced issues view
- `3`: archived issues view
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

## Architecture

High-level structure:

- [src/main.rs](src/main.rs): terminal bootstrap and event loop
- [src/app.rs](src/app.rs): app state and keyboard workflows
- [src/ui.rs](src/ui.rs): layout, panes, modals, help overlay
- [src/store.rs](src/store.rs): SQLite CRUD, query, archive, queue logic
- [src/domain.rs](src/domain.rs): issue and query types
- [src/sync.rs](src/sync.rs): sync boundary and placeholder service

## Tests

The current test suite covers:

- CRUD round trips
- Search and filtering
- Archive visibility rules
- Project and label persistence
- Mutation queue cleanup
- Sync-state transitions
- Placeholder sync success and failure paths

Run:

```bash
cargo test --locked
```

## Roadmap

Near-term directions:

1. Improve local editor UX with richer text input and more obvious field affordances.
2. Add stronger local planning workflows like notes, saved filters, and better sort/group options.
3. Replace the placeholder sync layer with real Linear GraphQL integration.
