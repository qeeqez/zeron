//! Working-tree git changes for the Changes panel — collected by shelling out
//! to `git` in the project root, plus the panel's write actions (stage,
//! unstage, commit, push) and the branch header. The `gh`-backed PR actions
//! (create, status) live in `crate::git::pr`, re-exported here. Output parsing
//! lives in `crate::git_parse` so tests can feed fixtures without a real
//! repository. Line-level diffs for expanded rows live in `crate::changes_diff`.

/// One file's working-tree change, as listed by `git status --porcelain`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    /// Source path of a rename/copy — porcelain `-z` emits it as a second
    /// field; `None` for every other change kind. `diff HEAD` needs both
    /// names to show the rename delta instead of an all-added new file.
    pub source: Option<String>,
    pub status: ChangeStatus,
    pub added: u32,
    pub deleted: u32,
    /// Whether the file has index changes staged for the next commit — the
    /// porcelain `X` column. Untracked and conflicted files report false so
    /// the panel's stage toggle offers `git add` for them.
    pub staged: bool,
    /// Parsed unified diff, loaded on demand when the panel row expands —
    /// `None` means collapsed. Collection never fills this; the workspace
    /// owns no extra per-file state, so the row doubles as the cache.
    pub diff: Option<crate::changes_diff::FileDiff>,
    /// Token of the in-flight diff load, 0 when none. Collapse clears it so
    /// a result that lands late is discarded instead of reopening the row;
    /// a fresh token per expand keeps an older load from attaching after a
    /// collapse+re-expand. UI-only — collection leaves it 0.
    pub diff_load: u64,
}

/// Display status derived from the two-column porcelain code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Conflicted,
}

/// The repo's empty-tree object — the diff base when HEAD is unborn (a fresh
/// repo with no commits), giving the same net worktree delta. Derived via
/// `hash-object` because the well-known SHA-1 id is invalid in SHA-256 repos.
pub(crate) fn empty_tree_id(dir: &std::path::Path) -> Option<String> {
    git(dir, &["hash-object", "-t", "tree", "/dev/null"]).map(|id| id.trim().to_string())
}

/// Collect changes under `dir`: porcelain status for the file list, numstat
/// for line counts, filesystem line count for untracked.
pub(crate) fn collect(dir: &std::path::Path) -> Vec<FileChange> {
    let Some(status) = git(dir, &["status", "--porcelain=v1", "-z", "--untracked-files=all"]) else {
        return Vec::new();
    };
    let mut changes = crate::git_parse::parse_status(&status);
    if changes.is_empty() {
        return changes;
    }
    // Net HEAD→worktree counts: `diff HEAD` covers staged + unstaged in one
    // pass, so a partially-staged file reports its combined delta rather than
    // whichever half was merged last. On an unborn HEAD there is nothing to
    // diff against — fall back to the empty tree.
    let numstat = git(dir, &["diff", "HEAD", "--numstat", "-z"])
        .or_else(|| git(dir, &["diff", &empty_tree_id(dir)?, "--numstat", "-z"]))
        .unwrap_or_default();
    let counts = crate::git_parse::parse_numstat(&numstat);
    for change in &mut changes {
        if let Some((added, deleted)) = counts.get(&change.path) {
            change.added = *added;
            change.deleted = *deleted;
        } else if change.status == ChangeStatus::Added {
            // Untracked files never appear in numstat — count lines on disk.
            change.added = crate::git_parse::line_count(&dir.join(&change.path));
        }
    }
    changes
}

/// Run `git` in `dir`; stdout on success, `None` on spawn failure, non-zero
/// exit (not a repo), or non-UTF-8 output.
pub(crate) fn git(dir: &std::path::Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").args(args).current_dir(dir).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Run `git` in `dir` with extra environment variables; stdout on success,
/// stderr text on failure. Checkpoint plumbing needs this over `git()`:
/// `GIT_INDEX_FILE` redirects index reads/writes to a scratch file so the
/// user's real index is never touched, and `commit-tree` needs a synthetic
/// identity when the repo has none configured.
pub(crate) fn git_env(dir: &std::path::Path, args: &[&str], envs: &[(&str, &str)]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .envs(envs.iter().copied())
        .output()
        .map_err(|e| format!("git {}: {e}", args[0]))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Branch header for the Changes panel — `None` when `dir` isn't a repo, so
/// the panel hides its git actions entirely.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BranchStatus {
    /// Branch name, or the short commit id on a detached HEAD.
    pub name: String,
    /// Upstream ref (`origin/main`) when the branch tracks one.
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}

/// Current branch state under `dir` via `status --porcelain=v2 --branch`.
/// `None` on non-repo dirs — the same probe the panel uses to decide whether
/// the git actions render at all.
pub(crate) fn branch_status(dir: &std::path::Path) -> Option<BranchStatus> {
    let out = git(dir, &["status", "--porcelain=v2", "--branch", "--untracked-files=no"])?;
    Some(parse_branch(&out))
}

/// Parse the `# branch.*` headers of `status --porcelain=v2 --branch`.
/// Detached HEADs report `(detached)` — fall back to the short commit id.
/// An unborn branch reports `(initial)` oid with the real name in `head`.
pub(crate) fn parse_branch(raw: &str) -> BranchStatus {
    let mut status = BranchStatus::default();
    let mut oid = "";
    for line in raw.lines() {
        let Some(rest) = line.strip_prefix("# ") else { continue };
        if let Some(v) = rest.strip_prefix("branch.oid ") {
            oid = v.trim();
        } else if let Some(v) = rest.strip_prefix("branch.head ") {
            status.name = v.trim().to_string();
        } else if let Some(v) = rest.strip_prefix("branch.upstream ") {
            status.upstream = Some(v.trim().to_string());
        } else if let Some(v) = rest.strip_prefix("branch.ab ") {
            // "+<ahead> -<behind>"
            let mut parts = v.split_whitespace();
            status.ahead = parts.next().and_then(|s| s.trim_start_matches('+').parse().ok()).unwrap_or(0);
            status.behind = parts.next().and_then(|s| s.trim_start_matches('-').parse().ok()).unwrap_or(0);
        }
    }
    if status.name.is_empty() || status.name == "(detached)" {
        status.name = oid.get(..8).unwrap_or(oid).to_string();
    }
    status
}

/// One local branch as the picker's list sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    /// Whether this branch is checked out — the picker's check mark.
    pub current: bool,
}

