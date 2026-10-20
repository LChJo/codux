import 'dart:async';

import '../models/remote_models.dart';
import 'log_service.dart';
import 'remote_protocol.dart';

typedef TerminalInputReliableSend = bool Function(RelayEnvelope message);
typedef TerminalInputActiveSession = String? Function();
typedef TerminalInputReliableTimerFactory =
    Timer Function(Duration delay, void Function() callback);

enum TerminalInputDeliveryStatus { succeeded, rejected, uncertain }

class TerminalInputDeliveryResult {
  const TerminalInputDeliveryResult({
    required this.inputId,
    required this.sessionId,
    required this.status,
    this.reason,
  });

  final String inputId;
  final String sessionId;
  final TerminalInputDeliveryStatus status;
  final String? reason;
}

class TerminalInputReliableSender {
  TerminalInputReliableSender({
    required TerminalInputReliableSend send,
    TerminalInputActiveSession? activeSessionId,
    this.maxAttempts = 4,
    this.retryBaseDelay = const Duration(milliseconds: 700),
    TerminalInputReliableTimerFactory? timerFactory,
    this.onResult,
  }) : assert(maxAttempts > 0),
       _send = send,
       _activeSessionId = activeSessionId,
       _timerFactory = timerFactory ?? Timer.new;

  final TerminalInputReliableSend _send;
  final TerminalInputActiveSession? _activeSessionId;
  final TerminalInputReliableTimerFactory _timerFactory;
  final int maxAttempts;
  final Duration retryBaseDelay;
  void Function(TerminalInputDeliveryResult result)? onResult;

  final Map<String, _PendingTerminalInput> _pending = {};
  final Map<String, String> _latestInput = {};
  final Set<String> _blockedSessions = {};
  bool _idempotentSupported = false;
  int _seq = 0;

  int get pendingCount => _pending.length;

  /// Reset this on every connection; only an explicitly capable host may retry.
  void setIdempotentSupport(bool supported) {
    _idempotentSupported = supported;
  }

  bool send({
    required String sessionId,
    required String data,
    required String source,
    bool retry = true,
  }) {
    if (sessionId.isEmpty ||
        data.isEmpty ||
        _blockedSessions.contains(sessionId)) {
      return false;
    }
    final active = _activeSessionId?.call();
    if (active != null && active != sessionId) return false;
    final inputId = '${DateTime.now().microsecondsSinceEpoch}-${++_seq}';
    CoduxLog.info(
      '[codux-flutter-input] source=$source id=$inputId bytes=${data.codeUnits.length} session=$sessionId',
    );
    _pending[inputId] = _PendingTerminalInput(
      inputId: inputId,
      sessionId: sessionId,
      data: data,
      source: source,
      retry: retry,
    );
    _latestInput[sessionId] = inputId;
    return _sendPending(inputId);
  }

  void handleAck(RelayEnvelope message) {
    final payload = message.payload;
    if (payload is! Map) return;
    final inputId = payload['inputId']?.toString();
    final pending = _pending[inputId];
    if (pending == null) return;
    if (message.sessionId != null && message.sessionId != pending.sessionId) {
      return;
    }
    final status = payload['status'];
    if (status == 'uncertain') {
      _blockedSessions.add(pending.sessionId);
      _finish(
        pending,
        TerminalInputDeliveryStatus.uncertain,
        payload['error']?.toString(),
      );
    } else if (status == 'rejected' ||
        payload['ok'] == false ||
        payload['accepted'] == false) {
      _blockedSessions.add(pending.sessionId);
      _finish(
        pending,
        TerminalInputDeliveryStatus.rejected,
        payload['error']?.toString(),
      );
    } else {
      _finish(pending, TerminalInputDeliveryStatus.succeeded);
    }
  }

  /// The outgoing queue calls this when an asynchronously queued native send
  /// fails. Stop retries and further input until the connection/session resets.
  void markSendFailed(String inputId, {String? reason}) {
    final pending = _pending[inputId];
    if (pending == null) return;
    _blockedSessions.add(pending.sessionId);
    _finish(
      pending,
      TerminalInputDeliveryStatus.uncertain,
      reason ?? 'transport-send-failed',
    );
  }

