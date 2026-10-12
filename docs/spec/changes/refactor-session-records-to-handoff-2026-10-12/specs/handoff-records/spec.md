## ADDED Requirements

### Requirement: Handoff Record Model
The system SHALL define a `HandoffRecord` as the durable record of where one AI session's work stopped, keyed by `(project_id, session_key)` so successive turns revise one record instead of accumulating one per turn. The record SHALL carry provenance (`source_tool`, `external_session_id`, `transcript_path`, `worktree_id`), versioning (`revision`, `created_at`, `updated_at`, `source_digest`), a derived `status`, a `canonical` block, an `advisory` block, an `artifact` list, an `environment` block, and an `integrity` digest.

#### Scenario: One record per session, revised per turn
- **WHEN** two turns of the same session are built into handoffs
- **THEN** both SHALL share the same record `id`
- **AND** the second SHALL have `revision` exactly one greater than the first
- **AND** the project's handoff list SHALL contain one row, not two

#### Scenario: Session key falls back to the terminal when no external id exists
- **WHEN** a session snapshot carries no `ai_session_id`
- **THEN** the record's `session_key` SHALL be the snapshot's `terminal_id`

#### Scenario: Functional fields are stored verbatim
- **WHEN** a record is persisted with a `transcript_path`, `project_path`, or artifact `source_path`
- **THEN** each SHALL be stored byte-identical to its input value
- **AND** none SHALL be passed through secret masking or truncation

### Requirement: Canonical Versus Advisory Trust Split
The system SHALL separate host-owned content from model-generated content into two distinct subtrees. `HandoffCanonical` SHALL contain only values derived deterministically from user-authored transcript lines or from the runtime's own plan snapshot. `HandoffAdvisory` SHALL contain model output. No renderer or consumer SHALL present advisory content as canonical, and advisory content SHALL never be marked as reproducible verbatim.

#### Scenario: Model assertions never become facts
- **WHEN** extraction runs over a transcript whose assistant output states a decision and whose user lines do not
- **THEN** `canonical.facts` and `canonical.decisions` SHALL NOT contain that decision
- **AND** the assistant text SHALL appear only under `advisory`

#### Scenario: Renderers keep the blocks separate
- **WHEN** a record is rendered to its injection block
- **THEN** the canonical goal section SHALL precede the advisory section
- **AND** the advisory section SHALL be labelled as possibly stale
- **AND** the security boundary statement SHALL be present

### Requirement: Deterministic Extraction
The system SHALL derive every canonical field by deterministic keyword and shape matching over user-authored lines, without invoking a model. Extraction SHALL be a pure function of its inputs: identical inputs SHALL produce byte-identical canonical blocks.

#### Scenario: Key/value shapes become facts
- **WHEN** a user line contains `MAX_RETRIES=5`
- **THEN** `canonical.facts` SHALL contain `{ key: "MAX_RETRIES", value: "5", source: "user" }`

#### Scenario: Requirement keywords select exact requirements
- **WHEN** a user line contains a requirement keyword (`必须`, `不得`, `禁止`, `保留`, `上限`, `要求`, `固定`, `待确认`, `未决`) or a `STATE-CHECK|` / `KEY=VALUE` shape
- **THEN** that line SHALL appear in `canonical.exact_requirements`

#### Scenario: Empty values are rejected
- **WHEN** a key/value match yields a value containing no alphanumeric character
- **THEN** no fact SHALL be registered for it

#### Scenario: Extraction is reproducible
- **WHEN** the same input is extracted twice
- **THEN** both canonical blocks SHALL be equal
- **AND** both source digests SHALL be equal

### Requirement: Incremental Merge Semantics
The system SHALL merge each round's extraction into the previous revision's canonical block. Merging SHALL be newest-first: a later user statement SHALL supersede an earlier one on the same topic, while statements about different topics SHALL accumulate. `canonical.exact_markers` SHALL be append-only and SHALL NOT drop an entry when the requirement that produced it is superseded.

