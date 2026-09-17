//! Update checking: the latest GitHub release for the app's repository
//! (`Cargo.toml`'s `repository`, mirrored by the git remote) compared against
//! `CARGO_PKG_VERSION`. A manual "Check for Updates" reports in a dialog —
//! up-to-date, the release notes with a View Release button, or a muted
//! error line; the automatic daily check stays a toast plus the About row in
//! Profile settings and the About dialog. `update_last_check`,
//! `update_latest` and `update_skip` persist in settings.json so a dismissed
//! release doesn't re-nag and a pending one survives restarts.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// The repository's releases page — the Download target and the fallback
/// URL when a restored pending update has no `html_url` on file.
pub(crate) const RELEASES_PAGE: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/releases");
/// A repository URL minus trailing slash and `.git` suffix — the base every
/// repo page link builds on so the env value's shape can't leak into it.
pub(crate) fn repo_base(repo: &str) -> &str {
    repo.trim_end_matches('/').trim_end_matches(".git")
}
/// `<repo>/<page>` in the browser — the Help menu's targets ("Report an
/// Issue" is `repo_page("issues")`).
pub(crate) fn repo_page(page: &str) -> String {
    format!("{}/{page}", repo_base(env!("CARGO_PKG_REPOSITORY")))
}
/// `"owner/repo"` from `CARGO_PKG_REPOSITORY` — the GitHub API path segment.
/// The repository URL is `https://github.com/<owner>/<repo>` (with or
/// without a `.git` suffix); anything else yields a slug that 404s, which
/// the check reports as "no releases" rather than crashing.
#[cfg(not(test))]
fn repo_slug() -> String {
    let repo = repo_base(env!("CARGO_PKG_REPOSITORY"));
    repo.rsplit_once("github.com/").map(|(_, slug)| slug).unwrap_or(repo).to_string()
}
/// Latest-release endpoint. `latest` already skips drafts and prereleases.
#[cfg(not(test))]
fn latest_release_url() -> String {
    format!("https://api.github.com/repos/{}/releases/latest", repo_slug())
}
/// Minimum time between automatic checks — one fetch per day at most.
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// How often the background loop re-reads `update_last_check`; the check
/// itself stays gated by `CHECK_INTERVAL`, so this only bounds how stale a
/// just-expired check can get.
pub(crate) const POLL_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// What the About row and toasts show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum UpdateStatus {
    /// No check has completed yet (or the last one failed).
    #[default]
    Unknown,
    /// A fetch is in flight.
    Checking,
    /// The latest release is not newer than this build.
    UpToDate,
    /// A newer release exists — the value is its tag (e.g. "v0.2.0").
    Available(String),
}

/// Update state for one window — every workspace window mirrors the same
/// persisted record, so they converge on the next check.
#[derive(Clone, Debug, Default)]
pub(crate) struct UpdateState {
    pub status: UpdateStatus,
    /// Release page URL for the pending update's Download button.
    pub url: String,
    /// The pending release was dismissed — it stays listed but won't
    /// re-notify.
    pub skipped: bool,
}

impl UpdateState {
    /// Seed from settings at workspace open: a pending update from a
    /// previous run shows in About immediately, without waiting for the
    /// next fetch.
    pub(crate) fn restored(s: &crate::persist::Settings) -> Self {
        Self {
            status: if s.update_latest.is_empty() {
                UpdateStatus::Unknown
            } else {
                UpdateStatus::Available(s.update_latest.clone())
            },
            url: RELEASES_PAGE.to_string(),
            skipped: !s.update_latest.is_empty() && s.update_skip == s.update_latest,
        }
    }

    /// Dismiss the pending release — it won't re-notify, and the About row
    /// shows it as skipped until a newer one lands.
    pub(crate) fn skip(&mut self, tag: &str) {
        self.skipped = true;
        persist_update(|s| {
            s.update_skip = tag.to_string();
            s.update_latest = String::new();
        });
    }
}

/// What `apply_release` decided — `land_update` maps it to a toast.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReleaseOutcome {
    /// A newer release the user hasn't skipped.
    Available,
    /// A newer release, but it matches `update_skip`.
    Skipped,
    /// The release is not newer than this build.
    UpToDate,
}

