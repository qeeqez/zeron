//! Parsers for `git` command output — sibling to `git.rs` so both stay under
//! the SLOC cap. Everything here is pure: tests feed fixture output without a
//! real repository.

use crate::git::{Branch, ChangeStatus, Commit, CommitFileDiff, FileChange, PrChecks, PrState, PrStatus, StashEntry};

/// Parse `git status --porcelain=v1 -z` output. Entries are NUL-separated
/// `XY path`; renames/copies append a second field holding the source path.
pub(crate) fn parse_status(raw: &str) -> Vec<FileChange> {
    let mut fields = raw.split('\0');
    let mut out = Vec::new();
    while let Some(field) = fields.next() {
        if field.len() < 4 {
            continue;
        }
        let (code, path) = (&field.as_bytes()[..2], &field[3..]);
        // `!!` ignored entries only appear with --ignored; never a change.
        if code == b"!!" {
            continue;
        }
        let status = status_of(code);
        // Renames/copies append the source path as a second NUL field —
        // consume it whenever the raw code says so, even when the display
        // status masks it (`RD` shows Deleted but still emits the field).
        let source = if code.contains(&b'R') || code.contains(&b'C') {
            fields.next().map(str::to_string).filter(|s| !s.is_empty())
        } else {
            None
        };
        out.push(FileChange {
            path: path.to_string(),
            source,
            status,
            staged: staged_of(code),
            added: 0,
            deleted: 0,
            diff: None,
            diff_load: 0,
        });
    }
    out
}

/// Whether the porcelain `X` column records an index change — the file is
/// (partially) staged. ` `/`?`/`!` mean the index is untouched; unmerged
/// entries (`U`, `AA`, `DD`) count as unstaged so the panel's stage toggle
/// offers `git add` to mark them resolved.
fn staged_of(code: &[u8]) -> bool {
    !matches!(code[0], b' ' | b'?' | b'!') && !unmerged(code)
}

/// Unmerged (conflicted) porcelain codes: any `U`, or the `AA`/`DD` pairs
/// where both sides added or deleted.
fn unmerged(code: &[u8]) -> bool {
    code.contains(&b'U') || code == b"AA" || code == b"DD"
}

/// Map the two-column porcelain code to a display status. Conflict wins over
/// delete, delete over rename, rename over add, add over modify — the most
/// surprising state is the one worth showing.
fn status_of(code: &[u8]) -> ChangeStatus {
    if unmerged(code) {
        ChangeStatus::Conflicted
    } else if code.contains(&b'D') {
        ChangeStatus::Deleted
    } else if code.contains(&b'R') || code.contains(&b'C') {
        ChangeStatus::Renamed
    } else if code.contains(&b'A') || code == b"??" {
        ChangeStatus::Added
    } else {
        ChangeStatus::Modified
    }
}

/// Parse `git diff --numstat -z` output into `path → (added, deleted)`. Binary
/// files report `-` counts and parse as zero. A rename's path field is empty;
/// the following two fields are the old then new path, keyed under the new.
pub(crate) fn parse_numstat(raw: &str) -> std::collections::HashMap<String, (u32, u32)> {
    let mut fields = raw.split('\0');
    let mut out = std::collections::HashMap::new();
    while let Some(field) = fields.next() {
        let mut cols = field.splitn(3, '\t');
        let (Some(added), Some(deleted), Some(path)) = (cols.next(), cols.next(), cols.next()) else {
            continue;
        };
        let path = if path.is_empty() {
            // Rename: skip the old path, take the new one.
            let _old = fields.next();
            fields.next().unwrap_or_default()
        } else {
            path
        };
        if path.is_empty() {
            continue;
        }
        out.insert(path.to_string(), (parse_num(added), parse_num(deleted)));
    }
    out
}

fn parse_num(s: &str) -> u32 {
    s.parse().unwrap_or(0)
}

