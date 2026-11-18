use super::*;
use crate::app::app_events::{
    ChildWindowUpdateKind, publish_child_window_git_operation, publish_child_window_update,
};
use codux_runtime::git::GitReviewFile;

mod branches;
mod clone;
mod commit;
mod comparison;
mod network;
mod review;
mod runner;
mod stash_tags;

pub(in crate::app) use comparison::{ReviewComparisonState, git_comparison_endpoints};

pub(in crate::app) fn merge_git_review_status_files(
    review: &mut GitReviewSummary,
    git: &GitSummary,
) {
    if review.mode == "taskBranch" {
        return;
    }
    let mut seen = review
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect::<HashSet<_>>();

    for file in &git.changed_files {
        if file.path.trim().is_empty() || file.path.ends_with('/') || seen.contains(&file.path) {
            continue;
        }
        let status = if file.index_status.trim() == "?" {
            "added"
        } else if !file.index_status.trim().is_empty() && file.index_status.trim() != "?" {
            "staged"
        } else if !file.worktree_status.trim().is_empty() {
            "modified"
        } else {
            continue;
        };
        seen.insert(file.path.clone());
        review.files.push(GitReviewFile {
            path: file.path.clone(),
            old_path: file.old_path.clone(),
            status: status.to_string(),
            additions: 0,
            deletions: 0,
        });
    }
}

fn git_error_needs_credentials(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("credential")
        || normalized.contains("authentication")
        || normalized.contains("auth")
        || normalized.contains("username")
        || normalized.contains("password")
        || normalized.contains("permission denied")
        || normalized.contains("access denied")
}
