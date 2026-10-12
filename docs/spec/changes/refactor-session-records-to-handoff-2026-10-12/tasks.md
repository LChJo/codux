---
created_at: 2026-10-12T00:00:00Z
updated_at: 2026-10-12T00:00:00Z
completed_at:
---

## 1. Crate skeleton + data model

- [x] 1.1 Create `crates/codux-handoff/` (`Cargo.toml`: `codux-ai-history`, `regex`, `rusqlite` bundled, `serde`, `serde_json`, `sha2`, `uuid`); register in workspace `members`
- [x] 1.2 `types.rs` — `HandoffRecord`, `HandoffCanonical`, `HandoffAdvisory`, `HandoffArtifact`, `HandoffEnvironment`, `HandoffIntegrity`, `HandoffStatus`, `HandoffArtifactKind`, `ArtifactAvailability`
- [x] 1.3 `types.rs` — input/context types: `HandoffSourceSession` (narrow mirror of `AISessionSnapshot`), `HandoffBuildContext`, `HandoffBuildResult`, `HandoffListRow`, `HandoffRecoveryRow`, `HandoffRecoveryStage`
- [x] 1.4 `types.rs` — bounded-budget constants and `HANDOFF_SCHEMA` / `HANDOFF_ARTIFACT_TRUST` / `HANDOFF_SECURITY_BOUNDARY`
- [x] 1.5 `lib.rs` — `handoff_digest`, `seal_record`, `validate_record`, `handoff_id`, `artifact_id`, `sha256_hex`, `now_seconds`

## 2. Sanitization boundary

- [x] 2.1 `sanitize.rs` — `mask_secrets` (line-oriented: private-key blocks, authorization headers, env assignments, known token shapes, leaked reasoning tags)
- [x] 2.2 `sanitize.rs` — `stabilize_text` iterating mask+clip to a fixed point; `UNSTABLE_TEXT_OMITTED` fallback
- [x] 2.3 `sanitize.rs` — `clip_text` always emitting a truncation marker, including when the budget cannot hold content and marker
- [x] 2.4 Document the functional-field rule at module head: paths and the delivery ledger are stored verbatim
- [x] 2.5 Tests: cross-line masking does not over-match; fixed point holds; tiny budgets still read as truncated; assignment-shaped values are rewritten (proving the bypass is necessary)

## 3. Deterministic extraction

- [x] 3.1 `extract.rs` — keyword patterns for requirements, constraints, decisions, open items, resolved lines, blockers, goal, completion criteria, `KEY=VALUE` facts, exact markers
- [x] 3.2 `extract.rs` — `extract_canonical` (pure; user lines are the only fact source)
- [x] 3.3 `extract.rs` — merge semantics: newest-first, topic/state-key dedup, resolved open items removed from both previous *and* current round, `exact_markers` append-only
- [x] 3.4 `extract.rs` — `merge_next_steps` from non-completed plan items + open items; interrupted turns mark pending steps `interrupted`
- [x] 3.5 `extract.rs` — `source_digest` covering transcript lines *and* session fields that feed advisory/status
- [x] 3.6 Tests: correction supersedes earlier value; resolved line closes its open item; markers accumulate; determinism; digest tracks advisory/status but not token totals

## 4. Transcript cleaning (shared with fork)

- [x] 4.1 `transcript.rs` — migrate `read_recent_lines`, `sanitize_text`, `clamp_snippet`, `strip_codux_injected_context`, `is_runtime_context_text`, base64/image omission, unsafe-key skipping from `codux-ai-sessions::session_fork`
- [x] 4.2 `transcript.rs` — `TranscriptExcerpt` returning `snippets`, role-separated `user_lines` / `assistant_texts`, `final_result`, `omitted_items`
- [x] 4.3 `transcript.rs` — predicate order preserved: unrecognized row type counts as an omission, runtime plumbing does not
- [x] 4.4 `codux-ai-sessions` depends on `codux-handoff`; the fork builder delegated `collect_snippets` to `read_transcript_tail` (newest-32-then-reverse ordering preserved) before phase 12 removed it entirely — the cleaner is now the only shared surface
- [x] 4.5 Tests: omitted-item count propagates through the single-line entry point; reasoning/noise/injected-memory dropped; snippet cap keeps the newest window

## 5. Artifacts

