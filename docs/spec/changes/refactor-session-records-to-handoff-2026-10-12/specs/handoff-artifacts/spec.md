## ADDED Requirements

### Requirement: Artifact Model
The system SHALL define a `HandoffArtifact` as an individually addressable work product attached to a handoff record. Each artifact SHALL carry a stable `id`, a `kind`, a display `name`, an optional `staged_path` and `source_path`, an optional content `sha256`, a byte count, a one-line `purpose`, an `availability`, and a `trust` value that SHALL always be `reference-data`.

#### Scenario: Every artifact is reference data
- **WHEN** a handoff record is built
- **THEN** every artifact SHALL carry `trust` equal to `reference-data`
- **AND** no artifact SHALL be marked as granting permission or as an instruction

#### Scenario: Artifact kinds cover the work products
- **WHEN** artifacts are classified
- **THEN** the kind SHALL be one of `plan`, `task`, `result`, `diff`, `changedFiles`, `transcript`, `file`

### Requirement: Content-Addressed Staging
The system SHALL stage artifact bytes under a filename derived from the content digest and extension only, never from the display name. Identical bytes SHALL produce one physical file and one artifact id regardless of the name supplied. Writes SHALL go to a temporary file and then be renamed, so a crash mid-write cannot leave a content-addressed path holding partial bytes.

#### Scenario: Identical content collapses to one file
- **WHEN** the same bytes are staged twice under different display names
- **THEN** both artifacts SHALL share the same `id` and `sha256`
- **AND** both SHALL resolve to the same `staged_path`
- **AND** exactly one physical file SHALL exist

#### Scenario: Artifact ids survive rebuilds
- **WHEN** a handoff is rebuilt and an artifact's content is unchanged
- **THEN** its `id` SHALL be unchanged
- **AND** a reference to that id captured before the rebuild SHALL still resolve

#### Scenario: Empty content is not staged
- **WHEN** staging is asked to store zero bytes
- **THEN** no artifact SHALL be produced

#### Scenario: A partial write cannot be read as complete
- **WHEN** staging is interrupted before the rename
- **THEN** no content-addressed path SHALL hold the partial bytes

### Requirement: Artifact Freshness Verification
The system SHALL re-verify an artifact's source before delivering it. Verification SHALL re-read the source and compare its size and content digest against the staged record. On any mismatch, or when the source is missing or unreadable, availability SHALL become `Unavailable` and the artifact SHALL resolve to no path. A staged copy SHALL NOT be treated as evidence that the source is current.

#### Scenario: A changed source makes the artifact unavailable
- **WHEN** an artifact's source file is modified after staging
- **THEN** verification SHALL report `Unavailable`
- **AND** `resolved_path` SHALL return none

#### Scenario: A deleted source makes the artifact unavailable
- **WHEN** an artifact's source file is removed after staging
- **THEN** verification SHALL report `Unavailable`

#### Scenario: An intact source resolves to the staged copy
- **WHEN** an artifact's source is unchanged and its staged copy exists
- **THEN** verification SHALL report `Staged`
- **AND** `resolved_path` SHALL return the staged path

#### Scenario: A replacement mid-read is detected
- **WHEN** the source changes between the initial stat and the post-read stat
- **THEN** the artifact SHALL NOT be recorded as staged from that read

### Requirement: Oversized Sources Are References
The system SHALL keep a source larger than the per-artifact staging budget as a `Reference` rather than copying it. A reference SHALL carry the source path and size, SHALL NOT carry a staged copy, and SHALL still resolve to the source path for a reader with local file access.

#### Scenario: A large plan document is referenced not copied
- **WHEN** a source file exceeds the staging budget
- **THEN** the artifact's availability SHALL be `Reference`
- **AND** `staged_path` SHALL be none
- **AND** `resolved_path` SHALL return the source path

### Requirement: Document Discovery
The system SHALL discover plan and task documents within a project checkout by filename and location, classifying plan-shaped names and paths as `plan` and task-shaped ones as `task`. Discovery SHALL be bounded by depth and by total directory entries examined, SHALL skip dependency and build trees, and SHALL return results newest-first.

#### Scenario: Plan and task documents are found
- **WHEN** a checkout contains `.claude/plans/refactor.md`, `docs/spec/changes/x/tasks.md`, and `docs/spec/changes/x/design.md`
- **THEN** discovery SHALL return all three
- **AND** the `tasks.md` entry SHALL be classified `task`
- **AND** the other two SHALL be classified `plan`

#### Scenario: Unrelated documents are not attached
- **WHEN** a checkout contains `README.md`
- **THEN** discovery SHALL NOT return it

#### Scenario: Dependency trees are never walked
- **WHEN** a plan-shaped file exists under `node_modules`
- **THEN** discovery SHALL NOT return it

#### Scenario: Results are newest-first
- **WHEN** discovery returns multiple documents
- **THEN** they SHALL be ordered by modification time, newest first

### Requirement: Generated Artifacts
The system SHALL synthesize artifacts for content that has no source file: the turn's final result, the cleaned transcript tail, and the changed-file list. The final-result artifact SHALL carry enough provenance to stand alone in a conversation that has never seen the handoff, and SHALL state that it is advisory model output.

#### Scenario: The final result becomes an artifact
- **WHEN** a record has a non-empty `advisory.final_result`
- **THEN** a `result` artifact SHALL be staged
- **AND** its content SHALL identify the source tool, session, and project
- **AND** it SHALL state that the content is advisory and may be stale

#### Scenario: An empty result produces no artifact
- **WHEN** `advisory.final_result` is absent
- **THEN** no `result` artifact SHALL be staged

#### Scenario: The cleaned transcript becomes an artifact
- **WHEN** the transcript walk produced snippets
- **THEN** a `transcript` artifact SHALL be staged from them

#### Scenario: Changed files are listed verbatim
- **WHEN** the checkout has files differing from HEAD
- **THEN** a `changedFiles` artifact SHALL be staged listing each path verbatim
- **AND** the paths SHALL NOT be sanitized or truncated

### Requirement: Artifact Prioritization
The system SHALL rank artifacts so the most load-bearing products survive the per-record cap, ordering `result` before `plan` before `task` before `diff` before `changedFiles` before `transcript` before `file`. Ranking SHALL be stable within a kind, preserving the caller's newest-first order, and SHALL deduplicate by artifact id.

#### Scenario: A result outranks a plan
- **WHEN** a record holds both a `result` and a `plan` artifact
- **THEN** the `result` SHALL be ordered first

#### Scenario: Duplicate ids collapse
- **WHEN** the same artifact id appears twice in the input
- **THEN** the prioritized list SHALL contain it once

### Requirement: Orphan Pruning Is Explicit
The system SHALL provide a maintenance operation that removes staged files no longer referenced by any artifact row. Because content addressing lets two handoffs share one physical file, this operation SHALL NOT run automatically during a build.

#### Scenario: Pruning is not part of a build
- **WHEN** a handoff is built
- **THEN** no staged file belonging to another handoff SHALL be removed
