use super::*;
use serde_json::json;

#[test]
fn blackboard_store_match_dispatch_queue_and_exited_target_work_together() {
    use crate::blackboard::{
        Actor, Component, Store,
        conditions::{Matcher, Rule},
    };
    let root = std::env::temp_dir().join(format!("codux-board-trigger-{}", uuid::Uuid::new_v4()));
    let store = Store::open(&root).unwrap();
    let rule = Rule::notes_created();
    let mut engine = TriggerEngine::default();
    let d = definition("a", TriggerKind::Blackboard { rule: rule.clone() });
    engine.upsert(d.clone(), 100).unwrap();
    engine.set_terminal_state(&d.terminal_id, TerminalState::Idle);
    let mut matcher = Matcher::default();
    for i in 0..3 {
        store
            .write(
                "a",
                Component::Notes,
                None,
                "Handoff",
                &format!("next task {i}"),
                json!({}),
                Actor::user(),
            )
            .unwrap();
        let snapshot = store.snapshot("a").unwrap();
        let hit = matcher
            .push(&d.id, &rule, &snapshot.events[0], &snapshot)
            .unwrap();
        engine.fire_blackboard(&d.id, &hit, 101 + i);
    }
    let sent = engine.take_dispatches();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].cause, TriggerCause::Blackboard);
    assert!(sent[0].message.contains("next task 0"));
    assert!(engine.has_pending(&d.id));
    assert_eq!(engine.definition(&d.id).unwrap().message, d.message);
    engine.complete("a", &d.terminal_id, "done", false, 110);
    let pending = engine.take_dispatches();
    assert_eq!(pending.len(), 1);
    assert!(pending[0].message.contains("next task 1"));
    assert!(!engine.has_pending(&d.id));
    engine.mark_exited(&d.id);
    let snapshot = store.snapshot("a").unwrap();
    let mut fresh = Matcher::default();
    let hit = fresh
        .push(&d.id, &rule, &snapshot.events[0], &snapshot)
        .unwrap();
    engine.fire_blackboard(&d.id, &hit, 111);
    assert!(engine.take_dispatches().is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn triggers_live_shell_without_agent_is_not_started_and_never_receives_messages() {
    let mut engine = TriggerEngine::default();
    let d = definition("a", TriggerKind::Interval { seconds: 5 });
    engine.upsert(d.clone(), 100).unwrap();
    engine.set_terminal_state(&d.terminal_id, TerminalState::NotStarted);
    engine.fire(&d.id, TriggerCause::Manual, 101);
    engine.tick(105);
    assert!(engine.take_dispatches().is_empty());
    assert!(!engine.has_pending(&d.id));
    assert!(!engine.definition(&d.id).unwrap().exited);
    assert!(
        engine
            .records("a")
            .iter()
            .all(|r| r.outcome == TriggerOutcome::NotStarted)
    );
    engine.set_terminal_state(&d.terminal_id, TerminalState::Idle);
    engine.fire(&d.id, TriggerCause::Manual, 106);
    assert_eq!(engine.take_dispatches().len(), 1);
}

#[test]
fn triggers_agent_exit_clears_pending_and_late_completion_does_not_restart_automation() {
    let mut engine = TriggerEngine::default();
    let d = definition("a", TriggerKind::TaskCompleted);
    engine.upsert(d.clone(), 100).unwrap();
    engine.set_terminal_state(&d.terminal_id, TerminalState::Idle);
    engine.fire(&d.id, TriggerCause::Manual, 101);
    engine.take_dispatches();
    engine.fire(&d.id, TriggerCause::Manual, 102);
    assert!(engine.has_pending(&d.id));
    engine.set_terminal_state(&d.terminal_id, TerminalState::NotStarted);
    engine.complete("a", &d.terminal_id, "late-agent-completion", true, 103);
    assert!(!engine.has_pending(&d.id));
    assert!(engine.take_dispatches().is_empty());
    assert_eq!(
        engine.terminal_state(&d.terminal_id),
        TerminalState::NotStarted
    );
    assert!(!engine.definition(&d.id).unwrap().exited);
}

#[test]
fn triggers_delivery_rechecks_agent_and_records_skipped_message_with_its_original_cause() {
    let mut engine = TriggerEngine::default();
    let d = definition("a", TriggerKind::Interval { seconds: 5 });
    engine.upsert(d.clone(), 100).unwrap();
    engine.set_terminal_state(&d.terminal_id, TerminalState::Idle);
    engine.tick(105);
    let job = engine.take_dispatches().pop().unwrap();
    engine.set_terminal_state(&d.terminal_id, TerminalState::NotStarted);
    engine.delivery_skipped(&job, TriggerOutcome::NotStarted, 106);
    let record = engine.records("a").last().unwrap();
    assert_eq!(record.outcome, TriggerOutcome::NotStarted);
    assert_eq!(record.cause, TriggerCause::Interval);
    assert_eq!(record.message, job.message);
    assert!(!engine.is_automatic(&d.terminal_id));
    engine.set_terminal_state(&d.terminal_id, TerminalState::Idle);
    engine.fire(&d.id, TriggerCause::Manual, 107);
    let job = engine.take_dispatches().pop().unwrap();
    engine.set_enabled(&d.id, false, 108).unwrap();
    engine.cancel_dispatch(&job);
    assert!(!engine.is_automatic(&d.terminal_id));
    assert!(!engine.has_pending(&d.id));
}

fn definition(project: &str, kind: TriggerKind) -> TriggerDefinition {
    TriggerDefinition {
        id: uuid::Uuid::new_v4().to_string(),
        project_id: project.into(),
        terminal_id: uuid::Uuid::new_v4().to_string(),
        title: "Review".into(),
        message: "check\nchanges".into(),
        enabled: true,
        kind,
        working_directory: "/project".into(),
        runtime_target: Default::default(),
        startup_command: String::new(),
        exited: false,
        next_at: None,
    }
}

#[test]
fn triggers_project_completion_git_filter_and_no_recursive_automation() {
    let mut engine = TriggerEngine::default();
    let a = definition("a", TriggerKind::TaskCompletedWithGitChanges);
    let b = definition("b", TriggerKind::TaskCompleted);
    engine.upsert(a.clone(), 100).unwrap();
    engine.upsert(b.clone(), 100).unwrap();
    engine.set_terminal_state(&a.terminal_id, TerminalState::Idle);
    engine.set_terminal_state(&b.terminal_id, TerminalState::Idle);
    engine.complete("a", "source", "one", false, 101);
    assert!(engine.take_dispatches().is_empty());
    engine.complete("a", "source", "two", true, 102);
    let jobs = engine.take_dispatches();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].trigger_id, a.id);
    engine.complete("a", &a.terminal_id, "three", true, 103);
    assert!(engine.take_dispatches().is_empty());
    engine.complete("a", "source", "two", true, 104);
    assert!(
        engine.take_dispatches().is_empty(),
        "duplicate completion must not replay"
    );
}

