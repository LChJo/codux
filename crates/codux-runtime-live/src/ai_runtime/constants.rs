/// Backstop window after which a `responding` turn with no fresh signal (hook,
/// transcript progress, or real terminal output) is silently retired via
/// `mark_timed_out`. Real output keeps the turn alive through the output
/// heartbeat, so this only fires on genuine total silence; because retiring is
/// now silent (no "已中断" notification), the window is generous to avoid
/// clearing the loading state during long model "thinking" gaps.
pub const RUNNING_STALE_SECONDS: f64 = 240.0;
pub const POLL_INTERVAL_SECONDS: u64 = 5;
pub const RUNNING_STATE_RENEWAL_SECONDS: f64 = 30.0;
/// Upper bound on how long a "responding" turn may have its heartbeat renewed
/// without genuine transcript progress. A dead session whose transcript still
/// parses as "responding" would otherwise be renewed every poll forever,
/// defeating the staleness aging in `reconcile_bridge_snapshot` and pinning the
/// desktop-pet "running" bubble on a turn that ended long ago. Genuinely active
/// long turns keep themselves fresh through real transcript timestamps, so this
/// only releases turns that have been silent for an extreme duration.
pub const RESPONDING_RENEWAL_MAX_SECONDS: f64 = 1_800.0;
pub const COMPLETION_TIMESTAMP_SKEW_SECONDS: f64 = 1.0;
/// How long a tool call may sit written-but-unanswered in the transcript before
/// the probe treats it as a user-decision wait (`needsInput`) rather than a
/// call about to be auto-answered. Neither Claude nor codex records the moment a
/// tool blocks on an approval/elicitation, so the wait is derived from the
/// pending-call signature plus this idle gap. The gap is comfortably above the
/// sub-second flush between a fast auto-approved call and its result, so quick
/// tool turnarounds never register as a wait. Disambiguating a genuine wait from
/// a slow *running* tool needs the process-tree check (Layer 3); until then this
/// only fires when the session's permission mode can still prompt at all.
pub const NEEDS_INPUT_IDLE_SECONDS: f64 = 4.0;
pub(crate) const NEEDS_INPUT_VISIBLE_SECONDS: f64 = 30.0;
pub const CODEX_INTERVAL_POLL_MINIMUM_SECONDS: f64 = 60.0;
pub const CODEX_LIVE_TRANSCRIPT_TAIL_BYTES: u64 = 128 * 1024;
pub const CODEX_LIVE_TRANSCRIPT_TAIL_LINES: usize = 260;
pub const CODEX_STALE_PRELAUNCH_OPEN_TURN_SOURCE: &str = "probe:codex-stale-prelaunch-open-turn";
pub const CLAUDE_STALE_PRELAUNCH_OPEN_TURN_SOURCE: &str = "probe:claude-stale-prelaunch-open-turn";
pub const TRANSCRIPT_MONITOR_INTERVAL_MS: u64 = 3_000;
pub const TRANSCRIPT_POLL_MINIMUM_SECONDS: f64 = 3.0;
pub const RUNTIME_EVENT_FILE_MAX_AGE_SECONDS: f64 = 300.0;
/// Drop an idle session whose terminal is no longer live after this long.
/// Explicit closes evict immediately via `remove_session`; this only reclaims
/// orphans left by crashes / abnormal terminal disappearance. Far longer than
/// the memory-extraction idle delay so a completed turn is always enqueued
/// before its session is reclaimed.
pub const IDLE_SESSION_RETENTION_SECONDS: f64 = 3_600.0;
