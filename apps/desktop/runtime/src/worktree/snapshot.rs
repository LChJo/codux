use crate::git::{GitService, GitSummary};

use super::{
    scan::{ScannedTask, ScannedWorktree},
    types::{ProjectWorktreeGitSummary, ProjectWorktreeSnapshot, WorktreeTaskSnapshot},
};

pub(super) fn scanned_worktree_to_snapshot(worktree: ScannedWorktree) -> ProjectWorktreeSnapshot {
    project_worktree_snapshot(worktree)
}

pub(super) fn scanned_task_to_snapshot(task: ScannedTask) -> WorktreeTaskSnapshot {
    WorktreeTaskSnapshot {
        worktree_id: task.worktree_id,
        title: task.title,
        base_branch: task.base_branch,
        base_commit: task.base_commit,
        status: task.status,
        created_at: task.created_at,
        updated_at: task.updated_at,
        started_at: task.started_at,
        completed_at: task.completed_at,
    }
}

pub(super) fn project_worktree_snapshot(worktree: ScannedWorktree) -> ProjectWorktreeSnapshot {
    let git_summary = project_worktree_git_summary_light(&worktree.path, None);
    ProjectWorktreeSnapshot {
        id: worktree.id,
        project_id: worktree.project_id,
        name: worktree.name,
        branch: worktree.branch,
        path: worktree.path,
        status: worktree.status,
        is_default: worktree.is_default,
        created_at: worktree.created_at,
        updated_at: worktree.updated_at,
        git_summary,
    }
}

/// Live change/ahead/behind counts without the review diff. `additions` and
/// `deletions` need a full diff, so reuse the last known values from `cached`
/// — the worktree scan runs for every worktree on each refresh, and paying the
/// diff cost per worktree dominated the refresh time on large repositories.
pub(super) fn project_worktree_git_summary_light(
    path: &str,
    cached: Option<&ProjectWorktreeGitSummary>,
) -> ProjectWorktreeGitSummary {
    let status_snapshot = GitService::status_counts(path);
    let (additions, deletions) = cached
        .map(|summary| (summary.additions, summary.deletions))
        .unwrap_or_default();
    ProjectWorktreeGitSummary {
        changes: status_snapshot.staged + status_snapshot.unstaged + status_snapshot.untracked,
        incoming: status_snapshot.behind,
        outgoing: status_snapshot.ahead,
        additions,
        deletions,
    }
}

pub(super) fn project_worktree_git_summary_from_status(
    status_snapshot: &GitSummary,
) -> ProjectWorktreeGitSummary {
    ProjectWorktreeGitSummary {
        changes: status_snapshot.staged + status_snapshot.unstaged + status_snapshot.untracked,
        incoming: status_snapshot.behind,
        outgoing: status_snapshot.ahead,
        additions: status_snapshot.additions,
        deletions: status_snapshot.deletions,
    }
}
