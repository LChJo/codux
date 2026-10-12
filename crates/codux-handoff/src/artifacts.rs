//! Artifact discovery and content-addressed staging.
//!
//! An artifact is the unit that gets dragged into a conversation. Two properties
//! matter:
//!
//! - **Content addressing.** The staged copy is named by its sha256, so the same
//!   bytes staged twice collapse to one file and the artifact id stays stable
//!   across handoff revisions. A reference dropped into a terminal keeps
//!   resolving after the handoff is rebuilt.
//! - **A stale copy is not evidence.** [`verify_artifact`] re-stats the source and
//!   compares size and mtime; if the source moved or changed, availability
//!   degrades to [`ArtifactAvailability::Unavailable`] instead of handing the
//!   reader yesterday's bytes.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::types::{
    ArtifactAvailability, HANDOFF_ARTIFACT_TRUST, HandoffArtifact, HandoffArtifactKind,
    MAX_ARTIFACTS,
};

/// Per-artifact staging budget. Larger sources are kept as a reference only.
pub const MAX_ARTIFACT_BYTES: u64 = 8 * 1024 * 1024;
/// Directories never worth walking.
const SKIPPED_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "coverage",
    ".git",
    ".gradle",
    ".next",
    "__pycache__",
    ".venv",
    "venv",
];
/// How deep document discovery walks. Plan/task docs live near the root.
const MAX_DISCOVERY_DEPTH: usize = 4;
/// Cap on directory entries examined, so a huge checkout cannot stall a build.
const MAX_DISCOVERY_ENTRIES: usize = 4000;

/// A document found in the project that may be worth attaching.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredDocument {
    pub kind: HandoffArtifactKind,
    pub path: PathBuf,
    /// Path relative to the project root, for display.
    pub relative_name: String,
    pub modified_at: Option<SystemTime>,
}

/// Stages artifacts under one handoff's directory.
#[derive(Clone, Debug)]
pub struct ArtifactStager {
    root: PathBuf,
    max_bytes: u64,
}

