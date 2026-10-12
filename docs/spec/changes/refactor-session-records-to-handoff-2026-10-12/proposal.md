---
created_at: 2026-10-12T00:00:00Z
updated_at: 2026-10-12T00:00:00Z
---

## Why

Codux's "session record" is not a record — it is a token statistic. `AISessionSummary` carries `id / title / source / last_seen_at / tokens / usage_amounts`, which answers "how much did this session cost?" and nothing else. The Task column's Session History list renders exactly those fields.

The only thing in the module with handover semantics is `fork_project_session` (`crates/codux-ai-sessions/src/session_fork.rs`): it reads the transcript tail, cleans it, writes a one-shot markdown file under the temp dir, and pastes `claude <path>` into a terminal. That artifact is **not persisted, not listed, not individually addressable**. Consequences:

- A finished turn leaves no durable answer to "where did this stop, and what is unfinished?".
- The runtime already knows the plan (`AISessionSnapshot.plan`), the final output (`latest_assistant_preview`), and whether the turn was interrupted (`was_interrupted`) — and all of it is discarded after the memory extractor reads it.
- Plan documents and task documents are never captured as referenceable products of a session.
- There is no way to hand *one* tail (a plan, a result summary, a diff) to different work. Fork is all-or-nothing at session granularity.

## What Changes

- **New `codux-handoff` crate** — GPUI-free, depends only on `codux-ai-history`, so the desktop and the headless agent build identical records. Contains the data model, deterministic extractor, transcript cleaner, artifact stager, SQLite store, renderer, and delivery ledger.
- **`HandoffRecord`** with a strict trust split: `canonical` (host-owned, deterministically extracted, reproducible verbatim, integrity-hashed) versus `advisory` (model text, labelled possibly-stale, never promoted to an instruction).
- **Artifacts as first-class, individually addressable products** — plan documents, task documents, the turn's final result, changed-file list, cleaned transcript. Content-addressed by sha256 into the support directory, so one physical copy serves every reference and the id survives rebuilds.
- **Automatic build on turn completion**, hooked into the existing `drain_ai_runtime_events_and_enqueue_memory` alongside memory extraction, plus a manual "Record Handoff Point" that forces a revision.
- **Drag any artifact into a terminal** to attach it. The drop re-verifies the source and pastes a shell-quoted path — reusing the existing terminal paste contract, no new protocol.
- **Task column Session History section replaced by a Handoffs section**: one row per unit of work that stopped somewhere, expanding into draggable artifact chips.
- **Transcript cleaning moved** from `codux-ai-sessions::session_fork` into `codux-handoff::transcript`, so the fork prompt and the handoff builder share one cleaning pass instead of two that could drift.
- **Delivery ledger** recording which artifact was handed to which terminal, so "which tail went to which work?" is answerable afterwards.

## Impact

- Affected specs: `handoff-records` (new), `handoff-artifacts` (new), `handoff-delivery` (new)
- Affected code:
  - `crates/codux-handoff/` — new crate (`types`, `sanitize`, `extract`, `transcript`, `artifacts`, `store`, `render`, `service`, `recovery`)
  - `crates/codux-ai-sessions/src/session_fork.rs` — cleaning helpers removed, now delegates to `codux_handoff::transcript`
  - `apps/desktop/runtime/src/handoff.rs` — new bridge (snapshot conversion, git environment)
  - `apps/desktop/runtime/src/runtime_state/service_handoff.rs` — new `RuntimeService` methods
  - `apps/desktop/runtime/src/runtime_state/service_ai_runtime.rs` — completion trigger
  - `apps/desktop/runtime/src/runtime_state/{types,state,loaders}.rs` — `RuntimeState.handoffs`
  - `apps/desktop/src/handoff.rs` — drag payloads shared by `app` and `terminal`
  - `apps/desktop/src/app/handoff_column.rs`, `handoff_actions.rs` — new UI section and actions
  - `apps/desktop/src/app/task_column.rs` — Session History section replaced
  - `apps/desktop/src/app/workspace_views/terminal_layout.rs` — drop targets
  - `apps/desktop/runtime-assets/i18n/locales/handoff.json` — new locale namespace (10 languages)
- Storage: new `handoff.sqlite3` in the support directory; staged artifacts under `handoffs/<project_id>/<handoff_id>/`. No migration needed — nothing is read from the old session tables that is not still there.
- Remote parity: **not included**. Handoff reads resolve against the local support dir. A remote-hosted project's records live on its host and need an `ai.handoff` op in the shared dispatch table (`codux-ai-sessions::dispatch`) before the desktop can serve them. Until then a hosted project yields no local rows rather than wrong ones.
