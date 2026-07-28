import 'package:codux_flutter/models/remote_models.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('parses review summary payload', () {
    final summary = RemoteGitReviewSummary.fromResult({
      'mode': 'taskBranch',
      'title': 'Worktree Review',
      'baseBranch': 'main',
      'diffStat': '2 changed files, 12 insertions(+), 4 deletions(-)',
      'isRepository': true,
      'files': [
        {
          'path': 'lib/screens/home/home_page.dart',
          'status': 'modified',
          'additions': 10,
          'deletions': 4,
        },
        {
          'path': 'lib/widgets/phone/phone_review_panel.dart',
          'status': 'added',
          'additions': 2,
          'deletions': 0,
        },
      ],
    });

    expect(summary.mode, 'taskBranch');
    expect(summary.title, 'Worktree Review');
    expect(summary.baseBranch, 'main');
    expect(
      summary.diffStat,
      '2 changed files, 12 insertions(+), 4 deletions(-)',
    );
    expect(summary.isRepository, true);
    expect(summary.error, null);
    expect(summary.files, hasLength(2));
    expect(summary.files[0].path, 'lib/screens/home/home_page.dart');
    expect(summary.files[0].status, 'modified');
    expect(summary.files[0].additions, 10);
    expect(summary.files[0].deletions, 4);
    expect(summary.files[1].status, 'added');
  });

  test('parses non-repository review result inline', () {
    final summary = RemoteGitReviewSummary.fromResult({
      'mode': 'workingTreeAudit',
      'title': 'Uncommitted Audit',
      'diffStat': '',
      'isRepository': false,
      'error': 'could not find repository',
      'files': [],
    });

    expect(summary.isRepository, false);
    expect(summary.error, 'could not find repository');
    expect(summary.files, isEmpty);
    expect(summary.baseBranch, null);
  });

  test('tolerates missing fields', () {
    final summary = RemoteGitReviewSummary.fromResult({});

    expect(summary.mode, '');
    expect(summary.title, '');
    expect(summary.diffStat, '');
    expect(summary.files, isEmpty);
    expect(summary.isRepository, false);
    expect(summary.error, null);
  });

  test('review_diff result reuses RemoteGitDiff', () {
    final diff = RemoteGitDiff.fromResult({
      'diff': '@@ -1 +1 @@\n-old\n+new\n',
    });

    expect(diff.diff, contains('+new'));
    expect(diff.path, '');
    expect(diff.isRepository, false);
    expect(diff.error, null);
  });
}
