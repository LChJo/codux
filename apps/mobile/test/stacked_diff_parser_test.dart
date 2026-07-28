import 'package:codux_flutter/widgets/phone/stacked_diff_lines.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('parses hunks sequentially with line numbers', () {
    const diff = '''
diff --git a/lib/a.dart b/lib/a.dart
index 1234567..abcdefg 100644
--- a/lib/a.dart
+++ b/lib/a.dart
@@ -10,4 +10,4 @@ void main() {
 keep
-old
+new
 tail''';

    final parse = parseStackedDiff(diff);

    expect(parse.truncated, false);
    expect(parse.lines, hasLength(5));
    expect(parse.lines[0].kind, StackedDiffKind.hunk);
    expect(parse.lines[1].kind, StackedDiffKind.context);
    expect(parse.lines[1].oldNo, 10);
    expect(parse.lines[1].newNo, 10);
    expect(parse.lines[1].text, 'keep');
    expect(parse.lines[2].kind, StackedDiffKind.del);
    expect(parse.lines[2].oldNo, 11);
    expect(parse.lines[2].newNo, null);
    expect(parse.lines[2].text, 'old');
    expect(parse.lines[3].kind, StackedDiffKind.add);
    expect(parse.lines[3].oldNo, null);
    expect(parse.lines[3].newNo, 11);
    expect(parse.lines[3].text, 'new');
    expect(parse.lines[4].kind, StackedDiffKind.context);
    expect(parse.lines[4].oldNo, 12);
    expect(parse.lines[4].newNo, 12);
    expect(parse.lines[4].text, 'tail');
  });

  test('keeps deletions before additions in file order', () {
    const diff = '''
@@ -1,3 +1,3 @@
 a
-b
-c
+B
+C
 d''';

    final kinds = parseStackedDiff(diff).lines.map((l) => l.kind).toList();
    expect(kinds, [
      StackedDiffKind.hunk,
      StackedDiffKind.context,
      StackedDiffKind.del,
      StackedDiffKind.del,
      StackedDiffKind.add,
      StackedDiffKind.add,
      StackedDiffKind.context,
    ]);
  });

  test('skips header and no-newline markers', () {
    const diff = '''
diff --git a/x b/x
new file mode 100644
index 0000000..1111111
--- /dev/null
+++ b/x
@@ -0,0 +1 @@
+line
\\ No newline at end of file''';

    final parse = parseStackedDiff(diff);
    expect(parse.lines, hasLength(2));
    expect(parse.lines[0].kind, StackedDiffKind.hunk);
    expect(parse.lines[1].kind, StackedDiffKind.add);
    expect(parse.lines[1].text, 'line');
  });

  test('flags truncation at the row cap', () {
    final buffer = StringBuffer('@@ -1,100 +1,100 @@\n');
    for (var i = 0; i < 100; i++) {
      buffer.writeln(' context $i');
    }
    final parse = parseStackedDiff(buffer.toString(), maxRows: 10);
    expect(parse.truncated, true);
    expect(parse.lines, hasLength(10));
  });

  test('empty diff yields no lines', () {
    final parse = parseStackedDiff('');
    expect(parse.lines, isEmpty);
    expect(parse.truncated, false);
  });
}
