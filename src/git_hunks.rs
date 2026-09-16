//! Hunk-level staging for the Changes panel — `stage_hunk`/`unstage_hunk`
//! apply a single-hunk patch to the index via `git apply --cached`. Split
//! from `git.rs` for the SLOC cap; re-exported there so callers keep using
//! `crate::git::stage_hunk` / `crate::git::unstage_hunk`.

use std::io::Write as _;
use std::path::Path;

/// Run `git` in `dir` with `input` piped to stdin; stdout on success, stderr
/// text on failure. `git apply` reads the patch from stdin, which keeps a
/// synthesized hunk patch off the filesystem. The write runs on a helper
/// thread so a child that stops reading can't deadlock the caller against a
/// full stderr pipe.
pub(crate) fn git_stdin(dir: &Path, args: &[&str], input: &str) -> Result<String, String> {
    let mut child = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("git {}: {e}", args[0]))?;
    let mut stdin = child.stdin.take();
    let patch = input.to_string();
    let writer = std::thread::spawn(move || {
        if let Some(stdin) = stdin.as_mut() {
            let _ = stdin.write_all(patch.as_bytes());
        }
    });
    let out = child.wait_with_output().map_err(|e| format!("git {}: {e}", args[0]))?;
    let _ = writer.join();
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// `git apply --cached < patch` — stage one hunk of `path`. `patch` is the
/// file's diff header plus the hunk's `@@` block (see
/// `FileDiff::hunk_patch`): the unstaged diff is index-relative, so its
/// context matches the index and applies cleanly even when other hunks are
/// already staged. `--ignore-whitespace` lets context lines match when the
/// diff was rendered with `--ignore-all-space`. Git's own refusal (stale
/// context, corrupt patch) lands as the panel's error note.
pub(crate) fn stage_hunk(dir: &Path, path: &str, patch: &str) -> Result<String, String> {
    git_stdin(dir, &["apply", "--cached", "--ignore-whitespace"], patch).map(|_| format!("Staged hunk in {path}"))
}

/// `git apply --cached --reverse < patch` — unstage one hunk of `path`. The
/// staged diff is HEAD-relative, so reversing it against the index restores
/// that hunk's HEAD content without touching the worktree.
pub(crate) fn unstage_hunk(dir: &Path, path: &str, patch: &str) -> Result<String, String> {
    git_stdin(dir, &["apply", "--cached", "--reverse", "--ignore-whitespace"], patch).map(|_| format!("Unstaged hunk in {path}"))
}
