import 'dart:async';

import 'package:codux_flutter/models/remote_models.dart';
import 'package:codux_flutter/services/terminal_input_reliable_sender.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test(
    'cancelling pending input after rejection preserves the session block',
    () {
      final sent = <RelayEnvelope>[];
      final sender = TerminalInputReliableSender(
        send: (message) {
          sent.add(message);
          return true;
        },
      );
      sender.onResult = (result) {
        sender.clear(sessionId: result.sessionId, unblock: false);
      };
      sender.send(sessionId: 's', data: 'a', source: 'test');
      sender.send(sessionId: 's', data: 'b', source: 'test');
      sender.handleAck(
        RelayEnvelope(
          type: 'terminal.input.ack',
          sessionId: 's',
          payload: {
            'inputId': (sent.first.payload as Map)['inputId'],
            'status': 'rejected',
          },
        ),
      );
      expect(sender.pendingCount, 0);
      expect(sender.send(sessionId: 's', data: 'c', source: 'test'), isFalse);
      sender.clear(sessionId: 's');
      expect(sender.send(sessionId: 's', data: 'c', source: 'test'), isTrue);
      sender.dispose();
    },
  );

  test('last attempt waits its full ACK window before becoming uncertain', () {
    final timers = <_FakeTimer>[];
    final sent = <RelayEnvelope>[];
    final results = <TerminalInputDeliveryResult>[];
    final sender = TerminalInputReliableSender(
      maxAttempts: 2,
      onResult: results.add,
      send: (message) {
        sent.add(message);
        return true;
      },
      timerFactory: (_, callback) {
        final timer = _FakeTimer(callback);
        timers.add(timer);
        return timer;
      },
    )..setIdempotentSupport(true);
    sender.send(sessionId: 's', data: 'a', source: 'key');
    timers.single.fire();
    expect(sent, hasLength(2));
    expect(sender.pendingCount, 1);
    expect(results, isEmpty);
    sender.handleAck(
      RelayEnvelope(
        type: 'terminal.input.ack',
        sessionId: 's',
        payload: {
          'inputId': (sent.last.payload as Map)['inputId'],
          'status': 'written',
          'ok': true,
        },
      ),
    );
    timers.last.fire();
    expect(results.single.status, TerminalInputDeliveryStatus.succeeded);
  });

  test(
    'exhausted ACKs produce one uncertain result and late ACK is ignored',
    () {
      final timers = <_FakeTimer>[];
      final sent = <RelayEnvelope>[];
      final results = <TerminalInputDeliveryResult>[];
      final sender = TerminalInputReliableSender(
        maxAttempts: 2,
        onResult: results.add,
        send: (message) {
          sent.add(message);
          return true;
        },
        timerFactory: (_, callback) {
          final timer = _FakeTimer(callback);
          timers.add(timer);
          return timer;
        },
      )..setIdempotentSupport(true);
      sender.send(sessionId: 's', data: 'a', source: 'key');
      timers.single.fire();
      timers.last.fire();
      expect(sent, hasLength(2));
      expect(sender.pendingCount, 0);
      expect(results.single.status, TerminalInputDeliveryStatus.uncertain);
      sender.handleAck(
        RelayEnvelope(
          type: 'terminal.input.ack',
          payload: {
            'inputId': (sent.last.payload as Map)['inputId'],
            'ok': true,
          },
        ),
      );
      expect(results, hasLength(1));
    },
  );

  test('older packets cannot retry after newer input is dispatched', () {
    final timers = <_FakeTimer>[];
    final sent = <RelayEnvelope>[];
    final sender = TerminalInputReliableSender(
      send: (message) {
        sent.add(message);
        return true;
      },
      timerFactory: (_, callback) {
        final timer = _FakeTimer(callback);
        timers.add(timer);
        return timer;
      },
    )..setIdempotentSupport(true);
    sender.send(sessionId: 's', data: 'a', source: 'key');
    sender.send(sessionId: 's', data: 'b', source: 'key');
    final secondInputRetry = timers.last;
    timers.first.fire();
    expect(sent, hasLength(2));
    expect(sender.pendingCount, 2);
    secondInputRetry.fire();
    expect(sent.map((m) => (m.payload as Map)['data']), ['a', 'b', 'b']);
    sender.dispose();
  });

  for (final status in ['rejected', 'uncertain']) {
    test('$status ACK fails closed until session reset', () {
      final results = <TerminalInputDeliveryResult>[];
      late RelayEnvelope sent;
      final sender = TerminalInputReliableSender(
        onResult: results.add,
        send: (message) {
          sent = message;
          return true;
        },
      )..setIdempotentSupport(true);
      sender.send(sessionId: 's', data: 'a', source: 'key');
      sender.handleAck(
        RelayEnvelope(
          type: 'terminal.input.ack',
          sessionId: 's',
          payload: {
            'inputId': (sent.payload as Map)['inputId'],
            'status': status,
            'ok': false,
            'error': 'failed',
          },
        ),
      );
      expect(
        results.single.status,
        status == 'rejected'
            ? TerminalInputDeliveryStatus.rejected
            : TerminalInputDeliveryStatus.uncertain,
      );
      expect(results.single.reason, 'failed');
      expect(sender.send(sessionId: 's', data: 'b', source: 'key'), isFalse);
      sender.clear(sessionId: 's');
      expect(sender.send(sessionId: 's', data: 'c', source: 'key'), isTrue);
      sender.dispose();
    });
  }

  test('async native send failure fails closed and clear cancels feedback', () {
    final results = <TerminalInputDeliveryResult>[];
    late RelayEnvelope sent;
    final sender = TerminalInputReliableSender(
      onResult: results.add,
      send: (message) {
        sent = message;
        return true;
      },
    );
    sender.send(sessionId: 's', data: 'a', source: 'key');
    sender.markSendFailed((sent.payload as Map)['inputId'] as String);
    expect(results.single.status, TerminalInputDeliveryStatus.uncertain);
    expect(sender.send(sessionId: 's', data: 'b', source: 'key'), isFalse);
    sender.clear();
    expect(sender.send(sessionId: 's', data: 'c', source: 'key'), isTrue);
    sender.clear();
    expect(results, hasLength(1));
  });

  test(
    'synchronous native rejection returns false without retaining input',
    () {
      final results = <TerminalInputDeliveryResult>[];
      final sender = TerminalInputReliableSender(
        onResult: results.add,
        send: (_) => false,
      );
      expect(sender.send(sessionId: 's', data: 'a', source: 'key'), isFalse);
      expect(sender.pendingCount, 0);
      expect(results.single.status, TerminalInputDeliveryStatus.uncertain);
    },
  );
  test('sends terminal input with input id and retries until ack', () {
    final timers = <_FakeTimer>[];
    final sent = <RelayEnvelope>[];
    final sender = TerminalInputReliableSender(
      activeSessionId: () => 'session-1',
      retryBaseDelay: const Duration(milliseconds: 10),
      send: (message) {
        sent.add(message);
        return true;
      },
      timerFactory: (delay, callback) {
        final timer = _FakeTimer(callback);
        timers.add(timer);
        return timer;
      },
    );

    expect(
      sender.send(sessionId: 'session-1', data: 'a', source: 'key'),
      isTrue,
    );
    sender.setIdempotentSupport(true);
    expect(sent.single.type, 'terminal.input');
    expect(sent.single.sessionId, 'session-1');
    expect((sent.single.payload as Map)['data'], 'a');
    expect((sent.single.payload as Map)['source'], 'key');

    timers.single.fire();

    expect(sent.length, 2);
    sender.handleAck(
      RelayEnvelope(
        type: 'terminal.input.ack',
        payload: {'inputId': (sent.last.payload as Map)['inputId']},
      ),
    );
    expect(sender.pendingCount, 0);
  });

  test('does not resend when active session changes', () {
    final timers = <_FakeTimer>[];
    final sent = <RelayEnvelope>[];
    var activeSession = 'session-1';
    final sender = TerminalInputReliableSender(
      activeSessionId: () => activeSession,
      send: (message) {
        sent.add(message);
        return true;
      },
      timerFactory: (delay, callback) {
        final timer = _FakeTimer(callback);
        timers.add(timer);
        return timer;
      },
    );

    sender.send(sessionId: 'session-1', data: 'a', source: 'key');
    activeSession = 'session-2';
    timers.single.fire();

    expect(sent.length, 1);
    expect(sender.pendingCount, 0);
  });

  test('legacy host does not retry without idempotent capability', () {
    final timers = <_FakeTimer>[];
    var sends = 0;
    final sender = TerminalInputReliableSender(
      send: (_) {
        sends += 1;
        return true;
      },
      timerFactory: (_, callback) {
        final timer = _FakeTimer(callback);
        timers.add(timer);
        return timer;
      },
    );
    sender.send(sessionId: 's', data: 'a', source: 'key');
    timers.single.fire();
    expect(sends, 1);
    expect(sender.pendingCount, 0);
  });
  test('clear cancels pending retry timers', () {
    final timers = <_FakeTimer>[];
    final sender = TerminalInputReliableSender(
      activeSessionId: () => 'session-1',
      send: (_) => true,
      timerFactory: (delay, callback) {
        final timer = _FakeTimer(callback);
        timers.add(timer);
        return timer;
      },
    );

    sender.send(sessionId: 'session-1', data: 'a', source: 'key');
    sender.clear(sessionId: 'session-1');

    expect(sender.pendingCount, 0);
    expect(timers.single.isActive, isFalse);
  });

  test('non retrying control input waits for ACK without retry', () {
    final timers = <_FakeTimer>[];
    final sent = <RelayEnvelope>[];
    final sender = TerminalInputReliableSender(
      activeSessionId: () => 'session-1',
      send: (message) {
        sent.add(message);
        return true;
      },
      timerFactory: (delay, callback) {
        final timer = _FakeTimer(callback);
        timers.add(timer);
        return timer;
      },
    );

    final ok = sender.send(
      sessionId: 'session-1',
      data: '\u0003',
      source: 'key',
      retry: false,
    );

    expect(ok, isTrue);
    expect(sent, hasLength(1));
    expect((sent.single.payload as Map)['data'], '\u0003');
    expect(sender.pendingCount, 1);
    timers.single.fire();
    expect(sender.pendingCount, 0);
    expect(sent, hasLength(1));
  });
}

final class _FakeTimer implements Timer {
  _FakeTimer(this._callback);

  final void Function() _callback;
  var _active = true;

  void fire() {
    if (!_active) return;
    _active = false;
    _callback();
  }

  @override
  void cancel() {
    _active = false;
  }

  @override
  bool get isActive => _active;

  @override
  int get tick => 0;
}
