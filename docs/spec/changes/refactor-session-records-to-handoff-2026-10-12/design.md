## Context

Codux already has every input a handoff needs — it just throws them away. `AISessionSnapshot` (`crates/codux-runtime-live/src/ai_runtime/snapshot.rs`) carries `transcript_path`, `plan: AIPlanSnapshot { items[{text, status, priority}] }`, `latest_assistant_preview`, `has_completed_turn`, `was_interrupted`, `completed_turn_started_at`. The completion event that carries it (`AIRuntimeSupervisorEvent::Completion`) is already drained in `drain_ai_runtime_events_and_enqueue_memory` (`apps/desktop/runtime/src/runtime_state/service_ai_runtime.rs`), where memory extraction is enqueued. And the terminal already accepts dropped paths: `.drag_over::<ExternalPaths>` → `terminal_paths_input` → shell-quote → `paste_text` (`apps/desktop/src/terminal/{render,view,keys}.rs`).

What was missing was the record itself. `AISessionSummary` is a token aggregate; `fork_project_session` produces a throwaway prompt file under the temp dir. Neither can answer "what is unfinished here, and which document carries it?".

The reference implementation is AImana's Cowork Session Handoff (`src/main/modules/cowork/session-handoff.ts`, `src/shared/cowork-session-handoff.ts`, `handoff-attachments.ts`, `handoff-recovery.ts`), plus the A2A handoff's security boundary (`src/main/modules/a2a-workspace/handoff-builder.ts`).

## Goals / Non-Goals

- Goals:
  - A durable, per-session record of where work stopped: goal, verified facts, unfinished steps, blockers, artifacts.
  - Artifacts individually addressable and draggable, so one tail can go to one conversation and another tail to another.
  - Deterministic extraction, so records are reproducible, integrity-checkable, and free of model confabulation in the canonical layer.
  - No new wire protocol for delivery — reuse the existing terminal path-paste contract.
  - One transcript cleaner shared by the fork prompt and the handoff builder.
- Non-Goals:
  - Remote-host parity. Reads resolve against the local support dir; hosted projects need an `ai.handoff` op in `codux-ai-sessions::dispatch` first.
  - LLM-generated summaries as a default. The advisory `summary` field exists and is plumbed, but nothing populates it yet.
  - A handoff detail window. v1 is the task-column section plus the rendered `handoff.md`; a detail surface is a follow-on.
  - Cross-session or cross-project handoff graphs (which tail descended from which) beyond the flat delivery ledger.

## Decisions

### D1: Canonical vs advisory is a structural split, not a label

`HandoffCanonical` holds only what the host derived deterministically from user-authored lines or from the runtime's own plan snapshot. `HandoffAdvisory` holds model text. They are separate subtrees, so the renderer cannot accidentally mix them and the digest covers the distinction.

- Decision: two structs, separate sections in both renderers, advisory always carries a "may be stale" line plus `HANDOFF_SECURITY_BOUNDARY`.
- Alternative considered: one flat struct with a `trust` field per entry (rejected — a reader that forgets to check the field silently promotes model text to fact).

### D2: Deterministic extraction, no model in the loop

Keyword and shape matching, ported from AImana: requirement lines (`STATE-CHECK|`, `KEY=VALUE`, `必须|不得|禁止|保留|上限|要求|固定|待确认|未决`), key/value facts, constraints, decisions, open items with resolved-topic removal, exact markers.

- Decision: pure functions in `extract.rs`, no I/O, no clock, no model.
- Why: reproducibility. A digest over a model-generated summary is not stable, so dedup and integrity both break. It also means a handoff never asserts something the user did not say.
- The optional LLM summary lands in `advisory.summary`, which is excluded from the "must reproduce verbatim" contract.

### D3: `source_digest` dedup + monotonic `revision`

`source_digest` fingerprints *every* input the build reads — including `latest_assistant_preview`, `state`, `was_interrupted`, and the plan. If it matches the stored value and the build is not forced, the previous record is returned and nothing is written.

`revision` only ever advances: `ON CONFLICT(id) DO UPDATE … WHERE revision < excluded.revision`. A stored row that fails its own digest does not block progress — the incoming record is rebased above it.

- Why both: dedup alone would let a stale snapshot roll a record back; monotonicity alone would churn a revision per turn even when nothing changed.
- Caught during implementation: an early `source_digest` covered only transcript lines, so a turn whose *only* change was the final result was treated as unchanged and the handoff silently stopped tracking the session. `source_digest_tracks_advisory_and_status_inputs` pins this.

### D4: Content-addressed artifacts, and a stale copy is not evidence

Staged files are named by sha256 (digest + extension only, never the display name), written via a temp file then renamed. `verify_artifact` re-reads the source and compares size and digest; on any mismatch availability becomes `Unavailable` and `resolved_path()` returns `None`.

- Decision: the drop handler refuses to paste anything that did not just re-verify.
- Why: handing a reader yesterday's `design.md` is worse than reporting it unavailable — the reader cannot tell the difference.
- The artifact id is the digest prefix, so it survives rebuilds and a reference dropped earlier still resolves.

### D5: Functional fields are never sanitized

`source_path`, `staged_path`, `project_path`, `transcript_path`, and the recovery ledger's `resolved_path` are stored verbatim. Sanitization (`sanitize.rs`) applies only to display fields.

