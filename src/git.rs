//! Working-tree git changes for the Changes panel — collected by shelling out
//! to `git` in the project root, plus the panel's write actions (stage,
//! unstage, commit, push, create-PR) and the branch header. Output parsing
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

/// `git push`; when the branch has no upstream, `push -u origin HEAD` sets it.
pub(crate) fn push(dir: &std::path::Path) -> Result<String, String> {
    let args: &[&str] = if branch_status(dir).is_some_and(|b| b.upstream.is_some()) {
        &["push"]
    } else {
        &["push", "-u", "origin", "HEAD"]
    };
    git_env(dir, args, &[]).map(|_| "Pushed".to_string())
}

/// Push, then open a PR via `gh pr create --fill`. Without `gh` on PATH the
/// push still happens and the note tells the user to open the PR by hand.
/// `envs` lets tests point PATH at a fake `gh`.
pub(crate) fn create_pr(dir: &std::path::Path, envs: &[(&str, &str)]) -> Result<String, String> {
    push(dir)?;
    match gh_pr_create(dir, envs) {
        Ok(url) => Ok(if url.is_empty() { "PR created".to_string() } else { format!("PR created: {url}") }),
        Err(Gh::Missing) => Ok("Pushed — install `gh` to create a PR from here".to_string()),
        Err(Gh::Failed(e)) => Err(e),
    }
}

/// Why `gh pr create` didn't produce a URL: the binary isn't installed, or it
/// ran and failed (no remote, existing PR, not logged in).
enum Gh {
    Missing,
    Failed(String),
}

/// `gh pr create --fill` — title/body from the branch's commits. Stdout is
/// the new PR's URL.
fn gh_pr_create(dir: &std::path::Path, envs: &[(&str, &str)]) -> Result<String, Gh> {
    let out = std::process::Command::new("gh")
        .args(["pr", "create", "--fill"])
        .current_dir(dir)
        .envs(envs.iter().copied())
        .output()
        .map_err(|_| Gh::Missing)?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(Gh::Failed(String::from_utf8_lossy(&out.stderr).trim().to_string()))
    }
}

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