#[test]
fn triggers_busy_hits_merge_once_and_use_first_message_snapshot() {
    let mut e = TriggerEngine::default();
    let mut d = definition("a", TriggerKind::Manual);
    e.upsert(d.clone(), 100).unwrap();
    e.set_terminal_state(&d.terminal_id, TerminalState::Idle);
    e.fire(&d.id, TriggerCause::Manual, 101);
    assert_eq!(e.take_dispatches().len(), 1);
    d.message = "first waiting".into();
    e.upsert(d.clone(), 102).unwrap();
    e.fire(&d.id, TriggerCause::Manual, 103);
    d.message = "newest".into();
    e.upsert(d.clone(), 104).unwrap();
    e.fire(&d.id, TriggerCause::Manual, 105);
    assert!(e.take_dispatches().is_empty());
    assert!(e.has_pending(&d.id));
    e.complete("a", &d.terminal_id, "done", false, 106);
    let jobs = e.take_dispatches();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].message, "first waiting");
    e.complete("a", &d.terminal_id, "done-next", false, 107);
    assert!(e.take_dispatches().is_empty());
}

#[test]
fn triggers_exit_clears_waiting_but_keeps_schedule_and_records_hits() {
    let mut e = TriggerEngine::default();
    let d = definition("a", TriggerKind::Interval { seconds: 5 });
    e.upsert(d.clone(), 100).unwrap();
    e.set_terminal_state(&d.terminal_id, TerminalState::Idle);
    e.fire(&d.id, TriggerCause::Manual, 101);
    e.take_dispatches();
    e.fire(&d.id, TriggerCause::Manual, 102);
    e.mark_exited(&d.id);
    e.tick(105);
    assert!(!e.has_pending(&d.id));
    assert!(e.take_dispatches().is_empty());
    assert_eq!(
        e.records("a").last().unwrap().outcome,
        TriggerOutcome::Exited
    );
    assert_eq!(e.definition(&d.id).unwrap().next_at, Some(110));
    assert!(e.definition(&d.id).unwrap().exited);
}

