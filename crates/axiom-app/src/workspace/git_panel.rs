use super::*;

#[derive(Clone)]
pub(super) struct GitDiffDisplayRow {
    pub(super) hunk_header: bool,
    pub(super) text: String,
    pub(super) kind: GitDiffLineKind,
}

pub(super) const GIT_DIFF_ROW_HEIGHT_PX: f32 = 22.;

pub(super) fn git_diff_display_rows(diff: Option<&GitFileDiff>) -> Vec<GitDiffDisplayRow> {
    let mut rows = Vec::new();
    for hunk in diff.map(|diff| &diff.hunks).into_iter().flatten() {
        rows.push(GitDiffDisplayRow {
            hunk_header: true,
            text: format!(
                "@@ -{},{} +{},{} @@",
                hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
            ),
            kind: GitDiffLineKind::Context,
        });
        rows.extend(hunk.lines.iter().map(|line| {
            let prefix = match line.kind {
                GitDiffLineKind::Context => ' ',
                GitDiffLineKind::Addition => '+',
                GitDiffLineKind::Deletion => '-',
            };
            let old_line = line.old_line.map(|line| line.to_string()).unwrap_or_default();
            let new_line = line.new_line.map(|line| line.to_string()).unwrap_or_default();
            GitDiffDisplayRow {
                hunk_header: false,
                text: format!("{old_line:>4} {new_line:>4} {prefix} {}", line.text),
                kind: line.kind,
            }
        }));
    }
    rows
}

pub(super) fn git_diff_uniform_item_count(diff: Option<&GitFileDiff>) -> usize {
    diff.map(|diff| {
        diff.hunks
            .len()
            .saturating_add(diff.hunks.iter().map(|hunk| hunk.lines.len()).sum())
    })
    .unwrap_or(0)
}

pub(super) fn git_diff_show_no_textual_changes(
    diff: Option<&GitFileDiff>,
    rows: &[GitDiffDisplayRow],
) -> bool {
    diff.is_some() && !diff.is_some_and(|diff| diff.binary) && rows.is_empty()
}

pub(super) fn reset_git_diff_scroll(handle: &UniformListScrollHandle) {
    handle
        .0
        .borrow_mut()
        .base_handle
        .set_offset(Point::default());
    handle.scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
}