  /// Discard pending input on device/connection/session changes. Cancellation
  /// intentionally produces no delivery feedback for an obsolete session.
  void clear({String? sessionId, bool unblock = true}) {
    for (final pending in _pending.values.toList()) {
      if (sessionId != null && pending.sessionId != sessionId) continue;
      pending.retryTimer?.cancel();
      _pending.remove(pending.inputId);
    }
    if (sessionId == null) {
      if (unblock) _blockedSessions.clear();
      _latestInput.clear();
    } else {
      if (unblock) _blockedSessions.remove(sessionId);
      _latestInput.remove(sessionId);
    }
  }

  void dispose() => clear();

  bool _sendPending(String inputId) {
    final pending = _pending[inputId];
    if (pending == null) return false;
    final active = _activeSessionId?.call();
    if (active != null && pending.sessionId != active) {
      clear(sessionId: pending.sessionId);
      return false;
    }
    pending.retryTimer?.cancel();
    pending.attempt += 1;
    final sent = _send(
      RelayEnvelope(
        type: RemoteMessageType.terminalInput,
        sessionId: pending.sessionId,
        payload: {
          'data': pending.data,
          'inputId': pending.inputId,
          'source': pending.source,
        },
      ),
    );
    if (!sent) {
      markSendFailed(inputId);
      return false;
    }
    // A synchronous adapter can deliver an ACK while sending.
    if (!_pending.containsKey(inputId)) return true;
    pending.retryTimer = _timerFactory(
      pending.retry && _idempotentSupported
          ? Duration(
              milliseconds: retryBaseDelay.inMilliseconds * pending.attempt,
            )
          : _remainingAckWindow(0),
      () => _onAckTimeout(pending),
    );
    return true;
  }

  void _onAckTimeout(_PendingTerminalInput pending) {
    if (!identical(_pending[pending.inputId], pending)) return;
    final active = _activeSessionId?.call();
    if (active != null && active != pending.sessionId) {
      clear(sessionId: pending.sessionId);
      return;
    }
    if (!pending.waitingOnly &&
        pending.retry &&
        _idempotentSupported &&
        !_blockedSessions.contains(pending.sessionId) &&
        _latestInput[pending.sessionId] != pending.inputId &&
        pending.attempt < maxAttempts) {
      // Suppress old retries without declaring delivery unknown at 700 ms on
      // a high-RTT connection. Keep its remaining normal ACK budget instead.
      pending.waitingOnly = true;
      pending.retryTimer = _timerFactory(
        _remainingAckWindow(pending.attempt),
        () => _onAckTimeout(pending),
      );
      return;
    }
    // Older packets must never append after newer input. Native transport is
    // ordered; once newer data is queued, await only the older packet's ACK.
    if (pending.retry &&
        _idempotentSupported &&
        !_blockedSessions.contains(pending.sessionId) &&
        _latestInput[pending.sessionId] == pending.inputId &&
        pending.attempt < maxAttempts) {
      _sendPending(pending.inputId);
      return;
    }
    _finish(pending, TerminalInputDeliveryStatus.uncertain, 'ack-timeout');
  }

  Duration _remainingAckWindow(int completedAttempt) {
    var multiplier = 0;
    for (
      var attempt = completedAttempt + 1;
      attempt <= maxAttempts;
      attempt++
    ) {
      multiplier += attempt;
    }
    return Duration(milliseconds: retryBaseDelay.inMilliseconds * multiplier);
  }

  void _finish(
    _PendingTerminalInput pending,
    TerminalInputDeliveryStatus status, [
    String? reason,
  ]) {
    if (_pending.remove(pending.inputId) == null) return;
    pending.retryTimer?.cancel();
    if (_latestInput[pending.sessionId] == pending.inputId) {
      _latestInput.remove(pending.sessionId);
    }
    onResult?.call(
      TerminalInputDeliveryResult(
        inputId: pending.inputId,
        sessionId: pending.sessionId,
        status: status,
        reason: reason,
      ),
    );
  }
}

class _PendingTerminalInput {
  _PendingTerminalInput({
    required this.inputId,
    required this.sessionId,
    required this.data,
    required this.source,
    required this.retry,
  });
  final String inputId;
  final String sessionId;
  final String data;
  final String source;
  final bool retry;
  int attempt = 0;
  bool waitingOnly = false;
  Timer? retryTimer;
}
