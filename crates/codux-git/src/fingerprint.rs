use crate::GitService;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GitChangeFingerprint {
    pub scope: String,
    pub fingerprint: String,
}

use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

impl GitService {
    /// Content identity for uncommitted changes, independent of staging state.
    /// File content is hashed by libgit2's streaming blob hasher. Index and HEAD
    /// blobs already have immutable content identities, so no diff truncation or
    /// object materialization is involved. No Git objects or files are written.
    pub fn change_fingerprint(project_path: &str) -> Result<Option<GitChangeFingerprint>, String> {
        let repo = crate::open_git_repository(project_path)?;
        let root = repo
            .workdir()
            .ok_or("Git worktree is required for change fingerprinting.")?;
        let mut options = git2::StatusOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false)
            .include_unmodified(false)
            .renames_head_to_index(false)
            .renames_index_to_workdir(false)
            .exclude_submodules(false)
            .update_index(false);
        let statuses = repo
            .statuses(Some(&mut options))
            .map_err(|e| e.to_string())?;
        if statuses.is_empty() {
            return Ok(None);
        }
        let index = repo.index().map_err(|e| e.to_string())?;
        let head = match repo.head() {
            Ok(head) => Some(head.peel_to_tree().map_err(|e| e.to_string())?),
            Err(e)
                if matches!(
                    e.code(),
                    git2::ErrorCode::UnbornBranch | git2::ErrorCode::NotFound
                ) =>
            {
                None
            }
            Err(e) => return Err(e.to_string()),
        };
        let mut changes: BTreeMap<Vec<u8>, (String, BTreeSet<String>)> = BTreeMap::new();
        for entry in statuses.iter() {
            let status = entry.status();
            if status.is_ignored() || status.is_empty() {
                continue;
            }
            let bytes = entry.path_bytes();
            let path = relative_path(bytes)?;
            let baseline = match head.as_ref().map(|tree| tree.get_path(&path)) {
                Some(Ok(entry)) => version(entry.filemode() as u32, entry.id()),
                Some(Err(e)) if e.code() != git2::ErrorCode::NotFound => return Err(e.to_string()),
                _ => "missing".to_string(),
            };
            let versions = &mut changes
                .entry(bytes.to_vec())
                .or_insert_with(|| (baseline, BTreeSet::new()))
                .1;
            let index_changed = status.intersects(
                git2::Status::INDEX_NEW
                    | git2::Status::INDEX_MODIFIED
                    | git2::Status::INDEX_DELETED
                    | git2::Status::INDEX_TYPECHANGE
                    | git2::Status::INDEX_RENAMED,
            );
            let worktree_changed = status.intersects(
                git2::Status::WT_NEW
                    | git2::Status::WT_MODIFIED
                    | git2::Status::WT_DELETED
                    | git2::Status::WT_TYPECHANGE
                    | git2::Status::WT_RENAMED
                    | git2::Status::WT_UNREADABLE,
            );
            if index_changed {
                let indexed = index.get_path(&path, 0);
                // Git can normalize CRLF or apply clean filters when staging.
                // When Git reports index == WT, represent that single version
                // by actual WT bytes both before and after stage/unstage.
                let value = if !worktree_changed && !status.is_conflicted() && indexed.is_some() {
                    worktree_version(&repo, root, &path, indexed.as_ref())?
                } else {
                    indexed
                        .map(|entry| version(entry.mode, entry.id))
                        .unwrap_or_else(|| "missing".into())
                };
                versions.insert(value);
            }
            if status.is_conflicted() {
                for stage in 1..=3 {
                    if let Some(entry) = index.get_path(&path, stage) {
                        versions.insert(version(entry.mode, entry.id));
                    }
                }
            }
            if worktree_changed || status.is_conflicted() {
                versions.insert(if status.is_wt_deleted() {
                    "missing".into()
                } else {
                    worktree_version(&repo, root, &path, index.get_path(&path, 0).as_ref())?
                });
            }
        }
        if changes.is_empty() {
            return Ok(None);
        }
        let mut hasher = Sha256::new();
        field(&mut hasher, b"codux-git-content-v1");
        for (path, (baseline, versions)) in changes {
            field(&mut hasher, &path);
            field(&mut hasher, baseline.as_bytes());
            field(&mut hasher, &(versions.len() as u64).to_be_bytes());
            for version in versions {
                field(&mut hasher, version.as_bytes());
            }
        }
        let mut scope = Sha256::new();
        field(&mut scope, b"codux-git-scope-v1");
        for path in [root, repo.path()] {
            let canonical = path
                .canonicalize()
                .map_err(|e| format!("Cannot identify repository {}: {e}", path.display()))?;
            field(&mut scope, canonical.as_os_str().as_encoded_bytes());
        }
        Ok(Some(GitChangeFingerprint {
            scope: format!("git-v1:{:x}", scope.finalize()),
            fingerprint: format!("{:x}", hasher.finalize()),
        }))
    }
}

fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}

fn version(mode: u32, oid: git2::Oid) -> String {
    format!("{mode:o}:{oid}")
}

fn relative_path(bytes: &[u8]) -> Result<PathBuf, String> {
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
    };
    #[cfg(not(unix))]
    let path = PathBuf::from(
        std::str::from_utf8(bytes).map_err(|_| "Git path is not valid UTF-8 on this platform.")?,
    );
    if path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err("Unsafe Git status path.".into());
    }
    Ok(path)
}

fn worktree_version(
    repo: &git2::Repository,
    root: &Path,
    relative: &Path,
    index: Option<&git2::IndexEntry>,
) -> Result<String, String> {
    let _ = repo; // core.filemode is consulted only on Unix.
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|e| format!("Cannot inspect {}: {e}", path.display()))?;
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(&path).map_err(|e| e.to_string())?;
        let oid = git2::Oid::hash_object(
            git2::ObjectType::Blob,
            target.as_os_str().as_encoded_bytes(),
        )
        .map_err(|e| e.to_string())?;
        return Ok(version(0o120000, oid));
    }
    if metadata.is_dir() {
        if index.is_some_and(|entry| entry.mode == 0o160000) {
            let sub = git2::Repository::open(&path)
                .map_err(|e| format!("Cannot inspect submodule {}: {e}", path.display()))?;
            let head = sub
                .head()
                .and_then(|h| h.peel_to_tree())
                .map_err(|e| e.to_string())?;
            let dirty = GitService::change_fingerprint(
                path.to_str().ok_or("Submodule path is not UTF-8.")?,
            )?;
            return Ok(format!(
                "160000:{}:{}",
                head.id(),
                dirty.map(|v| v.fingerprint).unwrap_or_default()
            ));
        }
        return Err(format!(
            "Cannot fingerprint untracked directory {} as a single file.",
            path.display()
        ));
    }
    if !metadata.is_file() {
        return Err(format!("Unsupported changed file type: {}", path.display()));
    }
    let oid = git2::Oid::hash_file(git2::ObjectType::Blob, &path)
        .map_err(|e| format!("Cannot hash {}: {e}", path.display()))?;
    let after = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if metadata.len() != after.len() || metadata.modified().ok() != after.modified().ok() {
        return Err(format!(
            "File changed while fingerprinting: {}",
            path.display()
        ));
    }
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        if repo
            .config()
            .ok()
            .and_then(|c| c.get_bool("core.filemode").ok())
            == Some(false)
        {
            index.map(|e| e.mode).unwrap_or(0o100644)
        } else if metadata.permissions().mode() & 0o111 != 0 {
            0o100755
        } else {
            0o100644
        }
    };
    #[cfg(not(unix))]
    let mode = index.map(|e| e.mode).unwrap_or(0o100644);
    Ok(version(mode, oid))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };
    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "codux-fingerprint-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&root).unwrap();
            let repo = git2::Repository::init(&root).unwrap();
            repo.config()
                .unwrap()
                .set_bool("core.autocrlf", false)
                .unwrap();
            fs::write(root.join("tracked.txt"), "base\n").unwrap();
            fs::write(root.join(".gitignore"), "ignored.bin\nignored-dir/\n").unwrap();
            let fixture = Self(root);
            fixture.commit();
            fixture
        }
        fn commit(&self) {
            let repo = git2::Repository::open(&self.0).unwrap();
            let mut index = repo.index().unwrap();
            index
                .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
                .unwrap();
            index.write().unwrap();
            let oid = index.write_tree().unwrap();
            let tree = repo.find_tree(oid).unwrap();
            let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
            let parents: Vec<_> = parent.iter().collect();
            let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "test", &tree, &parents)
                .unwrap();
        }
        fn state(&self) -> GitChangeFingerprint {
            state(&self.0)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }
    fn state(path: &Path) -> GitChangeFingerprint {
        GitService::change_fingerprint(path.to_str().unwrap())
            .unwrap()
            .unwrap()
    }
    fn stage(path: &Path, file: &str) {
        let repo = git2::Repository::open(path).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(file)).unwrap();
        index.write().unwrap();
    }

    #[test]
    fn fingerprint_changes_for_content_without_status_count_changes() {
        let repo = Fixture::new();
        assert_eq!(
            GitService::change_fingerprint(repo.0.to_str().unwrap()).unwrap(),
            None
        );
        fs::write(repo.0.join("tracked.txt"), "aaaa\n").unwrap();
        let counts = GitService::status_counts(repo.0.to_str().unwrap());
        let before = repo.state();
        fs::write(repo.0.join("tracked.txt"), "bbbb\n").unwrap();
        let after = repo.state();
        let after_counts = GitService::status_counts(repo.0.to_str().unwrap());
        assert_eq!(
            (counts.staged, counts.unstaged, counts.untracked),
            (
                after_counts.staged,
                after_counts.unstaged,
                after_counts.untracked
            )
        );
        assert_eq!(before.scope, after.scope);
        assert_ne!(before.fingerprint, after.fingerprint);
    }

    #[test]
    fn fingerprint_ignores_stage_movement_but_covers_both_versions() {
        let repo = Fixture::new();
        fs::write(repo.0.join("tracked.txt"), "reviewed\n").unwrap();
        let unstaged = repo.state();
        stage(&repo.0, "tracked.txt");
        let staged = repo.state();
        assert_eq!(
            unstaged, staged,
            "stage movement alone does not change content"
        );
        fs::write(repo.0.join("tracked.txt"), "second!!\n").unwrap();
        let both = repo.state();
        assert_ne!(both.fingerprint, staged.fingerprint);
        // Only change staged version; filesystem content and status counts stay equal.
        fs::write(repo.0.join("tracked.txt"), "third!!!\n").unwrap();
        stage(&repo.0, "tracked.txt");
        fs::write(repo.0.join("tracked.txt"), "second!!\n").unwrap();
        assert_ne!(both.fingerprint, repo.state().fingerprint);
    }

    #[test]
    fn fingerprint_tracks_nested_binary_untracked_and_ignores_ignored_files() {
        let repo = Fixture::new();
        fs::create_dir_all(repo.0.join("nested/deep")).unwrap();
        fs::write(repo.0.join("nested/deep/a.bin"), [0, 255, 1, 0]).unwrap();
        let first = repo.state();
        fs::write(repo.0.join("nested/deep/a.bin"), [0, 255, 2, 0]).unwrap();
        let second = repo.state();
        assert_ne!(first.fingerprint, second.fingerprint);
        fs::write(repo.0.join("ignored.bin"), [1, 2, 3]).unwrap();
        fs::create_dir_all(repo.0.join("ignored-dir")).unwrap();
        fs::write(repo.0.join("ignored-dir/a"), "ignored").unwrap();
        assert_eq!(second, repo.state());
        stage(&repo.0, "nested/deep/a.bin");
        assert_eq!(
            second,
            repo.state(),
            "adding the same untracked content to index is unchanged"
        );
    }

    #[test]
    fn fingerprint_tracks_delete_and_rename_and_is_stable() {
        let repo = Fixture::new();
        fs::remove_file(repo.0.join("tracked.txt")).unwrap();
        let deleted = repo.state();
        assert_eq!(deleted, repo.state());
        fs::write(repo.0.join("renamed.txt"), "base\n").unwrap();
        let renamed = repo.state();
        assert_ne!(deleted.fingerprint, renamed.fingerprint);
        let git = git2::Repository::open(&repo.0).unwrap();
        let mut index = git.index().unwrap();
        index.remove_path(Path::new("tracked.txt")).unwrap();
        index.add_path(Path::new("renamed.txt")).unwrap();
        index.write().unwrap();
        assert_eq!(
            renamed,
            repo.state(),
            "staging a rename preserves content identity"
        );
    }

    #[test]
    fn fingerprint_separates_repository_and_linked_worktree_scopes() {
        let repo = Fixture::new();
        let other = Fixture::new();
        fs::write(repo.0.join("tracked.txt"), "same\n").unwrap();
        fs::write(other.0.join("tracked.txt"), "same\n").unwrap();
        assert_ne!(repo.state().scope, other.state().scope);
        let linked = repo.0.with_extension("linked");
        let git = git2::Repository::open(&repo.0).unwrap();
        git.worktree("fingerprint-linked", &linked, None).unwrap();
        fs::write(linked.join("tracked.txt"), "same\n").unwrap();
        assert_ne!(repo.state().scope, state(&linked).scope);
        fs::create_dir_all(repo.0.join("subdir")).unwrap();
        assert_eq!(repo.state().scope, state(&repo.0.join("subdir")).scope);
        fs::remove_dir_all(linked).ok();
    }

    #[test]
    fn fingerprint_uses_only_changed_path_head_baselines() {
        let repo = Fixture::new();
        fs::write(repo.0.join("tracked.txt"), "review-me\n").unwrap();
        let before = repo.state();
        fs::write(repo.0.join("unrelated.txt"), "committed\n").unwrap();
        stage(&repo.0, "unrelated.txt");
        let git = git2::Repository::open(&repo.0).unwrap();
        let mut index = git.index().unwrap();
        let oid = index.write_tree().unwrap();
        let tree = git.find_tree(oid).unwrap();
        let parent = git.head().unwrap().peel_to_commit().unwrap();
        let sig = git2::Signature::now("Test", "test@example.invalid").unwrap();
        git.commit(Some("HEAD"), &sig, &sig, "unrelated", &tree, &[&parent])
            .unwrap();
        assert_eq!(before, repo.state(), "unrelated HEAD commit is irrelevant");
        fs::write(repo.0.join("tracked.txt"), "new baseline\n").unwrap();
        repo.commit();
        fs::write(repo.0.join("tracked.txt"), "review-me\n").unwrap();
        assert_ne!(
            before.fingerprint,
            repo.state().fingerprint,
            "same WT against new baseline is a new diff"
        );
    }

    #[test]
    fn fingerprint_streams_binary_content_past_diff_preview_limits() {
        let repo = Fixture::new();
        let mut content = vec![0; 1024 * 1024];
        fs::write(repo.0.join("large.bin"), &content).unwrap();
        let before = repo.state();
        *content.last_mut().unwrap() = 255;
        fs::write(repo.0.join("large.bin"), &content).unwrap();
        assert_ne!(before.fingerprint, repo.state().fingerprint);
    }

    #[test]
    fn fingerprint_stage_movement_keeps_crlf_worktree_content_identity() {
        let repo = Fixture::new();
        let git = git2::Repository::open(&repo.0).unwrap();
        git.config()
            .unwrap()
            .set_bool("core.autocrlf", true)
            .unwrap();
        fs::write(repo.0.join("tracked.txt"), "windows changed\r\n").unwrap();
        let before = repo.state();
        stage(&repo.0, "tracked.txt");
        assert_eq!(before, repo.state());
    }

    #[test]
    fn fingerprint_submodule_stage_movement_keeps_content_identity() {
        let repo = Fixture::new();
        let subpath = repo.0.join("submodule");
        git2::Repository::init(&subpath).unwrap();
        fs::write(subpath.join("tracked.txt"), "base\n").unwrap();
        let sub = Fixture(subpath);
        sub.commit();
        stage(&repo.0, "submodule");
        repo.commit();
        fs::write(sub.0.join("tracked.txt"), "submodule content\n").unwrap();
        sub.commit();
        let before = repo.state();
        stage(&repo.0, "submodule");
        assert_eq!(
            before,
            repo.state(),
            "staging a submodule pointer must not review identical content twice"
        );
        fs::write(sub.0.join("tracked.txt"), "dirty submodule\n").unwrap();
        assert_ne!(before.fingerprint, repo.state().fingerprint);
    }

    #[test]
    fn fingerprint_returns_errors_instead_of_clean_for_missing_repositories() {
        assert!(GitService::change_fingerprint("/codux-no-such-repository-12345").is_err());
    }
}
