//! `file_diff` — one path's unified diff for the Changes row's "Copy Diff".
//! Split from `git.rs` for the SLOC cap; re-exported there so callers keep
//! using `crate::git::file_diff`.

use std::fmt::Write as _;
use std::path::Path;

/// Whether `path` has an index entry under `dir` — untracked files have no
/// `git diff` output, so their patch is synthesized instead.
pub(crate) fn tracked(dir: &Path, path: &str) -> bool {
    super::git(dir, &["ls-files", "--error-unmatch", "--", path]).is_some()
}

/// Unified diff for `path` under `dir`: `git diff -- <path>` for worktree
/// changes, `git diff --cached -- <path>` when `staged` — the row's staged
/// marker picks which half a partially-staged file copies. Untracked files
/// have no index entry for git to diff, so their new-file patch is built
/// from the worktree bytes. `Err` when git fails, the file can't be read,
/// or there's no diff to copy.
pub(crate) fn file_diff(dir: &Path, path: &str, staged: bool) -> Result<String, String> {
    file_diff_at(dir, path, staged, None)
}

/// `file_diff` with an explicit base commit — the worktree diff-base mode:
/// `git diff <base> -- <path>` covers the whole delta, so `staged` is
/// ignored. Untracked files still get the synthesized new-file patch.
pub(crate) fn file_diff_at(dir: &Path, path: &str, staged: bool, base: Option<&str>) -> Result<String, String> {
    if !tracked(dir, path) {
        return new_file_diff(dir, path);
    }
    let args: &[&str] = match base {
        Some(base) => &["diff", base, "--", path],
        None if staged => &["diff", "--cached", "--", path],
        None => &["diff", "--", path],
    };
    match super::git_env(dir, args, &[])? {
        diff if diff.is_empty() => Err(format!("no changes in {path}")),
        diff => Ok(diff),
    }
}

/// `git diff` output for a file git doesn't track: a new-file patch built
/// from the worktree contents so "Copy Diff" works before the first
/// `git add`. Binary files get git's one-line "Binary files differ" form —
/// their bytes aren't losslessly copyable as text.
fn new_file_diff(dir: &Path, path: &str) -> Result<String, String> {
    let bytes = std::fs::read(dir.join(path)).map_err(|e| format!("{path}: {e}"))?;
    let mut out = format!("diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n");
    if bytes.contains(&0) {
        let _ = writeln!(out, "Binary files /dev/null and b/{path} differ");
        return Ok(out);
    }
    let text = String::from_utf8_lossy(&bytes);
    let count = text.lines().count();
    if count == 0 {
        return Ok(out);
    }
    let range = if count == 1 { "1".to_string() } else { format!("1,{count}") };
    let _ = writeln!(out, "@@ -0,0 +{range} @@");
    for line in text.lines() {
        let _ = writeln!(out, "+{line}");
    }
    if !text.ends_with('\n') {
        out.push_str("\\ No newline at end of file\n");
    }
    Ok(out)
}

/// Hunk-level staging — `stage_hunk`/`unstage_hunk` apply one hunk of a
/// file's diff to the index — split into `git_hunks.rs` for the SLOC cap;
/// `git.rs` re-exports it as `crate::git::stage_hunk`.
#[path = "git_hunks.rs"]
pub(crate) mod hunks;
/// `git diff` variant with bounded output: reads at most `max_bytes` of
/// stdout, then kills the child rather than buffering an unbounded diff.
/// Returns `(output, hit_cap)`. `None` on spawn failure or an exit code
/// other than 0/1 (1 = differences found, always for `--no-index`) — unless
/// the cap was hit, where the partial output is still the payload.
pub(crate) fn git_diff(dir: &std::path::Path, args: &[&str], max_bytes: u64) -> Option<(String, bool)> {
    use std::io::Read;
    let mut child = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        return None;
    };
    let mut buf = Vec::new();
    if stdout.take(max_bytes + 1).read_to_end(&mut buf).is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    let capped = buf.len() as u64 > max_bytes;
    buf.truncate(max_bytes as usize);
    if capped {
        // The child is likely still writing — kill it instead of waiting on
        // a full pipe.
        let _ = child.kill();
    }
    let code = child.wait().ok()?.code().unwrap_or(-1);
    (capped || code == 0 || code == 1).then(|| (String::from_utf8_lossy(&buf).into_owned(), capped))
}