#### Scenario: A correction replaces the value it corrects
- **WHEN** user lines contain `MAX_RETRIES=3` and then `MAX_RETRIES=5`
- **THEN** `canonical.facts` SHALL contain exactly one `MAX_RETRIES` entry
- **AND** its value SHALL be `5`

#### Scenario: A resolved line closes its open item
- **WHEN** user lines contain `待确认：X` and then `已确认：X` in the same round
- **THEN** `canonical.open_items` SHALL NOT contain `X`

#### Scenario: Resolved lines also close items carried from a previous revision
- **WHEN** the previous revision registered `待确认：X` and the current round contains `已解决：X`
- **THEN** `canonical.open_items` SHALL NOT contain `X`

#### Scenario: Markers survive supersession
- **WHEN** revision N registered `STATE-CHECK|BASELINE=1` and revision N+1 registers `STATE-CHECK|FINAL=2`
- **THEN** revision N+1's `canonical.exact_markers` SHALL contain both markers

### Requirement: Unfinished Work Derivation
The system SHALL populate `canonical.next_steps` from the runtime plan snapshot's non-completed items, followed by open items not already covered by the plan. When the turn was interrupted, pending plan-derived steps SHALL be marked `interrupted`. Each step SHALL record which source produced it.

#### Scenario: Completed plan items are not tails
- **WHEN** the plan snapshot contains items with statuses `completed`, `pending`, and `in_progress`
- **THEN** `canonical.next_steps` SHALL contain the `pending` and `in_progress` items
- **AND** SHALL NOT contain the `completed` item

#### Scenario: An interrupted turn marks its pending steps
- **WHEN** the session snapshot reports `was_interrupted`
- **THEN** each non-completed plan-derived step SHALL carry status `interrupted`

#### Scenario: Steps carried from a previous revision survive a stale plan
- **WHEN** the previous revision recorded a step and the current plan snapshot no longer mentions it
- **THEN** `canonical.next_steps` SHALL still contain that step

### Requirement: Handoff Status Derivation
The system SHALL derive `status` from runtime state only, never from model text. The derivation order SHALL be: `was_interrupted` → `Interrupted`; a non-empty blocker list or a `needsInput` state → `Blocked`; `has_completed_turn` → `Completed`; `is_running` → `InProgress`; otherwise `Completed`.

#### Scenario: An interrupted turn is recorded as interrupted
- **WHEN** the session snapshot reports `was_interrupted`
- **THEN** `status` SHALL be `Interrupted`

#### Scenario: Awaiting input is recorded as blocked
- **WHEN** the session snapshot's state is `needsInput`
- **THEN** `status` SHALL be `Blocked`

### Requirement: Source Digest Deduplication
The system SHALL fingerprint every input a build reads — transcript-derived user and assistant lines, session title, plan snapshot, and the session fields that feed status and advisory content — as `source_digest`. When the computed digest equals the stored one and the build is not forced, the system SHALL return the stored record and SHALL NOT write a new revision.

#### Scenario: An unchanged source does not churn revisions
- **WHEN** a build runs twice against an unchanged session snapshot
- **THEN** the second result SHALL report `reused`
- **AND** `revision` and `updated_at` SHALL be unchanged

#### Scenario: A changed final result is a changed source
- **WHEN** only `latest_assistant_preview` differs between two snapshots
- **THEN** their source digests SHALL differ
- **AND** the second build SHALL write a new revision

#### Scenario: Irrelevant fields do not churn revisions
- **WHEN** only `total_tokens` differs between two snapshots
- **THEN** their source digests SHALL be equal

#### Scenario: A forced build always writes
- **WHEN** a build runs with `force` set against an unchanged source
- **THEN** a new revision SHALL be written
- **AND** `created_at` SHALL be preserved from the original record

### Requirement: Record Integrity
The system SHALL seal every record with a sha256 digest computed over the record body excluding the `integrity` field itself. Reads SHALL re-validate the digest; a record whose digest does not match SHALL be reported as absent so the caller rebuilds from source. Persisting an unsealed record SHALL fail.

#### Scenario: A sealed record round-trips
- **WHEN** a sealed record is serialized and validated
- **THEN** validation SHALL succeed and return an equal record