/// Parse NUL-separated path output (`git diff --name-only -z`, `git ls-files
/// -z`): one path per field, empties dropped.
pub(crate) fn parse_names(raw: &str) -> Vec<String> {
    raw.split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect()
}

/// Parse `git branch --format=%(HEAD)%00%(refname:short)` output: one
/// `*`-or-space flag, a NUL, then the short name per line. A detached HEAD
/// emits a `(HEAD detached …)` pseudo-entry — not a local branch, so it's
/// dropped and nothing is marked current.
pub(crate) fn parse_branches(raw: &str) -> Vec<Branch> {
    raw.lines()
        .filter_map(|line| {
            let (flag, name) = line.split_once('\0')?;
            (!name.is_empty() && !name.starts_with("(HEAD detached")).then(|| Branch { name: name.to_string(), current: flag == "*" })
        })
        .collect()
}

/// Line count for an untracked file — newlines plus a trailing partial line.
/// Binary files (NUL in the first 8 KiB) and unreadable files report zero.
pub(crate) fn line_count(path: &std::path::Path) -> u32 {
    const MAX: u64 = 8 * 1024 * 1024;
    if std::fs::metadata(path).map(|m| m.len() > MAX).unwrap_or(true) {
        return 0;
    }
    let Ok(bytes) = std::fs::read(path) else { return 0 };
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return 0;
    }
    let newlines = bytes.iter().filter(|b| **b == b'\n').count() as u32;
    newlines + u32::from(!bytes.is_empty() && bytes.last() != Some(&b'\n'))
}

/// Parse `git log --format=%h%x00%s%x00%an%x00%ar` output: one commit per
/// line, fields NUL-separated. `%s` is always single-line so newlines stay a
/// safe record separator; malformed lines are skipped.
pub(crate) fn parse_log(raw: &str) -> Vec<Commit> {
    raw.lines()
        .filter_map(|line| {
            let mut f = line.split('\0');
            let hash = f.next()?;
            if hash.is_empty() {
                return None;
            }
            Some(Commit {
                hash: hash.to_string(),
                subject: f.next().unwrap_or_default().to_string(),
                author: f.next().unwrap_or_default().to_string(),
                rel_time: f.next().unwrap_or_default().to_string(),
                diff: None,
                diff_load: 0,
            })
        })
        .collect()
}

/// Parse `git stash list --format=%gd%x00%gs%x00%cr` output: one entry per
/// line, fields NUL-separated. `%gs` is always single-line so newlines stay
/// a safe record separator; malformed lines are skipped.
pub(crate) fn parse_stash_list(raw: &str) -> Vec<StashEntry> {
    raw.lines()
        .filter_map(|line| {
            let mut f = line.split('\0');
            let name = f.next()?;
            if name.is_empty() {
                return None;
            }
            Some(StashEntry {
                name: name.to_string(),
                message: f.next().unwrap_or_default().to_string(),
                rel_time: f.next().unwrap_or_default().to_string(),
            })
        })
        .collect()
}

/// Parse `git show --format=` output into per-file patches. The diff body
/// splits on `diff --git` boundaries — a line starting a new file can never
/// be content, since content lines carry a ` `/`+`/`-` prefix. Each file's
/// hunks go through `changes_diff::parse_diff`; sections with no diffable
/// lines (binary, mode-only) are dropped.
pub(crate) fn parse_commit_diff(raw: &str) -> crate::git::CommitDiff {
    let mut out = crate::git::CommitDiff::default();
    for section in raw.split("\ndiff --git ") {
        let Some(path) = patch_path(section) else { continue };
        let diff = crate::changes_diff::parse_diff(section);
        if diff.lines.is_empty() {
            continue;
        }
        out.truncated |= diff.truncated;
        out.files.push(CommitFileDiff { path, diff });
    }
    out
}

