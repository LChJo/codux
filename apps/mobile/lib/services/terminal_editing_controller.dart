import 'dart:collection';

import 'package:codux_protocol_ffi/codux_protocol_ffi.dart';
import 'package:flutter/widgets.dart';

/// A key intent is encoded at dispatch time, against the current terminal mode.
class TerminalKeyIntent {
  const TerminalKeyIntent(
    this.key, {
    this.keyChar = '',
    this.shift = false,
    this.alt = false,
    this.control = false,
    this.platform = false,
    this.interrupt = false,
  });

  const TerminalKeyIntent.interrupt()
    : this('c', control: true, interrupt: true);

  final String key;
  final String keyChar;
  final bool shift;
  final bool alt;
  final bool control;
  final bool platform;
  final bool interrupt;

  String encode(TerminalScreenInputMode mode) {
    if (interrupt) return '\u0003';
    if (keyChar.isNotEmpty && !shift && !alt && !control && !platform) {
      return terminalTextInput(keyChar);
    }
    return terminalKeyInput(
      key: key,
      keyChar: keyChar,
      shift: shift,
      alt: alt,
      control: control,
      platform: platform,
      applicationCursor: mode.applicationCursor,
      kittyFlags: mode.kittyFlags,
    );
  }
}

/// The hidden field is a local IME edit segment, never a mirror of the PTY line.
/// External navigation/submission ends the segment so later IME edits cannot
/// replay a suffix at a remote cursor that has moved independently.
class TerminalEditingController extends ChangeNotifier {
  TerminalEditingController({
    required this.onText,
    required this.onKeyBytes,
    this.onInsertBytes,
    TerminalScreenInputMode Function()? inputMode,
  }) : _inputMode = inputMode ?? (() => const TerminalScreenInputMode()) {
    field.addListener(_reconcile);
  }

  static const sentinel = '\u200b';
  final ValueChanged<String> onText;
  final ValueChanged<String> onKeyBytes;
  final ValueChanged<String>? onInsertBytes;
  final TerminalScreenInputMode Function() _inputMode;
  final TextEditingController field = TextEditingController(text: sentinel)
    ..selection = const TextSelection.collapsed(offset: 1);
  final Queue<VoidCallback> _deferred = Queue();
  TextEditingValue _committed = const TextEditingValue(
    selection: TextSelection.collapsed(offset: 0),
  );
  Object? _binding;
  bool _enabled = true;
  bool _resetting = false;
  bool _disposed = false;
  bool _wasComposing = false;
  int _connectionEpoch = 0;

  int get connectionEpoch => _connectionEpoch;
  bool get isComposing =>
      field.value.composing.isValid && !field.value.composing.isCollapsed;
  int get deferredCount => _deferred.length;

  void bind(Object? binding, {bool enabled = true}) {
    if (_binding == binding && _enabled == enabled) return;
    _binding = binding;
    _enabled = enabled;
    reset();
  }

  /// Invalidates the platform text client as well as all pending local edits.
  void reset() {
    if (_disposed) return;
    _deferred.clear();
    _resetField();
    _connectionEpoch++;
    notifyListeners();
  }

  void sendKey(TerminalKeyIntent intent) => _external(() {
    final bytes = intent.encode(_inputMode());
    if (bytes.isNotEmpty) onKeyBytes(bytes);
  });

  void insert(String text) {
    if (text.isEmpty) return;
    _external(() {
      final bytes = terminalInsertInput(
        text,
        bracketedPaste: _inputMode().bracketedPaste,
      );
      if (bytes.isNotEmpty) (onInsertBytes ?? onKeyBytes)(bytes);
    });
  }

  void _external(VoidCallback action) {
    if (_disposed || !_enabled) return;
    if (isComposing) {
      // Bound an accidental held toolbar key while a candidate is open.
      if (_deferred.length < 128) _deferred.add(action);
      return;
    }
    _resetField();
    action();
  }

  void _resetField() {
    _resetting = true;
    _wasComposing = false;
    field.value = const TextEditingValue(
      text: sentinel,
      selection: TextSelection.collapsed(offset: 1),
    );
    _committed = const TextEditingValue(
      selection: TextSelection.collapsed(offset: 0),
    );
    _resetting = false;
  }

