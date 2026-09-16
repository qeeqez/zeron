//! `gh`-backed pull-request actions for the Changes panel — `create_pr`
//! (push + `gh pr create --fill`) and `pr_status` (`gh pr view --json` for
//! the current branch). Split from `git.rs` for the SLOC cap; re-exported
//! there so callers keep using `crate::git::create_pr` /
//! `crate::git::pr_status`. JSON parsing lives in `crate::git_parse`.

/// The current branch's pull request as the Changes panel shows it —
/// `#N · state · check rollup`, plus the URL the Open button hands to
/// `cx.open_url`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrStatus {
    pub number: u64,
    pub url: String,
    pub state: PrState,
    pub checks: PrChecks,
}

/// Check-rollup tallies for a PR — completed checks split into pass/fail,
/// everything still running (queued, in progress, waiting) under `pending`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrChecks {
    pub pass: u32,
    pub fail: u32,
    pub pending: u32,
}

/// A PR's lifecycle state as `gh` reports it — anything unrecognized reads
/// as `Open` so the row still renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

impl PrState {
    /// Map `gh`'s `state` field (`OPEN`/`MERGED`/`CLOSED`) — unknown values
    /// fall back to `Open` rather than dropping the row.
    pub(crate) fn of(raw: &str) -> Self {
        match raw.to_ascii_uppercase().as_str() {
            "MERGED" => Self::Merged,
            "CLOSED" => Self::Closed,
            _ => Self::Open,
        }
    }

    /// The row's state label.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Merged => "Merged",
            Self::Closed => "Closed",
        }
    }
}

/// Push, then open a PR via `gh pr create --fill`. Without `gh` on PATH the
/// push still happens and the note tells the user to open the PR by hand.
/// `envs` lets tests point PATH at a fake `gh`.
pub(crate) fn create_pr(dir: &std::path::Path, envs: &[(&str, &str)]) -> Result<String, String> {
    super::push(dir)?;
    match gh(dir, &["pr", "create", "--fill"], envs) {
        Ok(url) => Ok(if url.is_empty() { "PR created".to_string() } else { format!("PR created: {url}") }),
        Err(Gh::Missing) => Ok("Pushed — install `gh` to create a PR from here".to_string()),
        Err(Gh::Failed(e)) => Err(e),
    }
}

/// The current branch's PR: `gh pr view --json number,url,state,
/// statusCheckRollup`. `None` when `gh` isn't installed, the branch has no
/// PR, or `dir` isn't a repo — the panel hides the row in every case.
/// `envs` lets tests point PATH at a fake `gh`.
pub(crate) fn pr_status(dir: &std::path::Path, envs: &[(&str, &str)]) -> Option<PrStatus> {
    let out = gh(dir, &["pr", "view", "--json", "number,url,state,statusCheckRollup"], envs).ok()?;
    crate::git_parse::parse_pr_status(&out)
}

/// Why a `gh` call didn't produce stdout: the binary isn't installed, or it
/// ran and failed (no remote, no PR for the branch, not logged in).
enum Gh {
    Missing,
    Failed(String),
}

/// Run `gh` in `dir`; trimmed stdout on success. `envs` overrides the
/// child's environment — tests point PATH at a stub script.
fn gh(dir: &std::path::Path, args: &[&str], envs: &[(&str, &str)]) -> Result<String, Gh> {
    let out = std::process::Command::new("gh")
        .args(args)
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

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "git_pr_tests.rs"]
mod git_pr_tests;
