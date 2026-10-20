# Codux Remote Protocol v3.2

This document records the wire semantics implemented by the desktop host,
headless agent host, and mobile/desktop controllers. It is the compatibility
reference for protocol changes.

## Envelope

All business messages use the same JSON envelope:

```json
{
  "type": "terminal.output",
  "deviceId": "device-1",
  "sessionId": "terminal-1",
  "seq": 42,
  "payload": {}
}
```

- `type` is the message kind.
- `deviceId` identifies the controller device when the message is device-scoped.
- `sessionId` identifies a terminal session when applicable.
- `requestId` optionally correlates a request and its reply; hosts echo it.
- `seq` is a transport receive-order guard, not a terminal byte offset.
- `payload` carries the domain-specific fields.

Receivers use a `SequenceGuard` window of 128 sequence numbers to drop duplicate
or old inbound envelopes.

## Capability Discovery

Hosts advertise `host.info.capabilities`. Controllers must tolerate unknown
fields and missing fields. For terminal recovery, v3.2 hosts advertise:

- `terminalBuffer.chunking`, `maxChars`, `chunkChars`, `requestId`, `screenData`,
  `baselineFailed`.
- `terminalOutput.sequence` and `staleOutput`.
- `terminalViewport.ownership`, `state`, `scroll`, `keyframe`.
- `domains.hostMetrics` for pull-based remote host resource metrics.

## Terminal Input Delivery

The desktop host advertises `terminalInput: {"idempotent": true, "ordered": true,
"ackAfterWrite": true}` in `host.info.capabilities`. These flags are opt-in:
headless or older hosts that omit them must receive send-once input without
controller retries.

`terminal.input.payload` contains `data` and an optional unique `inputId` (1-256
UTF-8 bytes). For capable hosts, retry the same device/session/inputId with the
same data; retry only within 60 seconds. The desktop serializes terminal operations
per authenticated device; unrelated control requests and ping remain concurrent.
It retains input results for at least 60 seconds,
including rejected and failed writes. Duplicate inputs replay the cached ACK,
including after terminal ownership changes. Runtime restart ends this guarantee;
controllers must discard outstanding input when reconnecting or observing a new
`runtimeInstanceId`. Inputs without an id remain compatible and execute once per
received envelope.

`terminal.input.ack` echoes `inputId`, `ok`, `accepted`, and `status`:

- `written`: `ok` and `accepted` are true, after the full PTY write and flush.
- `rejected`: both are false; validation, ownership, startup, or cache capacity
  prevented writing.
- `uncertain`: both are false; writing failed after delivery may have begun.
  Never replay the input with a new id because a prefix may have reached the PTY.

Failure ACKs include `error`. All ACK outcomes are final for that input id. A
full result cache rejects new inputs instead of evicting unexpired results.
After capacity rejection, admission of unknown ids resumes only after 60 seconds
without another such rejection, so delayed retries cannot reverse a rejection.
Sequence numbers are checked at ingress before work is scheduled. Queued messages
from obsolete host transport generations or disconnected device epochs are
discarded before processing. Fresh device connections use a new terminal queue;
already executing operations cannot be canceled.

Controllers also bind asynchronous input actions to the current terminal and
connection. AI session restore commands require a matching `requestId` and the
same project/session/input binding that initiated the request. Unsolicited,
duplicate, stale, or legacy restore replies without `requestId` must not insert
or submit a command into the currently active terminal.

## Host Metrics

`host.metrics` is a controller-initiated request/reply over the same transport
as `host.info`. It is deliberately pull-only: a desktop panel polls while it is
open and stops when hidden, so the host has no permanent watcher or broadcast
state.

Controllers correlate `host.metrics` replies by message kind, matching the
existing generic request path. Implementations may echo `requestId`, but it is
not required for this domain.

Controllers must first check `host.info.capabilities.domains.hostMetrics`.
Missing or false means the host is an older version; clients should show an
unsupported state and must not keep retrying `host.metrics`.

The reply payload is a `RemoteHostMetrics` snapshot:

- `sampledAtMillis`: host sample time in Unix milliseconds.
- `system`: `hostname`, `osName`, `osVersion`, `kernelVersion`, `arch`,
  `uptimeSeconds`, `utcOffsetSeconds`.
- `cpu`: `totalUsagePercent`, per-core `cores`, and optional `loadAvg`.
- `memory`: total/used/available/free RAM plus total/used swap bytes.
- `network`: aggregate receive/transmit totals and bytes-per-second rates.
- `disks`: mount/name/fs type, total/available bytes, read/write rates.
- `processes`: CPU-sorted process rows; hosts cap the list before sending.

Rates are host-side deltas between adjacent samples. The first sample reports
zero rates.

## Resource Subscription

Current controllers subscribe through the generic resource API:

- `resource.subscribe { resource, projectId?, sessionId?, baseline?, requestId?,
  maxChars?, chunkChars?, baselineSessionId?, viewportCols?, viewportRows? }`
- `resource.unsubscribe { resource, projectId?, sessionId? }`

For `resource: "terminals"`:

- A project subscription registers the device as a viewer for the project
  terminal set.
- A session subscription registers the device as a viewer for one session.
- `baseline: true` asks the host to send a catch-up baseline.
- `baselineSessionId` targets viewport metadata at the active split while still
  subscribing to the whole project.
- `viewportCols`/`viewportRows` are only applied when the requester already owns
  the viewport lease. They never implicitly steal ownership.