  TextEditingValue _withoutAnchor(TextEditingValue value) {
    // An IME can move before the anchor and insert there. Remove that anchor
    // wherever it ended up, retaining any subsequent user zero-width text.
    final anchor = value.text.indexOf(sentinel);
    final text = anchor < 0
        ? value.text
        : value.text.replaceRange(anchor, anchor + 1, '');
    int offset(int raw) => !value.selection.isValid
        ? text.length
        : (raw - (anchor >= 0 && raw > anchor ? 1 : 0)).clamp(0, text.length);
    return TextEditingValue(
      text: text,
      selection: TextSelection(
        baseOffset: offset(value.selection.baseOffset),
        extentOffset: offset(value.selection.extentOffset),
      ),
    );
  }

  int _graphemeOffset(String text, int offset) {
    var units = 0;
    var count = 0;
    for (final char in text.characters) {
      if (units + char.length > offset) break;
      units += char.length;
      count++;
    }
    return count;
  }

  bool _canEmit(int epoch) =>
      !_disposed && _enabled && epoch == _connectionEpoch;

  void _move(int delta, int epoch) {
    for (var i = 0; i < delta.abs(); i++) {
      _key(delta < 0 ? 'left' : 'right', epoch);
    }
  }

  void _key(String key, int epoch) {
    if (!_canEmit(epoch)) return;
    final bytes = TerminalKeyIntent(key).encode(_inputMode());
    if (bytes.isNotEmpty) onKeyBytes(bytes);
  }

  void _text(String text, int epoch) {
    final parts = text
        .replaceAll('\r\n', '\n')
        .replaceAll('\r', '\n')
        .split('\n');
    for (var i = 0; i < parts.length; i++) {
      if (!_canEmit(epoch)) return;
      if (parts[i].isNotEmpty) onText(parts[i]);
      if (i + 1 < parts.length) _key('enter', epoch);
    }
  }

  void _reconcile() {
    if (_disposed || _resetting) return;
    if (!_enabled) {
      _resetField();
      return;
    }
    if (isComposing) {
      _wasComposing = true;
      return;
    }
    final endedComposition = _wasComposing;
    _wasComposing = false;
    final epoch = _connectionEpoch;
    final raw = field.value;
    final next = _withoutAnchor(raw);
    final old = _committed;
    final before = old.text.characters.toList();
    final after = next.text.characters.toList();
    var prefix = 0;
    while (prefix < before.length &&
        prefix < after.length &&
        before[prefix] == after[prefix]) {
      prefix++;
    }
    var suffix = 0;
    while (suffix < before.length - prefix &&
        suffix < after.length - prefix &&
        before[before.length - 1 - suffix] ==
            after[after.length - 1 - suffix]) {
      suffix++;
    }
    final oldCaret = _graphemeOffset(old.text, old.selection.extentOffset);
    final newCaret = _graphemeOffset(next.text, next.selection.extentOffset);
    if (old.text == next.text) {
      // Removing the empty anchor represents a backspace into remote content.
      if (old.text.isEmpty && raw.text.isEmpty && !endedComposition) {
        _key('backspace', epoch);
      } else {
        _move(newCaret - oldCaret, epoch);
      }
    } else {
      final removedEnd = before.length - suffix;
      _move(removedEnd - oldCaret, epoch);
      for (var i = prefix; i < removedEnd; i++) {
        _key('backspace', epoch);
      }
      _text(after.sublist(prefix, after.length - suffix).join(), epoch);
      _move(newCaret - (after.length - suffix), epoch);
    }
    if (!_canEmit(epoch)) return;
    _committed = next;
    // Newline submission starts a new remote edit context; don't retain a local
    // multi-line buffer that an IME could subsequently try to rewrite.
    if (next.text.isEmpty ||
        next.text.contains('\n') ||
        next.text.contains('\r')) {
      _resetField();
    }
    while (_deferred.isNotEmpty &&
        !_disposed &&
        _enabled &&
        epoch == _connectionEpoch) {
      final action = _deferred.removeFirst();
      _resetField();
      action();
    }
  }

  @override
  void dispose() {
    _disposed = true;
    _deferred.clear();
    field.removeListener(_reconcile);
    field.dispose();
    super.dispose();
  }
}