- [x] 5.1 `artifacts.rs` — `ArtifactStager` with content-addressed writes (digest + extension, never the display name), temp-file-then-rename
- [x] 5.2 `artifacts.rs` — `stage_bytes` / `stage_file`, oversized sources kept as `Reference`, mid-read replacement detected
- [x] 5.3 `artifacts.rs` — `verify_artifact` re-reading the source and degrading to `Unavailable`
- [x] 5.4 `artifacts.rs` — `discover_documents` (plan/task classification, depth and entry caps, dependency/build trees skipped, newest-first)
- [x] 5.5 `artifacts.rs` — `prioritize` ranking result > plan > task > diff > changed files > transcript, stable within a kind
- [x] 5.6 Tests: same bytes collapse to one file and one id; changed/deleted source is unavailable; oversized is reference-only; discovery finds `.claude/plans` and `docs/spec/**/tasks.md` but never `node_modules`

## 6. Store

- [x] 6.1 `store.rs` — `handoff.sqlite3` schema: `handoff_records`, `handoff_revisions`, `handoff_artifacts`, `handoff_recoveries` + indexes; WAL, 5s busy timeout
- [x] 6.2 `store.rs` — `save` with monotonic revision (`ON CONFLICT … WHERE revision < excluded.revision`), unsealed records refused, self-healing rebase over a corrupt row
- [x] 6.3 `store.rs` — reads validate the digest; a corrupt row reads as absent so the caller rebuilds
- [x] 6.4 `store.rs` — revision pruning to `MAX_STORED_REVISIONS`, recovery rows to `MAX_RECOVERY_ROWS`
- [x] 6.5 `store.rs` — `list_for_project` / `list_for_worktree` from indexed columns (no payload deserialization), `artifact` lookup for drop resolution, `rename` resealing, `delete`, `prune_orphaned_artifacts`
- [x] 6.6 Tests: lower/equal revision refused; corrupt row rebased; pruning caps hold; unsealed refused

## 7. Render

- [x] 7.1 `render.rs` — `render_document` producing `handoff.md` (where / goal / unfinished / blockers / plan / facts / constraints / decisions / open items / exact requirements / exact markers / artifacts / advisory result)
- [x] 7.2 `render.rs` — `render_injection` producing `<structured_handoff>` with canonical and advisory in separate labelled sections
- [x] 7.3 `render.rs` — `render_within_budget`: single greedy pass plus bounded pop loop; closing tag always emitted; omission marker appended exactly once
- [x] 7.4 Tests: closing tag survives every budget; termination regression for the re-insertion loop; omission marker present when truncated; advisory labelled stale; artifacts rendered with resolved paths and `UNAVAILABLE` marking

## 8. Service + recovery ledger

- [x] 8.1 `service.rs` — `HandoffService::build` pipeline (collect → extract → dedup → artifacts → seal → save → render), `handoff_root`, `document_path`
- [x] 8.2 `service.rs` — status derivation (interrupted / blocked / completed / in-progress), title fallback chain, `force` path
- [x] 8.3 `service.rs` — `resolve_artifact` re-verifying before returning a path; `ResolvedArtifact::is_usable`
- [x] 8.4 `recovery.rs` — `prepared` / `submitted` / `observed` / `failed`, bounded rows, `has_unconfirmed_submission`
- [x] 8.5 Tests: end-to-end build produces a sealed record with document and artifacts; unchanged source reuses the record; forced rebuild bumps revision keeping `created_at`; successive turns revise one record; rename/delete; stale artifact does not resolve; ledger bounds

## 9. Runtime bridge + trigger

- [x] 9.1 `apps/desktop/runtime/src/handoff.rs` — `handoff_source_session` (serde round-trip + explicit plan conversion), `handoff_environment` from `git_status`, `changed_files_document`
- [x] 9.2 `runtime_state/service_handoff.rs` — `build_session_handoff`, `force_handoff_for_terminal`, list/get/rename/remove, `resolve_handoff_artifact`, `deliver_handoff_artifact` (verify → ledger → path), `handoff_injection_block`, `open_handoff_document`, `reveal_handoff_artifacts`
- [x] 9.3 `runtime_state/{types,state,loaders}.rs` — `RuntimeState.handoffs` + `load_handoffs`, refreshed on load and project switch
- [x] 9.4 `service_ai_runtime.rs` — build on `Completion` inside the existing drain; failures traced, never surfaced
- [x] 9.5 `files.rs` — `open_absolute_path` / `reveal_absolute_path` for documents outside any checkout
- [x] 9.6 `HandoffBuildContext.extra_documents` — caller-computed documents (git changed-file list) join the *same* build. An earlier second-pass `attach_git_artifacts` silently dropped them: it saved at the revision the build had just written, and `save` refuses a revision that has not advanced. `extra_documents_are_staged_by_the_build` pins this.

