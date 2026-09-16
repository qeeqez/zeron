//! The Changes panel's per-file "Discard changes" — the destructive
//! counterpart to stage/unstage. Split from `crate::git` for the SLOC cap;
//! re-exported there so callers keep using `crate::git::discard_file`.

use crate::git::{ChangeStatus, FileChange, git, git_env};

/// Discard one file's changes, always path-scoped — never `checkout .` or a
/// bare `clean`. Untracked files (`Added` + unstaged) are deleted with
/// `clean -f`; everything else is restored from HEAD with
/// `checkout HEAD -- <path>` so a staged file loses its index entry too.
/// When the path isn't in HEAD (staged-new file, rename target, an
/// added-on-both-sides conflict) there is nothing to restore — `rm -f`
/// drops it from index and worktree instead. A rename's source path is then
/// restored as well, or the discard would leave a staged deletion behind.
pub(crate) fn discard_file(dir: &std::path::Path, change: &FileChange) -> Result<String, String> {
    let path = change.path.as_str();
    if change.status == ChangeStatus::Added && !change.staged {
        return git_env(dir, &["clean", "-f", "--", path], &[]).map(|_| format!("Deleted {path}"));
    }
    let in_head = git(dir, &["ls-tree", "HEAD", "--", path]).is_some_and(|out| !out.trim().is_empty());
    let mut result = if in_head {
        git_env(dir, &["checkout", "HEAD", "--", path], &[]).map(|_| format!("Discarded {path}"))
    } else {
        git_env(dir, &["rm", "-f", "--", path], &[]).map(|_| format!("Deleted {path}"))
    };
    if result.is_ok()
        && let Some(source) = change.source.as_deref()
    {
        result = git_env(dir, &["checkout", "HEAD", "--", source], &[]).map(|_| format!("Discarded {path}"));
    }
    result
}
