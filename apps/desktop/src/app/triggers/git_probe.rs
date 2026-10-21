use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

const COALESCE: Duration = Duration::from_millis(300);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProbeToken {
    pub project_id: String,
    version: u64,
}

#[derive(Default)]
struct ProjectProbe {
    version: u64,
    due: Option<Instant>,
    in_flight: Option<u64>,
}

#[derive(Default)]
pub(super) struct GitProbeCoordinator {
    projects: HashMap<String, ProjectProbe>,
    version: u64,
}

impl GitProbeCoordinator {
    pub fn has_work(&self, project: &str) -> bool {
        self.projects
            .get(project)
            .is_some_and(|state| state.due.is_some() || state.in_flight.is_some())
    }
    pub fn request(&mut self, project: &str, now: Instant) {
        self.version = self.version.wrapping_add(1);
        let state = self.projects.entry(project.into()).or_default();
        state.version = self.version;
        state.due = Some(now + COALESCE);
    }

    pub fn cancel(&mut self, project: &str) {
        self.version = self.version.wrapping_add(1);
        if let Some(state) = self.projects.get_mut(project) {
            state.version = self.version;
            state.due = None;
        }
    }

    pub fn take_ready(&mut self, now: Instant) -> Vec<ProbeToken> {
        self.projects
            .iter_mut()
            .filter_map(|(project, state)| {
                if state.in_flight.is_some() || state.due.is_none_or(|due| due > now) {
                    return None;
                }
                state.due = None;
                state.in_flight = Some(state.version);
                Some(ProbeToken {
                    project_id: project.clone(),
                    version: state.version,
                })
            })
            .collect()
    }

    pub fn finish(&mut self, token: &ProbeToken) -> bool {
        let Some(state) = self.projects.get_mut(&token.project_id) else {
            return false;
        };
        if state.in_flight != Some(token.version) {
            return false;
        }
        state.in_flight = None;
        state.version == token.version
    }
}

/// Rules reviewing the same runtime/directory share a single sample. A rule's
/// target is explicit; the terminal which completed is never a query input.
pub(super) fn sample_targets<T: Clone + PartialEq, R: Clone>(
    targets: impl IntoIterator<Item = (String, T, String)>,
    mut sample: impl FnMut(&T, &str) -> R,
) -> HashMap<String, R> {
    let mut sampled: Vec<(T, String, R)> = Vec::new();
    targets
        .into_iter()
        .map(|(id, target, path)| {
            let value = sampled
                .iter()
                .find(|(previous, cwd, _)| previous == &target && cwd == &path)
                .map(|(_, _, value)| value.clone())
                .unwrap_or_else(|| {
                    let value = sample(&target, &path);
                    sampled.push((target, path, value.clone()));
                    value
                });
            (id, value)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_completion_burst_coalesces_before_querying() {
        let mut probes = GitProbeCoordinator::default();
        let start = Instant::now();
        probes.request("project", start);
        probes.request("project", start + Duration::from_millis(100));
        assert!(
            probes
                .take_ready(start + Duration::from_millis(200))
                .is_empty()
        );
        assert_eq!(
            probes.take_ready(start + Duration::from_millis(400)).len(),
            1
        );
        assert!(probes.take_ready(start + Duration::from_secs(1)).is_empty());
    }

    #[test]
    fn completion_during_probe_waits_and_rejects_old_result() {
        let mut probes = GitProbeCoordinator::default();
        let start = Instant::now();
        probes.request("project", start);
        let first = probes.take_ready(start + COALESCE).pop().unwrap();
        probes.request("project", start + Duration::from_secs(1));
        assert!(probes.take_ready(start + Duration::from_secs(2)).is_empty());
        assert!(!probes.finish(&first));
        let next = probes
            .take_ready(start + Duration::from_secs(2))
            .pop()
            .unwrap();
        assert_ne!(first, next);
        assert!(probes.finish(&next));
    }

    #[test]
    fn cancellation_rejects_result_and_other_projects_progress() {
        let mut probes = GitProbeCoordinator::default();
        let start = Instant::now();
        probes.request("first", start);
        let stale = probes.take_ready(start + COALESCE).pop().unwrap();
        probes.request("second", start);
        assert_eq!(
            probes
                .take_ready(start + COALESCE)
                .pop()
                .unwrap()
                .project_id,
            "second"
        );
        probes.cancel("first");
        assert!(!probes.finish(&stale));
        probes.request("first", start + Duration::from_secs(1));
        let current = probes
            .take_ready(start + Duration::from_secs(2))
            .pop()
            .unwrap();
        assert_ne!(stale, current);
    }

    #[test]
    fn rules_share_only_their_own_runtime_and_directory_sample() {
        let mut calls = Vec::new();
        let result = sample_targets(
            [
                ("review-a".into(), "local", "/review-tree".into()),
                ("review-b".into(), "local", "/review-tree".into()),
                ("review-c".into(), "hosted", "/review-tree".into()),
                ("review-d".into(), "local", "/other-tree".into()),
            ],
            |target, path| {
                calls.push((target.to_string(), path.to_string()));
                calls.len()
            },
        );
        assert_eq!(calls.len(), 3);
        assert_eq!(result["review-a"], result["review-b"]);
        assert_ne!(result["review-a"], result["review-c"]);
        assert_ne!(result["review-a"], result["review-d"]);
        assert!(
            calls
                .iter()
                .all(|(_, path)| path != "/source-terminal-tree")
        );
    }

    #[test]
    fn cancellation_does_not_allow_a_second_query_until_old_worker_finishes() {
        let mut probes = GitProbeCoordinator::default();
        let start = Instant::now();
        probes.request("project", start);
        let old = probes.take_ready(start + COALESCE).pop().unwrap();
        probes.cancel("project");
        probes.request("project", start + Duration::from_secs(1));
        assert!(probes.take_ready(start + Duration::from_secs(2)).is_empty());
        assert!(!probes.finish(&old));
        let current = probes
            .take_ready(start + Duration::from_secs(2))
            .pop()
            .unwrap();
        assert_ne!(old, current);
        assert!(probes.finish(&current));
    }

    #[test]
    fn errors_stay_errors_instead_of_becoming_clean_samples() {
        let results = sample_targets(
            [
                ("failed-a".into(), "local", "/unavailable".into()),
                ("failed-b".into(), "local", "/unavailable".into()),
                ("clean".into(), "local", "/clean".into()),
            ],
            |_, path| -> Result<Option<String>, String> {
                if path == "/unavailable" {
                    Err("query failed".into())
                } else {
                    Ok(None)
                }
            },
        );
        assert!(results["failed-a"].is_err());
        assert!(results["failed-b"].is_err());
        assert_eq!(results["clean"], Ok(None));
    }
}
