import 'package:codux_flutter/i18n.dart';
import 'package:codux_flutter/services/remote_terminal_output_controller.dart';
import 'package:codux_flutter/services/terminal_repaint_signal.dart';
import 'package:codux_flutter/theme/app_theme.dart';
import 'package:codux_flutter/models/workspace_mode.dart';
import 'package:codux_flutter/widgets/components/remote_terminal_pane.dart';
import 'package:codux_flutter/widgets/components/self_drawn_terminal_view.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('toolbar exits IME paired quotes and accepts outside text', (
    tester,
  ) async {
    final sent = <String>[];
    await tester.pumpWidget(
      MaterialApp(
        theme: buildAppTheme(),
        home: AppPreferences(
          accent: AccentChoices.cyan,
          locale: LocaleChoices.english,
          themeMode: ThemeMode.dark,
          child: SizedBox(
            width: 360,
            height: 720,
            child: _pane(onSendKey: sent.add, onInput: sent.add),
          ),
        ),
      ),
    );
    await tester.pump();
    final field = tester
        .widget<EditableText>(find.byType(EditableText))
        .controller;
    field.value = const TextEditingValue(
      text: '\u200b""',
      selection: TextSelection.collapsed(offset: 2),
    );
    await tester.tap(find.byIcon(Icons.keyboard_arrow_right_rounded));
    field.value = const TextEditingValue(
      text: '\u200bx',
      selection: TextSelection.collapsed(offset: 2),
    );
    expect(sent, ['""', '\u001b[D', '\u001b[C', 'x']);
  });

  testWidgets('toolbar Enter waits for candidate commit', (tester) async {
    final sent = <String>[];
    await tester.pumpWidget(
      MaterialApp(
        theme: buildAppTheme(),
        home: AppPreferences(
          accent: AccentChoices.cyan,
          locale: LocaleChoices.english,
          themeMode: ThemeMode.dark,
          child: SizedBox(
            width: 360,
            height: 720,
            child: _pane(onSendKey: sent.add, onInput: sent.add),
          ),
        ),
      ),
    );
    await tester.pump();
    final field = tester
        .widget<EditableText>(find.byType(EditableText))
        .controller;
    field.value = const TextEditingValue(
      text: '\u200bni',
      selection: TextSelection.collapsed(offset: 3),
      composing: TextRange(start: 1, end: 3),
    );
    await tester.tap(find.byIcon(Icons.keyboard_return_rounded));
    expect(sent, isEmpty);
    field.value = const TextEditingValue(
      text: '\u200b你',
      selection: TextSelection.collapsed(offset: 2),
    );
    expect(sent, ['你', '\r']);
  });

  testWidgets('terminal content starts at top of terminal body', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: buildAppTheme(),
        home: AppPreferences(
          accent: AccentChoices.cyan,
          locale: LocaleChoices.english,
          themeMode: ThemeMode.dark,
          child: SizedBox(width: 360, height: 720, child: _pane()),
        ),
      ),
    );
    await tester.pump();

    final paneTop = tester.getTopLeft(find.byType(RemoteTerminalPane)).dy;
    final terminalTop = tester
        .getTopLeft(find.byKey(const ValueKey('remote-terminal-body')))
        .dy;

    expect(terminalTop, paneTop);
  });

  testWidgets('ctrl c toolbar sends etx directly', (tester) async {
    final sent = <String>[];
    await tester.pumpWidget(
      MaterialApp(
        theme: buildAppTheme(),
        home: AppPreferences(
          accent: AccentChoices.cyan,
          locale: LocaleChoices.english,
          themeMode: ThemeMode.dark,
          child: SizedBox(
            width: 360,
            height: 720,
            child: _pane(onSendKey: sent.add),
          ),
        ),
      ),
    );
    await tester.pump();

    await tester.tap(find.text('^C'));
    await tester.pump();

    expect(sent, ['\u0003']);
  });

  test('keyboard lift follows cursor visibility', () {
    expect(
      terminalLiftForKeyboardForTest(
        terminalHeight: 600,
        keyboardLift: 260,
        cursorMetrics: const TerminalCursorMetrics(
          row: 4,
          col: 0,
          lineHeight: 20,
        ),
      ),
      0,
    );
    expect(
      terminalLiftForKeyboardForTest(
        terminalHeight: 600,
        keyboardLift: 260,
        cursorMetrics: const TerminalCursorMetrics(
          row: 20,
          col: 0,
          lineHeight: 20,
        ),
      ),
      80,
    );
  });
}

RemoteTerminalPane _pane({
  ValueChanged<String>? onSendKey,
  ValueChanged<String>? onInput,
}) {
  return RemoteTerminalPane(
    connected: true,
    showTerminal: true,
    hasDevice: true,
    status: '',
    workspaceMode: WorkspaceMode.terminal,
    projectListLoaded: true,
    projectCount: 1,
    terminalUploadLoading: false,
    terminalUploadStatus: '',
    terminalBufferLoading: false,
    sessionId: 'session-1',
    pendingBufferSessionId: null,
    connectionStatusText: 'connecting',
    terminalHistoryLoadingText: 'loading',
    keyboardVisible: false,
    keyboardRequested: false,
    keyboardRequestSerial: 0,
    repaintSignal: TerminalRepaintSignal(),
    outputController: RemoteTerminalOutputController(),
    terminalFontSize: 16,
    onConnect: () {},
    onInput: onInput ?? (_) {},
    onResize: (_, _) {},
    onSelectionChanged: (_) {},
    onSendKey: onSendKey ?? (_) {},
    onToggleKeyboard: () {},
    onRequestKeyboard: () {},
    onPaste: () {},
    onCopy: () {},
    onUpload: () {},
    onVoiceInput: () {},
    handedAway: false,
    onTakeOver: () {},
  );
}