#[test]
fn triggers_pause_clears_pending_without_interrupting_current_task() {
    let mut e = TriggerEngine::default();
    let d = definition("a", TriggerKind::Manual);
    e.upsert(d.clone(), 100).unwrap();
    e.set_terminal_state(&d.terminal_id, TerminalState::Idle);
    e.fire(&d.id, TriggerCause::Manual, 101);
    e.take_dispatches();
    e.fire(&d.id, TriggerCause::Manual, 102);
    e.set_enabled(&d.id, false, 103).unwrap();
    assert!(!e.has_pending(&d.id));
    e.complete("a", &d.terminal_id, "done", false, 104);
    e.fire(&d.id, TriggerCause::Manual, 105);
    assert!(e.take_dispatches().is_empty());
}

#[test]
fn triggers_interval_and_once_alarm_do_not_replay_missed_ticks() {
    let mut e = TriggerEngine::default();
    let d = definition("a", TriggerKind::Interval { seconds: 5 });
    e.upsert(d.clone(), 100).unwrap();
    e.tick(200);
    assert_eq!(e.definition(&d.id).unwrap().next_at, Some(205));
    assert_eq!(e.records("a").len(), 1);
    let alarm = definition(
        "a",
        TriggerKind::Alarm {
            hour: 12,
            minute: 0,
            once_at: Some(210),
        },
    );
    e.upsert(alarm.clone(), 200).unwrap();
    e.tick(210);
    assert_eq!(e.definition(&alarm.id).unwrap().next_at, None);
    let count = e
        .records("a")
        .iter()
        .filter(|r| r.trigger_id == alarm.id)
        .count();
    e.tick(220);
    assert_eq!(
        e.records("a")
            .iter()
            .filter(|r| r.trigger_id == alarm.id)
            .count(),
        count
    );
}

#[test]
fn triggers_store_preserves_each_project_layout_exits_and_rules_across_restart() {
    let path = std::env::temp_dir().join(format!("codux-triggers-{}", uuid::Uuid::new_v4()));
    let store = TriggerStore::open(path.clone()).unwrap();
    let mut e = TriggerEngine::default();
    let a = definition("a", TriggerKind::Manual);
    let b = definition("b", TriggerKind::Interval { seconds: 3 });
    e.upsert(a.clone(), 100).unwrap();
    e.upsert(b.clone(), 100).unwrap();
    e.mark_exited(&a.id);
    store.save(&e.data).unwrap();
    let restored = TriggerEngine::restore(store.load().unwrap(), 200).unwrap();
    assert_eq!(restored.project("a").unwrap().definitions.len(), 1);
    assert!(restored.definition(&a.id).unwrap().exited);
    assert!(restored.definition(&b.id).unwrap().next_at.unwrap() > 200);
    assert!(restored.project("a").unwrap().layout.is_some());
    std::fs::remove_file(path.join("triggers.json")).unwrap();
    std::fs::remove_dir(path).unwrap();
}