/// Local branches under `dir` via `branch --format` — `%(HEAD)` marks the
/// checked-out one. Empty on non-repo dirs and unborn HEADs; a detached HEAD
/// contributes no entry, so nothing is marked current.
pub(crate) fn list_branches(dir: &std::path::Path) -> Vec<Branch> {
    let Some(out) = git(dir, &["branch", "--format=%(HEAD)%00%(refname:short)"]) else {
        return Vec::new();
    };
    crate::git_parse::parse_branches(&out)
}

/// `git checkout <name>` — switch branches. Git refuses when local edits
/// would be overwritten; that stderr is the error the panel surfaces.
pub(crate) fn checkout(dir: &std::path::Path, name: &str) -> Result<String, String> {
    git_env(dir, &["checkout", name], &[]).map(|_| format!("Switched to {name}"))
}

/// `git checkout -b <name>` — create a branch at HEAD and switch to it.
pub(crate) fn create_branch(dir: &std::path::Path, name: &str) -> Result<String, String> {
    git_env(dir, &["checkout", "-b", name], &[]).map(|_| format!("Created {name}"))
}

/// `git add -- <path>` — stage the file's worktree changes. For a conflicted
/// path this marks the conflict resolved, matching the panel's toggle.
pub(crate) fn stage(dir: &std::path::Path, path: &str) -> Result<String, String> {
    git_env(dir, &["add", "--", path], &[]).map(|_| format!("Staged {path}"))
}

/// Unstage `path`: `restore --staged` on a normal HEAD, `reset` when HEAD is
/// unborn (a fresh repo where `restore` can't resolve the default source).
pub(crate) fn unstage(dir: &std::path::Path, path: &str) -> Result<String, String> {
    git_env(dir, &["restore", "--staged", "--", path], &[])
        .or_else(|_| git_env(dir, &["reset", "-q", "--", path], &[]))
        .map(|_| format!("Unstaged {path}"))
}

/// `git commit -m <message>` — commits whatever is staged.
pub(crate) fn commit(dir: &std::path::Path, message: &str) -> Result<String, String> {
    git_env(dir, &["commit", "-m", message], &[]).map(|_| "Committed".to_string())
}

/// `git commit --amend` — rewrite HEAD in place. `None` keeps the existing
/// message (`--no-edit`); `Some` replaces it (`-m`). Staged changes fold
/// into the commit either way; git's own refusal (unborn HEAD, nothing to
/// amend) lands as the panel's error note.
pub(crate) fn commit_amend(dir: &std::path::Path, message: Option<&str>) -> Result<String, String> {
    let args = match message {
        Some(message) => vec!["commit", "--amend", "-m", message],
        None => vec!["commit", "--amend", "--no-edit"],
    };
    git_env(dir, &args, &[]).map(|_| "Amended".to_string())
}

/// HEAD's subject line (`log -1 --format=%s`) — the commit box's prefill
/// when amend mode turns on. `None` on non-repo dirs and unborn HEADs.
pub(crate) fn last_commit_subject(dir: &std::path::Path) -> Option<String> {
    git(dir, &["log", "-1", "--format=%s"]).map(|s| s.trim().to_string())
}

/// `git revert --no-edit <sha>` — a new commit undoing `sha`, never a
/// history rewrite. Merge commits need `-m` and are refused by git itself;
/// conflicts land as the panel's error note.
pub(crate) fn revert(dir: &std::path::Path, sha: &str) -> Result<String, String> {
    git_env(dir, &["revert", "--no-edit", sha], &[]).map(|_| format!("Reverted {sha}"))
}

