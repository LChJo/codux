// Sequential (unified) parser for the review diff text — feeds the phone's
// stacked single-column diff view. The pad layout parses the same text into
// side-by-side pairs (`pad_workspace_main_pane.dart`); the phone keeps the
// lines in file order instead.

enum StackedDiffKind { hunk, add, del, context }

class StackedDiffLine {
  const StackedDiffLine(this.kind, this.oldNo, this.newNo, this.text);

  final StackedDiffKind kind;

  /// Line number in the original file (null for additions / hunk banners).
  final int? oldNo;

  /// Line number in the current file (null for deletions / hunk banners).
  final int? newNo;

  final String text;
}

class StackedDiffParse {
  const StackedDiffParse(this.lines, this.truncated);

  final List<StackedDiffLine> lines;

  /// True when parsing stopped early at [maxRows].
  final bool truncated;
}

final RegExp _hunkHeader = RegExp(r'^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@');

/// Parses a unified diff string into stacked lines, in encounter order.
/// Header/meta lines (`diff`, `index`, `---`, `+++`, rename markers, the
/// `\ No newline at end of file` marker) are skipped. Caps output at
/// [maxRows] rows and reports the truncation via [StackedDiffParse.truncated].
StackedDiffParse parseStackedDiff(String diff, {int maxRows = 800}) {
  final lines = <StackedDiffLine>[];
  var truncated = false;
  var oldLine = 0;
  var newLine = 0;
  for (final raw in diff.split('\n')) {
    if (lines.length >= maxRows) {
      truncated = true;
      break;
    }
    // Drop blank lines: a trailing '\n' would otherwise parse as a spurious
    // empty context row, and genuine blank context lines arrive as " ".
    if (raw.isEmpty) continue;
    final hunk = _hunkHeader.firstMatch(raw);
    if (hunk != null) {
      oldLine = int.parse(hunk.group(1)!);
      newLine = int.parse(hunk.group(2)!);
      lines.add(StackedDiffLine(StackedDiffKind.hunk, null, null, raw));
      continue;
    }
    if (raw.startsWith('diff ') ||
        raw.startsWith('index ') ||
        raw.startsWith('--- ') ||
        raw.startsWith('+++ ') ||
        raw.startsWith('new file') ||
        raw.startsWith('deleted file') ||
        raw.startsWith('similarity ') ||
        raw.startsWith('rename ') ||
        raw.startsWith(r'\')) {
      continue;
    }
    if (raw.startsWith('+')) {
      lines.add(
        StackedDiffLine(StackedDiffKind.add, null, newLine, raw.substring(1)),
      );
      newLine++;
    } else if (raw.startsWith('-')) {
      lines.add(
        StackedDiffLine(StackedDiffKind.del, oldLine, null, raw.substring(1)),
      );
      oldLine++;
    } else {
      final text = raw.startsWith(' ') ? raw.substring(1) : raw;
      lines.add(
        StackedDiffLine(StackedDiffKind.context, oldLine, newLine, text),
      );
      oldLine++;
      newLine++;
    }
  }
  return StackedDiffParse(lines, truncated);
}