/// Fold a fetched release into `state` and persist the outcome — `None`
/// means the repo has no releases yet, which counts as up-to-date. The
/// notification decision lives here so it's unit-testable without a window.
pub(crate) fn apply_release(state: &mut UpdateState, release: Option<&Release>) -> ReleaseOutcome {
    let newer = release.is_some_and(|r| semver_compare(&r.tag, env!("CARGO_PKG_VERSION")) == Some(std::cmp::Ordering::Greater));
    persist_update(|s| {
        s.update_last_check = Some(SystemTime::now());
        s.update_latest = if newer { release.map(|r| r.tag.clone()).unwrap_or_default() } else { String::new() };
    });
    let Some(release) = release.filter(|_| newer) else {
        state.status = UpdateStatus::UpToDate;
        state.url.clear();
        state.skipped = false;
        return ReleaseOutcome::UpToDate;
    };
    state.status = UpdateStatus::Available(release.tag.clone());
    state.url = release.url.clone();
    state.skipped = crate::persist::load_settings().update_skip == release.tag;
    if state.skipped { ReleaseOutcome::Skipped } else { ReleaseOutcome::Available }
}

/// Write the update fields of settings.json, preserving everything else —
/// the same pattern `window.rs` uses for `window_bounds`.
pub(crate) fn persist_update(f: impl FnOnce(&mut crate::persist::Settings)) {
    let mut s = crate::persist::load_settings();
    f(&mut s);
    crate::persist::save_settings(&s);
}

/// True when the last check is missing or older than `CHECK_INTERVAL` —
/// manual checks bypass this gate entirely.
pub(crate) fn update_due() -> bool {
    crate::persist::load_settings()
        .update_last_check
        .is_none_or(|t| t.elapsed().unwrap_or(CHECK_INTERVAL) >= CHECK_INTERVAL)
}

/// One GitHub release — `tag_name`, its page URL, and the release name and
/// notes the update dialog shows.
#[derive(Clone, Debug)]
pub(crate) struct Release {
    pub tag: String,
    pub url: String,
    /// The release's display name (`name` in the API) — often the tag again.
    pub name: Option<String>,
    /// Release notes (`body` in the API), markdown as published.
    pub notes: Option<String>,
}

/// Where release info comes from — the seam tests fake so no test touches
/// the network. `Ok(None)` means the repo has no releases (the `latest`
/// endpoint 404s) — a successful check, not an error.
pub(crate) trait ReleaseSource: Send + Sync {
    fn latest(&self) -> Result<Option<Release>, String>;
}

/// The real source: GitHub's latest-release API.
#[cfg(not(test))]
struct GitHubReleases;

#[cfg(not(test))]
impl ReleaseSource for GitHubReleases {
    fn latest(&self) -> Result<Option<Release>, String> {
        let mut resp = ureq::get(latest_release_url())
            .config()
            .timeout_global(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            .build()
            .header("User-Agent", concat!("rixlcode/", env!("CARGO_PKG_VERSION")))
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|e| e.to_string())?;
        // A repo with no releases 404s the `latest` endpoint — that's a
        // successful "nothing newer" answer, not a failure.
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(format!("GitHub returned {}", resp.status()));
        }
        let body = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
        parse_release(&body).map(Some)
    }
}

/// Parse the `releases/latest` response body into a `Release` — the seam
/// `update_tests` exercises without a network. Blank `name`/`body` collapse
/// to `None` so the dialog skips them instead of rendering empty lines.
pub(crate) fn parse_release(body: &str) -> Result<Release, String> {
    #[derive(serde::Deserialize)]
    struct Latest {
        tag_name: String,
        html_url: String,
        name: Option<String>,
        body: Option<String>,
    }
    let latest: Latest = serde_json::from_str(body).map_err(|e| format!("unparseable release JSON: {e}"))?;
    let nonblank = |s: Option<String>| s.filter(|s| !s.trim().is_empty());
    Ok(Release {
        tag: latest.tag_name,
        url: latest.html_url,
        name: nonblank(latest.name),
        notes: nonblank(latest.body),
    })
}