- Why: masking is not idempotent. A value stored masked and later compared against a freshly masked runtime string diverges for *some* inputs, which presents as an intermittent recovery failure rather than a masking bug. AImana hit exactly this and recorded it (`.claude/wiki/lessons/cowork-handoff-recovery-context.md`).
- The masker here is deliberately line-oriented so a line ending in `token =` cannot pull the next line's first token into the match — that cross-line over-match is the mechanism that broke idempotency there. `masking_does_not_reach_across_a_line_boundary` pins it.
- Where truncation is unavoidable (`clip_text`), the marker is always emitted, even when the budget cannot hold both content and marker. A silently truncated value reads as a complete one.

### D6: Delivery reuses the terminal path-paste contract

A dropped artifact becomes a shell-quoted absolute path pasted into the PTY input line — identical to dropping a file from the OS. The AI CLI reads the path with its own file tools.

- Decision: no new protocol, no attachment object, no composer UI.
- Why: every supported CLI already understands a path in its input line; anything richer would need per-tool integration. It also means the user can see and edit exactly what will be sent before pressing enter.
- The drag payload carries a best-effort path, but the drop handler re-resolves through `deliver_handoff_artifact` — the payload is never trusted.

### D7: One cleaner, two consumers

`codux_handoff::transcript` owns tail reading and cleaning; `codux-ai-sessions::session_fork` now calls it.

- Why: two cleaners would eventually disagree about what is safe to carry forward, and the disagreement would be invisible until a secret or a base64 blob leaked into a prompt.
- The cleaner also returns role-separated `user_lines` / `assistant_texts`, which is what lets `extract` treat only user text as fact-bearing.

### D8: Build hooks the existing completion drain

`build_session_handoff` runs inside `drain_ai_runtime_events_and_enqueue_memory`, next to memory enqueue. Failures are traced, not surfaced.

- Why: one definition of "the turn ended". A separate watcher would eventually disagree with the memory queue about which turns completed.
- A handoff that cannot be built must never block the event loop or the memory queue.

### D9: The digest is canonicalized, not serialized verbatim

`handoff_digest` hashes a canonicalized body: object keys sorted, every float quantized to integer milliseconds.

- Why key sorting: `serde_json` is built with `preserve_order` in this workspace, so a `Map` is an `IndexMap` and byte output follows insertion order. An integrity hash should not depend on that.
- Why float quantization: `float_roundtrip` is **not** enabled, so parsing a serialized `f64` can land one ULP away from the original. A record hashed before storing and re-hashed after reading back could disagree. The symptom was severe and intermittent — `validate_record` rejected rows the store had just written, so `get` returned `None`, the next build saw no previous record, and the revision count silently restarted from one. Only ~half of runs failed, and only for timestamps whose shortest representation did not parse back exactly.
- Alternative considered: enabling `serde_json/float_roundtrip` workspace-wide. Rejected as the primary fix — it changes a shared dependency's parsing behavior for every crate to work around one hash, and an integrity digest should not depend on float text formatting at all. Quantizing is local and removes the dependency permanently.

### D10: A build reports what was persisted, not what it intended to write

`HandoffService::build` re-reads the record from the store after saving and returns that, with `reused` reflecting whether the write actually landed.

- Why: `save` can rebase the revision above a row that failed its own digest check, and can refuse the write when an equal or newer revision arrived first. Returning the locally built record in those cases reports a revision that was never stored — which is how a forced "record handoff point" could show revision 1 twice.

### D11: A row envelope is never treated as message content

The transcript cleaner distinguishes "this row wraps its message" from "this payload *is* the message". When a wrapped row yields no usable text — a thinking-only assistant turn, for instance — it is counted as omitted and skipped. It is never handed to the generic text walk.

- Why: the generic walk collects every string it recurses into. Given a whole Claude row it collected `uuid`, `parentUuid`, `promptId`, `timestamp`, `sessionId`, `cwd`, `version`, `gitBranch`, `userType`, `permissionMode`, and the model name, and rendered them as conversation. Found by reading a real `~/.claude/projects/*.jsonl` through the pipeline: the transcript artifact for a two-message session was mostly metadata.
- Belt and braces: `is_envelope_metadata_key` also rejects those key names inside `collect_message_text`, normalizing camelCase and snake_case to one form first. The structural guard in `push_row` is the real fix; the denylist stops a future caller from reintroducing the leak.
- `cleaning_never_renders_row_envelope_metadata_as_conversation` pins this using row shapes copied verbatim from a live transcript.

## Risks / Trade-offs

- **Artifact discovery cost.** `discover_documents` walks the checkout to depth 4 with a 4000-entry cap and skips dependency/build trees. On a pathological checkout this could still be slow; it runs off the UI thread, and the cap bounds it.
- **Keyword extraction is language-shaped.** The requirement/constraint/decision patterns carry Chinese and English keywords. A session in another language yields a thinner canonical block; goal fallback (first user message) and the plan snapshot still work.
- **No remote parity yet.** A hosted project shows no handoffs. Chosen over a half-wired remote path that could serve stale rows.
- **Staged bytes accumulate.** Content addressing dedupes within a handoff and across handoffs that share content, but deletion only unlinks index rows. `prune_orphaned_artifacts` exists and is deliberately not called automatically — it needs a maintenance entry point.

## Migration

None. The new store is `handoff.sqlite3`, separate from `ai-usage.sqlite3` and `memory.sqlite3`. Nothing reads a handoff table that did not exist before, and the old session tables are untouched.
