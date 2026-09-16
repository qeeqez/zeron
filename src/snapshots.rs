//! Snapshots surface — the per-turn checkpoints from `crate::checkpoints`
//! browsable as one list across chats. `collect` describes every recorded
//! checkpoint (chat, age, size, how many files restoring it would touch)
//! via `crate::snapshot_store`; `prune` applies the retention policy.
//! Rendering lives in `crate::views::snapshots`.

use std::path::{Path, PathBuf};

use gpui_kit::*;

use crate::checkpoints::{Checkpoint, TurnCheckpoint};
use crate::workspace::Workspace;

/// Default retention when the user hasn't picked one — snapshots older than
/// this are pruned on refresh. `0` (the stored "forever" choice) disables it.
pub(crate) const DEFAULT_RETENTION_DAYS: u32 = 30;

/// One row in the Snapshots panel — a checkpoint plus the metadata the list
/// shows. `changed` counts files `restore` would rewrite or remove relative
/// to the chat's current workdir; `None` when that can't be computed (a
/// gc'd commit, a deleted copy dir, a missing workdir).
#[derive(Clone, Debug)]
pub struct SnapshotInfo {
    /// Owning chat — restore/delete resolve it at action time.
    pub chat_id: u64,
    /// Index of the user message whose turn this checkpoint precedes.
    pub message_ix: usize,
    /// When the turn started — the checkpoint's timestamp.
    pub at: std::time::SystemTime,
    pub chat_title: String,
    /// The chat's working directory — where restore writes and git refs live.
    pub workdir: PathBuf,
    /// Logical size: blob bytes for git snapshots, file bytes for copies.
    pub bytes: u64,
    /// Files restore would change vs the current workdir — see above.
    pub changed: Option<usize>,
    pub checkpoint: Checkpoint,
}

/// Identity for list bookkeeping — a checkpoint is its (chat, message,
/// timestamp) triple; metadata like `changed` is recomputed per refresh.
impl PartialEq for SnapshotInfo {
    fn eq(&self, other: &Self) -> bool {
        (self.chat_id, self.message_ix, self.at) == (other.chat_id, other.message_ix, other.at)
    }
}

/// Snapshots-panel state on `Workspace`: the collected list, a refresh
/// generation (stale collections are discarded), and the retention policy.
/// `Workspace::new` seeds the policy from settings — the derived defaults
/// (keep forever, no cap) only apply before that.
#[derive(Default)]
pub struct SnapshotsState {
    pub open: bool,
    pub list: Vec<SnapshotInfo>,
    /// Bumped per `refresh_snapshots`; a collection stamped older is dropped.
    pub generation: u64,
    /// Auto-prune snapshots older than this many days; `0` = keep forever.
    pub retention_days: u32,
    /// Auto-prune oldest snapshots once the total exceeds this many MiB;
    /// `0` = no cap.
    pub cap_mb: u32,
}

/// The `Send`-able slice of a chat `collect` needs — `Chat` holds `Rc`
/// messages, so it can't cross to the background executor itself.
pub(crate) struct ChatSeed {
    pub(crate) id: u64,
    pub(crate) title: String,
    pub(crate) workdir: PathBuf,
    pub(crate) checkpoints: Vec<TurnCheckpoint>,
}

/// Per-chat seeds for a background `collect` — workdirs resolved on the UI
/// thread so the background side never touches `Chat` or `Project`.
pub(crate) fn seeds(chats: &[crate::model::Chat], root: &Path) -> Vec<ChatSeed> {
    chats
        .iter()
        .filter(|c| !c.ephemeral && !c.checkpoints.is_empty())
        .map(|c| ChatSeed {
            id: c.id,
            title: c.title.to_string(),
            workdir: crate::worktree::workdir_for(c, root),
            checkpoints: c.checkpoints.clone(),
        })
        .collect()
}

/// Describe every checkpoint across `seeds`, newest first.
pub(crate) fn collect(seeds: &[ChatSeed]) -> Vec<SnapshotInfo> {
    let mut list: Vec<SnapshotInfo> = seeds
        .iter()
        .flat_map(|chat| {
            chat.checkpoints.iter().map(|turn| {
                let (bytes, changed) = crate::snapshot_store::describe(&chat.workdir, &turn.checkpoint);
                SnapshotInfo {
                    chat_id: chat.id,
                    message_ix: turn.ix,
                    at: turn.at,
                    chat_title: chat.title.clone(),
                    workdir: chat.workdir.clone(),
                    bytes,
                    changed,
                    checkpoint: turn.checkpoint.clone(),
                }
            })
        })
        .collect();
    list.sort_by_key(|s| std::cmp::Reverse(s.at));
    list
}

/// Pick the snapshots retention removes from a newest-first `list`:
/// everything older than `max_age`, then oldest-first once the survivors
/// exceed `cap_bytes`. `None` disables that rule.
pub(crate) fn prune(list: &[SnapshotInfo], max_age: Option<std::time::Duration>, cap_bytes: Option<u64>) -> Vec<SnapshotInfo> {
    let mut kept = 0u64;
    list.iter()
        .filter(|s| {
            let old = max_age.is_some_and(|age| s.at.elapsed().is_ok_and(|e| e > age));
            let over = cap_bytes.is_some_and(|cap| kept.saturating_add(s.bytes) > cap);
            if !old && !over {
                kept += s.bytes;
            }
            old || over
        })
        .cloned()
        .collect()
}