Legacy `terminal.subscribe` and `terminal.unsubscribe` are compatibility aliases
for session-scoped terminal subscriptions. New clients should use
`resource.subscribe`/`resource.unsubscribe`.

## Terminal Ownership

The terminal viewport has one owner at a time:

- `terminal.viewport.claim` claims ownership for `remote:<deviceId>`.
- `terminal.viewport.claim { renewOnly: true }` only renews the lease if the
  requester already owns it.
- `terminal.viewport.resize { cols, rows }` claims ownership and resizes the PTY
  to the requester grid.
- `terminal.viewport.release` releases the requester lease.
- `terminal.viewport.state` broadcasts `{ owner, cols, rows, generation,
  staleOutput, outputSeq }`.

The lease is 20 seconds. `generation` is monotonic per session and lets clients
ignore stale viewport-state messages.

Controllers must not mirror another owner grid into their renderer. If the owner
is the desktop or another remote device, the controller shows a handoff
placeholder until the user explicitly takes over.

## Terminal Output

`terminal.output` has two wire modes, distinguished by `payload.buffer`.

### Live Output

Live output has no `buffer: true`:

```json
{
  "type": "terminal.output",
  "sessionId": "terminal-1",
  "payload": {
    "data": "...",
    "outputSeq": 123,
    "bufferLength": 1000,
    "bufferEnd": 5000
  }
}
```

- `outputSeq` is a host flush counter. It is not a byte offset.
- `bufferLength` is the retained-history character count; `bufferEnd` is the
  monotonic character position for the terminal lifetime.
- The host emits live output in short batches.
- Clients ack received frames with `terminal.output.ack { outputSeq,
  bufferLength? }`. Controllers may throttle acks as long as they stay within
  the host stale-output tolerance.

### Baseline Output

Baseline output has `buffer: true`:

```json
{
  "type": "terminal.output",
  "sessionId": "terminal-1",
  "payload": {
    "buffer": true,
    "data": "...",
    "offset": 0,
    "startOffset": 0,
    "bufferLength": 1000,
    "bufferEnd": 5000,
    "truncated": false,
    "tail": true,
    "hasPrevious": false,
    "requestId": "req-1",
    "screenData": "\u001b[2J\u001b[H...",
    "outputSeq": 123
  }
}
```

- `data` is ANSI terminal history for replay. Normal-screen tail snapshots
  serialize the host's parsed scrollback into a compact ANSI stream, so repeated
  in-place TUI repaints cannot crowd older output out of a raw-byte tail.
  Alternate-screen snapshots retain raw replay plus the active screen keyframe.
- `offset` and `bufferLength` are character positions within the transferred
  history representation. For compact tail snapshots this starts at zero and
  counts the serialized snapshot, independently of the raw PTY stream.
- Tail baselines include `bufferEnd`, captured atomically with the history and
  screen keyframe. It always counts the raw PTY stream, even when `data` is a
  compact representation. New clients use it to remove live output already
  covered by that baseline; historical offset pages omit it.
- `startOffset` is the beginning of the retained window. Chunk assembly rewrites
  the assembled `offset` back to this value.
- `truncated` means the requested offset page has more data after this window.
- `tail: true` means the payload is a latest-history tail snapshot, not an
  offset page. It is present on every chunk of a tail baseline.
- `hasPrevious` means retained history exists before `startOffset`.
- `requestId` binds chunks to one explicit baseline request.
- `screenData` is an owner-gated viewport keyframe for the current grid.
  A live keyframe refreshes only the visible grid; it must preserve already
  parsed scrollback and the reader's scroll position independently of the raw
  wire-cache budget.
- `baselineFailed: true` marks a failed baseline snapshot. The client closes the
  current wait and schedules a retry instead of spinning forever.

Large baselines are chunked with `chunked`, `snapshotId`, `chunkIndex`,
and `chunkCount`. Each chunk still carries its bytes in `data` and its character
position in `offset`; there is no separate `chunkData` or `chunkOffset` field.
`screenData` is sent only on the first chunk. A chunked transfer is complete when
`chunkIndex == chunkCount - 1`; an unchunked baseline is a single complete
payload.

## Recovery State Machine

The controller recovery loop has three repair signals:

1. Local output-sequence gap detection.
2. Host `terminal.viewport.state.staleOutput` with an 8-frame lag tolerance.
3. `baselineFailed: true`.

All three request a fresh baseline with a 2.5 second backoff. Live output that
arrives while a baseline is pending is held and replayed after the baseline so
old snapshots do not overwrite newer output.

Baseline `outputSeq` is sampled close to the snapshot but is not byte-atomic
with history. Clients intentionally tolerate this skew through sequence guards,
held-live replay, gap detection, and baseline retries.

## Keepalive Channels

There are three separate keepalive roles:

- `transport.ping`/`transport.pong` proves the link is responsive.
- `terminal.viewport.claim { renewOnly: true }` renews the viewport lease without
  stealing ownership.
- `terminal.output.ack` proves the viewer is receiving terminal output and also
  keeps the current viewport lease alive.

Do not add another keepalive path without updating this section.

## Deprecated Compatibility

- `terminal.subscribe` and `terminal.unsubscribe` are kept for older
  controllers. New clients must use `resource.subscribe` and
  `resource.unsubscribe`.
- `terminal.resize` is legacy. New clients must use
  `terminal.viewport.resize`. Legacy resize messages with missing or invalid
  dimensions are rejected.
