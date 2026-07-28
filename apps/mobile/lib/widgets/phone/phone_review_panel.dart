import 'package:flutter/material.dart';

import '../../i18n.dart';
import '../../models/remote_models.dart';
import '../../theme/app_theme.dart';
import 'stacked_diff_lines.dart';

/// Phone-layout review screen: a flat list of changed files (driven by the
/// `git.read {op: "review"}` summary) that drills into a unified stacked diff
/// per file (driven by `review_diff`). Master-detail within the panel — the
/// phone layout has no router.
class PhoneReviewPanel extends StatelessWidget {
  const PhoneReviewPanel({
    super.key,
    required this.review,
    required this.gitDiff,
    required this.diffError,
    required this.selectedPath,
    required this.onSelectFile,
    required this.onClearSelection,
    required this.onRefreshReview,
  });

  final RemoteGitReviewSummary? review;
  final RemoteGitDiff? gitDiff;
  final String? diffError;
  final String? selectedPath;
  final ValueChanged<String> onSelectFile;
  final VoidCallback onClearSelection;
  final VoidCallback onRefreshReview;

  @override
  Widget build(BuildContext context) {
    final path = selectedPath;
    if (path != null) {
      return _PhoneDiffPane(
        path: path,
        gitDiff: gitDiff,
        diffError: diffError,
        onBack: onClearSelection,
      );
    }
    return _PhoneReviewList(
      review: review,
      onSelectFile: onSelectFile,
      onRefreshReview: onRefreshReview,
    );
  }
}

/// Status word -> (letter badge, color); mirrors the desktop's
/// `git_review_status_badge`.
(String, Color) _reviewStatusBadge(String status, Color accent) =>
    switch (status) {
      'added' => ('A', AppColors.success),
      'deleted' => ('D', AppColors.danger),
      'renamed' => ('R', AppColors.warning),
      'copied' => ('C', AppColors.warning),
      'typeChanged' => ('T', AppColors.warning),
      _ => ('M', accent),
    };

class _PhoneReviewList extends StatelessWidget {
  const _PhoneReviewList({
    required this.review,
    required this.onSelectFile,
    required this.onRefreshReview,
  });

  final RemoteGitReviewSummary? review;
  final ValueChanged<String> onSelectFile;
  final VoidCallback onRefreshReview;

  @override
  Widget build(BuildContext context) {
    final prefs = AppPreferences.of(context);
    final accent = Theme.of(context).colorScheme.secondary;
    return ColoredBox(
      color: AppColors.bgBase,
      child: Column(
        children: [
          _listHeader(prefs, accent),
          Expanded(child: _body(prefs, accent)),
        ],
      ),
    );
  }

