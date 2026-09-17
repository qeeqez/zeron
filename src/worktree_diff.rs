//! The Changes panel's diff base for worktree chats: `resolve_diff_base`
//! picks the commit the file list diffs against (default: the merge-base of
//! the worktree's HEAD and the project checkout's HEAD — the commit the
//! worktree was cut from), and `worktree_changes` collects the worktree's
//! full delta against it through a scratch index, so committed, staged,
//! unstaged and untracked work all show. The real index is never touched —
//! same trick as `worktree_merge::worktree_patch`.

use std::path::Path;

/// The resolved diff base for a worktree chat's Changes panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedBase {
    /// The merge-base commit the file list diffs against.
    pub commit: String,
    /// The ref the base was resolved from — the picker's trigger label.
    pub label: String,
    /// The picked ref no longer resolves (deleted branch/tag, or no common
    /// ancestor) — the panel fell back to the default and says so.
    pub stale: bool,
}

/// Resolve the commit `wt` diffs against. A picked `base_ref` wins when it
/// shares history with the worktree's HEAD; a stale or unrelated pick falls
/// back to the default (merge-base of the worktree's HEAD and the project
/// checkout's HEAD) and reports `stale`. `None` when even the default can't
/// resolve — the caller then shows a plain working-tree list.
pub(crate) fn resolve_diff_base(root: &Path, wt: &Path, picked: Option<&str>) -> Option<ResolvedBase> {
    if let Some(name) = picked.filter(|p| !p.is_empty())
        && let Ok(commit) = super::git_err(wt, &["merge-base", "HEAD", name])
    {
        return Some(ResolvedBase {
            commit: commit.trim().to_string(),
            label: name.to_string(),
            stale: false,
        });
    }
    let commit = super::merge::merge_base(root, wt).ok()?;
    let label = super::git_err(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "HEAD".to_string());
    Some(ResolvedBase { commit, label, stale: picked.is_some_and(|p| !p.is_empty()) })
}

/// The worktree's full delta against `base` as `FileChange` rows: a scratch
/// index is read from `base` then `add -A`'d to the worktree's files, so
/// `diff --cached` covers committed, staged, unstaged and untracked work
/// alike. `Err` when `base` doesn't resolve or git fails — the caller falls
/// back to a plain `collect`.
pub(crate) fn worktree_changes(wt: &Path, base: &str) -> Result<Vec<crate::git::FileChange>, String> {
    let index = crate::checkpoints::temp_index();
    let index_str = index.to_string_lossy().into_owned();
    let env = [("GIT_INDEX_FILE", index_str.as_str())];
    let result: Result<(String, String), String> = (|| {
        crate::git::git_env(wt, &["read-tree", base], &env)?;
        crate::git::git_env(wt, &["add", "-A"], &env)?;
        let names = crate::git::git_env(wt, &["diff", "--cached", "-M", "--name-status", "-z", base], &env)?;
        let numstat = crate::git::git_env(wt, &["diff", "--cached", "-M", "--numstat", "-z", base], &env)?;
        Ok((names, numstat))
    })();
    let _ = std::fs::remove_file(&index);
    let (names, numstat) = result?;
    let mut changes = parse_name_status(&names);
    let counts = crate::git_parse::parse_numstat(&numstat);
    for change in &mut changes {
        if let Some((added, deleted)) = counts.get(&change.path) {
            change.added = *added;
            change.deleted = *deleted;
        }
    }
    Ok(changes)
}

/// Parse `git diff --name-status -z` output — NUL-separated `STATUS PATH`
/// fields; `R`/`C` entries emit the source path between the code and the
/// destination. Every row is the synthetic index's delta against the base,
/// so nothing is "staged".
pub(crate) fn parse_name_status(raw: &str) -> Vec<crate::git::FileChange> {
    let mut fields = raw.split('\0');
    let mut out = Vec::new();
    while let Some(code) = fields.next() {
        if code.is_empty() {
            continue;
        }
        let (path, source) = if code.contains('R') || code.contains('C') {
            let source = fields.next().unwrap_or_default();
            (fields.next().unwrap_or_default(), Some(source).filter(|s| !s.is_empty()).map(str::to_string))
        } else {
            (fields.next().unwrap_or_default(), None)
        };
        if path.is_empty() {
            continue;
        }
        let status = match code.as_bytes()[0] {
            b'A' => crate::git::ChangeStatus::Added,
            b'D' => crate::git::ChangeStatus::Deleted,
            b'R' | b'C' => crate::git::ChangeStatus::Renamed,
            b'U' => crate::git::ChangeStatus::Conflicted,
            _ => crate::git::ChangeStatus::Modified,
        };
        out.push(crate::git::FileChange {
            path: path.to_string(),
            source,
            status,
            staged: false,
            added: 0,
            deleted: 0,
            diff: None,
            diff_load: 0,
        });
    }
    out
}