## 10. Desktop UI

- [x] 10.1 `apps/desktop/src/handoff.rs` — `HandoffArtifactDrag` / `HandoffRecordDrag` payloads at crate root, shared by `app` and `terminal`
- [x] 10.2 `app/handoff_actions.rs` — reload, deliver artifact, deliver record, record handoff point, open document, reveal artifacts, toggle expand, rename, delete-with-confirm, localized kind/status labels
- [x] 10.3 `app/handoff_column.rs` — `HandoffLabels`, `TaskHandoffRow`, snapshot, list view, section area, compact row, draggable artifact chips
- [x] 10.4 `app/task_column.rs` — `TaskSectionKind::Handoffs`, `task_section_heading` / `task_empty_state` exposed, Session History section replaced by Handoffs in `TaskColumnView`
- [x] 10.5 `app/app_state.rs` + both construction sites — `handoffs_refreshing`, `expanded_handoff_ids`, `handoff_delete_confirm_id`, `task_section_handoffs_collapsed`, `task_handoff_list_view`
- [x] 10.6 `workspace_views/terminal_layout.rs` — drop targets for both handoff payloads on the terminal pane overlay
- [x] 10.7 Trigger `reload_project_handoffs` on project/worktree switch and after the completion drain lands in the UI

## 11. i18n + docs

- [x] 11.1 `runtime-assets/i18n/locales/handoff.json` — 50 keys × 10 locales, generated by `tools/gen_handoff_i18n.py`
- [x] 11.2 Register the bundle in `runtime/src/i18n.rs`
- [x] 11.3 `docs/spec/changes/refactor-session-records-to-handoff-2026-10-12/` — `meta.json`, `proposal.md`, `design.md`, `tasks.md`, capability deltas
- [x] 11.4 CHANGELOG entries (en + zh-CN)

## 12. Removal of the superseded session-record surface

- [x] 12.1 Enumerate every consumer of `AISessionForkTarget`, the `fork` session op, and `TaskSessionDrag` (desktop, remote host dispatch, `apps/agent`, `apps/mobile`)
- [x] 12.2 Remove `session_fork.rs` prompt generation once handoff injection replaces it, keeping `codux_handoff::transcript` as the shared cleaner
- [x] 12.3 Remove the fork submenu and `TaskSessionDrag` restore path from the task column; drop `TaskSessionListView` and its snapshot
- [x] 12.4 Keep the `ai.session` op table identical on both hosts — remove `fork` from the shared dispatch, not from one side
- [ ] 12.5 Drop `ai.sessions.*` / `ai.session.*` keys that no longer have a caller
  — **deferred, deliberately.** A reference scan reported 56 of 76 `ai.session*`
  keys unreferenced, but that scan only covered Rust and Dart literals; keys
  reached through dynamic lookup or the mobile bundle would be reported unused
  and deleting them would silently drop translations. Pruning needs a scan that
  resolves dynamic key construction first.

## 14. List preview and hover detail

Added after the first live run showed rows carrying a title and counters but no
content — a handoff list you have to open to recognize defeats the purpose.