impl ArtifactStager {
    /// `root` is `support_dir/handoffs/<project_id>/<handoff_id>`.
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            max_bytes: MAX_ARTIFACT_BYTES,
        }
    }

    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Stage in-memory bytes. Used for the rendered result, the diff, and the
    /// cleaned transcript — content that has no source file.
    pub fn stage_bytes(
        &self,
        kind: HandoffArtifactKind,
        name: &str,
        purpose: &str,
        bytes: &[u8],
    ) -> Option<HandoffArtifact> {
        if bytes.is_empty() {
            return None;
        }
        let sha256 = crate::sha256_hex(bytes);
        let staged_path = self.write_content_addressed(&sha256, kind, name, bytes)?;
        Some(HandoffArtifact {
            id: crate::artifact_id(&sha256),
            kind,
            name: display_name(name),
            staged_path: Some(path_text(&staged_path)),
            source_path: None,
            sha256: Some(sha256),
            bytes: bytes.len() as u64,
            purpose: purpose.to_string(),
            availability: ArtifactAvailability::Staged,
            trust: HANDOFF_ARTIFACT_TRUST.to_string(),
        })
    }

    /// Stage a file from disk, or keep it as a reference when it is too large.
    ///
    /// `source_path` is stored verbatim: it is a functional field, later used to
    /// re-verify the source and to resolve a path for the reader.
    pub fn stage_file(
        &self,
        kind: HandoffArtifactKind,
        path: &Path,
        purpose: &str,
    ) -> Option<HandoffArtifact> {
        let metadata = fs::metadata(path).ok()?;
        if !metadata.is_file() {
            return None;
        }
        let source_path = path_text(path);
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| "artifact".to_string());
        if metadata.len() > self.max_bytes {
            return Some(HandoffArtifact {
                id: crate::artifact_id(&crate::sha256_hex(source_path.as_bytes())),
                kind,
                name: display_name(&name),
                staged_path: None,
                source_path: Some(source_path),
                sha256: None,
                bytes: metadata.len(),
                purpose: purpose.to_string(),
                availability: ArtifactAvailability::Reference,
                trust: HANDOFF_ARTIFACT_TRUST.to_string(),
            });
        }
        let bytes = fs::read(path).ok()?;
        let sha256 = crate::sha256_hex(&bytes);
        // Re-stat after reading: a file replaced mid-read would otherwise be
        // staged as if it were the source's current content.
        let after = fs::metadata(path).ok()?;
        if after.len() != metadata.len() || mtime_key(&after) != mtime_key(&metadata) {
            return Some(HandoffArtifact {
                id: crate::artifact_id(&sha256),
                kind,
                name: display_name(&name),
                staged_path: None,
                source_path: Some(source_path),
                sha256: None,
                bytes: metadata.len(),
                purpose: purpose.to_string(),
                availability: ArtifactAvailability::Unavailable,
                trust: HANDOFF_ARTIFACT_TRUST.to_string(),
            });
        }
        let staged_path = self.write_content_addressed(&sha256, kind, &name, &bytes)?;
        Some(HandoffArtifact {
            id: crate::artifact_id(&sha256),
            kind,
            name: display_name(&name),
            staged_path: Some(path_text(&staged_path)),
            source_path: Some(source_path),
            sha256: Some(sha256),
            bytes: bytes.len() as u64,
            purpose: purpose.to_string(),
            availability: ArtifactAvailability::Staged,
            trust: HANDOFF_ARTIFACT_TRUST.to_string(),
        })
    }

    /// Write via a temp file then rename, so a crash mid-write cannot leave a
    /// content-addressed path holding partial bytes. An existing file with the
    /// same digest is already correct by definition.
    ///
    /// The name is derived from the digest and extension only — deliberately not
    /// from the display name. Including the stem would make two artifacts with
    /// identical content but different names occupy two files, defeating the
    /// dedup that content addressing exists to provide. The human-readable name
    /// lives on the artifact record.
    fn write_content_addressed(
        &self,
        sha256: &str,
        kind: HandoffArtifactKind,
        name: &str,
        bytes: &[u8],
    ) -> Option<PathBuf> {
        fs::create_dir_all(&self.root).ok()?;
        let extension = Path::new(name)
            .extension()
            .map(|value| value.to_string_lossy().to_string())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| kind.extension().to_string());
        let file_name = format!(
            "{}.{}",
            &sha256[..sha256.len().min(32)],
            crate::transcript::safe_path_component(&extension)
        );
        let target = self.root.join(file_name);
        if target.is_file() {
            return Some(target);
        }
        let temporary = self.root.join(format!(
            "{}.part",
            uuid::Uuid::new_v4().simple()
        ));
        {
            let mut file = fs::File::create(&temporary).ok()?;
            file.write_all(bytes).ok()?;
            file.flush().ok()?;
        }
        if fs::rename(&temporary, &target).is_err() {
            let _ = fs::remove_file(&temporary);
            return None;
        }
        Some(target)
    }
}

/// Re-check that an artifact's source still matches what was staged.
///
/// Called at drop time. The staged copy is deliberately not trusted on its own:
/// handing a reader stale bytes is worse than reporting the artifact unavailable.
pub fn verify_artifact(artifact: &HandoffArtifact) -> ArtifactAvailability {
    let Some(source_path) = artifact.source_path.as_deref() else {
        return artifact.availability;
    };
    let Ok(metadata) = fs::metadata(source_path) else {
        return ArtifactAvailability::Unavailable;
    };
    if !metadata.is_file() {
        return ArtifactAvailability::Unavailable;
    }
    let Some(expected_sha256) = artifact.sha256.as_deref() else {
        return if artifact.staged_path.is_some() {
            ArtifactAvailability::Staged
        } else {
            ArtifactAvailability::Reference
        };
    };
    if metadata.len() != artifact.bytes {
        return ArtifactAvailability::Unavailable;
    }
    let Ok(bytes) = fs::read(source_path) else {
        return ArtifactAvailability::Unavailable;
    };
    if crate::sha256_hex(&bytes) != expected_sha256 {
        return ArtifactAvailability::Unavailable;
    }
    // Source is intact; confirm the staged copy is still there.
    match artifact.staged_path.as_deref() {
        Some(staged) if Path::new(staged).is_file() => ArtifactAvailability::Staged,
        Some(_) => ArtifactAvailability::Unavailable,
        None => ArtifactAvailability::Reference,
    }
}