/// The first `NOTES_LINES` lines of release notes for the dialog — the full
/// notes stay reachable via View Release.
pub(crate) fn release_notes_excerpt(notes: &str) -> String {
    const NOTES_LINES: usize = 20;
    let mut lines = notes.lines().take(NOTES_LINES + 1);
    let mut excerpt = lines.by_ref().take(NOTES_LINES).collect::<Vec<_>>().join("\n");
    if lines.next().is_some() {
        excerpt.push_str("\n…");
    }
    excerpt
}

/// The active source. Tests install a fake via `set_test_source`; the
/// `NoReleases` stub keeps every other headless test off the network.
pub(crate) fn release_source() -> Arc<dyn ReleaseSource> {
    #[cfg(test)]
    {
        TEST_SOURCE.read().clone().unwrap_or_else(|| Arc::new(NoReleases))
    }
    #[cfg(not(test))]
    {
        Arc::new(GitHubReleases)
    }
}

#[cfg(test)]
static TEST_SOURCE: parking_lot::RwLock<Option<Arc<dyn ReleaseSource>>> = parking_lot::RwLock::new(None);

/// Default test source: behaves like a repo with no releases, so mounting a
/// workspace in a test never reaches for the network.
#[cfg(test)]
struct NoReleases;

#[cfg(test)]
impl ReleaseSource for NoReleases {
    fn latest(&self) -> Result<Option<Release>, String> {
        Ok(None)
    }
}

/// Install the fake release source for a test — call before mounting the
/// workspace so the startup check sees it too.
#[cfg(test)]
pub(crate) fn set_test_source(src: Arc<dyn ReleaseSource>) {
    *TEST_SOURCE.write() = Some(src);
}

/// Semver ordering for release tags: optional `v` prefix, missing
/// minor/patch default to 0, `+build` metadata ignored, and a prerelease
/// sorts before its release (`1.0.0-rc.1` < `1.0.0`). `None` when either
/// side can't be parsed — an unparseable tag never counts as newer.
pub(crate) fn semver_compare(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    Some(Semver::parse(a)?.cmp(&Semver::parse(b)?))
}

/// A parsed semver: numeric core plus dot-separated prerelease identifiers.
#[derive(PartialEq, Eq)]
struct Semver {
    core: [u64; 3],
    pre: Vec<Pre>,
}

/// One prerelease identifier — numeric identifiers compare below
/// alphanumeric ones (semver §11.4).
#[derive(PartialEq, Eq)]
enum Pre {
    Num(u64),
    Str(String),
}

impl Semver {
    fn parse(v: &str) -> Option<Self> {
        let v = v.trim();
        let v = v.strip_prefix(['v', 'V']).unwrap_or(v);
        // Build metadata never affects ordering — drop it before the
        // prerelease split so "1.0.0+build" parses like "1.0.0".
        let v = v.split('+').next()?;
        let (core, pre) = v.split_once('-').unwrap_or((v, ""));
        let mut nums = core.split('.').map(|n| n.parse::<u64>().ok());
        let core = [nums.next().flatten()?, nums.next().flatten().unwrap_or(0), nums.next().flatten().unwrap_or(0)];
        let pre = if pre.is_empty() {
            Vec::new()
        } else {
            pre.split('.').map(Pre::parse).collect::<Option<Vec<_>>>()?
        };
        Some(Self { core, pre })
    }
}

impl Pre {
    /// Numeric identifiers must not have leading zeros (semver §9.1).
    fn parse(s: &str) -> Option<Self> {
        if s.is_empty() {
            return None;
        }
        if s.bytes().all(|b| b.is_ascii_digit()) {
            if s.len() > 1 && s.starts_with('0') {
                return None;
            }
            s.parse().ok().map(Self::Num)
        } else {
            Some(Self::Str(s.to_string()))
        }
    }
}

impl Ord for Semver {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.core.cmp(&other.core).then_with(|| {
            match (self.pre.is_empty(), other.pre.is_empty()) {
                // A bare release outranks any of its prereleases.
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                _ => self.pre.cmp(&other.pre),
            }
        })
    }
}

impl PartialOrd for Semver {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Pre {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (self, other) {
            (Self::Num(a), Self::Num(b)) => a.cmp(b),
            (Self::Num(_), Self::Str(_)) => Ordering::Less,
            (Self::Str(_), Self::Num(_)) => Ordering::Greater,
            (Self::Str(a), Self::Str(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for Pre {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