impl Workspace {
    /// Toggle the Snapshots panel; opening refreshes the list so the first
    /// render never shows stale rows.
    pub fn toggle_snapshots_panel(&mut self, cx: &mut Context<Self>) {
        self.snapshots.open = !self.snapshots.open;
        if self.snapshots.open {
            self.refresh_snapshots(cx);
        }
        cx.notify();
    }

    /// Re-collect snapshot metadata on the background executor — each entry
    /// shells out to git or walks a copy dir, so it can't run on the UI
    /// thread. Lands through `land_snapshots`, which applies retention.
    pub fn refresh_snapshots(&mut self, cx: &mut Context<Self>) {
        self.snapshots.generation += 1;
        let generation = self.snapshots.generation;
        let seeds = seeds(&self.chats, self.project.root());
        cx.spawn(async move |this, cx| {
            let list = cx.background_executor().spawn(async move { collect(&seeds) }).await;
            let _ = this.update(cx, |this, cx| this.land_snapshots(generation, list, cx));
        })
        .detach();
    }

    /// Publish a collected list — skipped when a newer refresh was requested
    /// while this one ran. Retention prunes before the list lands so stale
    /// snapshots never render; a prune that removed anything persists the
    /// shrunken checkpoint lists.
    pub(crate) fn land_snapshots(&mut self, generation: u64, list: Vec<SnapshotInfo>, cx: &mut Context<Self>) {
        if generation != self.snapshots.generation {
            return;
        }
        let victims = prune(&list, retention_age(self.snapshots.retention_days), cap_bytes(self.snapshots.cap_mb));
        let mut removed = false;
        let kept: Vec<SnapshotInfo> = list
            .into_iter()
            .filter(|s| {
                if !victims.contains(s) {
                    return true;
                }
                let dir = if s.workdir.is_dir() { s.workdir.clone() } else { self.project.root().to_path_buf() };
                if crate::snapshot_store::delete(&dir, &s.checkpoint).is_ok() {
                    removed |= self.unpin_checkpoint(s);
                    return false;
                }
                true
            })
            .collect();
        self.snapshots.list = kept;
        if removed {
            self.save();
        }
        cx.notify();
    }

    /// Revert the snapshot's chat workdir to it — the same restore the
    /// per-message "Undo turn" runs, reachable from the panel. The entry
    /// stays listed: snapshots are a history, not a queue.
    pub fn restore_snapshot(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(snap) = self.snapshots.list.get(ix) else { return };
        let checkpoint = snap.checkpoint.clone();
        let workdir = snap.workdir.clone();
        match crate::checkpoints::restore(&workdir, &checkpoint) {
            Ok(()) => {
                if self.changes_panel_open {
                    self.refresh_changes(cx);
                }
                self.refresh_snapshots(cx);
            },
            Err(e) => self.push_note(format!("**Snapshot restore failed:** {e}"), cx),
        }
    }

    /// Drop the snapshot at row `ix`: free its storage, unpin the
    /// checkpoint from its chat (persisted), and refresh the list.
    pub fn delete_snapshot(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(snap) = self.snapshots.list.get(ix) else { return };
        let snap = snap.clone();
        let dir = if snap.workdir.is_dir() { snap.workdir.clone() } else { self.project.root().to_path_buf() };
        match crate::snapshot_store::delete(&dir, &snap.checkpoint) {
            Ok(()) => {
                if self.unpin_checkpoint(&snap) {
                    self.save();
                }
                self.refresh_snapshots(cx);
            },
            Err(e) => self.push_note(format!("**Snapshot delete failed:** {e}"), cx),
        }
    }

    /// Remove `snap`'s `TurnCheckpoint` from its chat — the entry the
    /// per-message "Undo turn" looks up. `false` when nothing matched.
    fn unpin_checkpoint(&mut self, snap: &SnapshotInfo) -> bool {
        let Some(chat) = self.chats.iter_mut().find(|c| c.id == snap.chat_id) else { return false };
        let before = chat.checkpoints.len();
        chat.checkpoints.retain(|c| !(c.ix == snap.message_ix && c.at == snap.at));
        chat.checkpoints.len() != before
    }

    /// Set the age half of the retention policy (days; `0` = forever),
    /// persist it, and re-run collection so the new rule applies now.
    pub fn set_snapshot_retention(&mut self, days: u32, cx: &mut Context<Self>) {
        self.snapshots.retention_days = days;
        self.save_settings();
        self.refresh_snapshots(cx);
    }

    /// Set the size half of the retention policy (MiB; `0` = no cap),
    /// persist it, and re-run collection so the new rule applies now.
    pub fn set_snapshot_cap(&mut self, mb: u32, cx: &mut Context<Self>) {
        self.snapshots.cap_mb = mb;
        self.save_settings();
        self.refresh_snapshots(cx);
    }
}

/// `days` as a `Duration`; `0` disables the age rule.
fn retention_age(days: u32) -> Option<std::time::Duration> {
    (days > 0).then(|| std::time::Duration::from_secs(u64::from(days) * 86_400))
}

/// `mb` as bytes; `0` disables the cap.
fn cap_bytes(mb: u32) -> Option<u64> {
    (mb > 0).then(|| u64::from(mb) * 1024 * 1024)
}