- [x] 14.1 `render.rs` — `list_preview` (goal → final result → first unfinished step, one clipped line) and `list_detail` (goal, branch, unfinished with an explicit "… N more", blockers, first paragraph of the result, artifact kinds with counts, revision)
- [x] 14.2 `types.rs` — `preview` / `detail` on `HandoffListRow`
- [x] 14.3 `store.rs` — `preview` / `detail` columns, rendered once at write time so scanning or hovering a long list never deserializes a payload
- [x] 14.4 `store.rs` — `backfill_list_projections` on open, so records written by an earlier build gain a preview instead of sitting blank until their next turn
- [x] 14.5 `ui_helpers.rs` — tooltip layer renders one child per line (GPUI's `WhiteSpace` has no `pre-line`, so `whitespace_normal` would collapse the detail into one run-on line); wider max-width for multi-line only, so single-line tooltips are untouched
- [x] 14.6 `handoff_column.rs` — preview line under the title, detail on hover
- [x] 14.7 Tests: preview fallback chain, first-line-only and clipping, detail sections present and bounded, empty record still bounded, backfill on reopen matches a fresh write

## 15. Bugs found by the automated round

Each is pinned by a regression test named here so the pin is findable.

- [x] 15.1 **Digest unstable across a float round trip.** `float_roundtrip` is not enabled for `serde_json`, so a stored `f64` timestamp could parse back one ULP off, `validate_record` rejected rows the store had just written, `get` returned `None`, and the next build restarted the revision count at one. Intermittent (~half of parallel runs). Fixed by canonicalizing the digest body: sorted keys, floats quantized to integer milliseconds. → `digest_survives_a_float_timestamp_roundtrip`, `digest_is_independent_of_json_key_order`, `seal_serialize_validate_roundtrips_reliably`
- [x] 15.2 **A build reported the revision it intended, not the one stored.** `save` may rebase or refuse; `build` returned its local record either way, so a forced handoff point could show revision 1 twice. Fixed by re-reading after save and setting `reused` from whether the write landed. → `forcing_a_handoff_point_advances_the_revision`
- [x] 15.3 **Row envelope rendered as conversation.** A thinking-only assistant row fell through to the generic text walk with the whole row, emitting `uuid`, `timestamp`, `sessionId`, `cwd`, `version`, `gitBranch`, and the model name into the transcript artifact. Found by running a real `~/.claude/projects/*.jsonl` through the pipeline. → `cleaning_never_renders_row_envelope_metadata_as_conversation`
- [x] 15.4 **Staleness guard defeated by a fallback.** `resolve_artifact` set availability to `Unavailable` and then fell back to `source_path`, handing over exactly the path verification had just rejected. → `resolve_artifact_revalidates_a_stale_source`, `delivering_an_artifact_whose_source_changed_is_refused`
- [x] 15.5 **Renderer spun forever on a tight budget.** The omission marker was re-inserted after being popped. → `injection_terminates_for_every_budget`
- [x] 15.6 **`source_digest` missed advisory and status inputs**, so a turn whose only change was the final result looked unchanged and the handoff stopped tracking the session. → `source_digest_tracks_advisory_and_status_inputs`
- [x] 15.7 **Merge order was reversed twice**, letting the *oldest* statement win a dedup so a correction never superseded the value it replaced. → `later_correction_supersedes_earlier_value_for_same_key`
- [x] 15.8 **Caller-supplied documents were dropped.** A second `save` at the revision the build had just written is refused, so the changed-file list never persisted. Moved into the build via `HandoffBuildContext.extra_documents`. → `extra_documents_are_staged_by_the_build`, `caller_supplied_documents_reach_the_stored_record`
- [x] 15.9 **Bridge tests shared one logical identity.** Every fixture used `project-a` / `external-session-1`, so parallel tests collided on the same handoff id against process-global registries. Fixtures now mint unique ids.

## 16. Verification status

- [x] `cargo check --workspace --all-targets` — clean
- [x] `cargo clippy` on `codux-handoff`, `codux-ai-sessions`, `codux-runtime`, `codux` — no findings in changed code
- [x] `cargo test -p codux-handoff` — 71 passed
- [x] `cargo test -p codux-ai-sessions` — 6 passed
- [x] `cargo test -p codux` (desktop bin) — 254 passed
- [x] `cargo test -p codux-runtime --lib app_runtime_ready_tests` — 38 passed, **10 consecutive runs, 0 failures** (was 4/8 before 15.1–15.2)
- [x] Live dev run: a real `claude` turn produced a record with `status=completed`, two staged artifacts, a rendered `handoff.md`, and verbatim Windows paths in the functional fields
- [x] Live dev run: reopening the store backfilled `preview` / `detail` for the record written before those columns existed
- [x] Real transcript verified clean through `read_transcript_tail`: `user_lines == ["hi"]`, `assistant_texts == ["Hi! How can I help you today?"]`, 9 items omitted, zero envelope metadata
- [ ] **Manual, needs a human at the GUI**: run one turn, confirm the row shows its preview line and the hover card shows goal / unfinished / result / artifact kinds
- [ ] **Manual**: drag an artifact chip into a different terminal and confirm the shell-quoted path lands in the input line
- [ ] Pre-existing failures unrelated to this change: 6 in `codux-ai-history` (confirmed identical on a clean `HEAD` worktree), 2 in `codux-git` (uncommitted WIP predating this work), and a set of Windows path-separator assertions in `codux-runtime` (`files`, `remote::host`, `pet`, `project_store`) in files this change does not touch
