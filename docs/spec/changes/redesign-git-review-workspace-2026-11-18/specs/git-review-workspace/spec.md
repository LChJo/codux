## ADDED Requirements

### Requirement: Review Workspace Layout

The desktop app SHALL arrange the visible review columns from left to right as Git sidebar, changed files, and file diff. Entering review SHALL open the Git sidebar and reclaim the space occupied by the column that contains Worktree and Favorite Commands. Leaving review SHALL restore the destination workspace's column visibility. The review SHALL use the existing Codux GPUI theme, controls, focus indicators, and resizable panels. The diff SHALL support side-by-side and unified layouts.

#### Scenario: Enter review from the terminal workspace

- **GIVEN** the terminal workspace displays the column containing Worktree and Favorite Commands and the Git sidebar is closed
- **WHEN** the user opens review
- **THEN** the Git sidebar, changed files, and file diff SHALL appear in that order
- **AND** the Worktree and Favorite Commands column and its divider SHALL occupy no layout space in review
- **AND** the terminal workspace's original column visibility SHALL be restored when the user returns

### Requirement: Independent Git Sidebar

The desktop app SHALL provide an independent Git sidebar toggle in the terminal, file, and review workspaces. A sidebar opened outside review SHALL remain open when switching between the terminal and file workspaces until the user closes it. Review SHALL reuse the same sidebar. Review entry SHALL open it temporarily when needed, and a toggle inside review SHALL affect the current review session without changing the user's visibility preference outside review.

#### Scenario: Preserve the sidebar preference across workspace changes

- **GIVEN** the user opened the Git sidebar in the terminal workspace
- **WHEN** the user moves through the file and review workspaces
- **THEN** each workspace SHALL display one Git sidebar
- **AND** temporarily closing it in review SHALL leave more room for the changed files and diff
- **AND** returning to the file or terminal workspace SHALL show the sidebar again

#### Scenario: Restore a previously closed sidebar

- **GIVEN** the Git sidebar is closed outside review
- **WHEN** the user opens review and then returns to the previous workspace
- **THEN** the sidebar SHALL be visible upon entering review
- **AND** it SHALL be closed again upon returning

### Requirement: Source Control and History Navigation

The Git sidebar SHALL make working changes and history directly accessible. It SHALL place branch and sync context near the top and group files as staged, unstaged, and untracked. When the worktree is clean, the sidebar SHALL show one compact status with direct access to history. History SHALL support paging and filtering by commit subject, author, or ID within the displayed query scope. Switching to history and back SHALL preserve the commit draft and any still-valid change selection.

#### Scenario: Navigate from a clean worktree to a historical change

- **WHEN** the Git sidebar receives a successful status with no changes
- **THEN** it SHALL show a clean status and offer a history entry without requiring scrolling past empty change groups
- **AND** clicking a history row or activating it with the keyboard SHALL open that commit in the review workspace
- **AND** the review SHALL show the full commit message, author, timestamp, commit ID, and changed files without changing the checkout or index
- **AND** selecting a changed file SHALL open its diff in the adjacent pane

### Requirement: File Name and Path Legibility

Git status and review file rows SHALL prioritize the file name within the available width after required row controls are laid out. A row with no directory SHALL reserve no width for one. A populated directory SHALL use space left after the file name and SHALL truncate or disappear before the file name truncates. The file name MAY use an ellipsis when the remaining space is too narrow to display it in full.

#### Scenario: Show full names for files in the repository root

- **GIVEN** the list displays root-level `package.json`, `package-lock.json`, and `README.md`
- **WHEN** the row width can accommodate each complete name and its required controls
- **THEN** all three file names SHALL be fully legible
- **AND** no row SHALL reserve space for an empty directory label

#### Scenario: Give the file name priority as the pane narrows

- **GIVEN** a row contains a long parent directory and the file `package-lock.json`
- **WHEN** the user narrows the pane
- **THEN** the parent directory SHALL truncate or disappear before the file name truncates
- **AND** the row SHALL retain its status and any required controls

### Requirement: Full File Details on Hover

Hovering anywhere on a Git file row SHALL display the full file name, repository-relative path, and a localized status. Paths SHALL wrap as needed so they remain readable within the window. The detail SHALL include additions and deletions when those counts are available, and SHALL show both old and new paths when a rename is known. The detail SHALL use the existing row data and SHALL perform no Git or file-system I/O when shown. Mouse exit, list scrolling, and view changes SHALL dismiss the detail. Clicking the review file row SHALL continue to open its diff.

#### Scenario: Inspect a renamed file with an abbreviated row

- **GIVEN** a rename row has truncated its directory and already carries the original and destination paths and line counts
- **WHEN** the user hovers over the row's icon, name, unused row space, or status badge
- **THEN** the detail SHALL display both complete paths, the rename status, and the available line counts
- **AND** showing the detail SHALL initiate no background read
- **AND** scrolling the list SHALL dismiss the detail

#### Scenario: Distinguish identically named files in a narrow pane

- **GIVEN** two `index.ts` rows are in different deeply nested directories and their directory labels are abbreviated
- **WHEN** the user hovers over each row
- **THEN** each detail SHALL show its complete repository-relative path without clipping
- **AND** clicking one of the review rows SHALL open that row's diff

### Requirement: Explicit Review Comparison

The review SHALL label its active comparison and bind the files and diff to that comparison. Uncommitted review SHALL identify the worktree and distinguish `HEAD → index` from `index → worktree`. Branch review SHALL default to the common ancestor to target and allow a direct base-to-target comparison. It SHALL display both endpoints in either mode. Commit review SHALL compare the selected parent to the selected commit, starting with the first parent for a merge commit and using an empty tree for a root commit.

#### Scenario: Review an old commit in a dirty worktree

- **GIVEN** the current worktree contains local changes
- **WHEN** the user opens an older history entry
- **THEN** the changed files and file diff SHALL be derived from the selected parent and historical commit
- **AND** the working changes SHALL remain available through the uncommitted review

#### Scenario: Change the comparison parent of a merge commit

- **GIVEN** the selected history entry is a merge commit
- **WHEN** the user selects a different parent
- **THEN** the comparison label, changed files, and file diff SHALL reflect the newly selected parent

### Requirement: Review Loading and Result States

The review SHALL show distinct states for loading, errors, an empty comparison, an empty filter result, and a nonempty file list with no active file selection. A file diff SHALL show its own loading and error states. Refreshing the same comparison SHALL keep the prior list visible while loading and preserve the selected file and scroll position when that selection remains valid. Asynchronous results SHALL be applied only to the active repository, comparison, and file request.

#### Scenario: Distinguish an empty working tree from a loading or failed review

- **WHEN** the current comparison returns a successful result with no files
- **THEN** the workspace SHALL describe the active comparison's empty state
- **AND** an empty working tree SHALL be labeled as clean
- **AND** a pending or failed request SHALL use its corresponding loading or error state

#### Scenario: Select a second file while a diff is loading

- **GIVEN** a file diff is loading
- **WHEN** the user selects another file and the first result arrives afterward
- **THEN** the diff pane SHALL continue to show the second file's loading state or result