#[test]
fn triggers_validation_and_corrupt_storage_never_silently_overwrite() {
    let mut e = TriggerEngine::default();
    let mut d = definition("a", TriggerKind::Interval { seconds: 0 });
    assert!(e.upsert(d.clone(), 100).is_err());
    d.kind = TriggerKind::Manual;
    d.message = "   ".into();
    assert!(e.upsert(d, 100).is_err());
    let path =
        std::env::temp_dir().join(format!("codux-triggers-corrupt-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("triggers.json"), "broken").unwrap();
    assert!(TriggerStore::open(path.clone()).is_err());
    assert_eq!(
        std::fs::read_to_string(path.join("triggers.json")).unwrap(),
        "broken"
    );
    std::fs::remove_file(path.join("triggers.json")).unwrap();
    std::fs::remove_dir(path).unwrap();
}

#[test]
fn triggers_failed_delivery_is_recorded_without_retrying_or_leaving_busy() {
    let mut e = TriggerEngine::default();
    let d = definition("a", TriggerKind::Manual);
    e.upsert(d.clone(), 100).unwrap();
    e.set_terminal_state(&d.terminal_id, TerminalState::Idle);
    e.fire(&d.id, TriggerCause::Manual, 101);
    e.take_dispatches();
    e.delivery_failed(&d.id, "write failed", 102);
    assert_eq!(
        e.records("a").last().unwrap().outcome,
        TriggerOutcome::Failed
    );
    e.fire(&d.id, TriggerCause::Manual, 103);
    assert_eq!(e.take_dispatches().len(), 1);
}

#[test]
fn triggers_completed_one_time_alarm_can_be_paused_and_renamed() {
    let mut e = TriggerEngine::default();
    let mut d = definition(
        "a",
        TriggerKind::Alarm {
            hour: 12,
            minute: 0,
            once_at: Some(110),
        },
    );
    e.upsert(d.clone(), 100).unwrap();
    e.tick(110);
    d = e.definition(&d.id).unwrap().clone();
    d.title = "renamed".into();
    e.upsert(d.clone(), 120).unwrap();
    e.set_enabled(&d.id, false, 121).unwrap();
    assert!(!e.definition(&d.id).unwrap().enabled);
    assert!(e.set_enabled(&d.id, true, 122).is_err());
}

#[test]
fn triggers_exit_is_not_a_task_completion_and_late_events_do_not_fan_out() {
    let mut e = TriggerEngine::default();
    let d = definition("a", TriggerKind::TaskCompleted);
    e.upsert(d.clone(), 100).unwrap();
    e.set_terminal_state(&d.terminal_id, TerminalState::Idle);
    e.fire(&d.id, TriggerCause::Manual, 101);
    e.take_dispatches();
    e.mark_exited(&d.id);
    e.complete("a", &d.terminal_id, "late", true, 102);
    assert!(e.take_dispatches().is_empty());
    assert!(e.records("a").is_empty());
}

#[test]
fn triggers_host_assigned_terminal_id_preserves_automation_and_pending_message() {
    let mut e = TriggerEngine::default();
    let d = definition("a", TriggerKind::Manual);
    e.upsert(d.clone(), 100).unwrap();
    e.set_terminal_state(&d.terminal_id, TerminalState::Idle);
    e.fire(&d.id, TriggerCause::Manual, 101);
    e.take_dispatches();
    e.fire(&d.id, TriggerCause::Manual, 102);
    e.bind_terminal(&d.id, "host-session").unwrap();
    assert_eq!(e.definition(&d.id).unwrap().terminal_id, "host-session");
    assert!(e.has_pending(&d.id));
    assert!(e.is_automatic("host-session"));
    e.complete("a", "host-session", "done", false, 103);
    let jobs = e.take_dispatches();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].terminal_id, "host-session");
    let other = definition("a", TriggerKind::Manual);
    e.upsert(other.clone(), 104).unwrap();
    assert!(e.bind_terminal(&other.id, "host-session").is_err());
}

#[test]
fn triggers_daily_alarm_moves_to_the_next_calendar_day_after_firing() {
    use chrono::{TimeZone, Timelike};
    let now = chrono::Local::now().timestamp();
    let target = chrono::Local.timestamp_opt(now + 120, 0).single().unwrap();
    let mut e = TriggerEngine::default();
    let d = definition(
        "a",
        TriggerKind::Alarm {
            hour: target.hour(),
            minute: target.minute(),
            once_at: None,
        },
    );
    e.upsert(d.clone(), now).unwrap();
    let first = e.definition(&d.id).unwrap().next_at.unwrap();
    assert!(first > now);
    e.tick(first);
    let next = e.definition(&d.id).unwrap().next_at.unwrap();
    assert!(next > first);
    assert_ne!(
        chrono::Local
            .timestamp_opt(first, 0)
            .single()
            .unwrap()
            .date_naive(),
        chrono::Local
            .timestamp_opt(next, 0)
            .single()
            .unwrap()
            .date_naive()
    );
}
