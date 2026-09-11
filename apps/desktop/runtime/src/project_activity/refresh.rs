impl ProjectActivityCoordinator {
    pub fn refresh_git_once(&self, project: &ProjectSummary) {
        self.mark_project_summary(project);
        if project.runtime_target.is_hosted() {
            return;
        }
        let mut tracked_project = TrackedProject::from(project.clone());
        let now = Instant::now();
        if let Ok(mut guard) = self.projects.lock()
            && let Some(tracked) = guard.get_mut(&project.id)
        {
            let minimum_remote_interval = Duration::from_secs(MIN_GIT_REFRESH_SECONDS);
            if tracked
                .last_remote_git_refresh
                .map(|last| now.duration_since(last) < minimum_remote_interval)
                .unwrap_or(false)
            {
                runtime_trace(
                    "git",
                    &format!(
                        "refresh_remote skipped project={} path={} reason=throttled",
                        project.id, project.path
                    ),
                );
                return;
            }
            tracked.last_git_refresh = Some(now);
            tracked.last_remote_git_refresh = Some(now);
            tracked_project = tracked.clone();
        }
        self.git_jobs.submit(GitJob::Refresh {
            project: tracked_project,
            fetch_remote: true,
        });
    }

    pub fn refresh_git_sidecars_by_path(&self, project: ProjectSummary) {
        self.mark_project_summary(&project);
        if project.runtime_target.is_hosted() {
            return;
        }
        self.git_jobs.submit(GitJob::Worktree {
            support_dir: self.support_dir.clone(),
            project: project.clone(),
        });
    }

    pub fn refresh_git_changed(
        &self,
        project_store: &ProjectStore,
        project_path: String,
        repository_path: String,
        changed_paths: Vec<String>,
    ) {
        let Some(project) = project_store.workspace_summary_by_path(&project_path) else {
            return;
        };
        self.mark_project_summary(&project);
        // Branch/HEAD mutations must never be swallowed by the coalescing gates
        // below: they are the only signal that the worktree list's branch
        // labels are stale, and once the repository goes quiet no further event
        // arrives to retry them.
        let branch_sensitive = changed_paths
            .iter()
            .any(|path| is_branch_sensitive_changed_path(path));
        let now = Instant::now();
        let mut should_refresh = true;
        let mut should_refresh_sidecars = true;
        if let Ok(mut guard) = self.projects.lock()
            && let Some(tracked) = guard.get_mut(&project.id)
        {
            should_refresh = branch_sensitive
                || tracked
                    .last_git_changed_refresh
                    .map(|last| now.duration_since(last) >= Duration::from_millis(1200))
                    .unwrap_or(true);
            // The worktree sidecar gate is stamped only when a worktree job
            // is actually submitted. Plain file-save floods therefore no
            // longer starve worktree rescans (the old gate re-stamped on
            // every coalesced git refresh, which file activity kept hitting
            // faster than the 3s window could open).
            should_refresh_sidecars = branch_sensitive
                || tracked
                    .last_worktree_refresh
                    .map(|last| now.duration_since(last) >= Duration::from_millis(3000))
                    .unwrap_or(true);
            if should_refresh {
                tracked.last_git_changed_refresh = Some(now);
                tracked.last_git_refresh = Some(now);
            }
            if should_refresh_sidecars {
                tracked.last_worktree_refresh = Some(now);
            }
        }
        if let Ok(mut events) = self.events.lock() {
            events.push_back(ProjectActivityEvent::GitChanged {
                project_path,
                repository_path,
                changed_paths,
            });
            while events.len() > 128 {
                events.pop_front();
            }
        }
        if !should_refresh && !should_refresh_sidecars {
            return;
        }
        if should_refresh {
            self.git_jobs.submit(GitJob::Refresh {
                project: TrackedProject::from(project.clone()),
                fetch_remote: false,
            });
        }
        if should_refresh_sidecars {
            self.git_jobs.submit(GitJob::Worktree {
                support_dir: self.support_dir.clone(),
                project: project.clone(),
            });
        }
    }

    pub fn refresh_ai_once(&self, project: ProjectSummary) {
        self.mark_project_summary(&project);
        let _ = self.mark_ai_activation(&project.id);
        if let Ok(mut guard) = self.projects.lock()
            && let Some(tracked) = guard.get_mut(&project.id)
        {
            tracked.last_ai_refresh = Some(Instant::now());
        }
        let ai_history = self.ai_history.clone();
        thread::spawn(move || {
            let request: AIHistoryProjectRequest = project.clone().into();
            let _ = ai_history.refresh_project(request);
        });
    }

    pub fn run_tick(&self, settings: &SettingsSummary) {
        let git_interval =
            configured_interval_seconds(&settings.git_refresh, MIN_GIT_REFRESH_SECONDS);
        let ai_foreground_interval =
            configured_interval_seconds(&settings.ai_refresh, MIN_AI_REFRESH_SECONDS);
        let ai_background_interval =
            configured_interval_seconds(&settings.ai_background_refresh, MIN_AI_REFRESH_SECONDS);

        if let Some(interval) = git_interval {
            let background_interval = interval
                .checked_mul(4)
                .unwrap_or_else(|| Duration::from_secs(MIN_GIT_REFRESH_SECONDS * 4))
                .max(Duration::from_secs(MIN_GIT_REFRESH_SECONDS * 4));
            let due_projects = self.projects_due_for_git(interval, background_interval);
            if !due_projects.is_empty() {
                runtime_trace(
                    "project-activity",
                    &format!("git interval refresh due count={}", due_projects.len()),
                );
            }
            for project in due_projects {
                let now = Instant::now();
                if let Ok(mut guard) = self.projects.lock()
                    && let Some(tracked) = guard.get_mut(&project.id)
                {
                    tracked.last_remote_git_refresh = Some(now);
                    tracked.last_worktree_refresh = Some(now);
                }
                // The worktree list (left column branch labels) is otherwise
                // only refreshed by file-system events; rescan it on the same
                // interval so a missed/dropped event cannot leave it stale
                // forever.
                self.git_jobs.submit(GitJob::Worktree {
                    support_dir: self.support_dir.clone(),
                    project: ProjectSummary::from(project.clone()),
                });
                self.git_jobs.submit(GitJob::Refresh {
                    project,
                    fetch_remote: true,
                });
            }
        }

        if let Some(foreground_interval) = ai_foreground_interval.or(ai_background_interval) {
            let background_interval = ai_background_interval
                .unwrap_or_else(|| {
                    foreground_interval
                        .checked_mul(4)
                        .unwrap_or_else(|| Duration::from_secs(MIN_AI_REFRESH_SECONDS * 4))
                })
                .max(foreground_interval);
            let due_projects = self.projects_due_for_ai(foreground_interval, background_interval);
            if !due_projects.is_empty() {
                runtime_trace(
                    "project-activity",
                    &format!("ai interval refresh due count={}", due_projects.len()),
                );
            }
            for project in due_projects {
                self.refresh_ai_once(ProjectSummary::from(project));
            }
        }
    }

    fn mark_ai_activation(&self, project_id: &str) -> bool {
        self.activated_ai_projects
            .lock()
            .map(|mut activated| activated.insert(project_id.to_string()))
            .unwrap_or(false)
    }

    fn projects_due_for_git(
        &self,
        foreground_interval: Duration,
        background_interval: Duration,
    ) -> Vec<TrackedProject> {
        let active_project_id = self.active_project_id.lock().ok().and_then(|id| id.clone());
        let is_foreground = self.main_window_visible.load(Ordering::Relaxed)
            || self.main_window_focused.load(Ordering::Relaxed);
        projects_due_for_git_interval(
            &self.projects,
            active_project_id.as_deref(),
            is_foreground,
            foreground_interval,
            background_interval,
            MAX_BACKGROUND_GIT_REFRESH_PER_TICK,
        )
    }

    fn projects_due_for_ai(
        &self,
        foreground_interval: Duration,
        _background_interval: Duration,
    ) -> Vec<TrackedProject> {
        let active_project_id = self.active_project_id.lock().ok().and_then(|id| id.clone());
        let is_foreground = self.main_window_visible.load(Ordering::Relaxed)
            || self.main_window_focused.load(Ordering::Relaxed);
        if !is_foreground || active_project_id.is_none() {
            return Vec::new();
        }
        let now = Instant::now();
        let Ok(mut projects) = self.projects.lock() else {
            return Vec::new();
        };
        let mut due = Vec::new();
        for project in projects.values_mut() {
            if Some(project.id.as_str()) != active_project_id.as_deref() {
                continue;
            }
            let is_due = project
                .last_ai_refresh
                .map(|value| now.duration_since(value) >= foreground_interval)
                .unwrap_or(false);
            if !is_due {
                continue;
            }
            project.last_ai_refresh = Some(now);
            due.push(project.clone());
            break;
        }

        due.truncate(MAX_AI_REFRESH_PER_TICK);
        due
    }
}

/// True when a watched path mutation means a branch/HEAD may have moved
/// (checkout, commit, rebase, merge, fetch updating refs). Such events bypass
/// the coalescing gates in `refresh_git_changed` so the worktree list's branch
/// labels catch up immediately instead of waiting for the next quiet window.
///
/// Paths are normalized display paths (`/` separators, e.g.
/// `/repo/.git/HEAD`, `f:/repo/.git/worktrees/wt/HEAD`). Requiring a `.git`
/// directory segment avoids matching same-named working-tree files.
fn is_branch_sensitive_changed_path(path: &str) -> bool {
    let lowered = path.to_ascii_lowercase();
    let in_git_dir = lowered.contains("/.git/") || lowered.ends_with("/.git");
    if !in_git_dir {
        return false;
    }
    lowered.contains("/refs/")
        || lowered.contains("/logs/head")
        || lowered.ends_with("/head")
        || lowered.ends_with("/packed-refs")
        || lowered.ends_with("/orig_head")
        || lowered.ends_with("/fetch_head")
}
