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

/// `git branch -m <old> <new>` — rename a branch, including the checked-out
/// one. Git refuses when `new` already exists; that stderr is the note.
pub(crate) fn rename_branch(dir: &std::path::Path, old: &str, new: &str) -> Result<String, String> {
    git_env(dir, &["branch", "-m", old, new], &[]).map(|_| format!("Renamed {old} to {new}"))
}

/// `git branch -d <name>` — safe delete: git refuses an unmerged branch or
/// the checked-out one, and that stderr is the note. No force flag — the
/// picker never offers delete on the current branch anyway.
pub(crate) fn delete_branch(dir: &std::path::Path, name: &str) -> Result<String, String> {
    git_env(dir, &["branch", "-d", name], &[]).map(|_| format!("Deleted {name}"))
}

/// `git fetch --prune` — refresh remote refs and drop stale ones. Fetch
/// prints its progress to stderr, so the note is fixed text.
pub(crate) fn fetch(dir: &std::path::Path) -> Result<String, String> {
    git_env(dir, &["fetch", "--prune"], &[]).map(|_| "Fetched".to_string())
}

/// `git pull --ff-only` — fast-forward the current branch; a diverged pull
/// fails and its stderr is the note. The first output line ("Already up to
/// date.", "Updating abc..def") is the note; empty output means pulled.
pub(crate) fn pull_ff(dir: &std::path::Path) -> Result<String, String> {
    git_env(dir, &["pull", "--ff-only"], &[]).map(|out| {
        out.lines()
            .find(|l| !l.trim().is_empty())
            .map(|l| l.trim().to_string())
            .unwrap_or_else(|| "Pulled".to_string())
    })
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
pub(crate) use pr::{CheckVerdict, PrChecks, PrState, PrStatus, create_pr, pr_status};

/// `file_diff` — one path's unified diff for the row menu's "Copy Diff" —
/// split into `git_file_diff.rs` for the SLOC cap; re-exported so callers
/// keep using `crate::git::file_diff` / `crate::git::tracked`. Hunk-level
/// staging lives in `git_hunks.rs` under it for the same reason.
#[path = "git_file_diff.rs"]
pub(crate) mod file_diff;
pub(crate) use file_diff::hunks::{stage_hunk, unstage_hunk};
pub(crate) use file_diff::{file_diff, git_diff, tracked};

/// Per-file "Discard changes" — split into `git_discard.rs` for the SLOC
/// cap; re-exported so callers keep using `crate::git::discard_file`.
#[path = "git_discard.rs"]
pub(crate) mod discard;
pub(crate) use discard::discard_file;

/// Stash list + push/pop/apply/drop — split into `git_stash.rs` for the SLOC
/// cap; re-exported so callers keep using `crate::git::stash_list` etc.
#[path = "git_stash.rs"]
pub(crate) mod stash;
pub(crate) use stash::{StashEntry, stash_apply, stash_drop, stash_list, stash_pop, stash_push};

/// Per-line blame and per-file history for the file menu's "Blame" and
/// "File History" overlays — split into `git_blame.rs` for the SLOC cap;
/// re-exported so callers keep using `crate::git::blame` / `file_log`.
#[path = "git_blame.rs"]
pub(crate) mod blame;
pub(crate) use blame::{BlameLine, blame, commit_file_diff, file_log};
