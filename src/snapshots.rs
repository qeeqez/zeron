//! Snapshots surface — the per-turn checkpoints from `crate::checkpoints`
//! browsable as one list across chats. `collect` describes every recorded
//! checkpoint (chat, age, size, how many files restoring it would touch)
//! via `crate::snapshot_store`; `prune` applies the retention policy.
//! Rendering lives in `crate::views::snapshots`.

use std::path::{Path, PathBuf};

use crate::checkpoints::{Checkpoint, TurnCheckpoint};

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
    /// The expanded row's file list — computed lazily on expand (or on a
    /// restore click that needs names for the confirm) and cached here.
    pub files: SnapshotFiles,
    /// Whether the row shows its file list. UI-only.
    pub expanded: bool,
}

/// Load state of a row's expanded file list — `Idle` until the first expand
/// asks for it, `Failed` when the diff can't be computed (same cases as a
/// `None` `changed`). `Loaded` survives collapse+re-expand — it's the cache.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum SnapshotFiles {
    #[default]
    Idle,
    Loading,
    Loaded(Vec<crate::snapshot_store::SnapshotFile>),
    Failed,
}

/// Identity for list bookkeeping — a checkpoint is its (chat, message,
/// timestamp) triple; metadata like `changed` is recomputed per refresh.
impl PartialEq for SnapshotInfo {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl SnapshotInfo {
    /// The (chat, message, timestamp) triple `PartialEq` compares — also
    /// how an in-flight file-list load finds its row when it lands.
    pub(crate) fn key(&self) -> (u64, usize, std::time::SystemTime) {
        (self.chat_id, self.message_ix, self.at)
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
                    files: SnapshotFiles::Idle,
                    expanded: false,
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

/// `days` as a `Duration`; `0` disables the age rule.
pub(crate) fn retention_age(days: u32) -> Option<std::time::Duration> {
    (days > 0).then(|| std::time::Duration::from_secs(u64::from(days) * 86_400))
}

/// `mb` as bytes; `0` disables the cap.
pub(crate) fn cap_bytes(mb: u32) -> Option<u64> {
    (mb > 0).then(|| u64::from(mb) * 1024 * 1024)
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[path = "snapshot_ops.rs"]
mod snapshot_ops;