/// One stash entry as the Changes panel's stash list sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashEntry {
    /// Reflog selector (`stash@{0}`) — the row label and the argument
    /// pop/apply/drop pass back to git.
    pub name: String,
    /// Stash subject (`%gs`): "WIP on main: …" for an auto message, or
    /// "On main: <msg>" when `stash push -m` supplied one.
    pub message: String,
    /// Relative committer time (`%cr` — "2 hours ago").
    pub rel_time: String,
}

/// Stash entries under `dir`, newest first. Empty on non-repo dirs and when
/// nothing is stashed — `git stash list` prints nothing for an empty reflog.
pub(crate) fn stash_list(dir: &std::path::Path) -> Vec<StashEntry> {
    let out = git(dir, &["stash", "list", "--format=%gd%x00%gs%x00%cr"]);
    out.map(|o| crate::git_parse::parse_stash_list(&o)).unwrap_or_default()
}

/// `git stash push -u -m <message>` — stash tracked and untracked changes,
/// leaving a clean worktree. On a clean tree git exits 0 with "No local
/// changes to save"; that text is the note, not an error.
pub(crate) fn stash_push(dir: &std::path::Path, message: &str) -> Result<String, String> {
    git_env(dir, &["stash", "push", "-u", "-m", message], &[]).map(|out| out.trim().to_string())
}

/// `git stash pop <name>` — apply the stash and drop it on success. A merge
/// conflict exits non-zero, keeps the entry, and its stderr is the note.
pub(crate) fn stash_pop(dir: &std::path::Path, name: &str) -> Result<String, String> {
    git_env(dir, &["stash", "pop", name], &[]).map(|_| format!("Popped {name}"))
}

/// `git stash apply <name>` — apply the stash but keep it in the list.
pub(crate) fn stash_apply(dir: &std::path::Path, name: &str) -> Result<String, String> {
    git_env(dir, &["stash", "apply", name], &[]).map(|_| format!("Applied {name}"))
}

/// `git stash drop <name>` — remove the entry without applying it.
pub(crate) fn stash_drop(dir: &std::path::Path, name: &str) -> Result<String, String> {
    git_env(dir, &["stash", "drop", name], &[]).map(|_| format!("Dropped {name}"))
}

/// One commit as the Changes panel's "Recent commits" list sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    /// Abbreviated hash — shown in the row and passed to `git show`/`revert`.
    pub hash: String,
    /// Subject line (`%s` — always single-line).
    pub subject: String,
    /// Author name (`%an`).
    pub author: String,
    /// Relative committer time (`%ar` — "2 hours ago").
    pub rel_time: String,
    /// The expanded commit diff — `Some` while the row is open.
    pub diff: Option<CommitDiff>,
    /// In-flight diff-load token; see `FileChange::diff_load`.
    pub diff_load: u64,
}

/// One file's patch inside a commit diff — the path plus its parsed hunks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitFileDiff {
    /// The file's post-image path (old path for a deletion).
    pub path: String,
    pub diff: crate::changes_diff::FileDiff,
}

/// The parsed `git show` patch for one commit — one entry per touched file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitDiff {
    pub files: Vec<CommitFileDiff>,
    /// True when the raw output hit the byte or row cap and was cut off.
    pub truncated: bool,
}

/// Most stdout bytes read from `git show` — bounds the subprocess buffer
/// before `parse_commit_diff` applies its own row cap.
const MAX_SHOW_BYTES: u64 = 512 * 1024;

/// The `n` most recent commits under `dir`, newest first. Empty on non-repo
/// dirs and unborn HEADs — `git log` exits non-zero there, same probe the
/// branch header uses to hide the git block.
pub(crate) fn log(dir: &std::path::Path, n: usize) -> Vec<Commit> {
    let n = n.to_string();
    let out = git(dir, &["log", "-n", &n, "--format=%h%x00%s%x00%an%x00%ar"]);
    out.map(|o| crate::git_parse::parse_log(&o)).unwrap_or_default()
}

/// `git show --format= <sha>` — the commit's patch, split per file. `None`
/// when git can't run or `sha` doesn't resolve.
pub(crate) fn commit_diff(dir: &std::path::Path, sha: &str) -> Option<CommitDiff> {
    let (raw, capped) = git_diff(dir, &["show", "--format=", sha], MAX_SHOW_BYTES)?;
    let mut diff = crate::git_parse::parse_commit_diff(&raw);
    diff.truncated |= capped;
    Some(diff)
}

/// `git push`; when the branch has no upstream, `push -u origin HEAD` sets it.
pub(crate) fn push(dir: &std::path::Path) -> Result<String, String> {
    let args: &[&str] = if branch_status(dir).is_some_and(|b| b.upstream.is_some()) {
        &["push"]
    } else {
        &["push", "-u", "origin", "HEAD"]
    };
    git_env(dir, args, &[]).map(|_| "Pushed".to_string())
}

/// `gh`-backed pull-request actions — `create_pr` and `pr_status` — split
/// into `git_pr.rs` for the SLOC cap; re-exported so callers keep using
/// `crate::git::create_pr` / `crate::git::pr_status`.
#[path = "git_pr.rs"]
pub(crate) mod pr;
pub(crate) use pr::{PrChecks, PrState, PrStatus, create_pr, pr_status};

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
