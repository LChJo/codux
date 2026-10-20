import 'package:codux_flutter/models/remote_models.dart';
import 'package:codux_flutter/services/remote_terminal_output_controller.dart';
import 'package:codux_flutter/widgets/components/self_drawn_terminal_view.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('paired quote insertion preserves the IME caret', (tester) async {
    final input = await _pumpInput(tester);
    input.field.value = const TextEditingValue(
      text: '\u200b""',
      selection: TextSelection.collapsed(offset: 2),
    );
    expect(input.text, ['""']);
    expect(input.keys, ['\u001b[D']);
    input.text.clear();
    input.keys.clear();
    input.field.value = const TextEditingValue(
      text: '\u200b"x"',
      selection: TextSelection.collapsed(offset: 3),
    );
    expect(input.text, ['x']);
    expect(input.keys, isEmpty);
  });

  testWidgets('selection-only IME updates move the terminal caret', (
    tester,
  ) async {
    final input = await _pumpInput(tester);
    input.field.value = const TextEditingValue(
      text: '\u200bab',
      selection: TextSelection.collapsed(offset: 3),
    );
    input.text.clear();
    input.keys.clear();
    input.field.selection = const TextSelection.collapsed(offset: 2);
    expect(input.text, isEmpty);
    expect(input.keys, ['\u001b[D']);
  });

  testWidgets('one emoji deletion sends one backspace', (tester) async {
    final input = await _pumpInput(tester);
    input.field.value = const TextEditingValue(
      text: '\u200ba😀',
      selection: TextSelection.collapsed(offset: 4),
    );
    input.text.clear();
    input.keys.clear();
    input.field.value = const TextEditingValue(
      text: '\u200ba',
      selection: TextSelection.collapsed(offset: 2),
    );
    expect(input.keys, ['\u007f']);
    expect(input.text, isEmpty);
  });

  testWidgets('session switch clears composing and committed input', (
    tester,
  ) async {
    final input = await _pumpInput(tester);
    input.field.value = const TextEditingValue(
      text: '\u200bni',
      selection: TextSelection.collapsed(offset: 3),
      composing: TextRange(start: 1, end: 3),
    );
    await tester.pumpWidget(input.build('session-2'));
    expect(input.field.value.text, '\u200b');
    expect(input.field.value.composing, TextRange.empty);
    expect(input.text, isEmpty);
    expect(input.keys, isEmpty);
  });

  testWidgets('renders the Rust cell snapshot and reports a sane grid size', (
    tester,
  ) async {
    final controller = RemoteTerminalOutputController();
    addTearDown(controller.dispose);
    controller.bindSession('session-1', requireBaseline: true);
    controller.accept(
      const RelayEnvelope(
        type: 'terminal.output',
        sessionId: 'session-1',
        payload: {
          'data': 'hello world',
          'screenData': '[2J[Hhello world',
          'buffer': true,
          'offset': 0,
          'bufferLength': 11,
          'tail': true,
          'outputSeq': 1,
        },
      ),
      activeSessionId: 'session-1',
    );

    final signal = ValueNotifier<int>(0);
    addTearDown(signal.dispose);
    int? reportedCols;
    int? reportedRows;

    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: SizedBox(
            width: 320,
            height: 480,
            child: SelfDrawnTerminalView(
              sessionId: 'session-1',
              controller: controller,
              repaintSignal: signal,
              fontSize: 14,
              onResize: (cols, rows) {
                reportedCols = cols;
                reportedRows = rows;
              },
            ),
          ),
        ),
      ),
    );

    // Drain the post-frame resize + snapshot refresh callbacks.
    await tester.pump();
    await tester.pump();

    expect(find.byType(CustomPaint), findsWidgets);
    expect(reportedCols, isNotNull);
    expect(reportedRows, isNotNull);
    expect(reportedCols!, greaterThan(0));
    expect(reportedRows!, greaterThan(0));

    // A new output signal must re-read the snapshot and repaint without error.
    signal.value = 1;
    await tester.pump();
    expect(tester.takeException(), isNull);
  });

  testWidgets('long-press then drag selects text and reports it', (
    tester,
  ) async {
    final controller = RemoteTerminalOutputController();
    addTearDown(controller.dispose);
    controller.bindSession('session-1', requireBaseline: true);
    controller.accept(
      const RelayEnvelope(
        type: 'terminal.output',
        sessionId: 'session-1',
        payload: {
          'data': 'hello world',
          'buffer': true,
          'offset': 0,
          'bufferLength': 11,
          'tail': true,
          'outputSeq': 1,
        },
      ),
      activeSessionId: 'session-1',
    );

    final signal = ValueNotifier<int>(0);
    addTearDown(signal.dispose);
    String? selected = '';

    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: SizedBox(
            width: 320,
            height: 480,
            child: SelfDrawnTerminalView(
              sessionId: 'session-1',
              controller: controller,
              repaintSignal: signal,
              fontSize: 14,
              onSelectionChanged: (text) => selected = text,
            ),
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump();

    final origin = tester.getTopLeft(find.byType(SelfDrawnTerminalView));
    final gesture = await tester.startGesture(origin + const Offset(6, 8));
    await tester.pump(const Duration(milliseconds: 600)); // long-press fires
    await gesture.moveBy(const Offset(90, 0)); // extend across the first line
    await tester.pump();
    await gesture.up();
    await tester.pump();

    expect(tester.takeException(), isNull);
    expect(selected, isNotNull);
    expect(selected, isNotEmpty);
  });

  testWidgets('renders terminal built-in box drawing cells', (tester) async {
    final controller = RemoteTerminalOutputController();
    addTearDown(controller.dispose);
    controller.bindSession('session-1', requireBaseline: true);
    controller.accept(
      const RelayEnvelope(
        type: 'terminal.output',
        sessionId: 'session-1',
        payload: {
          'data': '┌─┐\n│█│\n└─┘',
          'screenData': '[2J[H┌─┐\r\n│█│\r\n└─┘',
          'buffer': true,
          'offset': 0,
          'bufferLength': 11,
          'tail': true,
          'outputSeq': 1,
        },
      ),
      activeSessionId: 'session-1',
    );

    final signal = ValueNotifier<int>(0);
    addTearDown(signal.dispose);

    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: SizedBox(
            width: 320,
            height: 480,
            child: SelfDrawnTerminalView(
              sessionId: 'session-1',
              controller: controller,
              repaintSignal: signal,
              fontSize: 14,
            ),
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump();

    expect(find.byType(CustomPaint), findsWidgets);
    expect(tester.takeException(), isNull);
  });
}

class _InputHarness {
  final controller = RemoteTerminalOutputController();
  final signal = ValueNotifier<int>(0);
  final text = <String>[];
  final keys = <String>[];
  late TextEditingController field;

  Widget build(String session) => MaterialApp(
    home: SizedBox(
      width: 320,
      height: 480,
      child: SelfDrawnTerminalView(
        sessionId: session,
        controller: controller,
        repaintSignal: signal,
        fontSize: 14,
        onInput: text.add,
        onSendKey: keys.add,
      ),
    ),
  );
}

Future<_InputHarness> _pumpInput(WidgetTester tester) async {
  final harness = _InputHarness();
  addTearDown(harness.controller.dispose);
  addTearDown(harness.signal.dispose);
  await tester.pumpWidget(harness.build('session-1'));
  await tester.pump();
  harness.field = tester
      .widget<EditableText>(find.byType(EditableText))
      .controller;
  return harness;
}