/// The file a `diff --git` section touches: the `+++ b/` post-image name,
/// or the `--- a/` name for a deletion (whose `+++` is `/dev/null`). Falls
/// back to the `diff --git` header's `b/` token for header-only sections.
fn patch_path(section: &str) -> Option<String> {
    let mut deleted = None;
    for line in section.lines() {
        if let Some(p) = line.strip_prefix("+++ ") {
            if p != "/dev/null" {
                return Some(strip_ab(p).to_string());
            }
        } else if let Some(p) = line.strip_prefix("--- ") {
            deleted = Some(strip_ab(p).to_string());
        }
    }
    deleted.or_else(|| section.rsplit_once(" b/").map(|(_, p)| p.trim().trim_matches('"').to_string()))
}

/// Drop the `a/`/`b/` prefix and surrounding quotes from a `---`/`+++` path.
fn strip_ab(path: &str) -> &str {
    let path = path.trim().trim_matches('"');
    path.strip_prefix("a/").or_else(|| path.strip_prefix("b/")).unwrap_or(path)
}

/// Parse `gh pr view --json number,url,state,statusCheckRollup` output into
/// the panel's PR row. `None` on malformed JSON or a missing/empty URL —
/// without one the row's Open button has nothing to launch. The rollup is
/// an array on current `gh`; older versions wrapped it in `{nodes: […]}` —
/// both shapes parse.
pub(crate) fn parse_pr_status(raw: &str) -> Option<PrStatus> {
    let json: serde_json::Value = serde_json::from_str(raw).ok()?;
    let rollup = match &json["statusCheckRollup"] {
        serde_json::Value::Array(checks) => checks.clone(),
        other => other["nodes"].as_array().cloned().unwrap_or_default(),
    };
    let mut checks = PrChecks::default();
    for check in &rollup {
        match check_outcome(check) {
            Outcome::Pass => checks.pass += 1,
            Outcome::Fail => {
                checks.fail += 1;
                // CheckRun entries name under `name`, StatusContext under
                // `context`. The cap keeps a mass failure's tooltip sane.
                if checks.failures.len() < MAX_FAILURE_NAMES
                    && let Some(name) = check_name(check)
                    && !checks.failures.contains(&name)
                {
                    checks.failures.push(name);
                }
            },
            Outcome::Pending => checks.pending += 1,
        }
    }
    Some(PrStatus {
        number: json["number"].as_u64().unwrap_or(0),
        url: json["url"].as_str().filter(|u| !u.is_empty())?.to_string(),
        state: PrState::of(json["state"].as_str().unwrap_or("")),
        checks,
    })
}

/// One rollup entry's verdict. Check runs report `status`+`conclusion`,
/// status contexts a single `state` — whichever is present decides. Queued,
/// in-progress, waiting, and expected checks all count as pending.
enum Outcome {
    Pass,
    Fail,
    Pending,
}

/// Most failing-check names kept for the chip's tooltip — past this the
/// list reads as noise; `detail` reports the overflow as "+N more".
const MAX_FAILURE_NAMES: usize = 8;

/// A rollup entry's display name — `name` on CheckRun, `context` on
/// StatusContext. `None` for entries carrying neither.
fn check_name(check: &serde_json::Value) -> Option<String> {
    check["name"]
        .as_str()
        .or_else(|| check["context"].as_str())
        .filter(|n| !n.is_empty())
        .map(str::to_string)
}

fn check_outcome(check: &serde_json::Value) -> Outcome {
    let conclusion = check["conclusion"].as_str().unwrap_or("").to_ascii_uppercase();
    if matches!(conclusion.as_str(), "SUCCESS" | "NEUTRAL" | "SKIPPED") {
        return Outcome::Pass;
    }
    if matches!(conclusion.as_str(), "FAILURE" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE" | "STALE") {
        return Outcome::Fail;
    }
    match check["state"].as_str().unwrap_or("").to_ascii_uppercase().as_str() {
        "SUCCESS" => Outcome::Pass,
        "FAILURE" | "ERROR" => Outcome::Fail,
        _ => Outcome::Pending,
    }
}
