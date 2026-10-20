import 'package:codux_flutter/services/terminal_editing_controller.dart';
import 'package:codux_protocol_ffi/codux_protocol_ffi.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('bulk insert followed by submit stays behind candidate commit', () {
    final sent = <String>[];
    final editor = _editor(sent);
    _edit(editor, 'ni', 2, composing: const TextRange(start: 1, end: 3));
    editor.insert('resume command');
    editor.sendKey(const TerminalKeyIntent('enter'));
    expect(sent, isEmpty);
    _edit(editor, '你', 1);
    expect(sent, ['你', 'resume command', '\r']);
  });

  test(
    'cancelling composition that replaced the anchor never deletes remote text',
    () {
      final sent = <String>[];
      final editor = _editor(sent);
      editor.field.value = const TextEditingValue(
        text: 'ni',
        selection: TextSelection.collapsed(offset: 2),
        composing: TextRange(start: 0, end: 2),
      );
      editor.sendKey(const TerminalKeyIntent('enter'));
      editor.field.value = const TextEditingValue(
        selection: TextSelection.collapsed(offset: 0),
      );
      expect(sent, ['\r']);
    },
  );

  test('toolbar exits paired quotes without replaying their suffix', () {
    final sent = <String>[];
    final editor = _editor(sent);
    _edit(editor, '""', 1);
    editor.sendKey(const TerminalKeyIntent('right'));
    _edit(editor, 'outside', 7);
    expect(sent, ['""', '\u001b[D', '\u001b[C', 'outside']);
  });

  test('candidate commit precedes queued navigation and Enter', () {
    final sent = <String>[];
    final editor = _editor(sent);
    _edit(editor, 'ni', 2, composing: const TextRange(start: 1, end: 3));
    editor.sendKey(const TerminalKeyIntent('right'));
    editor.sendKey(const TerminalKeyIntent('enter'));
    expect(sent, isEmpty);
    _edit(editor, '你', 1);
    expect(sent, ['你', '\u001b[C', '\r']);
    expect(editor.deferredCount, 0);
  });

  test('candidate cancellation runs queued key without committing pinyin', () {
    final sent = <String>[];
    final editor = _editor(sent);
    _edit(editor, 'ni', 2, composing: const TextRange(start: 1, end: 3));
    editor.sendKey(const TerminalKeyIntent('enter'));
    _edit(editor, '', 0);
    expect(sent, ['\r']);
  });

  test('rebinding cancels candidates and deferred input', () {
    final sent = <String>[];
    final editor = _editor(sent);
    editor.bind('old');
    _edit(editor, 'ni', 2, composing: const TextRange(start: 1, end: 3));
    editor.sendKey(const TerminalKeyIntent('enter'));
    editor.insert('old paste');
    editor.bind('new');
    _edit(editor, 'new', 3);
    expect(sent, ['new']);
    expect(editor.deferredCount, 0);
  });

  test('editing before anchor never forwards the internal sentinel', () {
    final sent = <String>[];
    final editor = _editor(sent);
    editor.field.value = const TextEditingValue(
      text: 'x\u200b',
      selection: TextSelection.collapsed(offset: 1),
    );
    expect(sent, ['x']);
  });

  test('IME replacing the anchor does not delete remote content', () {
    final sent = <String>[];
    final editor = _editor(sent);
    editor.field.value = const TextEditingValue(
      text: 'x',
      selection: TextSelection.collapsed(offset: 1),
    );
    expect(sent, ['x']);
    editor.field.value = const TextEditingValue(
      text: 'xy',
      selection: TextSelection.collapsed(offset: 2),
    );
    expect(sent, ['x', 'y']);
  });

  test('backspace at empty anchor reaches remote content once', () {
    final sent = <String>[];
    final editor = _editor(sent);
    editor.field.value = const TextEditingValue(
      selection: TextSelection.collapsed(offset: 0),
    );
    expect(sent, ['\u007f']);
    expect(editor.field.text, TerminalEditingController.sentinel);
  });

  test('ZWJ emoji and combining marks delete as single graphemes', () {
    final sent = <String>[];
    final editor = _editor(sent);
    const text = 'a👨‍👩‍👧‍👦e\u0301';
    _edit(editor, text, text.length);
    sent.clear();
    const remaining = 'a👨‍👩‍👧‍👦';
    _edit(editor, remaining, remaining.length);
    _edit(editor, 'a', 1);
    expect(sent, ['\u007f', '\u007f']);
  });

  test('selected middle replacement preserves following text', () {
    final sent = <String>[];
    final editor = _editor(sent);
    _edit(editor, 'abcd', 4);
    editor.field.selection = const TextSelection(
      baseOffset: 2,
      extentOffset: 4,
    );
    sent.clear();
    _edit(editor, 'aXd', 2);
    expect(sent, ['\u007f', '\u007f', 'X']);
  });

  test('synchronous rebind stops remaining old-session edits', () {
    final sent = <String>[];
    late TerminalEditingController editor;
    editor = TerminalEditingController(
      onText: (text) {
        sent.add(text);
        if (text == 'a') editor.bind('new');
      },
      onKeyBytes: sent.add,
    );
    addTearDown(editor.dispose);
    editor.bind('old');
    _edit(editor, 'a\nb', 3);
    expect(sent, ['a']);
    _edit(editor, 'x', 1);
    expect(sent, ['a', 'x']);
  });

  test('keys use current application cursor and kitty modes', () {
    final sent = <String>[];
    var mode = const TerminalScreenInputMode();
    final editor = _editor(sent, mode: () => mode);
    editor.sendKey(const TerminalKeyIntent('up'));
    mode = const TerminalScreenInputMode(
      applicationCursor: true,
      kittyFlags: 1,
    );
    editor.sendKey(const TerminalKeyIntent('up'));
    editor.sendKey(const TerminalKeyIntent('enter', shift: true));
    editor.sendKey(const TerminalKeyIntent.interrupt());
    expect(sent, ['\u001b[A', '\u001bOA', '\u001b[13;2u', '\u0003']);
  });

  test('bulk insertion respects active bracketed paste mode', () {
    final sent = <String>[];
    var mode = const TerminalScreenInputMode();
    final editor = _editor(sent, mode: () => mode);
    editor.insert('a\nb');
    mode = const TerminalScreenInputMode(bracketedPaste: true);
    editor.insert('a\nb');
    expect(sent, ['a\nb', '\u001b[200~a\nb\u001b[201~']);
  });

  test('disabled input cannot reach a terminal or retain deferred actions', () {
    final sent = <String>[];
    final editor = _editor(sent);
    editor.bind('offline', enabled: false);
    _edit(editor, 'stale', 5);
    editor.sendKey(const TerminalKeyIntent('enter'));
    editor.insert('stale paste');
    expect(sent, isEmpty);
    expect(editor.field.text, TerminalEditingController.sentinel);
  });
}

TerminalEditingController _editor(
  List<String> sent, {
  TerminalScreenInputMode Function()? mode,
}) {
  final editor = TerminalEditingController(
    onText: sent.add,
    onKeyBytes: sent.add,
    inputMode: mode,
  );
  addTearDown(editor.dispose);
  return editor;
}

void _edit(
  TerminalEditingController editor,
  String text,
  int caret, {
  TextRange composing = TextRange.empty,
}) {
  editor.field.value = TextEditingValue(
    text: '${TerminalEditingController.sentinel}$text',
    selection: TextSelection.collapsed(offset: caret + 1),
    composing: composing,
  );
}
