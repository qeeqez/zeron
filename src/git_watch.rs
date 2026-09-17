//! Live git state for the Changes panel and the explorer's badges: both
//! render the workspace's `changes` snapshot, which otherwise only moves
//! when something asks for it (panel open, refresh button, a git op). No
//! fs watcher — the workspace's 1s ticker fingerprints the active checkout
//! on the background executor every ~2s while either view is visible, and
//! only a changed fingerprint triggers `refresh_changes`, so an idle tree
//! costs a couple of short-lived git processes and zero UI work.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui_kit::*;

use crate::views::sidebar::SidebarTab;
use crate::workspace::Workspace;

/// Poll cadence in ticker beats — the ticker fires once a second.
pub(crate) const POLL_TICKS: u8 = 2;

/// Min gap between watch-triggered refreshes: a dirty burst (an agent
/// writing many files inside one window) collapses into one reload.
pub(crate) const REFRESH_DEBOUNCE: Duration = Duration::from_millis(500);

/// Poll/refresh bookkeeping for `tick_git_watch`. `Default` is the
/// never-polled state: the first landed fingerprint only sets the baseline
/// (the panel already refreshed on open), it doesn't fire a refresh.
#[derive(Default)]
pub(crate) struct GitWatch {
    /// Last landed fingerprint — `None` until a repo answer has landed.
    last: Option<u64>,
    /// False until the first poll lands — separates "never polled" from
    /// "polled a non-repo", so a repo appearing later counts as a change.
    primed: bool,
    /// A collection is on the background executor — polls skip until it lands.
    in_flight: bool,
    /// Ticks since the last poll started — gates the ~2s cadence.
    since_poll: u8,
    /// When the last refresh ran (manual ones count too — see
    /// `note_refresh`); the debounce gate for watch-triggered reloads.
    last_refresh: Option<Instant>,
}

impl GitWatch {
    /// Back to the never-polled state, armed to poll on the next beat —
    /// called while nothing is watching, so reopening a watching view
    /// re-baselines right away instead of waiting out the cadence.
    pub(crate) fn reset(&mut self) {
        *self = Self { since_poll: POLL_TICKS, ..Self::default() };
    }

    /// One ticker beat: true when a fingerprint collection should start —
    /// every `POLL_TICKS` beats, never while one is still in flight.
    pub(crate) fn due(&mut self) -> bool {
        self.since_poll = self.since_poll.saturating_add(1);
        if self.in_flight || self.since_poll < POLL_TICKS {
            return false;
        }
        self.since_poll = 0;
        self.in_flight = true;
        true
    }

    /// Record a `refresh_changes` triggered outside the watch (panel open,
    /// refresh button, a git op) — a watch refresh landing in the same
    /// instant would just re-collect what that refresh already shows.
    pub(crate) fn note_refresh(&mut self) {
        self.last_refresh = Some(Instant::now());
    }

    /// A landed fingerprint: true when the tree moved and a refresh should
    /// run. `None` means git couldn't answer (not a repo, transient
    /// failure) — it can't prove a change, so it never triggers and never
    /// rewrites the baseline. A change inside the debounce window keeps
    /// the old baseline so the next poll retries (trailing edge) instead
    /// of dropping the refresh.
    pub(crate) fn land(&mut self, fp: Option<u64>, now: Instant) -> bool {
        self.in_flight = false;
        let moved = self.primed && fp.is_some() && self.last != fp;
        if moved && self.last_refresh.is_some_and(|t| now.duration_since(t) < REFRESH_DEBOUNCE) {
            return false;
        }
        if fp.is_some() {
            self.last = fp;
        }
        self.primed = true;
        if moved {
            self.last_refresh = Some(now);
        }
        moved
    }
}

impl Workspace {
    /// One beat of the workspace ticker (`lifecycle::tick`): while the
    /// Changes panel or the Files tab is on screen, fingerprint the active
    /// checkout on the background executor and fire `refresh_changes` when
    /// it moved. With nothing watching, no git processes spawn at all.
    pub(crate) fn tick_git_watch(&mut self, cx: &mut Context<Self>) {
        if !self.git_watching() {
            self.git_watch.reset();
            return;
        }
        if !self.git_watch.due() {
            return;
        }
        let dirs = self.git_watch_dirs();
        cx.spawn(async move |this, cx| {
            let fp = cx.background_executor().spawn(async move { fingerprint(&dirs) }).await;
            let _ = this.update(cx, |this, cx| this.land_git_watch(fp, cx));
        })
        .detach();
    }

    /// A fingerprint landed on the UI thread: refresh only if the tree
    /// moved and the snapshot still has an audience — the view may have
    /// closed while the collection ran, in which case the landing just
    /// updates the baseline.
    fn land_git_watch(&mut self, fp: Option<u64>, cx: &mut Context<Self>) {
        if self.git_watch.land(fp, Instant::now()) && self.git_watching() {
            self.refresh_changes_watched(cx);
        }
    }

    /// Whether any view rendering the `changes` snapshot is on screen —
    /// the Changes panel, or the explorer's git badges on the Files tab.
    fn git_watching(&self) -> bool {
        self.changes_panel_open || (self.sidebar_tab == SidebarTab::Files && !self.sidebar_collapsed)
    }

    /// What to fingerprint: the active chat's checkout (its worktree for
    /// worktree chats — the same dir `refresh_changes` collects) plus the
    /// project root when it differs, since the panel's branch header,
    /// commits and stash rows come from the root.
    fn git_watch_dirs(&self) -> Vec<PathBuf> {
        let root = self.project.root().to_path_buf();
        let chat = &self.chats[self.active];
        let dir = if chat.worktree { crate::worktree::workdir_for(chat, &root) } else { root.clone() };
        if dir == root { vec![root] } else { vec![dir, root] }
    }
}

/// Hash the repo state the Changes panel and explorer badges render:
/// `status --porcelain=v2 --branch` covers staged/unstaged/untracked rows
/// plus HEAD oid and ahead/behind; `refs/stash` catches stash push/pop.
/// The dirs themselves are mixed in so switching to another checkout
/// counts as a change. `None` when no dir is a repo (or git fails).
/// Spawns git processes — background executor only.
pub(crate) fn fingerprint(dirs: &[PathBuf]) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut any = false;
    for dir in dirs {
        let status = crate::git::git(dir, &["status", "--porcelain=v2", "--branch", "--untracked-files=all"]);
        let stash = crate::git::git(dir, &["rev-parse", "--verify", "--quiet", "refs/stash"]);
        any |= status.is_some();
        dir.hash(&mut hasher);
        status.hash(&mut hasher);
        stash.hash(&mut hasher);
    }
    any.then(|| hasher.finish())
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "git_watch_tests.rs"]
mod git_watch_tests;
