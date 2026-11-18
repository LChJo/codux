#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum GitComparisonRequest {
    Staged,
    Unstaged,
    Branch {
        base: String,
        target: String,
        direct: bool,
    },
    Commit {
        commit: String,
        parent_index: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GitComparisonRange {
    WorkingTree,
    Staged {
        head: Option<String>,
    },
    Unstaged,
    Trees {
        base: Option<String>,
        target: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitComparison {
    pub range: GitComparisonRange,
    pub files: Vec<GitReviewFile>,
    pub commit: Option<GitCommitDetails>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitDetails {
    pub id: String,
    pub message: String,
    pub author: String,
    pub timestamp: String,
    pub parents: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHistoryPage {
    pub commits: Vec<GitCommitSummary>,
    pub has_more: bool,
}

const MAX_COMPARISON_FILE_BYTES: usize = 2 * 1024 * 1024;

impl GitService {
    pub fn comparison(
        project_path: &str,
        request: GitComparisonRequest,
    ) -> Result<GitComparison, String> {
        let repo = open_git_repository(project_path)?;
        let mut commit_details = None;
        let range = match request {
            GitComparisonRequest::Staged => GitComparisonRange::Staged {
                head: comparison_head(&repo)?,
            },
            GitComparisonRequest::Unstaged => GitComparisonRange::Unstaged,
            GitComparisonRequest::Branch {
                base,
                target,
                direct,
            } => {
                let base = comparison_commit(&repo, &base)?.id();
                let target = comparison_commit(&repo, &target)?.id();
                let base = if direct {
                    base
                } else {
                    repo.merge_base(base, target)
                        .map_err(|error| error.to_string())?
                };
                GitComparisonRange::Trees {
                    base: Some(base.to_string()),
                    target: target.to_string(),
                }
            }
            GitComparisonRequest::Commit {
                commit,
                parent_index,
            } => {
                let selected = comparison_commit(&repo, &commit)?;
                let parents = selected
                    .parent_ids()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>();
                let base = if parents.is_empty() && parent_index == 0 {
                    None
                } else {
                    Some(
                        parents
                            .get(parent_index)
                            .ok_or("Invalid commit parent")?
                            .clone(),
                    )
                };
                let timestamp = chrono::DateTime::from_timestamp(selected.time().seconds(), 0)
                    .map(|time| time.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                    .unwrap_or_default();
                commit_details = Some(GitCommitDetails {
                    id: selected.id().to_string(),
                    message: String::from_utf8_lossy(selected.message_bytes()).into_owned(),
                    author: selected.author().name().unwrap_or_default().to_string(),
                    timestamp,
                    parents,
                });
                GitComparisonRange::Trees {
                    base,
                    target: selected.id().to_string(),
                }
            }
        };
        let mut options = git2::DiffOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .show_untracked_content(true);
        let mut diff = match &range {
            GitComparisonRange::WorkingTree => {
                let tree = head_tree(&repo).ok();
                repo.diff_tree_to_workdir_with_index(tree.as_ref(), Some(&mut options))
            }
            GitComparisonRange::Staged { head } => {
                let tree = head
                    .as_deref()
                    .map(|id| resolve_commit_tree(&repo, id))
                    .transpose()?;
                repo.diff_tree_to_index(tree.as_ref(), None, Some(&mut options))
            }
            GitComparisonRange::Unstaged => repo.diff_index_to_workdir(None, Some(&mut options)),
            GitComparisonRange::Trees { base, target } => {
                let base = base
                    .as_deref()
                    .map(|id| resolve_commit_tree(&repo, id))
                    .transpose()?;
                let target = resolve_commit_tree(&repo, target)?;
                repo.diff_tree_to_tree(base.as_ref(), Some(&target), Some(&mut options))
            }
        }
        .map_err(|error| error.to_string())?;
        let _ = diff.find_similar(None);
        Ok(GitComparison {
            range,
            files: review_files_from_diff(&diff)?,
            commit: commit_details,
        })
    }

    pub fn comparison_file(
        project_path: &str,
        range: GitComparisonRange,
        file: GitReviewFile,
    ) -> Result<GitReviewContentSummary, String> {
        let repo = open_git_repository(project_path)?;
        let path = safe_git_path(&file.path)?;
        let old_path = safe_git_path(file.old_path.as_deref().unwrap_or(&path))?;
        if path.is_empty() || old_path.is_empty() {
            return Err("File path cannot be empty.".to_string());
        }
        let (old, current) = match range {
            GitComparisonRange::WorkingTree => {
                let head = comparison_head(&repo)?;
                (
                    comparison_tree_file(&repo, head.as_deref(), &old_path)?,
                    comparison_worktree_file(repo_root(&repo), &path)?,
                )
            }
            GitComparisonRange::Staged { head } => (
                comparison_tree_file(&repo, head.as_deref(), &old_path)?,
                comparison_index_file(&repo, &path)?,
            ),
            GitComparisonRange::Unstaged => (
                comparison_index_file(&repo, &old_path)?,
                comparison_worktree_file(repo_root(&repo), &path)?,
            ),
            GitComparisonRange::Trees { base, target } => (
                comparison_tree_file(&repo, base.as_deref(), &old_path)?,
                comparison_tree_file(&repo, Some(&target), &path)?,
            ),
        };
        let mut options = git2::DiffOptions::new();
        options.context_lines(0);
        // The code view renders logical lines. Match those lines even when Git
        // stores LF in the index and checks out CRLF into the worktree.
        let old_diff = comparison_display_text(&old);
        let current_diff = comparison_display_text(&current);
        let patch = git2::Patch::from_buffers(
            old_diff.as_bytes(),
            Some(Path::new(&old_path)),
            current_diff.as_bytes(),
            Some(Path::new(&path)),
            Some(&mut options),
        )
        .map_err(|error| error.to_string())?;
        let mut added_lines = Vec::new();
        let mut deleted_lines = Vec::new();
        for hunk in 0..patch.num_hunks() {
            let (_, count) = patch.hunk(hunk).map_err(|error| error.to_string())?;
            for index in 0..count {
                let line = patch
                    .line_in_hunk(hunk, index)
                    .map_err(|error| error.to_string())?;
                match line.origin() {
                    '+' => added_lines.extend(line.new_lineno().map(|number| number as usize)),
                    '-' => deleted_lines.extend(line.old_lineno().map(|number| number as usize)),
                    _ => {}
                }
            }
        }
        drop(patch);
        drop(old_diff);
        drop(current_diff);
        Ok(GitReviewContentSummary {
            path,
            head_content: old,
            worktree_content: current,
            added_lines,
            deleted_lines,
            is_repository: true,
            ..Default::default()
        })
    }

    pub fn history_page(
        project_path: &str,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<GitHistoryPage, String> {
        let repo = open_git_repository(project_path)?;
        let mut walk = repo.revwalk().map_err(|error| error.to_string())?;
        walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)
            .map_err(|error| error.to_string())?;
        if let Err(error) = walk.push_head() {
            return if matches!(
                error.code(),
                git2::ErrorCode::UnbornBranch | git2::ErrorCode::NotFound
            ) {
                Ok(GitHistoryPage {
                    commits: Vec::new(),
                    has_more: false,
                })
            } else {
                Err(error.to_string())
            };
        }
        let query = query.trim().to_lowercase();
        let limit = limit.clamp(1, 100);
        let tags = git2_tags_by_commit(&repo);
        let mut matched = 0;
        let mut commits = Vec::new();
        for oid in walk {
            let oid = oid.map_err(|error| error.to_string())?;
            let commit = repo.find_commit(oid).map_err(|error| error.to_string())?;
            let hash = oid.to_string();
            let title = commit
                .summary()
                .ok()
                .flatten()
                .unwrap_or_default()
                .to_string();
            let author = commit.author().name().unwrap_or_default().to_string();
            if !query.is_empty()
                && !hash.starts_with(&query)
                && !title.to_lowercase().contains(&query)
                && !author.to_lowercase().contains(&query)
            {
                continue;
            }
            if matched < offset {
                matched += 1;
                continue;
            }
            if commits.len() == limit {
                return Ok(GitHistoryPage {
                    commits,
                    has_more: true,
                });
            }
            commits.push(GitCommitSummary {
                hash,
                title,
                author,
                relative_time: relative_git_time(commit.time().seconds()),
                decorations: tags.get(&oid).map(|names| names.join(",")),
                graph_prefix: String::new(),
            });
        }
        Ok(GitHistoryPage {
            commits,
            has_more: false,
        })
    }
}

fn comparison_display_text(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains("\r\n") {
        std::borrow::Cow::Owned(text.replace("\r\n", "\n"))
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

fn comparison_commit<'repo>(
    repo: &'repo GitRepository,
    reference: &str,
) -> Result<git2::Commit<'repo>, String> {
    repo.revparse_single(reference)
        .and_then(|object| object.peel_to_commit())
        .map_err(|error| error.to_string())
}

fn comparison_head(repo: &GitRepository) -> Result<Option<String>, String> {
    match repo.head() {
        Ok(reference) => reference
            .peel_to_commit()
            .map(|commit| Some(commit.id().to_string()))
            .map_err(|error| error.to_string()),
        Err(error)
            if matches!(
                error.code(),
                git2::ErrorCode::UnbornBranch | git2::ErrorCode::NotFound
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error.to_string()),
    }
}

fn comparison_tree_file(
    repo: &GitRepository,
    reference: Option<&str>,
    path: &str,
) -> Result<String, String> {
    let Some(reference) = reference else {
        return Ok(String::new());
    };
    let tree = resolve_commit_tree(repo, reference)?;
    match tree.get_path(Path::new(path)) {
        Ok(entry) => comparison_blob(repo, entry.id()),
        Err(error) if error.code() == git2::ErrorCode::NotFound => Ok(String::new()),
        Err(error) => Err(error.to_string()),
    }
}

fn comparison_index_file(repo: &GitRepository, path: &str) -> Result<String, String> {
    let index = repo.index().map_err(|error| error.to_string())?;
    if let Some(entry) = index.get_path(Path::new(path), 0) {
        return comparison_blob(repo, entry.id);
    }
    if (1..=3).any(|stage| index.get_path(Path::new(path), stage).is_some()) {
        return Err("This file has unresolved merge conflicts.".to_string());
    }
    Ok(String::new())
}

fn comparison_blob(repo: &GitRepository, id: git2::Oid) -> Result<String, String> {
    let blob = repo
        .find_blob(id)
        .map_err(|_| "Binary or submodule content cannot be previewed.".to_string())?;
    if blob.is_binary() {
        return Err("Binary file cannot be previewed.".to_string());
    }
    comparison_text(blob.content())
}

fn comparison_worktree_file(root: &Path, path: &str) -> Result<String, String> {
    use std::io::Read as _;

    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let full = root.join(path);
    let metadata = match fs::symlink_metadata(&full) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(error.to_string()),
    };
    let parent = full
        .parent()
        .ok_or("Missing parent directory")?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !parent.starts_with(&root) {
        return Err("File path escapes repository root.".to_string());
    }
    if metadata.is_symlink() {
        return fs::read_link(&full)
            .map(|target| target.to_string_lossy().into_owned())
            .map_err(|error| error.to_string());
    }
    if !metadata.is_file() {
        return Err("Binary or submodule content cannot be previewed.".to_string());
    }
    let file = fs::File::open(&full).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take((MAX_COMPARISON_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    comparison_text(&bytes)
}

fn comparison_text(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > MAX_COMPARISON_FILE_BYTES {
        return Err("File exceeds the 2 MiB preview limit.".to_string());
    }
    if bytes.contains(&0) {
        return Err("Binary file cannot be previewed.".to_string());
    }
    Ok(String::from_utf8_lossy(bytes).into_owned())
}