#### Scenario: A tampered payload is rejected
- **WHEN** a stored payload's canonical goal is modified without resealing
- **THEN** validation SHALL return none

#### Scenario: An unsealed record cannot be saved
- **WHEN** `save` receives a record whose `integrity.sha256` does not match its body
- **THEN** the call SHALL return an error and write nothing

### Requirement: Monotonic Revision With Self-Healing
The system SHALL never allow a lower or equal revision to overwrite a higher one. When the stored row fails its own digest validation, the system SHALL treat it as unusable and SHALL rebase the incoming record above the stored revision rather than refusing the write.

#### Scenario: A stale build is refused
- **WHEN** revision 5 is stored and a build attempts revision 3
- **THEN** the write SHALL be refused
- **AND** the stored revision SHALL remain 5

#### Scenario: A corrupt row does not freeze the handoff
- **WHEN** the stored payload fails digest validation and an incoming record carries a lower revision
- **THEN** the incoming record SHALL be rebased to `stored_revision + 1`
- **AND** the write SHALL succeed

### Requirement: Bounded Revision History
The system SHALL retain at most `MAX_STORED_REVISIONS` revisions per record and at most `MAX_RECOVERY_ROWS` delivery-ledger rows per record, pruning the oldest on write.

#### Scenario: Old revisions are pruned
- **WHEN** more than `MAX_STORED_REVISIONS` revisions are written for one record
- **THEN** the oldest SHALL no longer be readable
- **AND** the newest SHALL be readable

### Requirement: Automatic Build On Turn Completion
The system SHALL build a handoff from the same drain that enqueues memory extraction for a completed turn, so both consumers share one definition of "the turn ended". A build failure SHALL be traced and SHALL NOT block the event loop or the memory queue.

#### Scenario: A completed turn produces a record
- **WHEN** the runtime drain yields a `Completion` event carrying a session snapshot
- **THEN** a handoff SHALL be built for that session

#### Scenario: A failed build does not break the drain
- **WHEN** a build returns an error
- **THEN** the error SHALL be traced
- **AND** memory enqueueing and event delivery SHALL proceed unaffected

### Requirement: Manual Handoff Point
The system SHALL provide a manual action that forces a revision for the session running in a given terminal, regardless of whether the source digest changed.

#### Scenario: Forcing records a boundary the digest cannot see
- **WHEN** the user invokes "Record Handoff Point" on a terminal whose session inputs are unchanged
- **THEN** a new revision SHALL be written

### Requirement: Rendered Handoff Document
The system SHALL render each record to a markdown document at a path that is a pure function of `(project_id, handoff_id)`, so a consumer can locate it without rebuilding. The document SHALL present canonical and advisory content in separate sections, SHALL list every artifact with its resolved path, and SHALL mark an unavailable artifact as such.

#### Scenario: The document path is derivable
- **WHEN** a caller knows a project id and handoff id
- **THEN** it SHALL be able to compute the document path without reading the store

#### Scenario: Unavailable artifacts are marked
- **WHEN** a record contains an artifact whose availability is `Unavailable`
- **THEN** the document SHALL label it as unavailable rather than printing a stale path as current

### Requirement: Bounded Injection Block
The system SHALL render a `<structured_handoff>` block within a caller-supplied character budget. The closing tag SHALL always be emitted, the opening tag SHALL survive even a degenerate budget, and any dropped section SHALL be replaced by an explicit omission marker. Rendering SHALL terminate for every budget.

#### Scenario: The closing tag survives a tight budget
- **WHEN** a record is rendered with a budget smaller than its full content
- **THEN** the output SHALL end with `</structured_handoff>`
- **AND** its length SHALL NOT exceed the budget

#### Scenario: Truncation is never silent
- **WHEN** sections are dropped to fit the budget
- **THEN** the output SHALL contain an explicit omission marker

#### Scenario: A degenerate budget still terminates
- **WHEN** a record is rendered with a budget smaller than the closing tag
- **THEN** rendering SHALL terminate
- **AND** the output SHALL be a clipped form of the closing tag
