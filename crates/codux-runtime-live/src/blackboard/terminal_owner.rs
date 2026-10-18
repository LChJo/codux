use codux_blackboard::Store;
use std::sync::OnceLock;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

fn current_owner() -> (u32, u64) {
    static OWNER: OnceLock<(u32, u64)> = OnceLock::new();
    *OWNER.get_or_init(|| {
        let pid = std::process::id();
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            true,
            ProcessRefreshKind::nothing(),
        );
        (
            pid,
            system
                .process(Pid::from_u32(pid))
                .map_or(0, |p| p.start_time()),
        )
    })
}
pub(super) fn claim(store: &Store, project: &str, terminal: &str) -> Result<(), String> {
    let (pid, started) = current_owner();
    store.claim_terminal_owner(project, terminal, pid, started)
}
pub(super) fn reconcile(store: &Store, project: &str) -> Result<(), String> {
    let owners = store.active_terminal_owners(project)?;
    let current = current_owner();
    let pids = owners
        .iter()
        .filter_map(|o| o.pid)
        .filter(|pid| *pid != current.0)
        .map(Pid::from_u32)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut system = System::new();
    if !pids.is_empty() {
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::nothing(),
        );
    }
    for owner in owners {
        let alive = match (owner.pid, owner.started_at) {
            (Some(pid), Some(started)) if pid == current.0 => started == current.1,
            (Some(pid), Some(started)) => system
                .process(Pid::from_u32(pid))
                .is_some_and(|p| p.start_time() == started),
            // Unowned bindings come from the pre-lease prototype. All current
            // runtime registrations claim their binding before publishing it.
            _ => false,
        };
        if !alive {
            store.expire_terminal_owner(project, &owner)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore]
    fn owner_probe_child() {
        if std::env::var("CODUX_BLACKBOARD_OWNER_TEST_CHILD").as_deref() == Ok("1") {
            use std::io::Read;
            let mut byte = [0];
            std::io::stdin().read(&mut byte).unwrap();
        }
    }
    #[test]
    fn dead_runtime_owners_are_inactive_and_current_process_bindings_survive() {
        let root =
            std::env::temp_dir().join(format!("codux-owner-reconcile-{}", uuid::Uuid::new_v4()));
        let store = Store::open(&root).unwrap();
        store
            .register_terminal("p", "live", "live", "Agent")
            .unwrap();
        claim(&store, "p", "live").unwrap();
        store.register_terminal("p", "old", "old", "Old").unwrap();
        store.set_terminal_state("p", "old", true, true).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let dead_pid = child.id();
        child.wait().unwrap();
        store.claim_terminal_owner("p", "old", dead_pid, 0).unwrap();
        store
            .register_terminal("p", "legacy", "legacy", "Legacy")
            .unwrap();
        let mut other = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "owner_probe_child"])
            .env("CODUX_BLACKBOARD_OWNER_TEST_CHILD", "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[Pid::from_u32(other.id())]),
            true,
            ProcessRefreshKind::nothing(),
        );
        let started = system
            .process(Pid::from_u32(other.id()))
            .expect("other runtime is alive")
            .start_time();
        store
            .register_terminal("p", "other", "other", "Other runtime")
            .unwrap();
        store
            .claim_terminal_owner("p", "other", other.id(), started)
            .unwrap();
        let cursor = store.snapshot("p").unwrap().cursor;
        reconcile(&store, "p").unwrap();
        assert!(store.terminal("p", "live").unwrap().active);
        assert!(!store.terminal("p", "old").unwrap().active);
        assert!(!store.terminal("p", "old").unwrap().automatic);
        assert!(!store.terminal("p", "legacy").unwrap().active);
        assert!(
            store.terminal("p", "other").unwrap().active,
            "another live runtime sharing storage must retain its binding"
        );
        assert_eq!(store.snapshot("p").unwrap().cursor, cursor);
        let binding = store
            .register_terminal("p", "live", "rebound", "Agent")
            .unwrap();
        claim(&store, "p", "rebound").unwrap();
        assert_eq!(binding.ordinal, 1);
        reconcile(&store, "p").unwrap();
        assert!(store.terminal("p", "rebound").unwrap().active);
        drop(other.stdin.take());
        assert!(other.wait().unwrap().success());
        reconcile(&store, "p").unwrap();
        assert!(!store.terminal("p", "other").unwrap().active);
        std::fs::remove_dir_all(root).unwrap();
    }
}