/// Find plan and task documents in a project checkout.
///
/// Results are newest-first so a build attaches the document the user is actually
/// working from, not an archived one.
pub fn discover_documents(project_path: &Path) -> Vec<DiscoveredDocument> {
    let mut documents = Vec::new();
    let mut visited = 0usize;
    walk_for_documents(
        project_path,
        project_path,
        0,
        &mut documents,
        &mut visited,
    );
    documents.sort_by(|left, right| {
        right
            .modified_at
            .cmp(&left.modified_at)
            .then_with(|| left.relative_name.cmp(&right.relative_name))
    });
    documents
}

fn walk_for_documents(
    root: &Path,
    directory: &Path,
    depth: usize,
    documents: &mut Vec<DiscoveredDocument>,
    visited: &mut usize,
) {
    if depth > MAX_DISCOVERY_DEPTH || documents.len() >= MAX_ARTIFACTS * 4 {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        *visited += 1;
        if *visited > MAX_DISCOVERY_ENTRIES {
            return;
        }
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if should_skip_dir(&file_name, depth) {
                continue;
            }
            walk_for_documents(root, &path, depth + 1, documents, visited);
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let Some(kind) = document_kind(&file_name, &path, root) else {
            continue;
        };
        let modified_at = fs::metadata(&path).and_then(|m| m.modified()).ok();
        documents.push(DiscoveredDocument {
            kind,
            relative_name: relative_name(root, &path),
            path,
            modified_at,
        });
    }
}

fn should_skip_dir(name: &str, depth: usize) -> bool {
    if SKIPPED_DIRS.contains(&name) {
        return true;
    }
    // Hidden directories are skipped except the ones that hold plans.
    if name.starts_with('.') {
        return depth > 0 || !matches!(name, ".claude" | ".codex" | ".specstory");
    }
    false
}

/// Classify a file as a plan or task document, or `None` if it is neither.
fn document_kind(file_name: &str, path: &Path, root: &Path) -> Option<HandoffArtifactKind> {
    let lower = file_name.to_lowercase();
    if !lower.ends_with(".md") && !lower.ends_with(".markdown") && !lower.ends_with(".txt") {
        return None;
    }
    let relative = relative_name(root, path).replace('\\', "/").to_lowercase();
    let stem = Path::new(&lower)
        .file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_default();

    let is_task = stem == "tasks"
        || stem == "todo"
        || stem == "todos"
        || stem.starts_with("task-")
        || relative.contains("/tasks/")
        || relative.ends_with("/tasks.md");
    if is_task {
        return Some(HandoffArtifactKind::Task);
    }

    let is_plan = stem == "plan"
        || stem == "proposal"
        || stem == "design"
        || stem.ends_with("-plan")
        || stem.starts_with("plan-")
        || relative.contains("/plans/")
        || relative.contains("/spec/changes/")
        || relative.contains("/.claude/plans/");
    if is_plan {
        return Some(HandoffArtifactKind::Plan);
    }
    None
}

fn relative_name(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|relative| relative.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string())
}

/// Display name: prefer the project-relative path so two `design.md` files in
/// different change folders stay distinguishable. Truncated for the UI only —
/// `source_path` remains the authoritative locator.
fn display_name(name: &str) -> String {
    crate::sanitize::clip_text(name, 160)
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn mtime_key(metadata: &fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

/// Keep the most relevant artifacts when discovery finds more than the cap.
/// Plan and task documents outrank generated content. The sort is stable, so
/// within a kind the caller's newest-first order is preserved.
pub fn prioritize(artifacts: Vec<HandoffArtifact>) -> Vec<HandoffArtifact> {
    let mut ranked = artifacts;
    ranked.sort_by_key(|artifact| kind_rank(artifact.kind));
    let mut seen_ids = std::collections::HashSet::new();
    ranked
        .into_iter()
        .filter(|artifact| seen_ids.insert(artifact.id.clone()))
        .take(MAX_ARTIFACTS)
        .collect()
}

fn kind_rank(kind: HandoffArtifactKind) -> u8 {
    match kind {
        HandoffArtifactKind::Result => 0,
        HandoffArtifactKind::Plan => 1,
        HandoffArtifactKind::Task => 2,
        HandoffArtifactKind::Diff => 3,
        HandoffArtifactKind::ChangedFiles => 4,
        HandoffArtifactKind::Transcript => 5,
        HandoffArtifactKind::File => 6,
    }
}
