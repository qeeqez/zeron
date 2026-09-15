//! Update checking: the latest GitHub release for `rixlhq/code` compared
//! against `CARGO_PKG_VERSION`, surfaced as a toast plus the About row in
//! Profile settings and the About dialog. `start_update_check` runs a check
//! at launch and re-checks daily; the "Check for Updates" menu item runs one
//! on demand and always reports the outcome. `update_last_check`,
//! `update_latest` and `update_skip` persist in settings.json so a dismissed
//! release doesn't re-nag and a pending one survives restarts.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui_kit::*;

use crate::workspace::Workspace;

/// The repository's releases page — the Download target and the fallback
/// URL when a restored pending update has no `html_url` on file.
pub(crate) const RELEASES_PAGE: &str = "https://github.com/rixlhq/code/releases";
/// Latest-release endpoint. `latest` already skips drafts and prereleases.
#[cfg(not(test))]
const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/rixlhq/code/releases/latest";
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

/// Fold a fetched release into `state` and persist the outcome. The
/// notification decision lives here so it's unit-testable without a window.
pub(crate) fn apply_release(state: &mut UpdateState, release: &Release) -> ReleaseOutcome {
    let newer = semver_compare(&release.tag, env!("CARGO_PKG_VERSION")) == Some(std::cmp::Ordering::Greater);
    persist_update(|s| {
        s.update_last_check = Some(SystemTime::now());
        s.update_latest = if newer { release.tag.clone() } else { String::new() };
    });
    if !newer {
        state.status = UpdateStatus::UpToDate;
        state.url.clear();
        state.skipped = false;
        return ReleaseOutcome::UpToDate;
    }
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

/// One GitHub release — `tag_name` plus its page URL.
#[derive(Clone, Debug)]
pub(crate) struct Release {
    pub tag: String,
    pub url: String,
}

/// Where release info comes from — the seam tests fake so no test touches
/// the network.
pub(crate) trait ReleaseSource: Send + Sync {
    fn latest(&self) -> Result<Release, String>;
}

/// The real source: GitHub's latest-release API.
#[cfg(not(test))]
struct GitHubReleases;

#[cfg(not(test))]
impl ReleaseSource for GitHubReleases {
    fn latest(&self) -> Result<Release, String> {
        #[derive(serde::Deserialize)]
        struct Latest {
            tag_name: String,
            html_url: String,
        }
        let mut resp = ureq::get(LATEST_RELEASE_URL)
            .config()
            .timeout_global(Some(Duration::from_secs(15)))
            .build()
            .header("User-Agent", concat!("rixlcode/", env!("CARGO_PKG_VERSION")))
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|e| e.to_string())?;
        let latest: Latest = resp.body_mut().read_json().map_err(|e| e.to_string())?;
        Ok(Release { tag: latest.tag_name, url: latest.html_url })
    }
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
    fn latest(&self) -> Result<Release, String> {
        Err("no releases".to_string())
    }
}

/// Install the fake release source for a test — call before mounting the
/// workspace so the startup check sees it too.
#[cfg(test)]
pub(crate) fn set_test_source(src: Arc<dyn ReleaseSource>) {
    *TEST_SOURCE.write() = Some(src);
}

/// The "Check for Updates" menu item: run a manual check in the active
/// window's workspace — or open a window first when none is open, the same
/// fallback `show_about` uses. Deferred because the dispatching window is
/// mid-update.
pub fn check_for_updates(cx: &mut App) {
    cx.defer(|cx| {
        if let Some(handle) = cx.active_window() {
            check_in_window(handle, cx);
            return;
        }
        cx.spawn(async move |cx| {
            let Ok(handle) = crate::lifecycle::open_workspace_window(cx) else { return };
            check_in_window(*handle, cx);
        })
        .detach();
    });
}

/// Run a manual check in `handle`'s workspace — a no-op for a window whose
/// root isn't a `Workspace` (there are none today, but the downcast keeps
/// the menu item safe if that changes).
fn check_in_window<C: AppContext>(handle: AnyWindowHandle, cx: &mut C) {
    let _ = handle.update(cx, |view, _window, cx| {
        // The window's root view is `Root`; the workspace sits inside it.
        let Ok(root) = view.downcast::<gpui_kit::component::Root>() else { return };
        let Some(ws) = root.read(cx).view().clone().downcast::<Workspace>().ok() else { return };
        ws.update(cx, |ws, cx| ws.check_updates(cx));
    });
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