  Widget _listHeader(AppPreferences prefs, Color accent) {
    final diffStat = review?.diffStat.trim() ?? '';
    return Container(
      height: 44,
      padding: const EdgeInsets.only(left: AppSpacing.m, right: AppSpacing.xs),
      decoration: BoxDecoration(
        color: AppColors.bgSurface,
        border: Border(bottom: BorderSide(color: AppColors.border)),
      ),
      child: Row(
        children: [
          Expanded(
            child: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  prefs.t('workspace.review'),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    color: AppColors.textPrimary,
                    fontSize: 13,
                    fontWeight: FontWeight.w700,
                  ),
                ),
                if (diffStat.isNotEmpty)
                  Text(
                    diffStat,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(color: AppColors.textMuted, fontSize: 11),
                  ),
              ],
            ),
          ),
          IconButton(
            onPressed: onRefreshReview,
            icon: const Icon(Icons.refresh_rounded, size: 18),
            color: AppColors.textMuted,
          ),
        ],
      ),
    );
  }

  Widget _body(AppPreferences prefs, Color accent) {
    final review = this.review;
    if (review == null) {
      return Center(
        child: CircularProgressIndicator(color: accent, strokeWidth: 2),
      );
    }
    final error = review.error?.trim() ?? '';
    if (!review.isRepository || error.isNotEmpty) {
      return _centerMessage(
        error.isNotEmpty ? error : prefs.t('review.notRepo'),
      );
    }
    if (review.files.isEmpty) {
      return _centerMessage(prefs.t('review.noChanges'));
    }
    return RefreshIndicator(
      onRefresh: () async => onRefreshReview(),
      color: accent,
      backgroundColor: AppColors.bgSurface,
      child: ListView.separated(
        physics: const AlwaysScrollableScrollPhysics(
          parent: BouncingScrollPhysics(),
        ),
        padding: const EdgeInsets.symmetric(vertical: AppSpacing.s),
        itemCount: review.files.length,
        separatorBuilder: (_, _) => const SizedBox(height: 2),
        itemBuilder: (context, index) => _fileRow(review.files[index], accent),
      ),
    );
  }

  Widget _centerMessage(String text) => Center(
    child: Padding(
      padding: const EdgeInsets.all(AppSpacing.l),
      child: Text(
        text,
        textAlign: TextAlign.center,
        style: TextStyle(color: AppColors.textSubtle, fontSize: 13),
      ),
    ),
  );

  Widget _fileRow(RemoteGitReviewFile file, Color accent) {
    final (letter, badgeColor) = _reviewStatusBadge(file.status, accent);
    final lastSlash = file.path.lastIndexOf('/');
    final name = lastSlash >= 0
        ? file.path.substring(lastSlash + 1)
        : file.path;
    final dir = lastSlash > 0 ? file.path.substring(0, lastSlash) : '';
    return InkWell(
      onTap: () => onSelectFile(file.path),
      child: Padding(
        padding: const EdgeInsets.symmetric(
          horizontal: AppSpacing.m,
          vertical: AppSpacing.s,
        ),
        child: Row(
          children: [
            Container(
              width: 22,
              height: 22,
              alignment: Alignment.center,
              decoration: BoxDecoration(
                color: badgeColor.withValues(alpha: 0.14),
                borderRadius: BorderRadius.circular(6),
              ),
              child: Text(
                letter,
                style: TextStyle(
                  color: badgeColor,
                  fontSize: 12,
                  fontWeight: FontWeight.w700,
                ),
              ),
            ),
            const SizedBox(width: AppSpacing.m),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      color: AppColors.textPrimary,
                      fontSize: 14,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  if (dir.isNotEmpty)
                    Text(
                      dir,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        color: AppColors.textMuted,
                        fontSize: 12,
                      ),
                    ),
                ],
              ),
            ),
            const SizedBox(width: AppSpacing.s),
            Text(
              '+${file.additions}',
              style: const TextStyle(
                color: AppColors.success,
                fontSize: 12,
                fontWeight: FontWeight.w700,
              ),
            ),
            const SizedBox(width: AppSpacing.xs),
            Text(
              '-${file.deletions}',
              style: const TextStyle(
                color: AppColors.danger,
                fontSize: 12,
                fontWeight: FontWeight.w700,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _PhoneDiffPane extends StatelessWidget {
  const _PhoneDiffPane({
    required this.path,
    required this.gitDiff,
    required this.diffError,
    required this.onBack,
  });

  final String path;
  final RemoteGitDiff? gitDiff;
  final String? diffError;
  final VoidCallback onBack;

  @override
  Widget build(BuildContext context) {
    final prefs = AppPreferences.of(context);
    final lastSlash = path.lastIndexOf('/');
    final name = lastSlash >= 0 ? path.substring(lastSlash + 1) : path;
    final dir = lastSlash > 0 ? path.substring(0, lastSlash) : '';
    return ColoredBox(
      color: AppColors.bgBase,
      child: Column(
        children: [
          Container(
            height: 44,
            padding: const EdgeInsets.only(right: AppSpacing.m),
            decoration: BoxDecoration(
              color: AppColors.bgSurface,
              border: Border(bottom: BorderSide(color: AppColors.border)),
            ),
            child: Row(
              children: [
                IconButton(
                  onPressed: onBack,
                  icon: const Icon(Icons.arrow_back_ios_new, size: 16),
                  color: AppColors.textPrimary,
                ),
                Expanded(
                  child: Column(
                    mainAxisAlignment: MainAxisAlignment.center,
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        name,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          color: AppColors.textPrimary,
                          fontSize: 13,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                      if (dir.isNotEmpty)
                        Text(
                          dir,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            color: AppColors.textMuted,
                            fontSize: 11,
                          ),
                        ),
                    ],
                  ),
                ),
              ],
            ),
          ),
          Expanded(child: _body(prefs)),
        ],
      ),
    );
  }

  Widget _body(AppPreferences prefs) {
    // Order matters: a failed git.read answers with REMOTE_ERROR (surfaced in
    // diffError), never with an empty git.read — check it before the spinner.
    final error = diffError?.trim() ?? '';
    if (error.isNotEmpty) {
      return Center(
        child: Padding(
          padding: const EdgeInsets.all(AppSpacing.l),
          child: Text(
            error,
            textAlign: TextAlign.center,
            style: const TextStyle(color: AppColors.danger, fontSize: 13),
          ),
        ),
      );
    }
    final gitDiff = this.gitDiff;
    if (gitDiff == null) {
      return Center(
        child: CircularProgressIndicator(
          color: AppColors.accent,
          strokeWidth: 2,
        ),
      );
    }
    if (gitDiff.diff.trim().isEmpty) {
      return Center(
        child: Text(
          prefs.t('review.noChanges'),
          style: TextStyle(color: AppColors.textSubtle, fontSize: 13),
        ),
      );
    }
    return _StackedDiffBody(diff: gitDiff.diff);
  }
}

class _StackedDiffBody extends StatelessWidget {
  const _StackedDiffBody({required this.diff});

  final String diff;

  @override
  Widget build(BuildContext context) {
    final prefs = AppPreferences.of(context);
    final accent = Theme.of(context).colorScheme.secondary;
    final parse = parseStackedDiff(diff);
    return LayoutBuilder(
      builder: (context, constraints) => SingleChildScrollView(
        physics: const BouncingScrollPhysics(),
        child: SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          physics: const BouncingScrollPhysics(),
          child: ConstrainedBox(
            constraints: BoxConstraints(minWidth: constraints.maxWidth),
            child: IntrinsicWidth(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (final line in parse.lines)
                    _DiffRow(line: line, accent: accent),
                  if (parse.truncated)
                    Padding(
                      padding: const EdgeInsets.all(AppSpacing.m),
                      child: Text(
                        prefs.t('review.truncated'),
                        textAlign: TextAlign.center,
                        style: TextStyle(
                          color: AppColors.textMuted,
                          fontSize: 12,
                        ),
                      ),
                    ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _DiffRow extends StatelessWidget {
  const _DiffRow({required this.line, required this.accent});

  final StackedDiffLine line;
  final Color accent;

  static const String _fontFamily = 'MapleMonoNFCN';

  @override
  Widget build(BuildContext context) {
    final (background, foreground) = switch (line.kind) {
      StackedDiffKind.add => (
        AppColors.success.withValues(alpha: 0.12),
        AppColors.success,
      ),
      StackedDiffKind.del => (
        AppColors.danger.withValues(alpha: 0.12),
        AppColors.danger,
      ),
      StackedDiffKind.hunk => (accent.withValues(alpha: 0.10), accent),
      StackedDiffKind.context => (Colors.transparent, AppColors.textSecondary),
    };
    return Container(
      height: 19,
      color: background,
      padding: const EdgeInsets.only(right: AppSpacing.s),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          _gutter(line.oldNo),
          _gutter(line.newNo),
          const SizedBox(width: AppSpacing.s),
          Text(
            line.text.isEmpty ? ' ' : line.text,
            maxLines: 1,
            softWrap: false,
            style: TextStyle(
              color: foreground,
              fontSize: 12,
              height: 1.4,
              fontFamily: _fontFamily,
            ),
          ),
        ],
      ),
    );
  }

  Widget _gutter(int? number) => SizedBox(
    width: 30,
    child: Text(
      number?.toString() ?? '',
      textAlign: TextAlign.right,
      style: TextStyle(
        color: AppColors.textSubtle,
        fontSize: 11,
        fontFamily: _fontFamily,
      ),
    ),
  );
}
