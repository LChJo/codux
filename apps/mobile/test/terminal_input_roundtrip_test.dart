import 'dart:async';

import 'package:codux_flutter/models/remote_models.dart';
import 'package:codux_flutter/services/terminal_editing_controller.dart';
import 'package:codux_flutter/services/terminal_input_reliable_sender.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  for (final rtt in [0, 300, 800, 2000]) {
    testWidgets('quote edits reach an idempotent peer once at ${rtt}ms RTT', (
      tester,
    ) async {
      final writes = <String>[];
      final accepted = <String>{};
      final timers = <Timer>[];
      final results = <TerminalInputDeliveryResult>[];
      late TerminalInputReliableSender sender;
      sender = TerminalInputReliableSender(
        activeSessionId: () => 's',
        onResult: results.add,
        send: (message) {
          final payload = message.payload as Map;
          final id = payload['inputId'] as String;
          if (accepted.add(id)) writes.add(payload['data'] as String);
          timers.add(
            Timer(Duration(milliseconds: rtt), () {
              sender.handleAck(
                RelayEnvelope(
                  type: 'terminal.input.ack',
                  sessionId: 's',
                  payload: {
                    'inputId': id,
                    'ok': true,
                    'accepted': true,
                    'status': 'written',
                  },
                ),
              );
            }),
          );
          return true;
        },
      )..setIdempotentSupport(true);
      final editor = TerminalEditingController(
        onText: (text) =>
            sender.send(sessionId: 's', data: text, source: 'ime'),
        onKeyBytes: (bytes) =>
            sender.send(sessionId: 's', data: bytes, source: 'key'),
      );
      editor.field.value = const TextEditingValue(
        text: '\u200b""',
        selection: TextSelection.collapsed(offset: 2),
      );
      editor.field.value = const TextEditingValue(
        text: '\u200b"中"',
        selection: TextSelection.collapsed(offset: 3),
      );
      editor.sendKey(const TerminalKeyIntent('right'));
      editor.field.value = const TextEditingValue(
        text: '\u200bx',
        selection: TextSelection.collapsed(offset: 2),
      );
      editor.sendKey(const TerminalKeyIntent('enter'));
      await tester.pump(Duration(milliseconds: rtt + 1));
      expect(writes, ['""', '\u001b[D', '中', '\u001b[C', 'x', '\r']);
      expect(sender.pendingCount, 0);
      expect(results, hasLength(6));
      expect(
        results.every((r) => r.status == TerminalInputDeliveryStatus.succeeded),
        isTrue,
      );
      for (final timer in timers) {
        timer.cancel();
      }
      sender.dispose();
      editor.dispose();
    });
  }
}
