## ADDED Requirements

### Requirement: Artifact Delivery As Attachment
The system SHALL deliver a handoff artifact into a terminal by pasting its resolved path into the PTY input line, shell-quoted, using the same contract as an operating-system file drop. Delivery SHALL NOT introduce a new wire protocol, an attachment object, or a separate composer. The paste SHALL NOT include a newline, so the user can review the command before running it.

#### Scenario: A dropped artifact lands as a quoted path
- **WHEN** an artifact chip is dropped on a terminal pane
- **THEN** the artifact's resolved path SHALL be written to that pane's input line
- **AND** a path containing spaces SHALL be single-quoted
- **AND** an embedded single quote SHALL be escaped as `'\''`
- **AND** no newline SHALL be sent

#### Scenario: The receiving CLI reads the file itself
- **WHEN** a path has been pasted into a running AI CLI's input line
- **THEN** delivery SHALL be complete without the system reading or transmitting the file's contents

#### Scenario: Quoting matches the existing file-drop path
- **WHEN** the same absolute path is delivered by artifact drop and by operating-system file drop
- **THEN** both SHALL produce identical input-line text

### Requirement: Delivery Re-Resolves At Drop Time
The system SHALL treat the drag payload as advisory and SHALL re-resolve the artifact through the store at drop time. A payload captured at drag start SHALL NOT be trusted to carry a current path, because the source may change mid-drag.

#### Scenario: A source changed mid-drag is refused
- **WHEN** an artifact's source changes between drag start and drop
- **THEN** the drop SHALL paste nothing
- **AND** the user SHALL be told the source changed

#### Scenario: An unknown artifact is refused
- **WHEN** a drop carries an artifact id that is not in the store
- **THEN** the drop SHALL paste nothing
- **AND** the failure SHALL be reported rather than silently ignored

### Requirement: Delivery Ledger
The system SHALL record each delivery attempt as a ledger row carrying the handoff id, artifact id, stage, target terminal, resolved path, timestamp, and an optional detail. Stages SHALL be `prepared`, `submitted`, `observed`, `failed`, and `superseded`. The ledger SHALL be bounded per handoff, pruning the oldest rows on write. The resolved path SHALL be stored verbatim.

#### Scenario: A successful drop is recorded as submitted
- **WHEN** an artifact path is pasted into a terminal
- **THEN** a ledger row SHALL be written with stage `submitted`
- **AND** it SHALL carry the target terminal id and the resolved path

#### Scenario: A refused delivery is recorded as failed
- **WHEN** delivery is refused because the source changed
- **THEN** a ledger row SHALL be written with stage `failed`
- **AND** its detail SHALL carry the availability that caused the refusal

#### Scenario: An unacknowledged submission is detectable
- **WHEN** a `submitted` row exists with no later `observed` row for the same handoff
- **THEN** the system SHALL report an unconfirmed submission

#### Scenario: The ledger is bounded
- **WHEN** more than `MAX_RECOVERY_ROWS` rows are written for one handoff
- **THEN** only the most recent `MAX_RECOVERY_ROWS` SHALL remain

### Requirement: Whole-Record Delivery
The system SHALL support delivering an entire handoff record by pasting the path of its rendered document. The document path SHALL be recomputed at drop time rather than taken from the drag payload, because the document is re-rendered on every build.

#### Scenario: A record drag delivers its document
- **WHEN** a handoff row is dropped on a terminal pane
- **THEN** the path of that record's rendered document SHALL be pasted

#### Scenario: A missing document is reported
- **WHEN** the rendered document does not exist at drop time
- **THEN** nothing SHALL be pasted
- **AND** the user SHALL be told the record was not found

### Requirement: Drag Availability
The system SHALL offer a drag for every artifact chip, including unavailable ones, because the drop handler is the authority on deliverability. Gating the drag at the source would duplicate that check in a place that can go stale between drag start and drop. An unavailable chip SHALL be visually distinguished and SHALL state that its source changed.

#### Scenario: An unavailable chip still drags but never delivers
- **WHEN** an artifact whose source changed is dragged to a terminal
- **THEN** the chip SHALL be draggable
- **AND** the drop SHALL paste nothing

#### Scenario: Availability is visible before dragging
- **WHEN** an artifact's availability is not `Staged` or `Reference`
- **THEN** its chip SHALL display an unavailable marker

### Requirement: Drop Target Highlighting
The system SHALL highlight a terminal pane as a drop target for handoff artifact drags and handoff record drags using the same overlay already used for session drags, so the affordance is consistent across drag sources.

#### Scenario: A handoff drag highlights the pane
- **WHEN** a handoff artifact or record drag hovers a terminal pane
- **THEN** that pane SHALL show the drop overlay

### Requirement: Terminal Resolution
The system SHALL resolve the delivery target from the drop site when the drop provides one, and otherwise from the active terminal. An absent or blank target SHALL be reported rather than defaulting to an arbitrary pane.

#### Scenario: A drop on a specific pane targets that pane
- **WHEN** an artifact is dropped on a particular terminal pane
- **THEN** the path SHALL be pasted into that pane and not another

#### Scenario: A missing target is reported
- **WHEN** delivery is requested with no resolvable terminal id
- **THEN** nothing SHALL be pasted
- **AND** the user SHALL be told the terminal split was not found

### Requirement: Delivery Focus
The system SHALL move focus to the receiving terminal after a successful paste, so the user can immediately review and submit the command.

#### Scenario: A successful paste selects and focuses the target
- **WHEN** an artifact path is pasted into a terminal
- **THEN** that terminal SHALL become the active runtime terminal
- **AND** it SHALL receive keyboard focus
